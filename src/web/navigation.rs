use crate::model::Id;
use axum::{
    http::HeaderMap,
    response::{IntoResponse, Redirect, Response},
};

pub(super) fn navigate(location: &str, headers: &HeaderMap) -> Response {
    if headers
        .get("HX-Request")
        .is_some_and(|value| value == "true")
    {
        let target = serde_json::json!({ "path": location, "target": "#app", "select": "#app", "swap": "outerHTML" });
        ([("HX-Location", target.to_string())], "").into_response()
    } else {
        Redirect::to(location).into_response()
    }
}

pub(super) fn safe_return_path(path: &str) -> String {
    let route = path.split('?').next().unwrap_or_default();
    if matches!(
        route,
        "/" | "/portfolios" | "/portfolios/new" | "/transactions" | "/assets" | "/assets/new"
    ) || ["/portfolios/", "/assets/"].iter().any(|prefix| {
        route
            .strip_prefix(prefix)
            .is_some_and(|id| id.parse::<Id>().is_ok())
    }) || route
        .strip_prefix("/transactions/")
        .and_then(|path| path.strip_suffix("/edit"))
        .is_some_and(|id| id.parse::<Id>().is_ok())
    {
        path.into()
    } else {
        "/".into()
    }
}
