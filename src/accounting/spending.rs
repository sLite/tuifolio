use std::collections::HashMap;

use rust_decimal::Decimal;

use crate::model::{Asset, Id, StoreData, Transaction};

use super::{currency_identity, prices_at, value_in_base};

enum RecordedSpendingChange {
    Unrecorded,
    Recorded(Decimal),
}

pub(super) fn recorded_net_spending(
    data: &StoreData,
    portfolio_id: Id,
    asset_id: Id,
    base: &str,
    assets: &HashMap<Id, &Asset>,
) -> Option<Decimal> {
    let mut spending = Decimal::ZERO;
    let mut seen = false;
    for transaction in &data.transactions {
        if transaction.portfolio_id != portfolio_id || transaction.base_asset_id != asset_id {
            continue;
        }
        if let RecordedSpendingChange::Recorded(amount) =
            recorded_spending_change(data, transaction, base, assets)?
        {
            spending += amount;
            seen = true;
        }
    }
    seen.then_some(spending)
}

fn recorded_spending_change(
    data: &StoreData,
    transaction: &Transaction,
    base: &str,
    assets: &HashMap<Id, &Asset>,
) -> Option<RecordedSpendingChange> {
    if transaction.kind.has_implicit_zero_cost_basis() {
        return Some(RecordedSpendingChange::Recorded(Decimal::ZERO));
    }
    if !transaction.kind.supports_quote()
        || transaction.quote_asset_id.is_none()
        || transaction.quote_amount.is_none()
    {
        return Some(RecordedSpendingChange::Unrecorded);
    }
    let value = transaction_quote_value(data, transaction, base, assets)?;
    Some(RecordedSpendingChange::Recorded(
        -transaction.kind.quote_cash_delta(value),
    ))
}

fn transaction_quote_value(
    data: &StoreData,
    transaction: &Transaction,
    base: &str,
    assets: &HashMap<Id, &Asset>,
) -> Option<Decimal> {
    let amount = transaction.quote_amount?;
    let asset = assets.get(&transaction.quote_asset_id?)?;
    if amount.is_zero() {
        return Some(Decimal::ZERO);
    }
    let amount =
        crate::ledger::split_adjusted_amount(data, asset.id, transaction.timestamp, amount).ok()?;
    if currency_identity(data, asset, base) {
        return Some(amount);
    }
    let primary = assets.get(&transaction.base_asset_id)?;
    if currency_identity(data, primary, base) && transaction.base_amount > Decimal::ZERO {
        return primary_execution_value(data, transaction);
    }
    let historical_prices = prices_at(data, transaction.timestamp);
    value_in_base(data, asset, amount, base, &historical_prices, assets)
}

// A trade into the reporting currency supplies its own execution value,
// without requiring an unrelated historical market observation.
fn primary_execution_value(data: &StoreData, transaction: &Transaction) -> Option<Decimal> {
    crate::ledger::split_adjusted_amount(
        data,
        transaction.base_asset_id,
        transaction.timestamp,
        transaction.base_amount,
    )
    .ok()
}
