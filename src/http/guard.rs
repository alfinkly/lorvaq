//! Guards in front of the scene and admin routes.

use super::sessions::CHECK_TIMEOUT;
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
use std::time::Instant;

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

/// Is there a valid admin session cookie? Sessions the database confirmed lately are
/// trusted for a few seconds; when the database fails or is slow, a session that was
/// confirmed within the last hour and has not expired keeps working.
pub(super) async fn has_session(state: &AppState, jar: &CookieJar) -> bool {
    let (Some(live), Some(cookie)) = (&state.live, jar.get(SESSION_COOKIE)) else {
        return false;
    };
    let hash = crypto::token_hash(cookie.value());
    let (now, at) = (now_ms(), Instant::now());
    let began = {
        let cache = live.sessions.lock().expect(LOCK);
        if cache.fresh(&hash, now, at) {
            return true;
        }
        cache.generation()
    };
    let checked = tokio::time::timeout(CHECK_TIMEOUT, live.store.session_expiry(&hash, now)).await;
    let mut cache = live.sessions.lock().expect(LOCK);
    match checked {
        Ok(Ok(Some(expires_ms))) => {
            cache.confirm(&hash, expires_ms, now, at, began);
            true
        }
        Ok(Ok(None)) => {
            cache.evict(&hash);
            false
        }
        Ok(Err(e)) => {
            tracing::warn!("session check failed: {e:#}");
            cache.known(&hash, now, at)
        }
        Err(_) => {
            tracing::warn!("session check timed out");
            cache.known(&hash, now, at)
        }
    }
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
    // The database is asked only when the mode alone does not let the request in.
    let allowed = access.viewer_allowed(peer, req.headers(), false) || has_session(&s, &jar).await;
    if allowed {
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
