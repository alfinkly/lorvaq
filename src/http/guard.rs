//! Guards in front of the scene and admin routes.

use super::AppState;
use crate::crypto;
use crate::scene::{now_ms, LOCK};
use axum::extract::connect_info::ConnectInfo;
use axum::extract::{FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use serde_json::json;
use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

pub(super) const SESSION_COOKIE: &str = "lorvaq_session";

/// The address the request came from (None when the server runs without connect info, as in tests).
pub(super) struct Peer(pub Option<IpAddr>);

impl<S: Send + Sync> FromRequestParts<S> for Peer {
    type Rejection = Infallible;
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        // The extractor (not a bare extension lookup) also understands MockConnectInfo in tests.
        Ok(Peer(
            ConnectInfo::<SocketAddr>::from_request_parts(parts, state)
                .await
                .ok()
                .map(|c| c.0.ip().to_canonical()),
        ))
    }
}

pub(super) fn json_error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

pub(super) fn db_error(e: anyhow::Error) -> Response {
    tracing::error!("database: {e:#}");
    json_error(
        StatusCode::SERVICE_UNAVAILABLE,
        "the database is unavailable",
    )
}

/// Is there a valid admin session cookie? A database error counts as "no".
pub(super) async fn has_session(state: &AppState, jar: &CookieJar) -> bool {
    let (Some(live), Some(cookie)) = (&state.live, jar.get(SESSION_COOKIE)) else {
        return false;
    };
    live.store
        .session_valid(&crypto::token_hash(cookie.value()), now_ms())
        .await
        .unwrap_or_else(|e| {
            tracing::warn!("session check failed: {e:#}");
            false
        })
}

pub(super) async fn viewer(
    State(s): State<Arc<AppState>>,
    Peer(peer): Peer,
    jar: CookieJar,
    req: Request,
    next: Next,
) -> Response {
    let Some(live) = &s.live else {
        return next.run(req).await;
    };
    let access = live.access.read().expect(LOCK).clone();
    let admin = has_session(&s, &jar).await;
    if access.viewer_allowed(peer, req.headers(), admin) {
        next.run(req).await
    } else {
        json_error(StatusCode::UNAUTHORIZED, "sign in to see the scenes")
    }
}

pub(super) async fn admin(
    State(s): State<Arc<AppState>>,
    jar: CookieJar,
    req: Request,
    next: Next,
) -> Response {
    if s.live.is_none() {
        return json_error(
            StatusCode::NOT_FOUND,
            "settings are not available in demo mode",
        );
    }
    if has_session(&s, &jar).await {
        next.run(req).await
    } else {
        json_error(StatusCode::UNAUTHORIZED, "sign in as the admin")
    }
}
