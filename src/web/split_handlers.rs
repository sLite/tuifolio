use axum::{
    Form,
    extract::{Path, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
};

use super::{
    asset_forms::AssetForm,
    asset_handlers::find_asset,
    asset_views::{AssetEditorPage, Feedback},
    error::{WebError, render},
    navigation::navigate,
    split_forms::SplitForm,
    state::AppState,
};
use crate::{
    model::{Id, StockSplit},
    stock_splits::{delete_split, save_split},
};

pub(super) async fn save(
    State(state): State<AppState>,
    Path(id): Path<Id>,
    headers: HeaderMap,
    Form(form): Form<SplitForm>,
) -> Result<Response, WebError> {
    find_asset(&state.snapshot().await?, id)?;
    let result = match split_input(id, &form) {
        Ok((split, previous)) => {
            state
                .edit(move |store| save_split(store, split, previous))
                .await
        }
        Err(error) => Err(WebError::invalid(error)),
    };
    finish(&state, id, &headers, form, result, "split-saved").await
}

fn split_input(id: Id, form: &SplitForm) -> anyhow::Result<(StockSplit, Option<StockSplit>)> {
    let previous = form.is_edit().then(|| form.previous(id)).transpose()?;
    Ok((form.input(id)?, previous))
}

pub(super) async fn delete(
    State(state): State<AppState>,
    Path(id): Path<Id>,
    headers: HeaderMap,
    Form(form): Form<SplitForm>,
) -> Result<Response, WebError> {
    find_asset(&state.snapshot().await?, id)?;
    let result = match form.previous(id) {
        Ok(previous) => {
            state
                .edit(move |store| delete_split(store, id, previous))
                .await
        }
        Err(error) => Err(WebError::invalid(error)),
    };
    finish(&state, id, &headers, form, result, "split-deleted").await
}

async fn finish(
    state: &AppState,
    id: Id,
    headers: &HeaderMap,
    form: SplitForm,
    result: Result<(), WebError>,
    notice: &str,
) -> Result<Response, WebError> {
    match result {
        Ok(()) => {
            tracing::info!(
                asset_id = id,
                operation = notice,
                "stock split change saved; ledger rebuilt"
            );
            Ok(navigate(&format!("/assets/{id}?notice={notice}"), headers))
        }
        Err(error) => error_page(state, id, form, error).await,
    }
}

async fn error_page(
    state: &AppState,
    id: Id,
    form: SplitForm,
    error: WebError,
) -> Result<Response, WebError> {
    let data = state.snapshot().await?;
    let asset = find_asset(&data, id)?;
    let page = AssetEditorPage::new(
        &data,
        Some(asset),
        AssetForm::from_asset(asset),
        Feedback::default(),
    )
    .with_split_error(form, error.message);
    Ok((error.status, render(page)?).into_response())
}
