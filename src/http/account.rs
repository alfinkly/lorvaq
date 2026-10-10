//! Who am I, first-run setup, sign in and out, password change.

use super::guard::{db_error, has_session, json_error, Peer, SESSION_COOKIE};
use super::sessions::CHECK_TIMEOUT;
use super::{AppState, Live};
use crate::auth;
use crate::crypto;
use crate::scene::{now_ms, LOCK};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use serde::Deserialize;
use serde_json::{json, Value};
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Instant;

pub const MIN_PASSWORD: usize = 12;
const SESSION_TTL_MS: i64 = 30 * 24 * 3600 * 1000;

#[derive(Deserialize)]
pub(super) struct SetupBody {
    code: String,
    password: String,
}

#[derive(Deserialize)]
pub(super) struct LoginBody {
    password: String,
}

#[derive(Deserialize)]
pub(super) struct PasswordBody {
    current: String,
    new: String,
}

fn short_password() -> Response {
    json_error(
        StatusCode::BAD_REQUEST,
        &format!("the password needs at least {MIN_PASSWORD} characters"),
    )
}

/// Password hashing is slow and memory-hungry: at most `ARGON_RUNS` run at once, the rest wait.
async fn hash_blocking(live: &Live, password: String) -> String {
    let _permit = live
        .argon
        .acquire()
        .await
        .expect("the semaphore is never closed");
    tokio::task::spawn_blocking(move || crypto::hash_password(&password))
        .await
        .expect("hashing does not panic")
}

async fn verify_blocking(live: &Live, password: String, hash: String) -> bool {
    let _permit = live
        .argon
        .acquire()
        .await
        .expect("the semaphore is never closed");
    tokio::task::spawn_blocking(move || crypto::verify_password(&password, &hash))
        .await
        .unwrap_or(false)
}

fn too_many_attempts() -> Response {
    json_error(
        StatusCode::TOO_MANY_REQUESTS,
        "too many attempts; try again in 15 minutes",
    )
}

fn session_cookie(token: String, secure: bool) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, token))
        .http_only(true)
        .secure(secure)
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(time::Duration::days(30))
        .build()
}

async fn start_session(live: &Live, jar: CookieJar, now: i64) -> Response {
    let token = crypto::random_token();
    let hash = crypto::token_hash(&token);
    if let Err(e) = live.store.create_session(&hash, now, SESSION_TTL_MS).await {
        return db_error(e);
    }
    live.sessions
        .lock()
        .expect(LOCK)
        .confirm(&hash, now + SESSION_TTL_MS, now, Instant::now());
    (
        jar.add(session_cookie(token, live.cookie_secure)),
        Json(json!({ "ok": true })),
    )
        .into_response()
}

fn limiter_key(live: &Live, peer: Option<IpAddr>, headers: &HeaderMap) -> IpAddr {
    live.access.read().expect(LOCK).limiter_key(peer, headers)
}

pub(super) async fn me(
    State(s): State<Arc<AppState>>,
    Peer(peer): Peer,
    jar: CookieJar,
    headers: HeaderMap,
) -> Json<Value> {
    let Some(live) = &s.live else {
        return Json(
            json!({ "demo": true, "setup_needed": false, "admin": false, "viewer": true }),
        );
    };
    let admin = has_session(&s, &jar).await;
    let setup_needed = matches!(
        tokio::time::timeout(CHECK_TIMEOUT, live.store.admin_hash()).await,
        Ok(Ok(None))
    );
    let access = live.access.read().expect(LOCK).clone();
    let viewer = access.viewer_allowed(peer, &headers, admin);
    Json(json!({ "demo": false, "setup_needed": setup_needed, "admin": admin, "viewer": viewer }))
}

