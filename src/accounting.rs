use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::model::{Asset, AssetKind, Id, Price, StoreData, TransactionKind};

#[derive(Debug, Clone)]
pub struct HoldingRow {
    pub portfolio: String,
    pub symbol: String,
    pub name: String,
    pub kind: AssetKind,
    pub quantity: Decimal,
    pub value: Option<Decimal>,
    pub net_invested: Option<Decimal>,
    pub unrealized_pnl: Option<Decimal>,
    pub stale_price: bool,
}

#[derive(Debug, Clone)]
pub struct PortfolioRow {
    pub name: String,
    pub assets: Decimal,
    pub liabilities: Decimal,
    pub net_value: Decimal,
    pub value: Decimal,
    pub unresolved: usize,
}

#[derive(Debug, Clone)]
pub struct Report {
    pub base_currency: String,
    pub total_assets: Decimal,
    pub total_liabilities: Decimal,
    pub net_value: Decimal,
    pub total_value: Decimal,
    pub total_unrealized_pnl: Decimal,
    pub portfolios: Vec<PortfolioRow>,
    pub holdings: Vec<HoldingRow>,
    pub negative_balances: Vec<HoldingRow>,
}

pub fn build_report(data: &StoreData) -> Report {
    let base = data.config.selected_base_currency.clone();
    let asset_map = asset_map(data);
    let balances = balances(data);
    let latest_prices = latest_prices(data);
    let mut holdings = Vec::new();

    for ((portfolio_id, asset_id), quantity) in balances {
        if quantity.is_zero() {
            continue;
        }
        let Some(asset) = asset_map.get(&asset_id) else {
            continue;
        };
        let value = value_in_base(asset, quantity, &base, &latest_prices, &asset_map);
        let net_invested = net_invested(
            data,
            portfolio_id,
            asset_id,
            &base,
            &latest_prices,
            &asset_map,
        );
        let unrealized_pnl = value.zip(net_invested).map(|(v, i)| v - i);
        holdings.push(HoldingRow {
            portfolio: portfolio_name(data, portfolio_id),
            symbol: asset.symbol.clone(),
            name: asset.name.clone(),
            kind: asset.kind,
            quantity,
            value,
            net_invested,
            unrealized_pnl,
            stale_price: value.is_none() && !is_cash_like(asset, &base),
        });
    }

    holdings.sort_by(|a, b| b.value.cmp(&a.value).then_with(|| a.symbol.cmp(&b.symbol)));
    let portfolios = portfolio_rows(data, &holdings);
    let total_assets = portfolios.iter().map(|p| p.assets).sum();
    let total_liabilities = portfolios.iter().map(|p| p.liabilities).sum();
    let net_value = total_assets - total_liabilities;
    let total_value = net_value;
    let total_unrealized_pnl = holdings.iter().filter_map(|h| h.unrealized_pnl).sum();
    let negative_balances = holdings
        .iter()
        .filter(|h| h.kind != AssetKind::Liability && h.quantity < Decimal::ZERO)
        .cloned()
        .collect();

    Report {
        base_currency: base,
        total_assets,
        total_liabilities,
        net_value,
        total_value,
        total_unrealized_pnl,
        portfolios,
        holdings,
        negative_balances,
    }
}

fn asset_map(data: &StoreData) -> HashMap<Id, &Asset> {
    data.assets.iter().map(|asset| (asset.id, asset)).collect()
}

fn balances(data: &StoreData) -> BTreeMap<(Id, Id), Decimal> {
    let mut balances = BTreeMap::new();
    for entry in &data.ledger_entries {
        *balances
            .entry((entry.portfolio_id, entry.asset_id))
            .or_insert(Decimal::ZERO) += entry.quantity_delta;
    }
    balances
}

fn latest_prices(data: &StoreData) -> HashMap<(Id, String), &Price> {
    let mut prices = HashMap::new();
    for price in &data.prices {
        let key = (price.asset_id, price.currency.clone());
        let should_replace = prices
            .get(&key)
            .map(|current: &&Price| current.timestamp < price.timestamp)
            .unwrap_or(true);
        if should_replace {
            prices.insert(key, price);
        }
    }
    prices
}

fn value_in_base(
    asset: &Asset,
    quantity: Decimal,
    base: &str,
    prices: &HashMap<(Id, String), &Price>,
    assets: &HashMap<Id, &Asset>,
) -> Option<Decimal> {
    if asset.symbol == base {
        return Some(quantity);
    }
    if let Some(currency) = &asset.valuation_currency {
        let rate = conversion_rate(currency, base, prices, assets)?;
        return Some(quantity * rate);
    }
    let price = price_for_asset(asset.id, base, prices)?;
    let rate = conversion_rate(&price.currency, base, prices, assets)?;
    Some(quantity * price.price * rate)
}

