use axum::{
    Json,
    http::StatusCode,
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
// NotFound is constructed from M1 (permalinks), Unauthorized from M2 (auth).
#[allow(dead_code)]
pub enum AppError {
    #[error("not found")]
    NotFound,

    #[error("unauthorized")]
    Unauthorized,

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

        (
            status,
            Json(json!({ "error": { "code": code, "message": message } })),
        )
            .into_response()
    }
}
