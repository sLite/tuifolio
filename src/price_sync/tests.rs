use super::*;
use rust_decimal_macros::dec;

fn conversion_assets() -> Vec<Asset> {
    [
        ("EUR", AssetKind::Fiat),
        ("USD", AssetKind::Fiat),
        ("BTC", AssetKind::Crypto),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (symbol, kind))| Asset {
        id: index as Id + 1,
        symbol: symbol.into(),
        name: symbol.into(),
        kind,
        yahoo_symbol: (kind == AssetKind::Crypto).then(|| "BTC-USD".into()),
        tradingview_symbol: None,
        valuation_currency: None,
    })
    .collect()
}

#[test]
fn fiat_conversion_fetches_a_single_direct_quote() {
    let assets = conversion_assets();
    let mut requests = Vec::new();
    let rate = conversion_rate("EUR", "USD", &assets, |symbol| {
        requests.push(symbol.to_string());
        Ok(Some((dec!(1.25), "USD".into())))
    })
    .unwrap();
    assert_eq!(rate, Some(dec!(1.25)));
    assert_eq!(requests, ["EURUSD=X"]);
}

#[test]
fn fiat_to_crypto_resolves_usd_without_recursion() {
    let assets = conversion_assets();
    let mut requests = Vec::new();
    let rate = conversion_rate("EUR", "BTC", &assets, |symbol| {
        requests.push(symbol.to_string());
        match symbol {
            "EURUSD=X" => Ok(Some((dec!(1.25), "USD".into()))),
            "BTC-USD" => Ok(Some((dec!(50000), "USD".into()))),
            _ => panic!("unexpected conversion request: {symbol}"),
        }
    })
    .unwrap();
    assert_eq!(rate, Some(dec!(0.000025)));
    assert_eq!(requests, ["EURUSD=X", "BTC-USD"]);
}

#[test]
fn usd_to_crypto_uses_identity_intermediary_without_fx_request() {
    let rate = conversion_rate("USD", "BTC", &conversion_assets(), |symbol| {
        assert_eq!(symbol, "BTC-USD");
        Ok(Some((dec!(50000), "USD".into())))
    })
    .unwrap();
    assert_eq!(rate, Some(dec!(0.00002)));
}

#[test]
fn identity_and_unsupported_routes_need_no_provider_requests() {
    let assets = conversion_assets();
    let no_fetch = |_: &str| -> anyhow::Result<Option<(Decimal, String)>> {
        panic!("this route must not request a quote");
    };
    assert_eq!(
        conversion_rate("EUR", "EUR", &assets, no_fetch).unwrap(),
        Some(Decimal::ONE)
    );
    for (from, to) in [("EUR", "UNKNOWN"), ("BTC", "EUR")] {
        assert_eq!(conversion_rate(from, to, &assets, no_fetch).unwrap(), None);
    }
    let mut missing_usd = assets.clone();
    missing_usd.retain(|asset| asset.symbol != "USD");
    assert_eq!(
        conversion_rate("EUR", "BTC", &missing_usd, |symbol| match symbol {
            "EURUSD=X" => Ok(Some((dec!(1.25), "USD".into()))),
            "BTC-USD" => Ok(Some((dec!(50000), "USD".into()))),
            _ => panic!("unexpected quote request"),
        })
        .unwrap(),
        Some(dec!(0.000025))
    );
    let mut unmapped = assets;
    unmapped[2].yahoo_symbol = None;
    assert_eq!(
        conversion_rate("EUR", "BTC", &unmapped, no_fetch).unwrap(),
        None
    );
}

#[test]
fn misclassified_usd_is_rejected_before_any_conversion_request() {
    for symbol in ["USD", "usd"] {
        let mut assets = conversion_assets();
        assets[1].symbol = symbol.into();
        assets[1].kind = AssetKind::Crypto;
        for (from, to) in [("EUR", "USD"), ("EUR", "BTC"), ("EUR", "EUR")] {
            let error = conversion_rate(from, to, &assets, |_| {
                panic!("invalid USD configuration must fail before fetching");
            })
            .unwrap_err();
            assert!(error.is::<crate::assets::InvalidUsdAsset>());
            assert!(
                error
                    .to_string()
                    .contains("USD must use the Cash asset type")
            );
        }
    }
}

#[test]
fn missing_conversion_quotes_remain_unavailable() {
    let assets = conversion_assets();
    assert_eq!(
        conversion_rate("EUR", "BTC", &assets, |symbol| {
            assert_eq!(symbol, "EURUSD=X");
            Ok(None)
        })
        .unwrap(),
        None
    );
    assert_eq!(
        conversion_rate("USD", "BTC", &assets, |symbol| {
            assert_eq!(symbol, "BTC-USD");
            Ok(None)
        })
        .unwrap(),
        None
    );
}

