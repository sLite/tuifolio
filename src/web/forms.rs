use anyhow::Context;
use chrono::{NaiveDateTime, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;

use super::{edit_revision::record_revision, query::PageQuery};
use crate::{
    model::{AssetKind, Id, LedgerEffect, StoreData, Transaction, TransactionKind},
    transactions::ManualTransactionInput,
};

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub(super) struct TransactionForm {
    pub expected_revision: String,
    pub portfolio_id: String,
    pub timestamp: String,
    pub kind: String,
    pub base_asset_id: String,
    pub base_amount: String,
    pub quote_asset_id: String,
    pub quote_amount: String,
    pub quote_ledger_effect: String,
    pub fee_asset_id: String,
    pub fee_amount: String,
    pub exchange: String,
    pub broker: String,
    pub notes: String,
}

impl TransactionForm {
    pub fn from_transaction(transaction: &Transaction) -> Self {
        Self {
            expected_revision: record_revision(transaction),
            portfolio_id: transaction.portfolio_id.to_string(),
            timestamp: transaction
                .timestamp
                .format("%Y-%m-%dT%H:%M:%S%.3f")
                .to_string(),
            kind: format!("{:?}", transaction.kind),
            base_asset_id: transaction.base_asset_id.to_string(),
            base_amount: transaction.base_amount.to_string(),
            quote_asset_id: id_string(transaction.quote_asset_id),
            quote_amount: amount_string(transaction.quote_amount),
            quote_ledger_effect: format!("{:?}", transaction.quote_ledger_effect),
            fee_asset_id: id_string(transaction.fee_asset_id),
            fee_amount: amount_string(transaction.fee_amount),
            exchange: transaction.exchange.clone().unwrap_or_default(),
            broker: transaction.broker.clone().unwrap_or_default(),
            notes: transaction.notes.clone().unwrap_or_default(),
        }
    }

    pub fn input_for_edit(&self, previous: &Transaction) -> anyhow::Result<ManualTransactionInput> {
        let mut input = self.input()?;
        let displayed_time =
            chrono::DateTime::from_timestamp_millis(previous.timestamp.timestamp_millis());
        if Some(input.timestamp) == displayed_time {
            input.timestamp = previous.timestamp;
        }
        if previous.kind == TransactionKind::StakingReward
            && input.kind == TransactionKind::StakingReward
            && input.quote_asset_id.is_none()
            && input.quote_amount.is_none()
        {
            input.quote_asset_id = previous.quote_asset_id;
        }
        Ok(input)
    }

    pub fn new(data: &StoreData, query: &PageQuery) -> Self {
        let portfolio = data
            .portfolios
            .iter()
            .find(|p| Some(p.id) == query.portfolio)
            .or(data.portfolios.first());
        let asset = data.assets.iter().find(|a| Some(a.id) == query.asset);
        let kind = initial_transaction_kind(asset.map(|asset| asset.kind));
        let quote_asset_id = initial_quote_asset(data, kind);
        Self {
            portfolio_id: portfolio
                .map(|portfolio| portfolio.id.to_string())
                .unwrap_or_default(),
            timestamp: Utc::now().format("%Y-%m-%dT%H:%M").to_string(),
            kind: kind.into(),
            base_asset_id: asset.map(|asset| asset.id.to_string()).unwrap_or_default(),
            quote_asset_id,
            quote_ledger_effect: "Ignore".into(),
            ..Self::default()
        }
    }

    pub fn input(&self) -> anyhow::Result<ManualTransactionInput> {
        let timestamp = parse_timestamp(&self.timestamp)?;
        let kind = enum_value(&self.kind, "transaction type")?;
        Ok(ManualTransactionInput {
            portfolio_id: self
                .portfolio_id
                .trim()
                .parse()
                .context("choose an existing portfolio")?,
            timestamp,
            kind,
            base_asset_id: self
                .base_asset_id
                .trim()
                .parse()
                .context("choose an existing asset")?,
            base_amount: decimal(&self.base_amount, "quantity")?,
            quote_asset_id: optional_asset_id(&self.quote_asset_id, "quote asset")?,
            quote_amount: optional_decimal(&self.quote_amount, "quote amount")?,
            quote_ledger_effect: self.cash_effect(kind)?,
            fee_asset_id: optional_asset_id(&self.fee_asset_id, "fee asset")?,
            fee_amount: optional_decimal(&self.fee_amount, "fee amount")?,
            exchange: optional(&self.exchange),
            broker: optional(&self.broker),
            notes: optional(&self.notes),
        })
    }

    pub fn supports_quote(&self) -> bool {
        enum_value::<TransactionKind>(&self.kind, "transaction type")
            .is_ok_and(TransactionKind::supports_quote)
    }

    pub fn has_implicit_zero_cost_basis(&self) -> bool {
        enum_value::<TransactionKind>(&self.kind, "transaction type")
            .is_ok_and(TransactionKind::has_implicit_zero_cost_basis)
    }

    fn cash_effect(&self, kind: TransactionKind) -> anyhow::Result<LedgerEffect> {
        if !kind.supports_quote() && self.quote_ledger_effect.is_empty() {
            Ok(LedgerEffect::Ignore)
        } else {
            enum_value(&self.quote_ledger_effect, "cash effect")
        }
    }
}

fn parse_timestamp(value: &str) -> anyhow::Result<chrono::DateTime<Utc>> {
    NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f")
        .or_else(|_| NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M"))
        .context("enter a valid date and time")
        .map(|timestamp| timestamp.and_utc())
}

fn id_string(id: Option<Id>) -> String {
    id.map(|id| id.to_string()).unwrap_or_default()
}

fn amount_string(amount: Option<Decimal>) -> String {
    amount.map(|amount| amount.to_string()).unwrap_or_default()
}

fn initial_quote_asset(data: &StoreData, kind: &str) -> String {
    if !matches!(kind, "Buy" | "Sell") {
        return String::new();
    }
    data.assets
        .iter()
        .find(|asset| {
            asset.symbol == data.config.selected_base_currency
                && matches!(asset.kind, AssetKind::Fiat | AssetKind::Crypto)
        })
        .map(|asset| asset.id.to_string())
        .unwrap_or_default()
}

fn initial_transaction_kind(kind: Option<AssetKind>) -> &'static str {
    match kind {
        Some(AssetKind::Fiat) => "Deposit",
        Some(AssetKind::Property | AssetKind::Custom) => "AssetIncrease",
        Some(AssetKind::Liability) => "LiabilityIncrease",
        _ => "Buy",
    }
}

fn optional_asset_id(value: &str, role: &str) -> anyhow::Result<Option<Id>> {
    optional(value)
        .map(|value| {
            value
                .parse()
                .with_context(|| format!("choose an existing {role}"))
        })
        .transpose()
}

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub(super) struct PriceForm {
    pub price: String,
    pub currency: String,
    pub observed_at: String,
}

impl PriceForm {
    pub fn input(&self) -> anyhow::Result<ManualPriceInput> {
        Ok(ManualPriceInput {
            price: decimal(&self.price, "price")?,
            currency: self.currency.trim().to_ascii_uppercase(),
            observed_at: optional(&self.observed_at)
                .map(|value| {
                    NaiveDateTime::parse_from_str(&value, "%Y-%m-%dT%H:%M:%S%.f")
                        .or_else(|_| NaiveDateTime::parse_from_str(&value, "%Y-%m-%dT%H:%M"))
                        .map(|date| date.and_utc())
                        .context("enter a valid price observation time in UTC")
                })
                .transpose()?,
        })
    }
}

pub(super) struct ManualPriceInput {
    pub price: Decimal,
    pub currency: String,
    pub observed_at: Option<chrono::DateTime<Utc>>,
}

#[derive(Deserialize)]
pub(super) struct BaseForm {
    pub currency: String,
    pub return_to: String,
}

fn decimal(value: &str, name: &str) -> anyhow::Result<Decimal> {
    value
        .trim()
        .parse()
        .with_context(|| format!("enter a valid {name}, using a dot as the decimal separator"))
}

fn optional_decimal(value: &str, name: &str) -> anyhow::Result<Option<Decimal>> {
    optional(value)
        .map(|value| decimal(&value, name))
        .transpose()
}

fn optional(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.trim().to_string())
}

pub(super) fn enum_value<T: serde::de::DeserializeOwned>(
    value: &str,
    name: &str,
) -> anyhow::Result<T> {
    serde_json::from_value(serde_json::Value::String(value.into()))
        .with_context(|| format!("select a valid {name}"))
}
