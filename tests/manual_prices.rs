pub mod support;

use rust_decimal_macros::dec;
use support::{create_asset, temp_store};
use tuifolio::{
    assets::{AssetInput, save_asset},
    model::AssetKind,
    price_sync::{add_manual_price, add_manual_price_for_asset},
};

const ASSET_KINDS: [AssetKind; 8] = [
    AssetKind::Fiat,
    AssetKind::Crypto,
    AssetKind::Stock,
    AssetKind::Fund,
    AssetKind::Commodity,
    AssetKind::Property,
    AssetKind::Custom,
    AssetKind::Liability,
];

fn yahoo_asset_input(symbol: &str, kind: AssetKind) -> AssetInput {
    AssetInput {
        symbol: symbol.into(),
        name: symbol.into(),
        kind,
        yahoo_symbol: Some("TEST-USD".into()),
        tradingview_symbol: None,
        valuation_currency: None,
    }
}

#[test]
fn manual_pricing_rejects_configured_yahoo_symbols_for_all_asset_types() {
    let mut store = temp_store();
    for kind in ASSET_KINDS {
        let symbol = format!("TEST-{kind:?}");
        let id = save_asset(&mut store, None, yahoo_asset_input(&symbol, kind)).unwrap();
        let original = serde_json::to_value(&store.data).unwrap();
        assert!(add_manual_price_for_asset(&mut store, id, dec!(100), "EUR").is_err());
        assert!(add_manual_price(&mut store, &symbol, dec!(100), "EUR").is_err());
        assert_eq!(serde_json::to_value(&store.data).unwrap(), original);
    }
}

#[test]
fn assets_without_yahoo_symbols_can_record_prices_in_any_currency() {
    let mut store = temp_store();
    let id = create_asset(&mut store, "TEST", "Test asset", AssetKind::Stock);
    add_manual_price_for_asset(&mut store, id, dec!(100.123456789), " eur ").unwrap();
    let quote = store.data.prices.last().unwrap();
    assert_eq!(quote.asset_id, id);
    assert_eq!(quote.currency, "EUR");
    assert_eq!(quote.price, dec!(100.123456789));
    assert_eq!(quote.source, "manual");
}

#[test]
fn zero_market_values_are_allowed_but_zero_cash_exchange_rates_are_not() {
    let mut store = temp_store();
    for kind in ASSET_KINDS {
        let id = create_asset(&mut store, &format!("ZERO-{kind:?}"), "Zero", kind);
        let before = store.data.prices.len();
        let result = add_manual_price_for_asset(&mut store, id, dec!(0), "EUR");
        if kind == AssetKind::Fiat {
            assert!(result.is_err());
            assert_eq!(store.data.prices.len(), before);
        } else {
            result.unwrap();
            assert_eq!(store.data.prices.last().unwrap().price, dec!(0));
        }
    }
}

#[test]
fn zero_crypto_market_value_does_not_become_a_division_by_zero_reporting_rate() {
    use tuifolio::{
        accounting::build_report,
        model::{LedgerEffect, TransactionKind},
        portfolios::{PortfolioInput, create_portfolio},
        transactions::{ManualTransactionInput, add_manual_transaction},
    };
    let mut store = temp_store();
    let portfolio = create_portfolio(
        &mut store,
        PortfolioInput {
            name: "Main".into(),
        },
    )
    .unwrap();
    create_asset(&mut store, "EUR", "Euro", AssetKind::Fiat);
    let usd = create_asset(&mut store, "USD", "Dollar", AssetKind::Fiat);
    let btc = create_asset(&mut store, "BTC", "Bitcoin", AssetKind::Crypto);
    let stock = create_asset(&mut store, "ABC", "Stock", AssetKind::Stock);
    for asset in [btc, stock] {
        add_manual_transaction(
            &mut store,
            ManualTransactionInput {
                portfolio_id: portfolio,
                timestamp: chrono::Utc::now(),
                kind: TransactionKind::Gift,
                base_asset_id: asset,
                base_amount: dec!(1),
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
    add_manual_price_for_asset(&mut store, usd, dec!(0.9), "EUR").unwrap();
    add_manual_price_for_asset(&mut store, stock, dec!(100), "USD").unwrap();
    add_manual_price_for_asset(&mut store, btc, dec!(0), "USD").unwrap();
    let report = build_report(&store.data);
    assert_eq!(
        report
            .holdings
            .iter()
            .find(|h| h.asset_id == btc)
            .unwrap()
            .value,
        Some(dec!(0))
    );
    store.data.config.selected_base_currency = "BTC".into();
    let report = build_report(&store.data);
    let holding = report
        .holdings
        .iter()
        .find(|h| h.asset_id == stock)
        .unwrap();
    assert_eq!(holding.value, None);
    assert!(holding.missing_valuation);
    store.save().unwrap();
}

#[test]
fn invalid_manual_prices_do_not_add_quotes() {
    let mut store = temp_store();
    let id = create_asset(&mut store, "TEST", "Test asset", AssetKind::Custom);
    for (asset_id, price, currency) in [
        (id, dec!(-1), "EUR"),
        (id, dec!(100), "  "),
        (99999, dec!(100), "EUR"),
    ] {
        assert!(add_manual_price_for_asset(&mut store, asset_id, price, currency).is_err());
        assert!(store.data.prices.is_empty());
    }
}
