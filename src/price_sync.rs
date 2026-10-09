use reqwest::StatusCode;
use rust_decimal::Decimal;
use serde_json::Value;

use crate::{
    accounting::now,
    assets::validate_usd_asset,
    model::{Asset, AssetKind, Id, Price, StoreData},
    store::Store,
};

pub struct SyncSummary {
    pub updated: usize,
    pub unsupported: usize,
}

pub struct PriceBatch {
    pub prices: Vec<Price>,
    pub summary: SyncSummary,
}

pub fn sync_prices(store: &mut Store) -> anyhow::Result<SyncSummary> {
    let batch = fetch_prices(&store.data)?;
    store.data.prices.extend(batch.prices);
    Ok(batch.summary)
}

pub fn fetch_prices(data: &StoreData) -> anyhow::Result<PriceBatch> {
    // Stored data may predate asset validation. Fail before any provider request.
    for asset in &data.assets {
        validate_usd_asset(asset)?;
    }
    tracing::info!(assets = data.assets.len(), "starting price refresh");
    let client = reqwest::blocking::Client::builder()
        .user_agent("tuifolio/0.1 local portfolio tracker")
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(15))
        .build()?;
    let mut prices = Vec::new();
    let mut unsupported = 0;
    let timestamp = now();
    for asset in &data.assets {
        let updates = fetch_asset_prices(&client, data, asset, timestamp)?;
        unsupported += usize::from(needs_price(asset) && updates.is_empty());
        prices.extend(updates);
    }
    let summary = SyncSummary {
        updated: prices.len(),
        unsupported,
    };
    tracing::info!(
        updated = summary.updated,
        unsupported,
        "price refresh completed"
    );
    Ok(PriceBatch { prices, summary })
}

fn fetch_asset_prices(
    client: &reqwest::blocking::Client,
    data: &StoreData,
    asset: &Asset,
    timestamp: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<Vec<Price>> {
    let mut prices = Vec::new();
    if asset.kind == AssetKind::Fiat {
        for base in &data.config.base_currencies {
            if let Some(price) = yahoo_conversion_rate(client, &asset.symbol, base, &data.assets)? {
                prices.push(Price {
                    asset_id: asset.id,
                    timestamp,
                    price,
                    currency: base.clone(),
                    source: "yahoo".into(),
                });
            }
        }
    } else if let Some((price, currency, source)) = yahoo_asset_native_price(client, asset)? {
        prices.push(Price {
            asset_id: asset.id,
            timestamp,
            price,
            currency,
            source,
        });
    }
    Ok(prices)
}

pub fn add_manual_price(
    store: &mut Store,
    symbol: &str,
    price: Decimal,
    currency: &str,
) -> anyhow::Result<()> {
    let matches = store
        .data
        .assets
        .iter()
        .filter(|asset| asset.symbol == symbol)
        .collect::<Vec<_>>();
    anyhow::ensure!(!matches.is_empty(), "unknown asset symbol: {symbol}");
    anyhow::ensure!(
        matches.len() == 1,
        "ambiguous asset symbol: {symbol}; use a unique symbol before adding prices"
    );
    let asset_id = matches[0].id;
    add_manual_price_for_asset(store, asset_id, price, currency)
}

pub fn add_manual_price_for_asset(
    store: &mut Store,
    asset_id: Id,
    price: Decimal,
    currency: &str,
) -> anyhow::Result<()> {
    anyhow::ensure!(price > Decimal::ZERO, "price must be greater than zero");
    anyhow::ensure!(!currency.trim().is_empty(), "price currency is required");
    let asset = store
        .data
        .assets
        .iter()
        .find(|asset| asset.id == asset_id)
        .ok_or_else(|| anyhow::anyhow!("unknown asset"))?;
    anyhow::ensure!(
        asset.allows_manual_pricing(),
        "clear the Yahoo symbol and save the asset before recording manual prices"
    );
    store.data.prices.push(Price {
        asset_id,
        timestamp: now(),
        price,
        currency: currency.trim().to_ascii_uppercase(),
        source: "manual".into(),
    });
    Ok(())
}

fn yahoo_asset_native_price(
    client: &reqwest::blocking::Client,
    asset: &Asset,
) -> anyhow::Result<Option<(Decimal, String, String)>> {
    match asset.kind {
        AssetKind::Crypto => yahoo_crypto_usd_price(client, asset),
        AssetKind::Stock | AssetKind::Fund | AssetKind::Commodity => {
            let Some(symbol) = asset.yahoo_quote_symbol() else {
                return Ok(None);
            };
            fetch_yahoo_price(client, symbol)
                .map(|price| price.map(|(p, c)| (p, c, "yahoo".into())))
        }
        AssetKind::Fiat | AssetKind::Custom | AssetKind::Property | AssetKind::Liability => {
            Ok(None)
        }
    }
}

fn yahoo_crypto_usd_price(
    client: &reqwest::blocking::Client,
    asset: &Asset,
) -> anyhow::Result<Option<(Decimal, String, String)>> {
    let Some(symbol) = asset.yahoo_symbol.as_deref() else {
        return Ok(None);
    };
    if let Some((price, currency)) = fetch_yahoo_price(client, symbol)? {
        return Ok(Some((price, currency, "yahoo".into())));
    }
    Ok(None)
}

fn yahoo_conversion_rate(
    client: &reqwest::blocking::Client,
    from: &str,
    to: &str,
    assets: &[Asset],
) -> anyhow::Result<Option<Decimal>> {
    conversion_rate(from, to, assets, |symbol| fetch_yahoo_price(client, symbol))
}

fn conversion_rate(
    from: &str,
    to: &str,
    assets: &[Asset],
    mut fetch: impl FnMut(&str) -> anyhow::Result<Option<(Decimal, String)>>,
) -> anyhow::Result<Option<Decimal>> {
    for asset in assets {
        validate_usd_asset(asset)?;
    }
    if from == to {
        return Ok(Some(Decimal::ONE));
    }
    if !is_fiat(from, assets) {
        return Ok(None);
    }
    if is_fiat(to, assets) {
        return conversion_quote(&format!("{from}{to}=X"), to, &mut fetch);
    }
    let Some(crypto) = assets
        .iter()
        .find(|asset| asset.symbol == to && asset.kind == AssetKind::Crypto)
    else {
        return Ok(None);
    };
    let Some(symbol) = crypto.yahoo_quote_symbol() else {
        return Ok(None);
    };
    // Resolve the USD leg directly. Never re-enter conversion for an intermediary.
    let from_usd = if from == "USD" {
        Decimal::ONE
    } else {
        if !is_fiat("USD", assets) {
            return Ok(None);
        }
        let Some(rate) = conversion_quote(&format!("{from}USD=X"), "USD", &mut fetch)? else {
            return Ok(None);
        };
        rate
    };
    let Some(to_usd) = conversion_quote(symbol, "USD", &mut fetch)? else {
        return Ok(None);
    };
    from_usd.checked_div(to_usd)
        .filter(|rate| *rate > Decimal::ZERO)
        .map(Some)
        .ok_or_else(|| {
            anyhow::anyhow!("conversion from {from} to {to} is outside the supported decimal range or rounds to zero")
        })
}

fn conversion_quote(
    symbol: &str,
    currency: &str,
    fetch: &mut impl FnMut(&str) -> anyhow::Result<Option<(Decimal, String)>>,
) -> anyhow::Result<Option<Decimal>> {
    let Some((price, quoted_currency)) = fetch(symbol)? else {
        return Ok(None);
    };
    anyhow::ensure!(
        price > Decimal::ZERO,
        "conversion quote for {symbol} must be greater than zero"
    );
    anyhow::ensure!(
        quoted_currency == currency,
        "conversion quote for {symbol} must be denominated in {currency}, not {quoted_currency}"
    );
    Ok(Some(price))
}

fn fetch_yahoo_price(
    client: &reqwest::blocking::Client,
    symbol: &str,
) -> anyhow::Result<Option<(Decimal, String)>> {
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?range=1d&interval=1d",
        urlencoding::encode(symbol)
    );
    tracing::debug!(symbol, "fetching Yahoo quote");
    let response = client.get(url).send()?;
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let json: Value = response.error_for_status()?.json()?;
    Ok(parse_yahoo_quote(&json))
}

