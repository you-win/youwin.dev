use axum::{
    Json,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde_json::json;

/// One error type for both listeners, one wire shape:
/// `{"error": {"code": "…", "message": "…"}}`.
///
/// `code` is a stable machine string the SPA can branch on; `message` is prose.
/// Server-side causes are logged and never serialized — the public listener is
/// on the open internet, and a leaked SQL string is a free schema disclosure.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("not found")]
    NotFound,

    #[error("unauthorized")]
    Unauthorized,

    #[error("forbidden")]
    Forbidden,

    /// A request the client could fix. The message is shown to me in the
    /// composer, so it is written for a person rather than a log.
    #[error("invalid: {0}")]
    Invalid(&'static str),

    /// Login throttling. Carries the seconds until the next attempt is allowed,
    /// which the client echoes as `Retry-After`.
    #[error("too many attempts")]
    TooManyAttempts(i64),

    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

impl AppError {
    fn parts(&self) -> (StatusCode, &'static str, &'static str) {
        match self {
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found", "Not found."),
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Authentication required.",
            ),
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden", "Refused."),
            Self::Invalid(message) => (StatusCode::UNPROCESSABLE_ENTITY, "invalid", message),
            Self::TooManyAttempts(_) => (
                StatusCode::TOO_MANY_REQUESTS,
                "too_many_attempts",
                "Too many attempts. Try again later.",
            ),
            Self::Database(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "Something went wrong.",
            ),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message) = self.parts();

        if status.is_server_error() {
            tracing::error!(error = ?self, "request failed");
        }

        let body = Json(json!({ "error": { "code": code, "message": message } }));

        match self {
            Self::TooManyAttempts(seconds) => {
                (status, [(header::RETRY_AFTER, seconds.to_string())], body).into_response()
            }
            _ => (status, body).into_response(),
        }
    }
}
