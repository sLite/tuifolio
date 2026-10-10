pub mod support;

use rust_decimal_macros::dec;
use support::{create_asset, create_portfolio, temp_store};
use tuifolio::{
    accounting::build_report,
    model::*,
    price_sync::add_manual_price_for_asset_at,
    store::Store,
    transactions::{ManualTransactionInput, add_manual_transaction},
};

fn movement(store: &mut Store, portfolio: Id, asset: Id, amount: rust_decimal::Decimal) {
    add_manual_transaction(
        store,
        ManualTransactionInput {
            portfolio_id: portfolio,
            timestamp: "2021-01-01T00:00:00Z".parse().unwrap(),
            kind: TransactionKind::Gift,
            base_asset_id: asset,
            base_amount: amount,
            quote_asset_id: None,
            quote_amount: None,
            quote_ledger_effect: LedgerEffect::Ignore,
            fee_asset_id: None,
            fee_amount: None,
            exchange: None,
            broker: None,
            notes: None,
        },
    )
    .unwrap();
}
fn point(store: &mut Store, asset: Id, value: rust_decimal::Decimal, currency: &str, date: &str) {
    add_manual_price_for_asset_at(store, asset, value, currency, date.parse().unwrap()).unwrap();
}

#[test]
fn newest_convertible_price_wins_over_older_reporting_currency_price() {
    let mut store = temp_store();
    let p = create_portfolio(&mut store, "Main");
    let usd = create_asset(&mut store, "USD", "Dollar", AssetKind::Fiat);
    let stock = create_asset(&mut store, "ABC", "Stock", AssetKind::Stock);
    movement(&mut store, p, stock, dec!(3));
    point(&mut store, stock, dec!(100), "EUR", "2020-01-01T00:00:00Z");
    point(&mut store, stock, dec!(200), "USD", "2021-01-01T00:00:00Z");
    point(&mut store, usd, dec!(0.9), "EUR", "2021-01-01T00:00:00Z");
    assert_eq!(build_report(&store.data).net_value, dec!(540));
    point(&mut store, stock, dec!(999), "ZZZ", "2022-01-01T00:00:00Z");
    assert_eq!(build_report(&store.data).net_value, dec!(540));
}

#[test]
fn fallback_and_equal_timestamp_selection_are_deterministic() {
    let mut store = temp_store();
    let p = create_portfolio(&mut store, "Main");
    let stock = create_asset(&mut store, "ABC", "Stock", AssetKind::Stock);
    movement(&mut store, p, stock, dec!(1));
    point(&mut store, stock, dec!(100), "EUR", "2020-01-01T00:00:00Z");
    point(&mut store, stock, dec!(999), "USD", "2021-01-01T00:00:00Z");
    assert_eq!(build_report(&store.data).net_value, dec!(100));
    let usd = create_asset(&mut store, "USD", "Dollar", AssetKind::Fiat);
    point(&mut store, usd, dec!(0.9), "EUR", "2020-01-01T00:00:00Z");
    point(&mut store, stock, dec!(120), "EUR", "2021-01-01T00:00:00Z");
    for _ in 0..20 {
        assert_eq!(build_report(&store.data).net_value, dec!(120));
    }
}