fn price_for_asset<'a>(
    asset_id: Id,
    base: &str,
    prices: &'a HashMap<(Id, String), &Price>,
) -> Option<&'a Price> {
    prices
        .get(&(asset_id, base.to_string()))
        .copied()
        .or_else(|| {
            prices
                .iter()
                .filter(|((id, _), _)| *id == asset_id)
                .map(|(_, price)| *price)
                .max_by_key(|price| price.timestamp)
        })
}

fn conversion_rate(
    from: &str,
    to: &str,
    prices: &HashMap<(Id, String), &Price>,
    assets: &HashMap<Id, &Asset>,
) -> Option<Decimal> {
    if from == to {
        return Some(Decimal::ONE);
    }
    if let Some(rate) = fiat_rate(from, to, prices, assets) {
        return Some(rate);
    }
    if let Some(crypto) = crypto_asset(from, assets) {
        let crypto_usd = prices.get(&(crypto.id, "USD".to_string()))?.price;
        let usd_to_target = if to == "USD" {
            Decimal::ONE
        } else {
            fiat_rate("USD", to, prices, assets)?
        };
        return Some(crypto_usd * usd_to_target);
    }
    if let Some(crypto) = crypto_asset(to, assets) {
        let from_usd = if from == "USD" {
            Decimal::ONE
        } else {
            fiat_rate(from, "USD", prices, assets)?
        };
        let crypto_usd = prices.get(&(crypto.id, "USD".to_string()))?.price;
        return Some(from_usd / crypto_usd);
    }
    None
}

fn fiat_rate(
    from: &str,
    to: &str,
    prices: &HashMap<(Id, String), &Price>,
    assets: &HashMap<Id, &Asset>,
) -> Option<Decimal> {
    if from == to {
        return Some(Decimal::ONE);
    }
    let from_asset = assets
        .values()
        .find(|asset| asset.symbol == from && asset.kind == AssetKind::Fiat)?;
    if let Some(price) = prices.get(&(from_asset.id, to.to_string())) {
        return Some(price.price);
    }
    let to_asset = assets
        .values()
        .find(|asset| asset.symbol == to && asset.kind == AssetKind::Fiat)?;
    prices
        .get(&(to_asset.id, from.to_string()))
        .map(|price| Decimal::ONE / price.price)
}

fn crypto_asset<'a>(symbol: &str, assets: &'a HashMap<Id, &Asset>) -> Option<&'a Asset> {
    assets
        .values()
        .copied()
        .find(|asset| asset.symbol == symbol && asset.kind == AssetKind::Crypto)
}

fn net_invested(
    data: &StoreData,
    portfolio_id: Id,
    asset_id: Id,
    base: &str,
    prices: &HashMap<(Id, String), &Price>,
    assets: &HashMap<Id, &Asset>,
) -> Option<Decimal> {
    let mut invested = Decimal::ZERO;
    let mut seen = false;
    for transaction in &data.transactions {
        if transaction.portfolio_id != portfolio_id || transaction.base_asset_id != asset_id {
            continue;
        }
        let Some(quote_asset_id) = transaction.quote_asset_id else {
            continue;
        };
        let quote_asset = assets.get(&quote_asset_id)?;
        let rate = conversion_rate(&quote_asset.symbol, base, prices, assets)?;
        let Some(quote_amount) = transaction.quote_amount else {
            continue;
        };
        match transaction.kind {
            TransactionKind::Buy => invested += quote_amount * rate,
            TransactionKind::Sell => invested -= quote_amount * rate,
            TransactionKind::Deposit
            | TransactionKind::Withdraw
            | TransactionKind::AssetIncrease
            | TransactionKind::AssetDecrease
            | TransactionKind::LiabilityIncrease
            | TransactionKind::LiabilityDecrease => {}
        }
        seen = true;
    }
    seen.then_some(invested)
}

fn portfolio_rows(data: &StoreData, holdings: &[HoldingRow]) -> Vec<PortfolioRow> {
    data.portfolios
        .iter()
        .map(|portfolio| {
            let rows = holdings
                .iter()
                .filter(|holding| holding.portfolio == portfolio.name);
            let assets = rows
                .clone()
                .filter(|holding| holding.kind != AssetKind::Liability)
                .filter_map(|holding| holding.value)
                .sum();
            let liabilities = rows
                .clone()
                .filter(|holding| holding.kind == AssetKind::Liability)
                .filter_map(|holding| holding.value)
                .sum();
            let net_value = assets - liabilities;
            let unresolved = rows.filter(|h| h.stale_price).count();
            PortfolioRow {
                name: portfolio.name.clone(),
                assets,
                liabilities,
                net_value,
                value: net_value,
                unresolved,
            }
        })
        .collect()
}

fn portfolio_name(data: &StoreData, id: Id) -> String {
    data.portfolios
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.name.clone())
        .unwrap_or_default()
}

fn is_cash_like(asset: &Asset, base: &str) -> bool {
    asset.kind == AssetKind::Fiat || asset.kind == AssetKind::Liability || asset.symbol == base
}

pub fn now() -> DateTime<Utc> {
    Utc::now()
}
