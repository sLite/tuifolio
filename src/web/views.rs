use askama::Template;

use super::{
    forms::TransactionForm,
    query::{AssetScope, PAGE_SIZE, PageQuery},
    tables::{HoldingView, PortfolioView, Summary, TransactionView},
};
use crate::model::{Id, StoreData, Transaction, TransactionKind};

pub(super) struct Common {
    pub title: String,
    pub section: &'static str,
    pub base: String,
    pub currencies: Vec<String>,
    pub return_to: String,
}

impl Common {
    pub fn new(
        data: &StoreData,
        title: impl Into<String>,
        section: &'static str,
        return_to: String,
    ) -> Self {
        Self {
            title: title.into(),
            section,
            base: data.config.selected_base_currency.clone(),
            currencies: data.config.base_currencies.clone(),
            return_to,
        }
    }
}

pub(super) struct Choice {
    pub value: String,
    pub label: String,
    pub selected: bool,
}

pub(super) fn portfolio_choices(data: &StoreData, query: &PageQuery) -> Vec<Choice> {
    let mut portfolios = data.portfolios.iter().collect::<Vec<_>>();
    portfolios.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    portfolios
        .into_iter()
        .map(|p| Choice {
            value: p.id.to_string(),
            label: portfolio_choice_label(data, p),
            selected: Some(p.id) == query.portfolio,
        })
        .collect()
}

fn portfolio_choice_label(data: &StoreData, portfolio: &crate::model::Portfolio) -> String {
    if data
        .portfolios
        .iter()
        .filter(|other| other.name.eq_ignore_ascii_case(&portfolio.name))
        .count()
        > 1
    {
        format!("{} · #{}", portfolio.name, portfolio.id)
    } else {
        portfolio.name.clone()
    }
}

pub(super) fn asset_choices(data: &StoreData, selected: Option<u64>) -> Vec<Choice> {
    let mut assets = data.assets.iter().collect::<Vec<_>>();
    assets.sort_by(|a, b| a.symbol.cmp(&b.symbol));
    assets
        .into_iter()
        .map(|a| Choice {
            value: a.id.to_string(),
            label: asset_choice_label(data, a),
            selected: Some(a.id) == selected,
        })
        .collect()
}

fn asset_choice_label(data: &StoreData, asset: &crate::model::Asset) -> String {
    let label = format!(
        "{} · {} · {}",
        asset.symbol,
        asset.name,
        super::tables::asset_kind(asset.kind)
    );
    if data
        .assets
        .iter()
        .filter(|other| other.symbol.eq_ignore_ascii_case(&asset.symbol))
        .count()
        > 1
    {
        format!("{label} · #{}", asset.id)
    } else {
        label
    }
}

pub(super) fn enum_choices(values: &[(&str, &str)], selected: &str) -> Vec<Choice> {
    values
        .iter()
        .map(|(value, label)| Choice {
            value: value.to_string(),
            label: label.to_string(),
            selected: *value == selected,
        })
        .collect()
}

pub(super) const ASSET_KINDS: &[(&str, &str)] = &[
    ("Fiat", "Cash"),
    ("Crypto", "Crypto"),
    ("Stock", "Stock"),
    ("Fund", "Fund"),
    ("Commodity", "Commodity"),
    ("Property", "Property"),
    ("Custom", "Custom"),
    ("Liability", "Liability"),
];
const TRANSACTION_KINDS: &[(TransactionKind, &str)] = &[
    (TransactionKind::Buy, "Buy"),
    (TransactionKind::Sell, "Sell"),
    (TransactionKind::Gift, "Gift"),
    (TransactionKind::Deposit, "Deposit"),
    (TransactionKind::Withdraw, "Withdraw"),
    (TransactionKind::AssetIncrease, "Asset increase"),
    (TransactionKind::AssetDecrease, "Asset decrease"),
    (TransactionKind::LiabilityIncrease, "Liability increase"),
    (TransactionKind::LiabilityDecrease, "Liability decrease"),
];

pub(super) struct TransactionKindChoice {
    pub value: String,
    pub label: &'static str,
    pub selected: bool,
    pub supports_quote: bool,
    pub requires_quote: bool,
    pub implicit_zero_cost_basis: bool,
}

fn transaction_kind_choices(selected: &str) -> Vec<TransactionKindChoice> {
    TRANSACTION_KINDS
        .iter()
        .map(|(kind, label)| {
            let value = format!("{kind:?}");
            TransactionKindChoice {
                selected: value == selected,
                value,
                label,
                supports_quote: kind.supports_quote(),
                requires_quote: kind.requires_quote(),
                implicit_zero_cost_basis: kind.has_implicit_zero_cost_basis(),
            }
        })
        .collect()
}

#[derive(Template)]
#[template(path = "overview.html")]
pub(super) struct OverviewPage {
    pub common: Common,
    pub summary: Summary,
    pub holdings: Vec<HoldingView>,
    pub portfolios: Vec<Choice>,
    pub search: String,
    pub empty_store: bool,
}

#[derive(Template)]
#[template(path = "portfolios.html")]
pub(super) struct PortfoliosPage {
    pub common: Common,
    pub portfolios: Vec<PortfolioView>,
}

