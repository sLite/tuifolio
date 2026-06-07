use reqwest::StatusCode;
use rust_decimal::Decimal;
use serde_json::Value;

use crate::{
    accounting::now,
    model::{Asset, AssetKind, Price},
    store::Store,
};

pub struct SyncSummary {
    pub updated: usize,
    pub unsupported: usize,
}

pub fn sync_free_crypto_prices(store: &mut Store) -> anyhow::Result<SyncSummary> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("tuifolio/0.1 local portfolio tracker")
        .build()?;
    let assets = store.data.assets.clone();
    let bases = store.data.config.base_currencies.clone();
    let timestamp = now();
    let mut updated = 0;
    let mut unsupported = 0;

    for asset in assets.iter().cloned() {
        let mut asset_updates = 0;
        if asset.kind == AssetKind::Fiat {
            for base in &bases {
                let price = if asset.symbol == *base {
                    Some(Decimal::ONE)
                } else {
                    yahoo_conversion_rate(&client, &asset.symbol, base, &assets)?
                };
                if let Some(price) = price {
                    push_price(store, asset.id, timestamp, price, base, "yahoo");
                    asset_updates += 1;
                    updated += 1;
                }
            }
        } else if let Some((price, currency, source)) = yahoo_asset_native_price(&client, &asset)? {
            push_price(store, asset.id, timestamp, price, &currency, &source);
            asset_updates += 1;
            updated += 1;
        }

        if needs_price(&asset) && asset_updates == 0 {
            unsupported += 1;
        }
    }

    Ok(SyncSummary {
        updated,
        unsupported,
    })
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
    store.data.prices.push(Price {
        asset_id,
        timestamp: now(),
        price,
        currency: currency.to_string(),
        source: "manual".into(),
    });
    Ok(())
}

fn push_price(
    store: &mut Store,
    asset_id: u64,
    timestamp: chrono::DateTime<chrono::Utc>,
    price: Decimal,
    currency: &str,
    source: &str,
) {
    store.data.prices.push(Price {
        asset_id,
        timestamp,
        price,
        currency: currency.to_string(),
        source: source.to_string(),
    });
}

fn yahoo_asset_native_price(
    client: &reqwest::blocking::Client,
    asset: &Asset,
) -> anyhow::Result<Option<(Decimal, String, String)>> {
    match asset.kind {
        AssetKind::Crypto => yahoo_crypto_usd_price(client, asset),
        AssetKind::Stock | AssetKind::Fund | AssetKind::Commodity => {
            fetch_yahoo_price(client, yahoo_market_symbol(asset))
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
    let url =
        format!("https://query1.finance.yahoo.com/v8/finance/chart/{symbol}?range=1d&interval=1d");
    let response = client.get(url).send()?;
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let json: Value = response.error_for_status()?.json()?;
    let Some(result) = json
        .get("chart")
        .and_then(|chart| chart.get("result"))
        .and_then(|result| result.as_array())
        .and_then(|results| results.first())
    else {
        return Ok(None);
    };
    let Some(meta) = result.get("meta") else {
        return Ok(None);
    };
    let Some(price) = meta
        .get("regularMarketPrice")
        .and_then(|price| price.as_f64())
        .and_then(Decimal::from_f64_retain)
    else {
        return Ok(None);
    };
    let Some(currency) = meta.get("currency").and_then(|currency| currency.as_str()) else {
        return Ok(None);
    };
    Ok(Some((price, currency.to_string())))
}

fn yahoo_market_symbol(asset: &Asset) -> &str {
    asset.yahoo_symbol.as_deref().unwrap_or(&asset.symbol)
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
        assert_eq!(yahoo_market_symbol(&asset), "GC=F");
    }
}
