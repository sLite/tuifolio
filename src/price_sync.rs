use reqwest::StatusCode;
use rust_decimal::Decimal;
use serde_json::Value;

use crate::{
    accounting::now,
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
    if from == to {
        return Ok(Some(Decimal::ONE));
    }
    if is_fiat(from, assets) && is_fiat(to, assets) {
        return fetch_yahoo_price(client, &format!("{from}{to}=X"))
            .map(|quote| quote.map(|(price, _)| price));
    }
    if let Some(crypto) = assets
        .iter()
        .find(|asset| asset.symbol == to && asset.kind == AssetKind::Crypto)
    {
        let Some(from_usd) = yahoo_conversion_rate(client, from, "USD", assets)? else {
            return Ok(None);
        };
        let Some((to_usd, _, _)) = yahoo_crypto_usd_price(client, crypto)? else {
            return Ok(None);
        };
        return Ok(Some(from_usd / to_usd));
    }
    Ok(None)
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
