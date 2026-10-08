pub mod support;

use rust_decimal::Decimal;
use support::{create_asset, create_portfolio, temp_store};
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
    let portfolio_id = store.data.portfolios[0].id;
    let id = store.data.allocate_id();
    Transaction {
        id,
        portfolio_id,
        timestamp: chrono::Utc::now(),
        kind,
        base_asset_id: asset_id,
        base_amount: amount,
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

fn ledger_store() -> Store {
    let mut store = temp_store();
    create_portfolio(&mut store, "Manual");
    store
}

#[test]
fn ledger_posts_quote_amounts_even_when_cash_goes_negative() {
    let mut store = ledger_store();
    let stock_id = create_asset(&mut store, "ABC", "ABC", AssetKind::Stock);
    let usd_id = create_asset(&mut store, "USD", "USD", AssetKind::Fiat);
    let mut transaction = transaction(&mut store, stock_id, TransactionKind::Buy, Decimal::ONE);
    transaction.quote_asset_id = Some(usd_id);
    transaction.quote_amount = Some(Decimal::from(100));
    store.data.transactions.push(transaction);
    rebuild_ledger(&mut store.data).unwrap();
    assert_eq!(balance(&store, usd_id), Decimal::from(-100));
}

#[test]
fn explicit_split_events_adjust_only_the_target_asset() {
    let mut store = ledger_store();
    let split_id = create_asset(&mut store, "ABC", "ABC", AssetKind::Stock);
    let other_id = create_asset(&mut store, "XYZ", "XYZ", AssetKind::Stock);
    store.data.config.stock_splits.push(StockSplit {
        asset_id: split_id,
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
    let mut store = ledger_store();
    let transaction = transaction(&mut store, 999, TransactionKind::Deposit, Decimal::ONE);
    store.data.transactions.push(transaction);
    assert!(rebuild_ledger(&mut store.data).is_err());
}

#[test]
fn legacy_asset_posting_flags_cannot_suppress_asset_movements() {
    for effect in ["Post", "Ignore"] {
        let mut store = ledger_store();
        let asset_id = create_asset(&mut store, "EUR", "Euro", AssetKind::Fiat);
        let movement = transaction(&mut store, asset_id, TransactionKind::Deposit, Decimal::ONE);
        let mut legacy = serde_json::to_value(&movement).unwrap();
        legacy["base_ledger_effect"] = effect.into();
        let loaded: Transaction = serde_json::from_value(legacy).unwrap();
        assert!(
            serde_json::to_value(&loaded)
                .unwrap()
                .get("base_ledger_effect")
                .is_none()
        );
        store.data.transactions.push(loaded);
        rebuild_ledger(&mut store.data).unwrap();
        assert_eq!(balance(&store, asset_id), Decimal::ONE);
    }
}

fn dated_movement(store: &mut Store, asset: Id, kind: TransactionKind, date: &str) -> Id {
    let mut movement = transaction(store, asset, kind, Decimal::from(10));
    movement.timestamp = chrono::DateTime::parse_from_rfc3339(date)
        .unwrap()
        .with_timezone(&chrono::Utc);
    let id = movement.id;
    store.data.transactions.push(movement);
    id
}

fn entry_quantity(store: &Store, id: Id) -> Decimal {
    store
        .data
        .ledger_entries
        .iter()
        .filter(|entry| {
            entry.transaction_id == id && matches!(entry.role, tuifolio::model::LedgerRole::Base)
        })
        .map(|entry| entry.quantity_delta)
        .sum()
}

#[test]
fn configured_splits_use_effective_dates_and_do_not_mutate_transactions() {
    let mut store = ledger_store();
    let asset = create_asset(&mut store, "ABC", "ABC", AssetKind::Stock);
    store.data.config.stock_splits.push(StockSplit {
        asset_id: asset,
        effective_date: "2024-01-01".into(),
        numerator: Decimal::from(4),
        denominator: Decimal::ONE,
    });
    let cases = [
        (TransactionKind::Deposit, "2023-12-31T23:59:59Z", 40),
        (TransactionKind::Deposit, "2024-01-01T00:00:00Z", 10),
        (TransactionKind::Withdraw, "2024-01-02T00:00:00Z", -10),
    ]
    .map(|(kind, date, expected)| {
        (
            dated_movement(&mut store, asset, kind, date),
            Decimal::from(expected),
        )
    });
    let transactions = serde_json::to_value(&store.data.transactions).unwrap();
    rebuild_ledger(&mut store.data).unwrap();
    for (id, expected) in cases {
        assert_eq!(entry_quantity(&store, id), expected);
    }
    let entries = serde_json::to_value(&store.data.ledger_entries).unwrap();
    rebuild_ledger(&mut store.data).unwrap();
    assert_eq!(
        serde_json::to_value(&store.data.ledger_entries).unwrap(),
        entries
    );
    assert_eq!(
        serde_json::to_value(&store.data.transactions).unwrap(),
        transactions
    );
}

#[test]
fn splits_compound_reverse_ratios_without_adjusting_quote_or_fee_amounts() {
    let mut store = ledger_store();
    let asset = create_asset(&mut store, "ABC", "ABC", AssetKind::Stock);
    let cash = create_asset(&mut store, "EUR", "Euro", AssetKind::Fiat);
    for (date, numerator, denominator) in [("2024-01-01", 4, 1), ("2025-01-01", 1, 2)] {
        store.data.config.stock_splits.push(StockSplit {
            asset_id: asset,
            effective_date: date.into(),
            numerator: Decimal::from(numerator),
            denominator: Decimal::from(denominator),
        });
    }
    let mut movement = transaction(&mut store, asset, TransactionKind::Buy, Decimal::from(10));
    movement.timestamp = chrono::DateTime::parse_from_rfc3339("2023-01-01T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    movement.quote_asset_id = Some(cash);
    movement.quote_amount = Some(Decimal::from(100));
    movement.fee_asset_id = Some(cash);
    movement.fee_amount = Some(Decimal::from(2));
    let id = movement.id;
    store.data.transactions.push(movement);
    rebuild_ledger(&mut store.data).unwrap();
    assert_eq!(entry_quantity(&store, id), Decimal::from(20));
    assert_eq!(balance(&store, cash), Decimal::from(-102));
}

#[test]
fn creating_gme_does_not_create_automatic_split_events() {
    let mut store = ledger_store();
    create_asset(&mut store, "GME", "GameStop", AssetKind::Stock);
    assert!(store.data.config.stock_splits.is_empty());
}