pub(super) async fn setup(
    State(s): State<Arc<AppState>>,
    Peer(peer): Peer,
    jar: CookieJar,
    headers: HeaderMap,
    Json(body): Json<SetupBody>,
) -> Response {
    let Some(live) = &s.live else {
        return json_error(StatusCode::NOT_FOUND, "demo mode has no admin");
    };
    let (key, now) = (limiter_key(live, peer, &headers), now_ms());
    if !live.limiter.lock().expect(LOCK).try_begin(key, now) {
        return too_many_attempts();
    }
    // Not a guess at the code: give the attempt back.
    let release = || live.limiter.lock().expect(LOCK).release(key);
    match live.store.admin_hash().await {
        Ok(Some(_)) => {
            release();
            return json_error(StatusCode::CONFLICT, "the admin already exists");
        }
        Ok(None) => {}
        Err(e) => {
            release();
            return db_error(e);
        }
    }
    if body.password.chars().count() < MIN_PASSWORD {
        release();
        return short_password();
    }
    match auth::check_setup_code(&live.store, &body.code, now).await {
        Ok(true) => live.limiter.lock().expect(LOCK).reset(key),
        Ok(false) => return json_error(StatusCode::FORBIDDEN, "wrong or expired setup code"),
        Err(e) => {
            release();
            return db_error(e);
        }
    }
    if let Err(e) = live
        .store
        .set_admin_hash(&hash_blocking(live, body.password).await, now)
        .await
    {
        return db_error(e);
    }
    if let Err(e) = live.store.delete_setting(auth::SETUP_CODE_KEY).await {
        tracing::warn!("the used setup code stays until it expires: {e:#}");
    }
    start_session(live, jar, now).await
}

pub(super) async fn login(
    State(s): State<Arc<AppState>>,
    Peer(peer): Peer,
    jar: CookieJar,
    headers: HeaderMap,
    Json(body): Json<LoginBody>,
) -> Response {
    let Some(live) = &s.live else {
        return json_error(StatusCode::NOT_FOUND, "demo mode has no admin");
    };
    let (key, now) = (limiter_key(live, peer, &headers), now_ms());
    if !live.limiter.lock().expect(LOCK).try_begin(key, now) {
        return too_many_attempts();
    }
    let hash = match live.store.admin_hash().await {
        Ok(Some(hash)) => hash,
        other => {
            // Not a password guess: give the attempt back.
            live.limiter.lock().expect(LOCK).release(key);
            return match other {
                Ok(_) => json_error(StatusCode::CONFLICT, "no admin yet: finish the setup first"),
                Err(e) => db_error(e),
            };
        }
    };
    if !verify_blocking(live, body.password, hash).await {
        return json_error(StatusCode::UNAUTHORIZED, "wrong password");
    }
    live.limiter.lock().expect(LOCK).reset(key);
    start_session(live, jar, now).await
}

pub(super) async fn logout(State(s): State<Arc<AppState>>, jar: CookieJar) -> Response {
    if let (Some(live), Some(cookie)) = (&s.live, jar.get(SESSION_COOKIE)) {
        let hash = crypto::token_hash(cookie.value());
        live.sessions.lock().expect(LOCK).evict(&hash);
        if let Err(e) = live.store.delete_session(&hash).await {
            return db_error(e);
        }
    }
    let mut gone = Cookie::from(SESSION_COOKIE);
    gone.set_path("/");
    (jar.remove(gone), Json(json!({ "ok": true }))).into_response()
}

/// Behind the admin guard.
pub(super) async fn change_password(
    State(s): State<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<PasswordBody>,
) -> Response {
    let live = s
        .live
        .as_ref()
        .expect("the admin guard lets only live requests in");
    if body.new.chars().count() < MIN_PASSWORD {
        return short_password();
    }
    let hash = match live.store.admin_hash().await {
        Ok(Some(hash)) => hash,
        Ok(None) => return json_error(StatusCode::CONFLICT, "no admin yet"),
        Err(e) => return db_error(e),
    };
    if !verify_blocking(live, body.current, hash).await {
        return json_error(StatusCode::FORBIDDEN, "the current password is wrong");
    }
    if let Err(e) = live
        .store
        .set_admin_hash(&hash_blocking(live, body.new).await, now_ms())
        .await
    {
        return db_error(e);
    }
    let mine = jar
        .get(SESSION_COOKIE)
        .map(|c| crypto::token_hash(c.value()))
        .expect("the admin guard checked the cookie");
    live.sessions.lock().expect(LOCK).evict_except(&mine);
    if let Err(e) = live.store.delete_other_sessions(&mine).await {
        return db_error(e);
    }
    Json(json!({ "ok": true })).into_response()
}