#[test]
fn invalid_conversion_quotes_return_errors_instead_of_dividing() {
    let assets = conversion_assets();
    for (price, currency) in [
        (Decimal::ZERO, "USD"),
        (dec!(-1), "USD"),
        (dec!(50000), "EUR"),
    ] {
        let error = conversion_rate("USD", "BTC", &assets, |_| {
            Ok(Some((price, currency.into())))
        })
        .unwrap_err();
        assert!(error.to_string().contains("conversion quote"));
    }
    assert!(
        conversion_rate("EUR", "BTC", &assets, |symbol| {
            assert_eq!(symbol, "EURUSD=X");
            Ok(Some((Decimal::ZERO, "USD".into())))
        })
        .is_err()
    );
}

#[test]
fn unrepresentable_conversion_rates_return_errors_instead_of_panicking() {
    let smallest = dec!(0.0000000000000000000000000001);
    for (numerator, denominator) in [(Decimal::MAX, smallest), (smallest, Decimal::MAX)] {
        let error = conversion_rate("EUR", "BTC", &conversion_assets(), |symbol| {
            Ok(Some((
                if symbol == "EURUSD=X" {
                    numerator
                } else {
                    denominator
                },
                "USD".into(),
            )))
        })
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("outside the supported decimal range")
        );
    }
}

#[test]
fn fiat_units_do_not_use_crypto_assets_with_the_same_display_label() {
    let mut assets = conversion_assets();
    assets[2].symbol = "GBP".into();
    let rate = conversion_rate("GBP", "EUR", &assets, |symbol| {
        assert_eq!(symbol, "GBPEUR=X");
        Ok(Some((dec!(1.25), "EUR".into())))
    })
    .unwrap();
    assert_eq!(rate, Some(dec!(1.25)));
}

#[test]
fn normalizes_pence_without_changing_pound_quotes() {
    for currency in ["GBp", "GBX"] {
        assert_eq!(
            normalize_yahoo_units(dec!(250), currency),
            Some((dec!(2.5), "GBP".into()))
        );
    }
    assert_eq!(
        normalize_yahoo_units(dec!(250), "GBP"),
        Some((dec!(250), "GBP".into()))
    );
    assert_eq!(
        normalize_yahoo_units(Decimal::ZERO, "GBp"),
        Some((Decimal::ZERO, "GBP".into()))
    );
    assert!(normalize_yahoo_units(dec!(-1), "USD").is_none());
    assert!(normalize_yahoo_units(dec!(1), "UNSUPPORTED").is_none());
}

#[test]
fn provider_numbers_keep_their_exact_decimal_spelling() {
    let json: Value = serde_json::from_str(r#"{"chart":{"result":[{"meta":{"regularMarketPrice":1.234567890123456789,"currency":"USD"}}]}}"#).unwrap();
    assert_eq!(
        parse_yahoo_quote(&json),
        Some((dec!(1.234567890123456789), "USD".into()))
    );
    let scientific: Value = serde_json::from_str(
        r#"{"chart":{"result":[{"meta":{"regularMarketPrice":1.25e-7,"currency":"USD"}}]}}"#,
    )
    .unwrap();
    assert_eq!(
        parse_yahoo_quote(&scientific),
        Some((dec!(0.000000125), "USD".into()))
    );
}

#[test]
fn keeps_yahoo_native_symbols() {
    let asset = Asset {
        id: 1,
        symbol: "GOLD".into(),
        name: "Gold".into(),
        kind: AssetKind::Commodity,
        yahoo_symbol: Some("GC=F".into()),
        tradingview_symbol: Some("OANDA:XAUUSD".into()),
        valuation_currency: None,
    };
    assert_eq!(asset.yahoo_quote_symbol(), Some("GC=F"));
}

#[test]
fn parses_market_quotes_and_skips_missing_provider_data() {
    let quote = serde_json::json!({"chart": {"result": [{"meta": {"regularMarketPrice": 100.0, "currency": "USD"}}]}});
    assert_eq!(
        parse_yahoo_quote(&quote),
        Some((Decimal::from(100), "USD".into()))
    );
    for quote in [
        serde_json::json!({"chart": {"result": null}}),
        serde_json::json!({"chart": {"result": [{"meta": {"currency": "USD"}}]}}),
        serde_json::json!({"chart": {"result": [{"meta": {"regularMarketPrice": 100.0}}]}}),
    ] {
        assert!(parse_yahoo_quote(&quote).is_none());
    }
}
