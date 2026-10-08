use axum::{
    Form,
    extract::{Path, Query, State},
    http::HeaderMap,
    response::{Html, IntoResponse, Response},
};

use super::{
    error::{WebError, render},
    forms::TransactionForm,
    navigation::navigate,
    query::PageQuery,
    state::AppState,
    views::TransactionPage,
};
use crate::{
    model::{Id, StoreData, Transaction},
    transactions::{
        ManualTransactionResult, add_manual_transaction, delete_transaction, update_transaction,
    },
};

pub(super) async fn new_transaction(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
) -> Result<Html<String>, WebError> {
    let data = state.snapshot().await?;
    query.validate(&data)?;
    render(TransactionPage::new(
        &data,
        TransactionForm::new(&data, &query),
        String::new(),
        None,
    ))
}

pub(super) async fn edit_transaction(
    State(state): State<AppState>,
    Path(id): Path<Id>,
) -> Result<Html<String>, WebError> {
    editor_page(&state, id, None, String::new()).await
}

fn find_transaction(data: &StoreData, id: Id) -> Result<&Transaction, WebError> {
    data.transactions
        .iter()
        .find(|transaction| transaction.id == id)
        .ok_or_else(WebError::not_found)
}

async fn editor_page(
    state: &AppState,
    id: Id,
    form: Option<TransactionForm>,
    error: String,
) -> Result<Html<String>, WebError> {
    let data = state.snapshot().await?;
    let transaction = find_transaction(&data, id)?;
    let form = form.unwrap_or_else(|| TransactionForm::from_transaction(transaction));
    render(TransactionPage::new(&data, form, error, Some(transaction)))
}

pub(super) async fn create_transaction(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<TransactionForm>,
) -> Result<Response, WebError> {
    match save_transaction(&state, None, &form).await {
        Ok(result) => Ok(saved_navigation(result, "added", &headers)),
        Err(error) => {
            let data = state.snapshot().await?;
            Ok((
                error.status,
                render(TransactionPage::new(&data, form, error.message, None))?,
            )
                .into_response())
        }
    }
}

pub(super) async fn save_edit(
    State(state): State<AppState>,
    Path(id): Path<Id>,
    headers: HeaderMap,
    Form(form): Form<TransactionForm>,
) -> Result<Response, WebError> {
    match save_transaction(&state, Some(id), &form).await {
        Ok(result) => Ok(saved_navigation(result, "updated", &headers)),
        Err(error) => Ok((
            error.status,
            editor_page(&state, id, Some(form), error.message).await?,
        )
            .into_response()),
    }
}

async fn save_transaction(
    state: &AppState,
    id: Option<Id>,
    form: &TransactionForm,
) -> Result<ManualTransactionResult, WebError> {
    let input = if let Some(id) = id {
        let data = state.snapshot().await?;
        form.input_for_edit(find_transaction(&data, id)?)
    } else {
        form.input()
    }
    .map_err(WebError::invalid)?;
    state
        .edit(move |store| match id {
            Some(id) => update_transaction(store, id, input),
            None => add_manual_transaction(store, input),
        })
        .await
}

fn saved_navigation(
    result: ManualTransactionResult,
    notice: &str,
    headers: &HeaderMap,
) -> Response {
    tracing::info!(
        transaction_id = result.transaction_id,
        operation = notice,
        "transaction change saved"
    );
    navigate(
        &format!(
            "/transactions?portfolio={}&asset={}&notice={notice}",
            result.portfolio_id, result.asset_id
        ),
        headers,
    )
}

pub(super) async fn remove_transaction(
    State(state): State<AppState>,
    Path(id): Path<Id>,
    headers: HeaderMap,
) -> Result<Response, WebError> {
    find_transaction(&state.snapshot().await?, id)?;
    match state.edit(move |store| delete_transaction(store, id)).await {
        Ok(result) => Ok(saved_navigation(result, "deleted", &headers)),
        Err(error) => Ok((
            error.status,
            editor_page(&state, id, None, error.message).await?,
        )
            .into_response()),
    }
}
