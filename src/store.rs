use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Context;

use crate::model::{
    Asset, AssetKind, AssetMetadataSource, Id, Portfolio, StoreData, default_tradingview_symbol,
};

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

    pub fn reset(&mut self) {
        self.data = StoreData::default();
    }

    pub fn portfolio_id(&mut self, name: &str) -> Id {
        if let Some(portfolio) = self.data.portfolios.iter().find(|p| p.name == name) {
            return portfolio.id;
        }
        let id = self.data.allocate_id();
        self.data.portfolios.push(Portfolio {
            id,
            name: name.to_string(),
        });
        id
    }

    pub fn asset_id(&mut self, symbol: &str, name: &str, kind: AssetKind) -> Id {
        self.asset_id_with_metadata(symbol, name, kind, None, None, None)
    }

    pub fn asset_id_with_yahoo_symbol(
        &mut self,
        symbol: &str,
        name: &str,
        kind: AssetKind,
        yahoo_symbol: Option<String>,
    ) -> Id {
        self.asset_id_with_metadata(symbol, name, kind, yahoo_symbol, None, None)
    }

    pub fn asset_id_with_metadata(
        &mut self,
        symbol: &str,
        name: &str,
        kind: AssetKind,
        yahoo_symbol: Option<String>,
        tradingview_symbol: Option<String>,
        valuation_currency: Option<String>,
    ) -> Id {
        if let Some(index) = existing_asset_index(&self.data, symbol, kind) {
            let asset = &mut self.data.assets[index];
            update_imported_metadata(asset, yahoo_symbol, tradingview_symbol, valuation_currency);
            return asset.id;
        }
        let id = self.data.allocate_id();
        self.data.assets.push(Asset {
            id,
            symbol: symbol.to_string(),
            name: name.to_string(),
            kind,
            yahoo_symbol,
            tradingview_symbol,
            valuation_currency,
            metadata_source: AssetMetadataSource::Automatic,
        });
        id
    }

    pub fn asset_by_symbol(&self, symbol: &str) -> Option<&Asset> {
        self.data
            .assets
            .iter()
            .find(|asset| asset.symbol.eq_ignore_ascii_case(symbol))
    }
}

fn existing_asset_index(data: &StoreData, symbol: &str, kind: AssetKind) -> Option<usize> {
    data.assets
        .iter()
        .position(|asset| asset.symbol.eq_ignore_ascii_case(symbol) && asset.kind == kind)
        .or_else(|| {
            data.assets.iter().position(|asset| {
                asset.symbol.eq_ignore_ascii_case(symbol)
                    && asset.metadata_source == AssetMetadataSource::User
            })
        })
}

fn update_imported_metadata(
    asset: &mut Asset,
    yahoo_symbol: Option<String>,
    tradingview_symbol: Option<String>,
    valuation_currency: Option<String>,
) {
    if asset.metadata_source == AssetMetadataSource::User {
        return;
    }
    if yahoo_symbol.is_some() {
        asset.yahoo_symbol = yahoo_symbol;
    }
    if tradingview_symbol.is_some() {
        asset.tradingview_symbol = tradingview_symbol;
    }
    if valuation_currency.is_some() {
        asset.valuation_currency = valuation_currency;
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
    let mut data: StoreData = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    migrate_legacy_stock_splits(&mut data);
    migrate_tradingview_symbols(&mut data);
    Ok(data)
}

fn migrate_tradingview_symbols(data: &mut StoreData) {
    for asset in &mut data.assets {
        if asset.tradingview_symbol.is_none()
            && asset.metadata_source == AssetMetadataSource::Automatic
        {
            asset.tradingview_symbol = default_tradingview_symbol(&asset.symbol, asset.kind);
        }
    }
}

fn default_store_path() -> anyhow::Result<PathBuf> {
    let base = dirs::data_local_dir().context("could not determine local data directory")?;
    Ok(base.join("tuifolio").join("store.json"))
}

fn migrate_legacy_stock_splits(data: &mut StoreData) {
    for split in &mut data.config.stock_splits {
        if split.asset_id != 0 {
            continue;
        }
        let Some(symbol) = split.legacy_symbol.as_deref() else {
            continue;
        };
        let matches = data
            .assets
            .iter()
            .filter(|asset| asset.symbol == symbol)
            .collect::<Vec<_>>();
        if matches.len() == 1 {
            split.asset_id = matches[0].id;
        } else {
            tracing::warn!(
                symbol,
                "could not migrate legacy symbol-based stock split to an asset id"
            );
        }
    }
}
