use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

pub type Id = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetKind {
    Fiat,
    Crypto,
    Stock,
    Fund,
    Commodity,
    Property,
    Custom,
    Liability,
}

impl AssetKind {
    pub fn from_delta(value: &str) -> Self {
        match value {
            "FIAT" => Self::Fiat,
            "CRYPTO" => Self::Crypto,
            "STOCK" => Self::Stock,
            "FUND" => Self::Fund,
            "COMMODITY" => Self::Commodity,
            _ => Self::Custom,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransactionKind {
    Buy,
    Sell,
    Deposit,
    Withdraw,
    AssetIncrease,
    AssetDecrease,
    LiabilityIncrease,
    LiabilityDecrease,
}

impl TransactionKind {
    pub fn from_delta(value: &str) -> anyhow::Result<Self> {
        match value {
            "BUY" => Ok(Self::Buy),
            "SELL" => Ok(Self::Sell),
            "DEPOSIT" => Ok(Self::Deposit),
            "WITHDRAW" => Ok(Self::Withdraw),
            other => anyhow::bail!("unsupported Delta transaction kind: {other}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Portfolio {
    pub id: Id,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub id: Id,
    pub symbol: String,
    pub name: String,
    pub kind: AssetKind,
    #[serde(default)]
    pub yahoo_symbol: Option<String>,
    #[serde(default)]
    pub valuation_currency: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub id: Id,
    pub portfolio_id: Id,
    pub timestamp: DateTime<Utc>,
    pub kind: TransactionKind,
    pub base_asset_id: Id,
    #[serde(with = "rust_decimal::serde::str")]
    pub base_amount: Decimal,
    #[serde(default)]
    pub base_ledger_effect: LedgerEffect,
    pub quote_asset_id: Option<Id>,
    #[serde(with = "rust_decimal::serde::str_option")]
    pub quote_amount: Option<Decimal>,
    #[serde(default)]
    pub quote_ledger_effect: LedgerEffect,
    pub fee_asset_id: Option<Id>,
    #[serde(with = "rust_decimal::serde::str_option")]
    pub fee_amount: Option<Decimal>,
    pub exchange: Option<String>,
    pub broker: Option<String>,
    pub notes: Option<String>,
    pub source: String,
    pub source_row_hash: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LedgerEffect {
    #[default]
    Post,
    Ignore,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub transaction_id: Id,
    pub portfolio_id: Id,
    pub asset_id: Id,
    #[serde(with = "rust_decimal::serde::str")]
    pub quantity_delta: Decimal,
    pub role: LedgerRole,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum LedgerRole {
    Base,
    Quote,
    Fee,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Price {
    pub asset_id: Id,
    pub timestamp: DateTime<Utc>,
    #[serde(with = "rust_decimal::serde::str")]
    pub price: Decimal,
    pub currency: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub base_currencies: Vec<String>,
    pub default_base_currency: String,
    pub selected_base_currency: String,
    #[serde(default = "default_stock_splits")]
    pub stock_splits: Vec<StockSplit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StockSplit {
    pub symbol: String,
    pub effective_date: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub numerator: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub denominator: Decimal,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            base_currencies: vec!["EUR".into(), "USD".into(), "BTC".into(), "ETH".into()],
            default_base_currency: "EUR".into(),
            selected_base_currency: "EUR".into(),
            stock_splits: default_stock_splits(),
        }
    }
}

fn default_stock_splits() -> Vec<StockSplit> {
    vec![StockSplit {
        symbol: "GME".into(),
        effective_date: "2022-07-22".into(),
        numerator: Decimal::new(4, 0),
        denominator: Decimal::ONE,
    }]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreData {
    pub config: Config,
    pub next_id: Id,
    pub portfolios: Vec<Portfolio>,
    pub assets: Vec<Asset>,
    pub transactions: Vec<Transaction>,
    pub ledger_entries: Vec<LedgerEntry>,
    pub prices: Vec<Price>,
    pub raw_rows: BTreeMap<String, String>,
}

impl Default for StoreData {
    fn default() -> Self {
        Self {
            config: Config::default(),
            next_id: 1,
            portfolios: Vec::new(),
            assets: Vec::new(),
            transactions: Vec::new(),
            ledger_entries: Vec::new(),
            prices: Vec::new(),
            raw_rows: BTreeMap::new(),
        }
    }
}

impl StoreData {
    pub fn allocate_id(&mut self) -> Id {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

pub fn split_delta_asset(label: &str) -> (String, String) {
    let trimmed = label.trim();
    if let Some((symbol, rest)) = trimmed.split_once(" (") {
        let name = rest.strip_suffix(')').unwrap_or(rest);
        (
            symbol.trim().trim_end_matches('*').to_string(),
            name.to_string(),
        )
    } else {
        (trimmed.to_string(), trimmed.to_string())
    }
}
