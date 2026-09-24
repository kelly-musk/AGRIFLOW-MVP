//! A drop-in replacement for `axum::Json<T>` as a request extractor. Plain
//! `Json<T>` returns a plain-text body on rejection (malformed JSON, wrong
//! Content-Type, an invalid enum value) — inconsistent with every
//! hand-written error in this API, which returns `{"error": "..."}` via
//! `AppError`. `AppJson<T>` does the same extraction but converts a
//! rejection into an `AppError` first, so every request-body error looks
//! the same to callers regardless of where it originated.

use axum::{
    Json,
    extract::{FromRequest, Request},
};

use crate::error::AppError;

pub struct AppJson<T>(pub T);

impl<S, T> FromRequest<S> for AppJson<T>
where
    Json<T>: FromRequest<S, Rejection = axum::extract::rejection::JsonRejection>,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Json(value) = Json::<T>::from_request(req, state).await?;
        Ok(AppJson(value))
    }
}
