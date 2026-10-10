use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

pub type Id = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    pub fn supports_intrinsic_valuation(self) -> bool {
        matches!(self, Self::Property | Self::Liability | Self::Custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TransactionKind {
    Buy,
    Sell,
    Gift,
    StakingReward,
    Deposit,
    Withdraw,
    AssetIncrease,
    AssetDecrease,
    LiabilityIncrease,
    LiabilityDecrease,
}

impl TransactionKind {
    pub fn has_implicit_zero_cost_basis(self) -> bool {
        self == Self::Gift
    }

    pub fn supports_quote(self) -> bool {
        matches!(
            self,
            Self::Buy | Self::Sell | Self::AssetIncrease | Self::AssetDecrease
        )
    }

    pub fn requires_quote(self) -> bool {
        matches!(self, Self::Buy | Self::Sell)
    }

    pub fn quote_cash_delta(self, amount: Decimal) -> Decimal {
        match self {
            Self::Buy | Self::AssetIncrease => -amount,
            Self::Sell | Self::AssetDecrease => amount,
            Self::Gift
            | Self::StakingReward
            | Self::Deposit
            | Self::Withdraw
            | Self::LiabilityIncrease
            | Self::LiabilityDecrease => Decimal::ZERO,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Portfolio {
    pub id: Id,
    pub name: String,
}

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct Asset {
    pub id: Id,
    pub symbol: String,
    pub name: String,
    pub kind: AssetKind,
    #[serde(default)]
    pub yahoo_symbol: Option<String>,
    #[serde(default)]
    pub tradingview_symbol: Option<String>,
    #[serde(default)]
    pub valuation_currency: Option<String>,
    #[serde(default)]
    pub metadata_source: AssetMetadataSource,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AssetMetadataSource {
    #[default]
    Automatic,
    User,
}

impl Asset {
    pub fn allows_manual_pricing(&self) -> bool {
        self.yahoo_symbol
            .as_deref()
            .is_none_or(|symbol| symbol.trim().is_empty())
    }

    pub fn yahoo_quote_symbol(&self) -> Option<&str> {
        match self.kind {
            AssetKind::Crypto | AssetKind::Stock | AssetKind::Fund | AssetKind::Commodity => {
                self.yahoo_symbol.as_deref()
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transaction {
    pub id: Id,
    pub portfolio_id: Id,
    pub timestamp: DateTime<Utc>,
    pub kind: TransactionKind,
    pub base_asset_id: Id,
    #[serde(with = "rust_decimal::serde::str")]
    pub base_amount: Decimal,
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    #[serde(default)]
    pub stock_splits: Vec<StockSplit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StockSplit {
    pub asset_id: Id,
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
            stock_splits: Vec::new(),
        }
    }
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
