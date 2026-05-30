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
    let mut entries = Vec::new();
    if transaction.base_ledger_effect == LedgerEffect::Post {
        let asset = require_asset(
            assets,
            transaction.base_asset_id,
            transaction.id,
            LedgerRole::Base,
        )?;
        entries.push(LedgerEntry {
            transaction_id: transaction.id,
            portfolio_id: transaction.portfolio_id,
            asset_id: transaction.base_asset_id,
            quantity_delta: split_adjusted_amount(
                data,
                asset.id,
                transaction.timestamp,
                transaction.base_amount,
            )? * base_sign(transaction.kind),
            role: LedgerRole::Base,
        });
    }

    if let (Some(asset_id), Some(amount)) = (transaction.quote_asset_id, transaction.quote_amount)
        && !quote_sign(transaction.kind).is_zero()
        && !amount.is_zero()
        && transaction.quote_ledger_effect == LedgerEffect::Post
    {
        require_asset(assets, asset_id, transaction.id, LedgerRole::Quote)?;
        entries.push(LedgerEntry {
            transaction_id: transaction.id,
            portfolio_id: transaction.portfolio_id,
            asset_id,
            quantity_delta: amount * quote_sign(transaction.kind),
            role: LedgerRole::Quote,
        });
    }

    if let (Some(asset_id), Some(amount)) = (transaction.fee_asset_id, transaction.fee_amount)
        && !amount.is_zero()
    {
        require_asset(assets, asset_id, transaction.id, LedgerRole::Fee)?;
        entries.push(LedgerEntry {
            transaction_id: transaction.id,
            portfolio_id: transaction.portfolio_id,
            asset_id,
            quantity_delta: -amount,
            role: LedgerRole::Fee,
        });
    }

    Ok(entries)
}

fn base_sign(kind: TransactionKind) -> Decimal {
    match kind {
        TransactionKind::Buy
        | TransactionKind::Deposit
        | TransactionKind::AssetIncrease
        | TransactionKind::LiabilityIncrease => Decimal::ONE,
        TransactionKind::Sell
        | TransactionKind::Withdraw
        | TransactionKind::AssetDecrease
        | TransactionKind::LiabilityDecrease => -Decimal::ONE,
    }
}

fn quote_sign(kind: TransactionKind) -> Decimal {
    match kind {
        TransactionKind::Buy => -Decimal::ONE,
        TransactionKind::Sell => Decimal::ONE,
        TransactionKind::Deposit
        | TransactionKind::Withdraw
        | TransactionKind::AssetIncrease
        | TransactionKind::AssetDecrease
        | TransactionKind::LiabilityIncrease
        | TransactionKind::LiabilityDecrease => Decimal::ZERO,
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
            adjusted *= split.numerator / split.denominator;
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
