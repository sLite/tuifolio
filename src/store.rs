use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Context;

use crate::model::{Asset, StoreData};

#[derive(Clone)]
pub struct Store {
    path: PathBuf,
    _lock: Arc<fs::File>,
    pub data: StoreData,
}

impl Store {
    pub fn open(path: Option<PathBuf>) -> anyhow::Result<Self> {
        let path = path.unwrap_or(default_store_path()?);
        let lock = lock_store(&path)?;
        let data = load_data(&path)?;
        Ok(Self {
            path,
            _lock: Arc::new(lock),
            data,
        })
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let mut file = tempfile::NamedTempFile::new_in(store_parent(&self.path))?;
        serde_json::to_writer_pretty(&mut file, &self.data)?;
        file.write_all(b"\n")?;
        file.as_file().sync_all()?;
        file.persist(&self.path)
            .with_context(|| format!("failed to replace {}", self.path.display()))?;
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