#[test]
fn ticker_collisions_use_asset_ids_for_both_values_and_historical_costs() {
    let mut store = temp_store();
    store.data.config.selected_base_currency = "BTC".into();
    let p = create_portfolio(&mut store, "Main");
    let btc = create_asset(&mut store, "BTC", "Bitcoin", AssetKind::Crypto);
    let xyz = create_asset(&mut store, "XYZ", "XYZ", AssetKind::Stock);
    let mut shadow = store
        .data
        .assets
        .iter()
        .find(|a| a.id == xyz)
        .unwrap()
        .clone();
    shadow.id = store.data.allocate_id();
    shadow.symbol = "BTC".into();
    shadow.name = "Ticker BTC".into();
    let stock = shadow.id;
    store.data.assets.push(shadow);
    movement(&mut store, p, btc, dec!(1));
    movement(&mut store, p, stock, dec!(3));
    point(&mut store, btc, dec!(100), "USD", "2020-01-01T00:00:00Z");
    point(&mut store, stock, dec!(10), "USD", "2020-01-01T00:00:00Z");
    assert_eq!(build_report(&store.data).net_value, dec!(1.3));
    add_manual_transaction(
        &mut store,
        ManualTransactionInput {
            portfolio_id: p,
            timestamp: "2021-01-01T00:00:00Z".parse().unwrap(),
            kind: TransactionKind::Buy,
            base_asset_id: xyz,
            base_amount: dec!(1),
            quote_asset_id: Some(stock),
            quote_amount: Some(dec!(3)),
            quote_ledger_effect: LedgerEffect::Ignore,
            fee_asset_id: None,
            fee_amount: None,
            exchange: None,
            broker: None,
            notes: None,
        },
    )
    .unwrap();
    let report = build_report(&store.data);
    assert_eq!(
        report
            .holdings
            .iter()
            .find(|h| h.asset_id == xyz)
            .unwrap()
            .net_invested,
        Some(dec!(0.3))
    );
    store.save().unwrap();
}

#[test]
fn retained_pence_observations_are_scaled_in_calculations_without_rewriting_records() {
    let mut store = temp_store();
    store.data.config.selected_base_currency = "GBP".into();
    store.data.config.base_currencies.push("GBP".into());
    let p = create_portfolio(&mut store, "Main");
    create_asset(&mut store, "GBP", "Pound", AssetKind::Fiat);
    let usd = create_asset(&mut store, "USD", "Dollar", AssetKind::Fiat);
    let stock = create_asset(&mut store, "ABC", "Stock", AssetKind::Stock);
    movement(&mut store, p, stock, dec!(1));
    movement(&mut store, p, usd, dec!(2));
    for (asset_id, price) in [(stock, dec!(100)), (usd, dec!(250))] {
        store.data.prices.push(Price {
            asset_id,
            price,
            currency: "GBp".into(),
            source: "yahoo".into(),
            timestamp: "2020-01-01T00:00:00Z".parse().unwrap(),
        });
    }
    let original = serde_json::to_value(&store.data).unwrap();
    assert_eq!(build_report(&store.data).net_value, dec!(6));
    assert_eq!(serde_json::to_value(&store.data).unwrap(), original);
}

#[test]
fn a_crypto_ticker_matching_a_fiat_code_does_not_define_that_fiat_unit() {
    let mut store = temp_store();
    let p = create_portfolio(&mut store, "Main");
    let eur = create_asset(&mut store, "EUR", "Euro", AssetKind::Fiat);
    let usd = create_asset(&mut store, "USD", "Dollar", AssetKind::Fiat);
    let coin = create_asset(&mut store, "GBP", "Coin named GBP", AssetKind::Crypto);
    let stock = create_asset(&mut store, "ABC", "Stock", AssetKind::Stock);
    movement(&mut store, p, coin, dec!(1));
    movement(&mut store, p, stock, dec!(1));
    point(&mut store, coin, dec!(40), "USD", "2020-01-01T00:00:00Z");
    point(&mut store, stock, dec!(100), "GBP", "2020-01-01T00:00:00Z");
    point(&mut store, usd, dec!(0.9), "EUR", "2020-01-01T00:00:00Z");
    point(&mut store, eur, dec!(0.8), "GBP", "2020-01-01T00:00:00Z");
    let report = build_report(&store.data);
    assert_eq!(
        report
            .holdings
            .iter()
            .find(|h| h.asset_id == coin)
            .unwrap()
            .value,
        Some(dec!(36))
    );
    assert_eq!(
        report
            .holdings
            .iter()
            .find(|h| h.asset_id == stock)
            .unwrap()
            .value,
        Some(dec!(125))
    );
}
