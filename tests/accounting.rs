pub mod support;

use rust_decimal::Decimal;
use support::{create_asset, create_portfolio, temp_store};
use tuifolio::{
    accounting::build_report,
    assets::{AssetInput, save_asset},
    ledger::rebuild_ledger,
    model::{AssetKind, Id, LedgerEffect, TransactionKind},
    price_sync::add_manual_price,
    store::Store,
    transactions::{ManualTransactionInput, add_manual_transaction},
};

fn movement(
    portfolio_id: Id,
    asset_id: Id,
    kind: TransactionKind,
    quantity: i64,
) -> ManualTransactionInput {
    ManualTransactionInput {
        portfolio_id,
        timestamp: chrono::Utc::now(),
        kind,
        base_asset_id: asset_id,
        base_amount: Decimal::from(quantity),
        quote_asset_id: None,
        quote_amount: None,
        quote_ledger_effect: LedgerEffect::Ignore,
        fee_asset_id: None,
        fee_amount: None,
        exchange: None,
        broker: None,
        notes: None,
    }
}

fn intrinsic_asset(store: &mut Store, symbol: &str, kind: AssetKind, currency: &str) -> Id {
    save_asset(
        store,
        None,
        AssetInput {
            symbol: symbol.into(),
            name: symbol.into(),
            kind,
            yahoo_symbol: None,
            tradingview_symbol: None,
            valuation_currency: Some(currency.into()),
        },
    )
    .unwrap()
}

fn post(store: &mut Store, portfolio: Id, asset: Id, kind: TransactionKind, quantity: i64) {
    add_manual_transaction(store, movement(portfolio, asset, kind, quantity)).unwrap();
}

#[test]
fn property_and_liability_movements_produce_net_worth_without_synthetic_history() {
    let mut store = temp_store();
    let portfolio = create_portfolio(&mut store, "Home");
    let property = intrinsic_asset(&mut store, "HOME", AssetKind::Property, "EUR");
    let debt = intrinsic_asset(&mut store, "DEBT", AssetKind::Liability, "EUR");
    for (asset, kind, quantity) in [
        (property, TransactionKind::AssetIncrease, 100000),
        (property, TransactionKind::AssetIncrease, 25000),
        (property, TransactionKind::AssetDecrease, 5000),
        (debt, TransactionKind::LiabilityIncrease, 80000),
        (debt, TransactionKind::LiabilityDecrease, 10000),
    ] {
        post(&mut store, portfolio, asset, kind, quantity);
    }
    let report = build_report(&store.data);
    assert_eq!(report.total_assets, Decimal::from(120000));
    assert_eq!(report.total_liabilities, Decimal::from(70000));
    assert_eq!(report.net_value, Decimal::from(50000));
    assert!(report.negative_balances.is_empty());
    assert_eq!(store.data.transactions.len(), 5);
}

#[test]
fn intrinsic_foreign_currency_values_use_explicit_exchange_rates() {
    let mut store = temp_store();
    let portfolio = create_portfolio(&mut store, "Home");
    create_asset(&mut store, "USD", "US dollar", AssetKind::Fiat);
    create_asset(&mut store, "EUR", "Euro", AssetKind::Fiat);
    let property = intrinsic_asset(&mut store, "HOME", AssetKind::Property, "USD");
    add_manual_price(&mut store, "USD", Decimal::new(9, 1), "EUR").unwrap();
    post(
        &mut store,
        portfolio,
        property,
        TransactionKind::AssetIncrease,
        100000,
    );
    assert_eq!(build_report(&store.data).net_value, Decimal::from(90000));
}

#[test]
fn manual_prices_enable_commodity_valuation() {
    let mut store = temp_store();
    let portfolio = create_portfolio(&mut store, "Metal");
    let gold = create_asset(&mut store, "GOLD", "Gold", AssetKind::Commodity);
    post(&mut store, portfolio, gold, TransactionKind::Deposit, 2);
    assert!(build_report(&store.data).holdings[0].stale_price);
    add_manual_price(&mut store, "GOLD", Decimal::from(3000), "EUR").unwrap();
    assert_eq!(build_report(&store.data).net_value, Decimal::from(6000));
}

#[test]
fn every_transaction_kind_uses_the_expected_quantity_direction() {
    let mut store = temp_store();
    let asset = create_asset(&mut store, "ABC", "Stock", AssetKind::Stock);
    let cash = create_asset(&mut store, "EUR", "Euro", AssetKind::Fiat);
    for (kind, expected) in [
        (TransactionKind::Buy, 10),
        (TransactionKind::Sell, -10),
        (TransactionKind::Deposit, 10),
        (TransactionKind::Withdraw, -10),
        (TransactionKind::AssetIncrease, 10),
        (TransactionKind::AssetDecrease, -10),
        (TransactionKind::LiabilityIncrease, 10),
        (TransactionKind::LiabilityDecrease, -10),
    ] {
        let portfolio = create_portfolio(&mut store, &format!("{kind:?}"));
        let mut input = movement(portfolio, asset, kind, 10);
        if matches!(kind, TransactionKind::Buy | TransactionKind::Sell) {
            input.quote_asset_id = Some(cash);
            input.quote_amount = Some(Decimal::from(100));
        }
        add_manual_transaction(&mut store, input).unwrap();
        let balance: Decimal = store
            .data
            .ledger_entries
            .iter()
            .filter(|entry| entry.portfolio_id == portfolio && entry.asset_id == asset)
            .map(|entry| entry.quantity_delta)
            .sum();
        assert_eq!(balance, Decimal::from(expected));
    }
}

#[test]
fn fiat_deposits_post_in_any_portfolio_and_withdrawals_expose_real_negative_balances() {
    let mut store = temp_store();
    let cash = create_asset(&mut store, "EUR", "Euro", AssetKind::Fiat);
    for name in ["Stocks", "Crypto", "Metal", "Home"] {
        let portfolio = create_portfolio(&mut store, name);
        post(&mut store, portfolio, cash, TransactionKind::Deposit, 100);
        post(&mut store, portfolio, cash, TransactionKind::Withdraw, 150);
    }
    let report = build_report(&store.data);
    assert_eq!(report.negative_balances.len(), 4);
    assert!(
        report
            .holdings
            .iter()
            .all(|holding| holding.quantity == Decimal::from(-50))
    );
}

#[test]
fn accounting_ignores_origin_identifiers_and_preserved_raw_rows() {
    let mut store = temp_store();
    let portfolio = create_portfolio(&mut store, "Main");
    let asset = create_asset(&mut store, "ABC", "Stock", AssetKind::Stock);
    post(&mut store, portfolio, asset, TransactionKind::Deposit, 5);
    add_manual_price(&mut store, "ABC", Decimal::from(100), "EUR").unwrap();
    let entries = serde_json::to_value(&store.data.ledger_entries).unwrap();
    let value = build_report(&store.data).net_value;
    store.data.transactions[0].source = "delta_Fiat_historical.csv".into();
    store.data.transactions[0].source_row_hash = "old-hash".into();
    store.data.transactions[0].notes = Some("SYNC-BASE-HOLDINGS_historical".into());
    store
        .data
        .raw_rows
        .insert("old-hash".into(), "original payload".into());
    rebuild_ledger(&mut store.data).unwrap();
    assert_eq!(
        serde_json::to_value(&store.data.ledger_entries).unwrap(),
        entries
    );
    assert_eq!(build_report(&store.data).net_value, value);
}
