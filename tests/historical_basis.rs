pub mod support;

use rust_decimal_macros::dec;
use support::{create_asset, create_portfolio, temp_store};
use tuifolio::{
    accounting::build_report,
    model::*,
    price_sync::{add_manual_price_for_asset, add_manual_price_for_asset_at},
    transactions::{ManualTransactionInput, add_manual_transaction},
};

#[test]
fn newer_fx_never_fills_missing_historical_basis_and_dated_corrections_are_stable() {
    let mut store = temp_store();
    let portfolio = create_portfolio(&mut store, "Main");
    let usd = create_asset(&mut store, "USD", "Dollar", AssetKind::Fiat);
    let stock = create_asset(&mut store, "ABC", "Stock", AssetKind::Stock);
    add_manual_transaction(
        &mut store,
        ManualTransactionInput {
            portfolio_id: portfolio,
            timestamp: "2021-01-01T00:00:00Z".parse().unwrap(),
            kind: TransactionKind::Buy,
            base_asset_id: stock,
            base_amount: dec!(2),
            quote_asset_id: Some(usd),
            quote_amount: Some(dec!(100)),
            quote_ledger_effect: LedgerEffect::Ignore,
            fee_asset_id: None,
            fee_amount: None,
            exchange: None,
            broker: None,
            notes: None,
        },
    )
    .unwrap();
    let original = serde_json::to_value(&store.data.transactions).unwrap();
    add_manual_price_for_asset(&mut store, stock, dec!(100), "EUR").unwrap();
    add_manual_price_for_asset(&mut store, usd, dec!(0.8), "EUR").unwrap();
    let report = build_report(&store.data);
    let h = report
        .holdings
        .iter()
        .find(|h| h.asset_id == stock)
        .unwrap();
    assert_eq!(h.value, Some(dec!(200)));
    assert_eq!(h.net_invested, None);
    assert_eq!(h.unrealized_pnl, None);
    assert_eq!(report.total_unrealized_pnl, None);
    add_manual_price_for_asset_at(
        &mut store,
        usd,
        dec!(0.9),
        "EUR",
        "2020-12-31T23:59:59Z".parse().unwrap(),
    )
    .unwrap();
    add_manual_price_for_asset(&mut store, usd, dec!(0.5), "EUR").unwrap();
    let report = build_report(&store.data);
    let h = report
        .holdings
        .iter()
        .find(|h| h.asset_id == stock)
        .unwrap();
    assert_eq!(h.net_invested, Some(dec!(90)));
    assert_eq!(h.unrealized_pnl, Some(dec!(110)));
    assert_eq!(
        serde_json::to_value(&store.data.transactions).unwrap(),
        original
    );
    store.save().unwrap();
    let path = store.path().to_owned();
    drop(store);
    let reopened = tuifolio::store::Store::open(Some(path)).unwrap();
    assert_eq!(
        build_report(&reopened.data).total_unrealized_pnl,
        Some(dec!(110))
    );
}

#[test]
fn a_currency_trade_records_its_execution_cost_in_that_currency_without_latest_fx() {
    let mut store = temp_store();
    store.data.config.selected_base_currency = "BTC".into();
    let portfolio = create_portfolio(&mut store, "Main");
    let euro = create_asset(&mut store, "EUR", "Euro", AssetKind::Fiat);
    let btc = create_asset(&mut store, "BTC", "Bitcoin", AssetKind::Crypto);
    add_manual_transaction(
        &mut store,
        ManualTransactionInput {
            portfolio_id: portfolio,
            timestamp: "2021-01-01T00:00:00Z".parse().unwrap(),
            kind: TransactionKind::Buy,
            base_asset_id: btc,
            base_amount: dec!(2),
            quote_asset_id: Some(euro),
            quote_amount: Some(dec!(30)),
            quote_ledger_effect: LedgerEffect::Ignore,
            fee_asset_id: None,
            fee_amount: None,
            exchange: None,
            broker: None,
            notes: None,
        },
    )
    .unwrap();
    add_manual_price_for_asset(&mut store, euro, dec!(99), "BTC").unwrap();
    let report = build_report(&store.data);
    let holding = &report.holdings[0];
    assert_eq!(holding.net_invested, Some(dec!(2)));
    assert_eq!(holding.unrealized_pnl, Some(dec!(0)));
}

#[test]
fn future_price_corrections_are_rejected_atomically_in_core_and_cli() {
    let mut store = temp_store();
    let usd = create_asset(&mut store, "USD", "Dollar", AssetKind::Fiat);
    store.save().unwrap();
    let original = std::fs::read(store.path()).unwrap();
    let data = serde_json::to_value(&store.data).unwrap();
    assert!(
        add_manual_price_for_asset_at(
            &mut store,
            usd,
            dec!(0.9),
            "EUR",
            "2099-01-01T00:00:00Z".parse().unwrap()
        )
        .is_err()
    );
    assert_eq!(serde_json::to_value(&store.data).unwrap(), data);
    let path = store.path().to_owned();
    drop(store);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_tuifolio"))
        .arg("--store")
        .arg(&path)
        .args([
            "add-price",
            "USD",
            "0.9",
            "EUR",
            "--at",
            "2099-01-01T00:00:00Z",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be in the future"));
    assert_eq!(std::fs::read(&path).unwrap(), original);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_tuifolio"))
        .arg("--store")
        .arg(&path)
        .args([
            "add-price",
            "USD",
            "0.9",
            "EUR",
            "--at",
            "2020-01-01T00:00:00Z",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
    let reopened = tuifolio::store::Store::open(Some(path)).unwrap();
    assert_eq!(
        reopened.data.prices.last().unwrap().timestamp,
        "2020-01-01T00:00:00Z"
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap()
    );
}
