mod asset_forms;
mod asset_handlers;
mod asset_views;
mod error;
mod forms;
mod handlers;
mod navigation;
mod portfolio_handlers;
mod query;
mod state;
mod tables;
mod transaction_handlers;
mod views;

#[cfg(test)]
mod tests;

use axum::{
    Router,
    body::Body,
    extract::{Request, State},
    http::{HeaderValue, Method, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use state::AppState;

use crate::store::Store;
use error::WebError;

pub fn run(store: Store, port: u16) -> anyhow::Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(serve(store, port))
}

async fn serve(store: Store, port: u16) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    let port = listener.local_addr()?.port();
    tracing::info!(port, store = %store.path().display(), "starting local web interface");
    println!("Tuifolio: http://127.0.0.1:{port}\nPress Ctrl+C to stop.");
    axum::serve(listener, router(AppState::new(store, port)))
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

async fn shutdown() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "could not listen for shutdown signal");
    }
    tracing::info!("stopping web interface");
}

fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(handlers::overview))
        .route("/portfolios", get(handlers::portfolios))
        .route(
            "/portfolios/new",
            get(portfolio_handlers::new_portfolio).post(portfolio_handlers::save_portfolio),
        )
        .route("/portfolios/{id}", get(handlers::portfolio))
        .route("/transactions", get(handlers::transactions))
        .merge(transaction_routes())
        .merge(asset_routes())
        .route("/base", post(handlers::change_base))
        .route("/static/{name}", get(static_asset))
        .fallback(handlers::not_found)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            local_requests,
        ))
        .with_state(state)
}

fn transaction_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/transactions/new",
            get(transaction_handlers::new_transaction)
                .post(transaction_handlers::create_transaction),
        )
        .route(
            "/transactions/{id}/edit",
            get(transaction_handlers::edit_transaction).post(transaction_handlers::save_edit),
        )
        .route(
            "/transactions/{id}/delete",
            post(transaction_handlers::remove_transaction),
        )
}

fn asset_routes() -> Router<AppState> {
    Router::new()
        .route("/assets", get(asset_handlers::assets))
        .route(
            "/assets/new",
            get(asset_handlers::new_asset).post(asset_handlers::create_asset),
        )
        .route(
            "/assets/{id}",
            get(asset_handlers::asset).post(asset_handlers::update_asset),
        )
        .route("/assets/prices", post(asset_handlers::create_price))
        .route("/assets/sync", post(asset_handlers::sync_prices))
}

async fn local_requests(State(state): State<AppState>, request: Request, next: Next) -> Response {
    if let Err(error) = validate_local_request(&state, &request) {
        return error.into_response();
    }
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response.headers_mut().insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"));
    response
}

fn validate_local_request(state: &AppState, request: &Request) -> Result<(), WebError> {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok());
    let allowed_hosts = [
        format!("127.0.0.1:{}", state.port),
        format!("localhost:{}", state.port),
    ];
    if !allowed_hosts
        .iter()
        .any(|allowed| Some(allowed.as_str()) == host)
    {
        return Err(WebError::forbidden(
            "Use the localhost address printed in the terminal.",
        ));
    }
    if request.method() != Method::GET && request.method() != Method::HEAD {
        validate_origin(request, &allowed_hosts)?;
    }
    Ok(())
}

fn validate_origin(request: &Request, allowed_hosts: &[String]) -> Result<(), WebError> {
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok());
    if !allowed_hosts
        .iter()
        .any(|host| Some(format!("http://{host}").as_str()) == origin)
    {
        tracing::warn!("rejected a cross-origin write request");
        return Err(WebError::forbidden(
            "This form must be submitted from Tuifolio.",
        ));
    }
    Ok(())
}

async fn static_asset(axum::extract::Path(name): axum::extract::Path<String>) -> Response {
    let (content_type, content): (&str, &[u8]) = match name.as_str() {
        "style.css" => (
            "text/css; charset=utf-8",
            include_bytes!("../../static/style.css"),
        ),
        "app.js" => (
            "text/javascript; charset=utf-8",
            include_bytes!("../../static/app.js"),
        ),
        "htmx.min.js" => (
            "text/javascript; charset=utf-8",
            include_bytes!("../../static/htmx.min.js"),
        ),
        "plex-sans-latin.woff2" => (
            "font/woff2",
            include_bytes!("../../static/plex-sans-latin.woff2"),
        ),
        _ => return WebError::not_found().into_response(),
    };
    ([(header::CONTENT_TYPE, content_type)], Body::from(content)).into_response()
}
