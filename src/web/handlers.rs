use axum::{
    Form,
    extract::{OriginalUri, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{Html, Response},
};

use super::{
    error::{WebError, render},
    forms::BaseForm,
    navigation::{navigate, safe_return_path},
    query::PageQuery,
    state::AppState,
    tables::{PortfolioView, Summary, holdings, precision},
    views::{
        Common, OverviewPage, PortfolioPage, PortfoliosPage, TransactionsPage, portfolio_choices,
    },
};
use crate::{accounting::build_report, model::Id};

pub(super) async fn overview(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
    OriginalUri(uri): OriginalUri,
) -> Result<Html<String>, WebError> {
    let data = state.snapshot().await?;
    query.validate(&data)?;
    let report = build_report(&data);
    render(OverviewPage {
        common: Common::new(&data, "Overview", "overview", uri.to_string()),
        summary: Summary::new(&report, precision(&data)),
        holdings: holdings(&report, &data, &query),
        portfolios: portfolio_choices(&data, &query),
        search: query.search,
        empty_store: data.transactions.is_empty(),
    })
}

pub(super) async fn portfolios(State(state): State<AppState>) -> Result<Html<String>, WebError> {
    let data = state.snapshot().await?;
    let report = build_report(&data);
    render(PortfoliosPage {
        common: Common::new(&data, "Portfolios", "portfolios", "/portfolios".into()),
        portfolios: report
            .portfolios
            .iter()
            .map(|p| PortfolioView::new(p, precision(&data)))
            .collect(),
    })
}

pub(super) async fn portfolio(
    State(state): State<AppState>,
    Path(id): Path<Id>,
) -> Result<Html<String>, WebError> {
    let data = state.snapshot().await?;
    let report = build_report(&data);
    let portfolio = report
        .portfolios
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(WebError::not_found)?;
    let query = PageQuery {
        portfolio: Some(id),
        ..PageQuery::default()
    };
    render(PortfolioPage {
        common: Common::new(
            &data,
            portfolio.name.clone(),
            "portfolios",
            format!("/portfolios/{id}"),
        ),
        portfolio: PortfolioView::new(portfolio, precision(&data)),
        holdings: holdings(&report, &data, &query),
    })
}

pub(super) async fn transactions(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
) -> Result<Html<String>, WebError> {
    let data = state.snapshot().await?;
    query.validate(&data)?;
    render(TransactionsPage::new(&data, &query))
}

pub(super) async fn change_base(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<BaseForm>,
) -> Result<Response, WebError> {
    let return_to = safe_return_path(&form.return_to);
    state
        .edit(move |store| {
            anyhow::ensure!(
                store.data.config.base_currencies.contains(&form.currency),
                "select a configured base currency"
            );
            store.data.config.selected_base_currency = form.currency;
            Ok(())
        })
        .await?;
    Ok(navigate(&return_to, &headers))
}

pub(super) async fn not_found() -> (StatusCode, WebError) {
    (StatusCode::NOT_FOUND, WebError::not_found())
}
