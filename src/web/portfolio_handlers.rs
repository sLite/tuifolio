use askama::Template;
use axum::{
    Form,
    extract::State,
    http::HeaderMap,
    response::{Html, IntoResponse, Response},
};
use serde::Deserialize;

use super::{
    error::{WebError, render},
    navigation::navigate,
    state::AppState,
    views::Common,
};
use crate::portfolios::{PortfolioInput, create_portfolio};

#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct PortfolioForm {
    name: String,
}

#[derive(Template)]
#[template(path = "portfolio_form.html")]
struct PortfolioPage {
    common: Common,
    form: PortfolioForm,
    error: String,
}

pub(super) async fn new_portfolio(State(state): State<AppState>) -> Result<Html<String>, WebError> {
    portfolio_page(&state, PortfolioForm::default(), String::new()).await
}

async fn portfolio_page(
    state: &AppState,
    form: PortfolioForm,
    error: String,
) -> Result<Html<String>, WebError> {
    let data = state.snapshot().await?;
    render(PortfolioPage {
        common: Common::new(
            &data,
            "Create portfolio",
            "portfolios",
            "/portfolios/new".into(),
        ),
        form,
        error,
    })
}

pub(super) async fn save_portfolio(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<PortfolioForm>,
) -> Result<Response, WebError> {
    let input = PortfolioInput {
        name: form.name.clone(),
    };
    match state
        .edit(move |store| create_portfolio(store, input))
        .await
    {
        Ok(id) => {
            tracing::info!(portfolio_id = id, "portfolio created");
            Ok(navigate(&format!("/portfolios/{id}"), &headers))
        }
        Err(error) => Ok((
            error.status,
            portfolio_page(&state, form, error.message).await?,
        )
            .into_response()),
    }
}
