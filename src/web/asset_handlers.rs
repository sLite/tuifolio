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
    state::AppState,
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
    let notice = if query.notice == "price-added" {
        "Manual price saved.".into()
    } else {
        String::new()
    };
    asset_list_page(&state, &query, None, Feedback::notice(notice)).await
}

async fn asset_list_page(
    state: &AppState,
    query: &PageQuery,
    form: Option<PriceForm>,
    feedback: Feedback,
) -> Result<Html<String>, WebError> {
    let data = state.snapshot().await?;
    query.validate(&data)?;
    let form = form.unwrap_or_else(|| PriceForm {
        asset_id: query.asset.map(|id| id.to_string()).unwrap_or_default(),
        currency: data.config.selected_base_currency.clone(),
        ..PriceForm::default()
    });
    render(AssetsPage::new(
        &data,
        query,
        form,
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
    let notice = if query.notice == "saved" {
        "Asset saved.".into()
    } else {
        String::new()
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
    headers: HeaderMap,
    Form(form): Form<PriceForm>,
) -> Result<Response, WebError> {
    match save_price(&state, &form).await {
        Ok(()) => {
            tracing::info!(asset_id = form.asset_id, "manual price saved");
            Ok(navigate(
                &format!("/assets?notice=price-added&asset={}", form.asset_id),
                &headers,
            ))
        }
        Err(error) => Ok((
            error.status,
            asset_list_page(
                &state,
                &PageQuery::default(),
                Some(form),
                Feedback::error(error.message),
            )
            .await?,
        )
            .into_response()),
    }
}

async fn save_price(state: &AppState, form: &PriceForm) -> Result<(), WebError> {
    let input = form.input().map_err(WebError::invalid)?;
    state
        .edit(move |store| {
            add_manual_price_for_asset(store, input.asset_id, input.price, &input.currency)
        })
        .await
}

pub(super) async fn sync_prices(State(state): State<AppState>) -> Result<Response, WebError> {
    match state.refresh_prices().await {
        Ok(summary) => {
            let notice = format!(
                "Updated {} quotes. {} assets have no supported provider quote.",
                summary.updated, summary.unsupported
            );
            Ok(asset_list_page(
                &state,
                &PageQuery::default(),
                None,
                Feedback::notice(notice),
            )
            .await?
            .into_response())
        }
        Err(error) => Ok((
            error.status,
            asset_list_page(
                &state,
                &PageQuery::default(),
                None,
                Feedback::error(error.message),
            )
            .await?,
        )
            .into_response()),
    }
}
