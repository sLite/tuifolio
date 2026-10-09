use axum::{
    Form,
    extract::{Path, Query, State},
    http::HeaderMap,
    response::{Html, IntoResponse, Response},
};

use super::{
    asset_forms::AssetForm,
    asset_views::{AssetEditorPage, AssetsPage, Feedback},
    error::{WebError, render},
    forms::PriceForm,
    navigation::navigate,
    query::PageQuery,
    state::{AppState, PriceRefresh},
};
use crate::{
    assets::save_asset,
    model::{Asset, Id, StoreData},
    price_sync::add_manual_price_for_asset,
};

pub(super) async fn assets(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
) -> Result<Html<String>, WebError> {
    asset_list_page(&state, &query, Feedback::default()).await
}

async fn asset_list_page(
    state: &AppState,
    query: &PageQuery,
    feedback: Feedback,
) -> Result<Html<String>, WebError> {
    let data = state.snapshot().await?;
    query.validate(&data)?;
    render(AssetsPage::new(
        &data,
        query,
        feedback,
        state.is_refreshing(),
    ))
}

pub(super) async fn new_asset(State(state): State<AppState>) -> Result<Html<String>, WebError> {
    editor_page(&state, None, Some(AssetForm::new()), Feedback::default()).await
}

pub(super) async fn asset(
    State(state): State<AppState>,
    Path(id): Path<Id>,
    Query(query): Query<PageQuery>,
) -> Result<Html<String>, WebError> {
    let notice = match query.notice.as_str() {
        "saved" => "Asset saved.",
        "price-added" => "Manual price saved.",
        _ => "",
    };
    editor_page(&state, Some(id), None, Feedback::notice(notice)).await
}

async fn editor_page(
    state: &AppState,
    id: Option<Id>,
    form: Option<AssetForm>,
    feedback: Feedback,
) -> Result<Html<String>, WebError> {
    let data = state.snapshot().await?;
    let asset = id.map(|id| find_asset(&data, id)).transpose()?;
    let mut form = form.unwrap_or_else(|| {
        asset
            .map(AssetForm::from_asset)
            .unwrap_or_else(AssetForm::new)
    });
    if let Some(asset) = asset {
        form.symbol = asset.symbol.clone();
    }
    render(AssetEditorPage::new(&data, asset, form, feedback))
}

fn find_asset(data: &StoreData, id: Id) -> Result<&Asset, WebError> {
    data.assets
        .iter()
        .find(|asset| asset.id == id)
        .ok_or_else(WebError::not_found)
}

pub(super) async fn create_asset(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<AssetForm>,
) -> Result<Response, WebError> {
    save_asset_form(&state, None, &headers, form).await
}

pub(super) async fn update_asset(
    State(state): State<AppState>,
    Path(id): Path<Id>,
    headers: HeaderMap,
    Form(form): Form<AssetForm>,
) -> Result<Response, WebError> {
    find_asset(&state.snapshot().await?, id)?;
    save_asset_form(&state, Some(id), &headers, form).await
}

async fn save_asset_form(
    state: &AppState,
    id: Option<Id>,
    headers: &HeaderMap,
    form: AssetForm,
) -> Result<Response, WebError> {
    let result = match form.input() {
        Ok(input) => state.edit(move |store| save_asset(store, id, input)).await,
        Err(error) => Err(WebError::invalid(error)),
    };
    match result {
        Ok(id) => {
            tracing::info!(asset_id = id, "asset saved");
            Ok(navigate(&format!("/assets/{id}?notice=saved"), headers))
        }
        Err(error) => Ok((
            error.status,
            editor_page(state, id, Some(form), Feedback::error(error.message)).await?,
        )
            .into_response()),
    }
}

pub(super) async fn create_price(
    State(state): State<AppState>,
    Path(id): Path<Id>,
    headers: HeaderMap,
    Form(form): Form<PriceForm>,
) -> Result<Response, WebError> {
    find_asset(&state.snapshot().await?, id)?;
    match save_price(&state, id, &form).await {
        Ok(()) => {
            tracing::info!(asset_id = id, "manual price saved");
            Ok(navigate(
                &format!("/assets/{id}?notice=price-added"),
                &headers,
            ))
        }
        Err(error) => Ok((
            error.status,
            price_error_page(&state, id, form, error.message).await?,
        )
            .into_response()),
    }
}

async fn price_error_page(
    state: &AppState,
    id: Id,
    form: PriceForm,
    error: String,
) -> Result<Html<String>, WebError> {
    let data = state.snapshot().await?;
    let asset = find_asset(&data, id)?;
    render(
        AssetEditorPage::new(
            &data,
            Some(asset),
            AssetForm::from_asset(asset),
            Feedback::default(),
        )
        .with_price_error(form, error),
    )
}

async fn save_price(state: &AppState, id: Id, form: &PriceForm) -> Result<(), WebError> {
    let input = form.input().map_err(WebError::invalid)?;
    state
        .edit(move |store| add_manual_price_for_asset(store, id, input.price, &input.currency))
        .await
}

pub(super) async fn sync_prices(State(state): State<AppState>) -> Result<Response, WebError> {
    match state.refresh_prices().await {
        Ok(PriceRefresh::Updated(summary)) => {
            let notice = format!(
                "Updated {} quotes. {} assets have no supported provider quote.",
                summary.updated, summary.unsupported
            );
            Ok(
                asset_list_page(&state, &PageQuery::default(), Feedback::notice(notice))
                    .await?
                    .into_response(),
            )
        }
        Ok(PriceRefresh::AlreadyRunning) => {
            price_refresh_error(
                &state,
                WebError::invalid("A price refresh is already running."),
            )
            .await
        }
        Err(error) => price_refresh_error(&state, error).await,
    }
}

async fn price_refresh_error(state: &AppState, error: WebError) -> Result<Response, WebError> {
    Ok((
        error.status,
        asset_list_page(state, &PageQuery::default(), Feedback::error(error.message)).await?,
    )
        .into_response())
}
