use axum::{
    Json,
    extract::rejection::JsonRejection,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Unauthorized(String),
    #[error("{0}")]
    Forbidden(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Jwt(#[from] jsonwebtoken::errors::Error),
    #[error("internal server error")]
    Internal(#[from] anyhow::Error),
    /// Catch-all for framework-level request rejections (malformed JSON body,
    /// invalid enum value, wrong Content-Type) that would otherwise bypass
    /// AppError entirely and return a plain-text body instead of the
    /// `{"error": ...}` shape every other response uses. Carries whatever
    /// status the rejection itself specifies rather than hardcoding one.
    #[error("{1}")]
    Rejection(StatusCode, String),
}

impl From<JsonRejection> for AppError {
    fn from(rejection: JsonRejection) -> Self {
        AppError::Rejection(rejection.status(), rejection.body_text())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::BadRequest(m) => (StatusCode::BAD_REQUEST, m.clone()),
            AppError::Unauthorized(m) => (StatusCode::UNAUTHORIZED, m.clone()),
            AppError::Forbidden(m) => (StatusCode::FORBIDDEN, m.clone()),
            AppError::NotFound(m) => (StatusCode::NOT_FOUND, m.clone()),
            AppError::Conflict(m) => (StatusCode::CONFLICT, m.clone()),
            AppError::Jwt(_) => (
                StatusCode::UNAUTHORIZED,
                "Invalid or expired token.".to_string(),
            ),
            AppError::Database(e) => {
                tracing::error!(error = %e, "database error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "A database error occurred.".to_string(),
                )
            }
            AppError::Internal(e) => {
                tracing::error!(error = %e, "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "An internal error occurred.".to_string(),
                )
            }
            AppError::Rejection(status, m) => (*status, m.clone()),
        };

        (status, Json(json!({ "error": message }))).into_response()
    }
}

pub type AppResult<T> = Result<T, AppError>;
