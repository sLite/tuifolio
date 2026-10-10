use askama::Template;
use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};

use super::edit_revision::EditConflict;

#[derive(Debug)]
pub(super) struct WebError {
    pub status: StatusCode,
    pub message: String,
}

#[derive(Template)]
#[template(path = "error.html")]
struct ErrorPage<'a> {
    message: &'a str,
    status: u16,
}

impl WebError {
    pub fn edit(error: anyhow::Error) -> Self {
        if error.is::<EditConflict>() {
            Self {
                status: StatusCode::CONFLICT,
                message: error.to_string(),
            }
        } else {
            Self::invalid(error)
        }
    }

    pub fn invalid(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            message: error.to_string(),
        }
    }

    pub fn not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: "That page or record does not exist.".into(),
        }
    }

    pub fn forbidden(message: &str) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message: message.into(),
        }
    }
}

impl From<anyhow::Error> for WebError {
    fn from(error: anyhow::Error) -> Self {
        tracing::error!(error = %format!("{error:#}"), "web operation failed");
        if error.is::<crate::store::SaveDurabilityError>() {
            return Self {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                message: error.to_string(),
            };
        }
        Self { status: StatusCode::INTERNAL_SERVER_ERROR, message: "Could not complete this operation. Your edit was not saved. See the terminal for details.".into() }
    }
}

impl IntoResponse for WebError {
    fn into_response(self) -> Response {
        let page = ErrorPage {
            message: &self.message,
            status: self.status.as_u16(),
        };
        (
            self.status,
            Html(
                page.render()
                    .unwrap_or_else(|_| "Could not render the error page.".into()),
            ),
        )
            .into_response()
    }
}

pub(super) fn render(template: impl Template) -> Result<Html<String>, WebError> {
    Ok(Html(template.render().map_err(anyhow::Error::from)?))
}