#[derive(Template)]
#[template(path = "portfolio.html")]
pub(super) struct PortfolioPage {
    pub common: Common,
    pub portfolio: PortfolioView,
    pub holdings: Vec<HoldingView>,
}

pub(super) struct Pagination {
    pub total: usize,
    pub page: usize,
    pub pages: usize,
    pub start: usize,
    pub end: usize,
    pub previous: String,
    pub next: String,
}

impl Pagination {
    pub fn new(total: usize, query: &PageQuery) -> Self {
        let pages = total.div_ceil(PAGE_SIZE).max(1);
        let page = query.page.max(1).min(pages);
        let start = (page - 1) * PAGE_SIZE;
        Self {
            total,
            page,
            pages,
            start,
            end: (start + PAGE_SIZE).min(total),
            previous: query.url(page.saturating_sub(1)),
            next: query.url(page + 1),
        }
    }
}

#[derive(Template)]
#[template(path = "transactions.html")]
pub(super) struct TransactionsPage {
    pub common: Common,
    pub transactions: Vec<TransactionView>,
    pub portfolios: Vec<Choice>,
    pub assets: Vec<Choice>,
    pub search: String,
    pub any_side: bool,
    pub pagination: Pagination,
    pub notice: String,
}

impl TransactionsPage {
    pub fn new(data: &StoreData, query: &PageQuery) -> Self {
        let transactions = matching_transactions(data, query);
        let pagination = Pagination::new(transactions.len(), query);
        let visible = transactions
            .into_iter()
            .skip(pagination.start)
            .take(PAGE_SIZE)
            .map(|t| TransactionView::new(t, data))
            .collect();
        let common = Common::new(
            data,
            "Transactions",
            "transactions",
            query.url(pagination.page),
        );
        Self {
            common,
            transactions: visible,
            portfolios: portfolio_choices(data, query),
            assets: asset_choices(data, query.asset),
            search: query.search.clone(),
            any_side: query.scope == AssetScope::Any,
            pagination,
            notice: transaction_notice(&query.notice),
        }
    }
}

fn transaction_notice(notice: &str) -> String {
    match notice {
        "added" => "Transaction saved.",
        "updated" => "Transaction updated.",
        "deleted" => "Transaction deleted.",
        _ => "",
    }
    .into()
}

fn matching_transactions<'a>(
    data: &'a StoreData,
    query: &PageQuery,
) -> Vec<&'a crate::model::Transaction> {
    let search = query.search.to_lowercase();
    let mut transactions = data
        .transactions
        .iter()
        .filter(|t| query.matches(t))
        .filter(|t| search.is_empty() || TransactionView::new(t, data).matches_search(&search))
        .collect::<Vec<_>>();
    transactions.sort_by(|a, b| b.timestamp.cmp(&a.timestamp).then_with(|| b.id.cmp(&a.id)));
    transactions
}

pub(super) struct TransactionDetails {
    pub id: Id,
    pub source: String,
    pub exchange: String,
    pub broker: String,
}

impl From<&Transaction> for TransactionDetails {
    fn from(transaction: &Transaction) -> Self {
        Self {
            id: transaction.id,
            source: transaction.source.clone(),
            exchange: transaction.exchange.clone().unwrap_or_default(),
            broker: transaction.broker.clone().unwrap_or_default(),
        }
    }
}

#[derive(Template)]
#[template(path = "transaction_form.html")]
pub(super) struct TransactionPage {
    pub common: Common,
    pub id: Option<crate::model::Id>,
    pub details: Option<TransactionDetails>,
    pub form: TransactionForm,
    pub error: String,
    pub portfolios: Vec<Choice>,
    pub assets: Vec<Choice>,
    pub quote_assets: Vec<Choice>,
    pub fee_assets: Vec<Choice>,
    pub kinds: Vec<TransactionKindChoice>,
    pub supports_quote: bool,
    pub implicit_zero_cost_basis: bool,
}

impl TransactionPage {
    pub fn new(
        data: &StoreData,
        form: TransactionForm,
        error: String,
        transaction: Option<&crate::model::Transaction>,
    ) -> Self {
        let query = PageQuery {
            portfolio: form.portfolio_id.parse().ok(),
            ..PageQuery::default()
        };
        let id = transaction.map(|transaction| transaction.id);
        Self {
            common: transaction_common(data, id),
            id,
            details: transaction.map(TransactionDetails::from),
            error,
            portfolios: portfolio_choices(data, &query),
            assets: asset_choices(data, form.base_asset_id.parse().ok()),
            quote_assets: asset_choices(data, form.quote_asset_id.parse().ok()),
            fee_assets: asset_choices(data, form.fee_asset_id.parse().ok()),
            kinds: transaction_kind_choices(&form.kind),
            supports_quote: form.supports_quote(),
            implicit_zero_cost_basis: form.has_implicit_zero_cost_basis(),
            form,
        }
    }
}

fn transaction_common(data: &StoreData, id: Option<crate::model::Id>) -> Common {
    let title = if id.is_some() {
        "Edit transaction"
    } else {
        "Add transaction"
    };
    let path = id
        .map(|id| format!("/transactions/{id}/edit"))
        .unwrap_or_else(|| "/transactions/new".into());
    Common::new(data, title, "transactions", path)
}
