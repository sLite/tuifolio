use std::collections::BTreeMap;

use rust_decimal::Decimal;

use super::query::PageQuery;
use crate::{
    accounting::{HoldingRow, PortfolioRow, Report},
    model::{AssetKind, Id, LedgerEffect, StoreData, Transaction},
};

pub(super) struct Amount {
    pub text: String,
    pub tone: &'static str,
}

impl Amount {
    pub fn new(value: Option<Decimal>, precision: u32) -> Self {
        let Some(value) = value else {
            return Self {
                text: "Unavailable".into(),
                tone: "muted",
            };
        };
        Self {
            text: grouped_amount(value, precision),
            tone: amount_tone(value),
        }
    }
}

fn grouped_amount(value: Decimal, precision: u32) -> String {
    let text = format!("{:.*}", precision as usize, value.round_dp(precision));
    let (integer, fraction) = text.split_once('.').unwrap_or((&text, ""));
    let digits = integer.trim_start_matches('-');
    let grouped = digits
        .chars()
        .enumerate()
        .fold(String::new(), |mut result, (index, digit)| {
            if index > 0 && (digits.len() - index) % 3 == 0 {
                result.push(',');
            }
            result.push(digit);
            result
        });
    let sign = if value.is_sign_negative() { "-" } else { "" };
    format!("{sign}{grouped}.{fraction}")
}

fn amount_tone(value: Decimal) -> &'static str {
    if value < Decimal::ZERO {
        "negative"
    } else if value > Decimal::ZERO {
        "positive"
    } else {
        "muted"
    }
}

pub(super) fn precision(data: &StoreData) -> u32 {
    if data
        .assets
        .iter()
        .any(|a| a.symbol == data.config.selected_base_currency && a.kind == AssetKind::Crypto)
        || matches!(data.config.selected_base_currency.as_str(), "BTC" | "ETH")
    {
        8
    } else {
        2
    }
}

pub(super) struct Summary {
    pub assets: Amount,
    pub liabilities: Amount,
    pub net: Amount,
    pub pnl: Amount,
    pub missing: usize,
    pub missing_pnl: usize,
    pub realized: Amount,
    pub missing_realized: usize,
    pub negative: usize,
}

impl Summary {
    pub fn new(report: &Report, precision: u32) -> Self {
        Self {
            assets: Amount::new(Some(report.total_assets), precision),
            liabilities: Amount::new(Some(report.total_liabilities), precision),
            net: Amount::new(Some(report.net_value), precision),
            pnl: Amount::new(report.total_unrealized_pnl, precision),
            missing: report
                .holdings
                .iter()
                .filter(|h| h.missing_valuation)
                .count(),
            missing_pnl: report.unresolved_pnl,
            realized: Amount::new(report.total_realized_pnl, precision),
            missing_realized: report.unresolved_realized_pnl,
            negative: report.negative_balances.len(),
        }
    }
}

pub(super) struct HoldingView {
    pub asset_id: Id,
    pub symbol: String,
    pub name: String,
    pub kind: &'static str,
    pub portfolio: String,
    pub quantity: String,
    pub value: Amount,
    pub invested: Amount,
    pub basis: Amount,
    pub pnl: Amount,
    pub missing: bool,
    pub negative: bool,
    pub transactions_url: String,
    pub chart_url: Option<String>,
    pub add_url: String,
}

impl HoldingView {
    pub fn new(holding: &HoldingRow, data: &StoreData) -> Self {
        let precision = precision(data);
        Self {
            asset_id: holding.asset_id,
            symbol: holding.symbol.clone(),
            name: holding.name.clone(),
            kind: asset_kind(holding.kind),
            portfolio: holding.portfolio.clone(),
            quantity: holding.quantity.normalize().to_string(),
            value: Amount::new(holding.value, precision),
            invested: Amount::new(holding.net_invested, precision),
            basis: Amount::new(holding.remaining_cost_basis, precision),
            pnl: Amount::new(holding.unrealized_pnl, precision),
            missing: holding.missing_valuation,
            negative: holding.kind != AssetKind::Liability && holding.quantity < Decimal::ZERO,
            transactions_url: holding_url("/transactions", holding),
            add_url: holding_url("/transactions/new", holding),
            chart_url: chart_url(data, holding.asset_id),
        }
    }
}

fn holding_url(route: &str, holding: &HoldingRow) -> String {
    format!(
        "{route}?portfolio={}&asset={}",
        holding.portfolio_id, holding.asset_id
    )
}

