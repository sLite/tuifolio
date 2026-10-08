mod support;

use rust_decimal::Decimal;
use support::temp_store;
use tuifolio::{
    ledger::rebuild_ledger,
    model::{AssetKind, Id, LedgerEffect, StockSplit, Transaction, TransactionKind},
    store::Store,
};

fn transaction(
    store: &mut Store,
    asset_id: Id,
    kind: TransactionKind,
    amount: Decimal,
) -> Transaction {
    let portfolio_id = store.portfolio_id("Manual");
    let id = store.data.allocate_id();
    Transaction {
        id,
        portfolio_id,
        timestamp: chrono::Utc::now(),
        kind,
        base_asset_id: asset_id,
        base_amount: amount,
        base_ledger_effect: LedgerEffect::Post,
        quote_asset_id: None,
        quote_amount: None,
        quote_ledger_effect: LedgerEffect::Post,
        fee_asset_id: None,
        fee_amount: None,
        exchange: None,
        broker: None,
        notes: None,
        source: "manual".into(),
        source_row_hash: format!("manual:{id}"),
    }
}

fn balance(store: &Store, asset_id: Id) -> Decimal {
    store
        .data
        .ledger_entries
        .iter()
        .filter(|entry| entry.asset_id == asset_id)
        .map(|entry| entry.quantity_delta)
        .sum()
}

#[test]
fn ledger_posts_quote_amounts_even_when_cash_goes_negative() {
    let mut store = temp_store();
    let stock_id = store.asset_id("ABC", "ABC", AssetKind::Stock);
    let usd_id = store.asset_id("USD", "USD", AssetKind::Fiat);
    let mut transaction = transaction(&mut store, stock_id, TransactionKind::Buy, Decimal::ONE);
    transaction.quote_asset_id = Some(usd_id);
    transaction.quote_amount = Some(Decimal::from(100));
    store.data.transactions.push(transaction);
    rebuild_ledger(&mut store.data).unwrap();
    assert_eq!(balance(&store, usd_id), Decimal::from(-100));
}

#[test]
fn explicit_split_events_adjust_only_the_target_asset() {
    let mut store = temp_store();
    let split_id = store.asset_id("ABC", "ABC", AssetKind::Stock);
    let other_id = store.asset_id("XYZ", "XYZ", AssetKind::Stock);
    store.data.config.stock_splits.push(StockSplit {
        asset_id: split_id,
        legacy_symbol: None,
        effective_date: "2024-01-01".into(),
        numerator: Decimal::from(2),
        denominator: Decimal::ONE,
    });
    for asset_id in [split_id, other_id] {
        let mut transaction = transaction(
            &mut store,
            asset_id,
            TransactionKind::Deposit,
            Decimal::from(10),
        );
        transaction.timestamp = chrono::DateTime::parse_from_rfc3339("2023-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        store.data.transactions.push(transaction);
    }
    rebuild_ledger(&mut store.data).unwrap();
    assert_eq!(balance(&store, split_id), Decimal::from(20));
    assert_eq!(balance(&store, other_id), Decimal::from(10));
}

#[test]
fn ledger_rebuild_fails_on_missing_posted_asset() {
    let mut store = temp_store();
    let transaction = transaction(&mut store, 999, TransactionKind::Deposit, Decimal::ONE);
    store.data.transactions.push(transaction);
    assert!(rebuild_ledger(&mut store.data).is_err());
}
