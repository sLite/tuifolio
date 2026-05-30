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
    transactions.sort_by_key(|transaction| transaction.id);

    for transaction in transactions {
        entries.extend(transaction_entries(transaction, &entries, &assets, data)?);
    }

    Ok(entries)
}

fn transaction_entries(
    transaction: &Transaction,
    existing_entries: &[LedgerEntry],
    assets: &HashMap<Id, &Asset>,
    data: &StoreData,
) -> anyhow::Result<Vec<LedgerEntry>> {
    let mut entries = Vec::new();
    let base_asset = assets.get(&transaction.base_asset_id).copied();
    if let Some(asset) = base_asset
        && transaction.base_ledger_effect == LedgerEffect::Post
    {
        entries.push(LedgerEntry {
            transaction_id: transaction.id,
            portfolio_id: transaction.portfolio_id,
            asset_id: transaction.base_asset_id,
            quantity_delta: split_adjusted_amount(
                data,
                &asset.symbol,
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
        && let Some(_asset) = assets.get(&asset_id).copied()
    {
        let amount = match transaction.kind {
            TransactionKind::Buy => {
                spendable_amount(existing_entries, transaction.portfolio_id, asset_id, amount)
            }
            TransactionKind::Sell => amount,
            TransactionKind::Deposit
            | TransactionKind::Withdraw
            | TransactionKind::AssetIncrease
            | TransactionKind::AssetDecrease
            | TransactionKind::LiabilityIncrease
            | TransactionKind::LiabilityDecrease => Decimal::ZERO,
        };
        if !amount.is_zero() {
            entries.push(LedgerEntry {
                transaction_id: transaction.id,
                portfolio_id: transaction.portfolio_id,
                asset_id,
                quantity_delta: amount * quote_sign(transaction.kind),
                role: LedgerRole::Quote,
            });
        }
    }

    if let (Some(asset_id), Some(amount)) = (transaction.fee_asset_id, transaction.fee_amount)
        && !amount.is_zero()
        && let Some(_asset) = assets.get(&asset_id).copied()
    {
        let amount = spendable_amount(existing_entries, transaction.portfolio_id, asset_id, amount);
        if !amount.is_zero() {
            entries.push(LedgerEntry {
                transaction_id: transaction.id,
                portfolio_id: transaction.portfolio_id,
                asset_id,
                quantity_delta: -amount,
                role: LedgerRole::Fee,
            });
        }
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

fn spendable_amount(
    entries: &[LedgerEntry],
    portfolio_id: Id,
    asset_id: Id,
    requested: Decimal,
) -> Decimal {
    let balance = entries
        .iter()
        .filter(|entry| entry.portfolio_id == portfolio_id && entry.asset_id == asset_id)
        .map(|entry| entry.quantity_delta)
        .sum::<Decimal>();
    if balance <= Decimal::ZERO {
        Decimal::ZERO
    } else {
        requested.min(balance)
    }
}

fn split_adjusted_amount(
    data: &StoreData,
    symbol: &str,
    timestamp: DateTime<Utc>,
    amount: Decimal,
) -> anyhow::Result<Decimal> {
    let mut adjusted = amount;
    let transaction_date = timestamp.date_naive();
    for split in &data.config.stock_splits {
        if split.symbol != symbol {
            continue;
        }
        let effective_date = NaiveDate::parse_from_str(&split.effective_date, "%Y-%m-%d")?;
        if transaction_date < effective_date {
            adjusted *= split.numerator / split.denominator;
        }
    }
    Ok(adjusted)
}
