use reqwest::StatusCode;
use rust_decimal::Decimal;
use serde_json::Value;

use crate::{
    assets::validate_usd_asset,
    model::{Asset, AssetKind, Id, Price, StoreData},
    price_history::compact_price_history,
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
    let timestamp = chrono::Utc::now();
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

pub fn add_manual_price_for_asset(
    store: &mut Store,
    asset_id: Id,
    price: Decimal,
    currency: &str,
) -> anyhow::Result<()> {
    add_manual_price_for_asset_at(store, asset_id, price, currency, chrono::Utc::now())
}

pub fn add_manual_price_for_asset_at(
    store: &mut Store,
    asset_id: Id,
    price: Decimal,
    currency: &str,
    observed_at: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        observed_at <= chrono::Utc::now(),
        "price observation time cannot be in the future"
    );
    anyhow::ensure!(!currency.trim().is_empty(), "price currency is required");
    let asset = store
        .data
        .assets
        .iter()
        .find(|asset| asset.id == asset_id)
        .ok_or_else(|| anyhow::anyhow!("unknown asset"))?;
    if asset.kind == AssetKind::Fiat {
        anyhow::ensure!(
            price > Decimal::ZERO,
            "price must be greater than zero for Cash exchange rates"
        );
    } else {
        anyhow::ensure!(price >= Decimal::ZERO, "price cannot be negative");
    }
    anyhow::ensure!(
        asset.allows_manual_pricing(),
        "clear the Yahoo symbol and save the asset before recording manual prices"
    );
    store.data.prices.push(Price {
        asset_id,
        timestamp: observed_at,
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
    // arbitrary_precision preserves the JSON number's decimal spelling.
    let raw = meta.get("regularMarketPrice")?.as_number()?.to_string();
    let price = Decimal::from_str_exact(&raw)
        .or_else(|_| Decimal::from_scientific(&raw))
        .ok()?;
    let currency = meta.get("currency")?.as_str()?.trim();
    normalize_yahoo_units(price, currency)
}

fn normalize_yahoo_units(price: Decimal, currency: &str) -> Option<(Decimal, String)> {
    if price < Decimal::ZERO {
        return None;
    }
    let (price, currency) = match currency {
        "GBp" | "GBX" => {
            let normalized = price.checked_div(Decimal::from(100))?;
            if !price.is_zero() && normalized.is_zero() {
                return None;
            }
            (normalized, "GBP".to_string())
        }
        _ => (price, currency.to_ascii_uppercase()),
    };
    if !crate::currencies::is_fiat_code(&currency) && !matches!(currency.as_str(), "BTC" | "ETH") {
        return None;
    }
    Some((price, currency))
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
    let removed = compact_price_history(data, chrono::Utc::now());
    if removed > 0 {
        tracing::info!(
            removed,
            remaining = data.prices.len(),
            "compacted daily price history"
        );
    }
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
    if crate::currencies::is_fiat_code(&symbol.trim().to_ascii_uppercase()) {
        return true;
    }
    crate::currencies::currency_asset_from(assets.iter(), symbol)
        .is_some_and(|asset| asset.kind == AssetKind::Fiat)
}

fn needs_price(asset: &Asset) -> bool {
    matches!(
        asset.kind,
        AssetKind::Crypto | AssetKind::Stock | AssetKind::Fund | AssetKind::Commodity
    )
}

#[cfg(test)]
mod tests;
