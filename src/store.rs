use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Context;

use crate::{
    integrity::validate_store,
    ledger::rebuild_ledger,
    model::{Asset, StoreData},
};

/// Replacement completed. Callers must publish the saved state even though
/// crash durability could not be confirmed.
#[derive(Debug, thiserror::Error)]
#[error(
    "The store was saved, but crash durability could not be confirmed: {source}. Reload the saved record before retrying."
)]
pub struct SaveDurabilityError {
    #[source]
    pub source: std::io::Error,
}

#[derive(Clone)]
pub struct Store {
    path: PathBuf,
    _lock: Arc<fs::File>,
    pub data: StoreData,
}

impl Store {
    pub fn open(path: Option<PathBuf>) -> anyhow::Result<Self> {
        let path = resolve_store_path(path.unwrap_or(default_store_path()?))?;
        let lock = lock_store(&path)?;
        let mut data = load_data(&path)?;
        validate_store(&data).with_context(|| format!("invalid datastore {}", path.display()))?;
        rebuild_ledger(&mut data)
            .with_context(|| format!("failed to rebuild ledger for {}", path.display()))?;
        tracing::debug!(
            entries = data.ledger_entries.len(),
            "datastore ledger rebuilt"
        );
        Ok(Self {
            path,
            _lock: Arc::new(lock),
            data,
        })
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.save_with_directory_sync(|directory| directory.sync_all())
    }

    fn save_with_directory_sync(
        &self,
        sync: impl FnOnce(&fs::File) -> std::io::Result<()>,
    ) -> anyhow::Result<()> {
        validate_store(&self.data)?;
        // Open the directory before replacement, so failure here cannot commit data.
        #[cfg(unix)]
        let directory = Some(fs::File::open(store_parent(&self.path))?);
        #[cfg(not(unix))]
        let directory: Option<fs::File> = None;
        let mut file = tempfile::NamedTempFile::new_in(store_parent(&self.path))?;
        serde_json::to_writer_pretty(&mut file, &self.data)?;
        file.write_all(b"\n")?;
        file.as_file().sync_all()?;
        file.persist(&self.path)
            .with_context(|| format!("failed to replace {}", self.path.display()))?;
        if let Some(directory) = directory {
            sync(&directory).map_err(|source| SaveDurabilityError { source })?;
        } else {
            tracing::warn!(
                "directory synchronization is unavailable on this platform; save durability is not guaranteed"
            );
        }
        tracing::debug!(path = %self.path.display(), "datastore saved");
        Ok(())
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub fn asset_by_symbol(&self, symbol: &str) -> Option<&Asset> {
        self.data
            .assets
            .iter()
            .find(|asset| asset.symbol.eq_ignore_ascii_case(symbol))
    }
}

fn resolve_store_path(path: PathBuf) -> anyhow::Result<PathBuf> {
    fs::create_dir_all(store_parent(&path))?;
    // symlink_metadata also notices dangling symlinks: do not replace one with a new store.
    if fs::symlink_metadata(&path).is_ok() {
        return fs::canonicalize(&path)
            .with_context(|| format!("failed to resolve datastore {}", path.display()));
    }
    let parent = fs::canonicalize(store_parent(&path))?;
    let name = path.file_name().context("datastore path has no filename")?;
    Ok(parent.join(name))
}

fn store_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

fn lock_store(path: &Path) -> anyhow::Result<fs::File> {
    fs::create_dir_all(store_parent(path))?;
    let mut lock_path = path.as_os_str().to_os_string();
    lock_path.push(".lock");
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    lock.try_lock().with_context(|| {
        format!(
            "datastore {} is already in use; close the other Tuifolio process first",
            path.display()
        )
    })?;
    Ok(lock)
}

fn load_data(path: &Path) -> anyhow::Result<StoreData> {
    if !path.exists() {
        return Ok(StoreData::default());
    }
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))
}

fn default_store_path() -> anyhow::Result<PathBuf> {
    let base = dirs::data_local_dir().context("could not determine local data directory")?;
    Ok(base.join("tuifolio").join("store.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn directory_sync_failure_reports_committed_data_without_rolling_it_back() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("store.json");
        let mut store = Store::open(Some(path.clone())).unwrap();
        crate::portfolios::create_portfolio(
            &mut store,
            crate::portfolios::PortfolioInput {
                name: "Saved".into(),
            },
        )
        .unwrap();
        let error = store
            .save_with_directory_sync(|_| {
                let saved: StoreData = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                assert_eq!(saved.portfolios[0].name, "Saved");
                Err(std::io::Error::other("injected directory-sync failure"))
            })
            .unwrap_err();
        assert!(error.is::<SaveDurabilityError>());
        assert!(error.to_string().contains("store was saved"));
        drop(store);
        assert_eq!(
            Store::open(Some(path)).unwrap().data.portfolios[0].name,
            "Saved"
        );
    }

    #[test]
    fn invalid_candidate_cannot_replace_existing_file_or_reach_directory_sync() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("store.json");
        let mut store = Store::open(Some(path.clone())).unwrap();
        store.save().unwrap();
        let original = fs::read(&path).unwrap();
        store.data.next_id = 0;
        assert!(
            store
                .save_with_directory_sync(|_| panic!("validation must fail before saving"))
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), original);
    }
}
