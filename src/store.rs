use std::{fs, path::PathBuf};

use anyhow::Context;

use crate::model::{Asset, AssetKind, Id, Portfolio, StoreData, default_tradingview_symbol};

pub struct Store {
    path: PathBuf,
    pub data: StoreData,
}

impl Store {
    pub fn open(path: Option<PathBuf>) -> anyhow::Result<Self> {
        let path = path.unwrap_or(default_store_path()?);
        if !path.exists() {
            return Ok(Self {
                path,
                data: StoreData::default(),
            });
        }
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let mut data: StoreData = serde_json::from_str(&content)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        migrate_legacy_stock_splits(&mut data);
        migrate_tradingview_symbols(&mut data);
        Ok(Self { path, data })
    }

    pub fn save(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let content = serde_json::to_string_pretty(&self.data)?;
        fs::write(&self.path, content)
            .with_context(|| format!("failed to write {}", self.path.display()))
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
        if let Some(asset) = self
            .data
            .assets
            .iter_mut()
            .find(|a| a.symbol == symbol && a.kind == kind)
        {
            if yahoo_symbol.is_some() {
                asset.yahoo_symbol = yahoo_symbol;
            }
            if tradingview_symbol.is_some() {
                asset.tradingview_symbol = tradingview_symbol;
            }
            if valuation_currency.is_some() {
                asset.valuation_currency = valuation_currency;
            }
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
        });
        id
    }

    pub fn asset_by_symbol(&self, symbol: &str) -> Option<&Asset> {
        self.data.assets.iter().find(|a| a.symbol == symbol)
    }
}

fn migrate_tradingview_symbols(data: &mut StoreData) {
    for asset in &mut data.assets {
        if asset.tradingview_symbol.is_none() {
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
