use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::model::{Asset, AssetKind, Id, Portfolio, Price, StoreData, Transaction};

#[derive(Debug, Clone)]
pub struct HoldingRow {
    pub portfolio_id: Id,
    pub asset_id: Id,
    pub portfolio: String,
    pub symbol: String,
    pub name: String,
    pub kind: AssetKind,
    pub quantity: Decimal,
    pub value: Option<Decimal>,
    pub net_invested: Option<Decimal>,
    pub unrealized_pnl: Option<Decimal>,
    pub missing_valuation: bool,
}

#[derive(Debug, Clone)]
pub struct PortfolioRow {
    pub id: Id,
    pub name: String,
    pub assets: Decimal,
    pub liabilities: Decimal,
    pub net_value: Decimal,
    pub unrealized_pnl: Option<Decimal>,
    pub unresolved: usize,
    pub unresolved_pnl: usize,
}

#[derive(Debug, Clone)]
pub struct Report {
    pub base_currency: String,
    pub total_assets: Decimal,
    pub total_liabilities: Decimal,
    pub net_value: Decimal,
    pub total_unrealized_pnl: Option<Decimal>,
    pub unresolved_pnl: usize,
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
        let value = value_in_base(data, asset, quantity, &base, &latest_prices, &asset_map);
        let net_invested = net_invested(data, portfolio_id, asset_id, &base, &asset_map);
        let unrealized_pnl = value.zip(net_invested).map(|(v, i)| v - i);
        holdings.push(HoldingRow {
            portfolio_id,
            asset_id,
            portfolio: portfolio_name(data, portfolio_id),
            symbol: asset.symbol.clone(),
            name: asset.name.clone(),
            kind: asset.kind,
            quantity,
            value,
            net_invested,
            unrealized_pnl,
            missing_valuation: value.is_none(),
        });
    }

    holdings.sort_by(|a, b| b.value.cmp(&a.value).then_with(|| a.symbol.cmp(&b.symbol)));
    let portfolios = portfolio_rows(data, &holdings);
    let total_assets = portfolios.iter().map(|p| p.assets).sum();
    let total_liabilities = portfolios.iter().map(|p| p.liabilities).sum();
    let net_value = total_assets - total_liabilities;
    let (total_unrealized_pnl, unresolved_pnl) = pnl_total(holdings.iter());
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
        total_unrealized_pnl,
        unresolved_pnl,
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

fn prices_at(data: &StoreData, timestamp: DateTime<Utc>) -> HashMap<(Id, String), &Price> {
    let mut prices = HashMap::new();
    for price in data
        .prices
        .iter()
        .filter(|price| price.timestamp <= timestamp)
    {
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
    data: &StoreData,
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
    if asset.kind == AssetKind::Fiat && price.price <= Decimal::ZERO {
        return None;
    }
    let rate = conversion_rate(&price.currency, base, prices, assets)?;
    let unit_price =
        crate::ledger::split_adjusted_price(data, asset.id, price.timestamp, price.price).ok()?;
    quantity.checked_mul(unit_price)?.checked_mul(rate)
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
        if crypto_usd <= Decimal::ZERO {
            return None;
        }
        return crypto_usd
            .checked_mul(usd_to_target)
            .filter(|rate| *rate > Decimal::ZERO);
    }
    if let Some(crypto) = crypto_asset(to, assets) {
        let from_usd = if from == "USD" {
            Decimal::ONE
        } else {
            fiat_rate(from, "USD", prices, assets)?
        };
        let crypto_usd = prices.get(&(crypto.id, "USD".to_string()))?.price;
        if crypto_usd <= Decimal::ZERO {
            return None;
        }
        return from_usd
            .checked_div(crypto_usd)
            .filter(|rate| *rate > Decimal::ZERO);
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
    if let Some(price) = prices.get(&(from_asset.id, to.to_string()))
        && price.price > Decimal::ZERO
    {
        return Some(price.price);
    }
    let to_asset = assets
        .values()
        .find(|asset| asset.symbol == to && asset.kind == AssetKind::Fiat)?;
    prices
        .get(&(to_asset.id, from.to_string()))
        .filter(|price| price.price > Decimal::ZERO)
        .and_then(|price| Decimal::ONE.checked_div(price.price))
        .filter(|rate| *rate > Decimal::ZERO)
}

fn crypto_asset<'a>(symbol: &str, assets: &'a HashMap<Id, &Asset>) -> Option<&'a Asset> {
    assets
        .values()
        .copied()
        .find(|asset| asset.symbol == symbol && asset.kind == AssetKind::Crypto)
}

