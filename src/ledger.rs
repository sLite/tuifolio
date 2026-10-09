use std::collections::HashMap;

use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;

use crate::model::{
    Asset, Id, LedgerEffect, LedgerEntry, LedgerRole, StoreData, Transaction, TransactionKind,
};

pub fn rebuild_ledger(data: &mut StoreData) -> anyhow::Result<()> {
    data.ledger_entries = build_ledger(data)?;
    Ok(())
}

pub fn build_ledger(data: &StoreData) -> anyhow::Result<Vec<LedgerEntry>> {
    let assets = data
        .assets
        .iter()
        .map(|asset| (asset.id, asset))
        .collect::<HashMap<_, _>>();
    let mut entries = Vec::new();
    let mut transactions = data.transactions.iter().collect::<Vec<_>>();
    transactions.sort_by_key(|transaction| (transaction.timestamp, transaction.id));

    for transaction in transactions {
        entries.extend(transaction_entries(transaction, &assets, data)?);
    }

    Ok(entries)
}

fn transaction_entries(
    transaction: &Transaction,
    assets: &HashMap<Id, &Asset>,
    data: &StoreData,
) -> anyhow::Result<Vec<LedgerEntry>> {
    let mut entries = vec![base_entry(transaction, assets, data)?];
    if let Some(entry) = quote_entry(transaction, assets)? {
        entries.push(entry);
    }
    if let Some(entry) = fee_entry(transaction, assets)? {
        entries.push(entry);
    }
    Ok(entries)
}

fn base_entry(
    transaction: &Transaction,
    assets: &HashMap<Id, &Asset>,
    data: &StoreData,
) -> anyhow::Result<LedgerEntry> {
    let asset = require_asset(
        assets,
        transaction.base_asset_id,
        transaction.id,
        LedgerRole::Base,
    )?;
    Ok(LedgerEntry {
        transaction_id: transaction.id,
        portfolio_id: transaction.portfolio_id,
        asset_id: asset.id,
        quantity_delta: split_adjusted_amount(
            data,
            asset.id,
            transaction.timestamp,
            transaction.base_amount,
        )? * base_sign(transaction.kind),
        role: LedgerRole::Base,
    })
}

fn quote_entry(
    transaction: &Transaction,
    assets: &HashMap<Id, &Asset>,
) -> anyhow::Result<Option<LedgerEntry>> {
    let (Some(asset_id), Some(amount)) = (transaction.quote_asset_id, transaction.quote_amount)
    else {
        return Ok(None);
    };
    let cash_delta = transaction.kind.quote_cash_delta(amount);
    if cash_delta.is_zero() || transaction.quote_ledger_effect == LedgerEffect::Ignore {
        return Ok(None);
    }
    require_asset(assets, asset_id, transaction.id, LedgerRole::Quote)?;
    Ok(Some(LedgerEntry {
        transaction_id: transaction.id,
        portfolio_id: transaction.portfolio_id,
        asset_id,
        quantity_delta: cash_delta,
        role: LedgerRole::Quote,
    }))
}

fn fee_entry(
    transaction: &Transaction,
    assets: &HashMap<Id, &Asset>,
) -> anyhow::Result<Option<LedgerEntry>> {
    let (Some(asset_id), Some(amount)) = (transaction.fee_asset_id, transaction.fee_amount) else {
        return Ok(None);
    };
    if amount.is_zero() {
        return Ok(None);
    }
    require_asset(assets, asset_id, transaction.id, LedgerRole::Fee)?;
    Ok(Some(LedgerEntry {
        transaction_id: transaction.id,
        portfolio_id: transaction.portfolio_id,
        asset_id,
        quantity_delta: -amount,
        role: LedgerRole::Fee,
    }))
}

fn base_sign(kind: TransactionKind) -> Decimal {
    match kind {
        TransactionKind::Buy
        | TransactionKind::Gift
        | TransactionKind::Deposit
        | TransactionKind::AssetIncrease
        | TransactionKind::LiabilityIncrease => Decimal::ONE,
        TransactionKind::Sell
        | TransactionKind::Withdraw
        | TransactionKind::AssetDecrease
        | TransactionKind::LiabilityDecrease => -Decimal::ONE,
    }
}

fn split_adjusted_amount(
    data: &StoreData,
    asset_id: Id,
    timestamp: DateTime<Utc>,
    amount: Decimal,
) -> anyhow::Result<Decimal> {
    let mut adjusted = amount;
    let transaction_date = timestamp.date_naive();
    for split in &data.config.stock_splits {
        if split.asset_id != asset_id {
            continue;
        }
        let effective_date = NaiveDate::parse_from_str(&split.effective_date, "%Y-%m-%d")?;
        if transaction_date < effective_date {
            adjusted = split
                .numerator
                .checked_div(split.denominator)
                .and_then(|ratio| adjusted.checked_mul(ratio))
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "split-adjusted quantity is outside the supported decimal range"
                    )
                })?;
        }
    }
    Ok(adjusted)
}

fn require_asset<'a>(
    assets: &'a HashMap<Id, &Asset>,
    asset_id: Id,
    transaction_id: Id,
    role: LedgerRole,
) -> anyhow::Result<&'a Asset> {
    assets.get(&asset_id).copied().ok_or_else(|| {
        anyhow::anyhow!("transaction {transaction_id} references missing {role:?} asset {asset_id}")
    })
}