pub(super) fn chart_url(data: &StoreData, asset_id: Id) -> Option<String> {
    data.assets
        .iter()
        .find(|a| a.id == asset_id)
        .and_then(|a| a.tradingview_symbol.as_ref())
        .map(|symbol| {
            format!(
                "https://www.tradingview.com/chart/?symbol={}",
                urlencoding::encode(symbol)
            )
        })
}

pub(super) fn holdings(report: &Report, data: &StoreData, query: &PageQuery) -> Vec<HoldingView> {
    let search = query.search.to_lowercase();
    report
        .holdings
        .iter()
        .filter(|h| query.portfolio.is_none_or(|id| h.portfolio_id == id))
        .filter(|h| {
            format!("{} {} {}", h.symbol, h.name, h.portfolio)
                .to_lowercase()
                .contains(&search)
        })
        .map(|h| HoldingView::new(h, data))
        .collect()
}

pub(super) struct PortfolioView {
    pub id: Id,
    pub name: String,
    pub assets: Amount,
    pub liabilities: Amount,
    pub net: Amount,
    pub pnl: Amount,
    pub missing: usize,
    pub missing_pnl: usize,
    pub realized: Amount,
    pub missing_realized: usize,
}

impl PortfolioView {
    pub fn new(portfolio: &PortfolioRow, precision: u32) -> Self {
        Self {
            id: portfolio.id,
            name: portfolio.name.clone(),
            assets: Amount::new(Some(portfolio.assets), precision),
            liabilities: Amount::new(Some(portfolio.liabilities), precision),
            net: Amount::new(Some(portfolio.net_value), precision),
            pnl: Amount::new(portfolio.unrealized_pnl, precision),
            missing: portfolio.unresolved,
            missing_pnl: portfolio.unresolved_pnl,
            realized: Amount::new(portfolio.realized_pnl, precision),
            missing_realized: portfolio.unresolved_realized_pnl,
        }
    }
}

pub(super) struct RealizedView {
    pub portfolio: String,
    pub symbol: String,
    pub pnl: Amount,
}

pub(super) fn realized(
    report: &Report,
    portfolio: Option<Id>,
    precision: u32,
) -> Vec<RealizedView> {
    report
        .realized_returns
        .iter()
        .filter(|r| portfolio.is_none_or(|id| id == r.portfolio_id))
        .map(|r| RealizedView {
            portfolio: r.portfolio.clone(),
            symbol: r.symbol.clone(),
            pnl: Amount::new(r.pnl, precision),
        })
        .collect()
}

pub(super) struct TransactionView {
    pub id: Id,
    pub date: String,
    pub time: String,
    pub portfolio: String,
    pub kind: &'static str,
    pub base: String,
    pub quote: String,
    pub cash_effect: &'static str,
    pub fee: String,
    pub exchange: String,
    pub broker: String,
    pub notes: String,
}

impl TransactionView {
    pub fn new(transaction: &Transaction, data: &StoreData) -> Self {
        Self {
            id: transaction.id,
            date: transaction.timestamp.format("%d %b %Y").to_string(),
            time: transaction.timestamp.format("%H:%M UTC").to_string(),
            portfolio: portfolio_name(data, transaction.portfolio_id),
            kind: transaction_kind(transaction.kind),
            base: asset_amount(
                data,
                Some(transaction.base_asset_id),
                Some(transaction.base_amount),
            ),
            quote: transaction_quote(transaction, data),
            cash_effect: cash_effect(transaction),
            fee: asset_amount(data, transaction.fee_asset_id, transaction.fee_amount),
            exchange: transaction.exchange.clone().unwrap_or_default(),
            broker: transaction.broker.clone().unwrap_or_default(),
            notes: transaction.notes.clone().unwrap_or_default(),
        }
    }

    pub fn matches_search(&self, search: &str) -> bool {
        format!(
            "{} {} {} {} {} {} {} {}",
            self.date,
            self.portfolio,
            self.kind,
            self.base,
            self.quote,
            self.exchange,
            self.broker,
            self.notes
        )
        .to_lowercase()
        .contains(search)
    }
}

fn portfolio_name(data: &StoreData, id: Id) -> String {
    data.portfolios
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.name.clone())
        .unwrap_or_default()
}