enum CostBasisChange {
    Unrecorded,
    Recorded(Decimal),
}

fn net_invested(
    data: &StoreData,
    portfolio_id: Id,
    asset_id: Id,
    base: &str,
    assets: &HashMap<Id, &Asset>,
) -> Option<Decimal> {
    let mut invested = Decimal::ZERO;
    let mut seen = false;
    for transaction in &data.transactions {
        if transaction.portfolio_id != portfolio_id || transaction.base_asset_id != asset_id {
            continue;
        }
        if let CostBasisChange::Recorded(amount) =
            cost_basis_change(data, transaction, base, assets)?
        {
            invested += amount;
            seen = true;
        }
    }
    seen.then_some(invested)
}

fn cost_basis_change(
    data: &StoreData,
    transaction: &Transaction,
    base: &str,
    assets: &HashMap<Id, &Asset>,
) -> Option<CostBasisChange> {
    if transaction.kind.has_implicit_zero_cost_basis() {
        return Some(CostBasisChange::Recorded(Decimal::ZERO));
    }
    if !transaction.kind.supports_quote()
        || transaction.quote_asset_id.is_none()
        || transaction.quote_amount.is_none()
    {
        return Some(CostBasisChange::Unrecorded);
    }
    let value = transaction_quote_value(data, transaction, base, assets)?;
    Some(CostBasisChange::Recorded(
        -transaction.kind.quote_cash_delta(value),
    ))
}

fn transaction_quote_value(
    data: &StoreData,
    transaction: &Transaction,
    base: &str,
    assets: &HashMap<Id, &Asset>,
) -> Option<Decimal> {
    let amount = transaction.quote_amount?;
    let asset = assets.get(&transaction.quote_asset_id?)?;
    if amount.is_zero() {
        return Some(Decimal::ZERO);
    }
    let historical_prices = prices_at(data, transaction.timestamp);
    let latest_prices = latest_prices(data);
    let rate = conversion_rate(&asset.symbol, base, &historical_prices, assets)
        .or_else(|| conversion_rate(&asset.symbol, base, &latest_prices, assets))?;
    Some(amount * rate)
}

fn portfolio_rows(data: &StoreData, holdings: &[HoldingRow]) -> Vec<PortfolioRow> {
    data.portfolios
        .iter()
        .map(|portfolio| portfolio_row(portfolio, holdings))
        .collect()
}

fn portfolio_row(portfolio: &Portfolio, holdings: &[HoldingRow]) -> PortfolioRow {
    let rows = holdings
        .iter()
        .filter(|holding| holding.portfolio_id == portfolio.id);
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
    let (unrealized_pnl, unresolved_pnl) = pnl_total(rows.clone());
    PortfolioRow {
        id: portfolio.id,
        name: portfolio.name.clone(),
        assets,
        liabilities,
        net_value: assets - liabilities,
        unrealized_pnl,
        unresolved: rows.filter(|holding| holding.missing_valuation).count(),
        unresolved_pnl,
    }
}

fn portfolio_name(data: &StoreData, id: Id) -> String {
    data.portfolios
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.name.clone())
        .unwrap_or_default()
}

fn pnl_total<'a>(holdings: impl Iterator<Item = &'a HoldingRow>) -> (Option<Decimal>, usize) {
    let mut total = Decimal::ZERO;
    let mut known = false;
    let mut unresolved = 0;
    for holding in holdings {
        if let Some(pnl) = holding.unrealized_pnl {
            total += pnl;
            known = true;
        } else if !matches!(holding.kind, AssetKind::Fiat | AssetKind::Liability) {
            // Cash and debt do not require acquisition basis for this investment metric.
            unresolved += 1;
        }
    }
    ((known || unresolved == 0).then_some(total), unresolved)
}

pub fn now() -> DateTime<Utc> {
    Utc::now()
}