fn parse_yahoo_quote(json: &Value) -> Option<(Decimal, String)> {
    let meta = json
        .get("chart")?
        .get("result")?
        .as_array()?
        .first()?
        .get("meta")?;
    let price = Decimal::from_f64_retain(meta.get("regularMarketPrice")?.as_f64()?)?;
    let currency = meta.get("currency")?.as_str()?;
    Some((price, currency.to_string()))
}

pub fn merge_price_batch(
    data: &mut StoreData,
    snapshot: &StoreData,
    mut batch: PriceBatch,
) -> SyncSummary {
    let fetched = batch.prices.len();
    batch.prices.retain(|price| {
        let previous = snapshot
            .assets
            .iter()
            .find(|asset| asset.id == price.asset_id);
        let current = data.assets.iter().find(|asset| asset.id == price.asset_id);
        previous
            .zip(current)
            .is_some_and(|(previous, current)| quote_settings_match(previous, current))
    });
    batch.summary.updated = batch.prices.len();
    if fetched != batch.summary.updated {
        tracing::info!(
            discarded = fetched - batch.summary.updated,
            "discarded quotes after asset settings changed during refresh"
        );
    }
    data.prices.extend(batch.prices);
    batch.summary
}

fn quote_settings_match(previous: &Asset, current: &Asset) -> bool {
    previous.kind == current.kind
        && previous.valuation_currency == current.valuation_currency
        && if previous.kind == AssetKind::Fiat {
            previous.symbol == current.symbol
        } else {
            previous.yahoo_quote_symbol() == current.yahoo_quote_symbol()
        }
}

fn is_fiat(symbol: &str, assets: &[Asset]) -> bool {
    assets
        .iter()
        .any(|asset| asset.symbol == symbol && asset.kind == AssetKind::Fiat)
}

fn needs_price(asset: &Asset) -> bool {
    matches!(
        asset.kind,
        AssetKind::Crypto | AssetKind::Stock | AssetKind::Fund | AssetKind::Commodity
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AssetMetadataSource;
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
            metadata_source: AssetMetadataSource::User,
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
            conversion_rate("EUR", "BTC", &missing_usd, no_fetch).unwrap(),
            None
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
    fn keeps_yahoo_native_symbols() {
        let asset = Asset {
            id: 1,
            symbol: "GOLD".into(),
            name: "Gold".into(),
            kind: AssetKind::Commodity,
            yahoo_symbol: Some("GC=F".into()),
            tradingview_symbol: Some("OANDA:XAUUSD".into()),
            valuation_currency: None,
            metadata_source: AssetMetadataSource::Automatic,
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
}
