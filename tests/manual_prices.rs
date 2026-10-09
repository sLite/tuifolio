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
fn invalid_manual_prices_do_not_add_quotes() {
    let mut store = temp_store();
    let id = create_asset(&mut store, "TEST", "Test asset", AssetKind::Custom);
    for (asset_id, price, currency) in [
        (id, dec!(0), "EUR"),
        (id, dec!(-1), "EUR"),
        (id, dec!(100), "  "),
        (99999, dec!(100), "EUR"),
    ] {
        assert!(add_manual_price_for_asset(&mut store, asset_id, price, currency).is_err());
        assert!(store.data.prices.is_empty());
    }
}
