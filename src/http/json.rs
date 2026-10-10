//! `Json` for request bodies whose rejections answer `{"error": "<message>"}`.

use super::guard::json_error;
use axum::extract::{FromRequest, Request};
use axum::response::Response;
use serde::de::DeserializeOwned;

pub(super) struct ApiJson<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequest<S> for ApiJson<T> {
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match axum::Json::<T>::from_request(req, state).await {
            Ok(axum::Json(value)) => Ok(ApiJson(value)),
            Err(rejection) => Err(json_error(rejection.status(), &rejection.body_text())),
        }
    }
}