fn cash_effect(transaction: &Transaction) -> &'static str {
    if transaction.kind.has_implicit_zero_cost_basis() {
        "Zero cost basis"
    } else if !transaction.kind.supports_quote() || transaction.quote_amount.is_none() {
        ""
    } else if transaction.quote_ledger_effect == LedgerEffect::Ignore {
        "Cost basis only"
    } else {
        "Cash posted"
    }
}

fn transaction_quote(transaction: &Transaction, data: &StoreData) -> String {
    if transaction.kind.has_implicit_zero_cost_basis() {
        "0".into()
    } else {
        asset_amount(data, transaction.quote_asset_id, transaction.quote_amount)
    }
}

fn asset_amount(data: &StoreData, id: Option<Id>, amount: Option<Decimal>) -> String {
    amount
        .zip(id)
        .map(|(amount, id)| {
            let symbol = data
                .assets
                .iter()
                .find(|a| a.id == id)
                .map(|a| a.symbol.as_str())
                .unwrap_or("Unknown");
            format!("{} {symbol}", amount.normalize())
        })
        .unwrap_or_default()
}

pub(super) struct PriceView {
    pub asset_id: Id,
    pub value: String,
    pub currency: String,
    pub source: String,
    pub timestamp: String,
    pub missing: bool,
}

pub(super) fn prices(data: &StoreData) -> Vec<PriceView> {
    let latest = latest_prices(data);
    let mut assets = data.assets.iter().collect::<Vec<_>>();
    assets.sort_by(|a, b| a.symbol.cmp(&b.symbol));
    assets
        .into_iter()
        .flat_map(|asset| asset_prices(asset, data, &latest))
        .collect()
}

type LatestPrices<'a> = BTreeMap<(Id, &'a str), &'a crate::model::Price>;

fn latest_prices(data: &StoreData) -> LatestPrices<'_> {
    let mut latest = BTreeMap::new();
    for price in &data.prices {
        let current = latest
            .entry((price.asset_id, price.currency.as_str()))
            .or_insert(price);
        if current.timestamp < price.timestamp {
            *current = price;
        }
    }
    latest
}

fn asset_prices(
    asset: &crate::model::Asset,
    data: &StoreData,
    latest: &LatestPrices<'_>,
) -> Vec<PriceView> {
    let quotes = latest
        .iter()
        .filter(|((id, _), _)| *id == asset.id)
        .map(|(_, price)| *price)
        .collect::<Vec<_>>();
    if quotes.is_empty() {
        return vec![unquoted_asset(asset, data)];
    }
    quotes
        .into_iter()
        .map(|price| PriceView {
            asset_id: asset.id,
            value: price.price.normalize().to_string(),
            currency: price.currency.clone(),
            source: price.source.clone(),
            timestamp: price.timestamp.format("%d %b %Y, %H:%M UTC").to_string(),
            missing: false,
        })
        .collect()
}

fn unquoted_asset(asset: &crate::model::Asset, data: &StoreData) -> PriceView {
    let currency = asset.valuation_currency.clone().or_else(|| {
        (asset.symbol == data.config.selected_base_currency).then(|| asset.symbol.clone())
    });
    PriceView {
        asset_id: asset.id,
        value: currency
            .as_ref()
            .map(|_| "1".into())
            .unwrap_or_else(|| "Unavailable".into()),
        source: if asset.valuation_currency.is_some() {
            "Intrinsic value".into()
        } else if currency.is_some() {
            "Base currency".into()
        } else {
            "No quote".into()
        },
        timestamp: String::new(),
        missing: currency.is_none(),
        currency: currency.unwrap_or_default(),
    }
}

pub(super) fn asset_kind(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Fiat => "Cash",
        AssetKind::Crypto => "Crypto",
        AssetKind::Stock => "Stock",
        AssetKind::Fund => "Fund",
        AssetKind::Commodity => "Commodity",
        AssetKind::Property => "Property",
        AssetKind::Custom => "Custom",
        AssetKind::Liability => "Liability",
    }
}

pub(super) fn transaction_kind(kind: crate::model::TransactionKind) -> &'static str {
    use crate::model::TransactionKind::*;
    match kind {
        Buy => "Buy",
        Sell => "Sell",
        Gift => "Gift",
        Deposit => "Deposit",
        Withdraw => "Withdraw",
        AssetIncrease => "Asset increase",
        AssetDecrease => "Asset decrease",
        LiabilityIncrease => "Liability increase",
        LiabilityDecrease => "Liability decrease",
    }
}
