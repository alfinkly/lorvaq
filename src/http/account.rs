//! Who am I, first-run setup, sign in and out, password change.

use super::guard::{db_error, has_session, json_error, Peer, SESSION_COOKIE};
use super::json::ApiJson;
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
use tokio::sync::Semaphore;

pub const MIN_PASSWORD: usize = 12;
/// Longer passwords are refused up front: hashing one costs time and memory for nothing.
pub const MAX_PASSWORD: usize = 1024;
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

fn long_password() -> Response {
    json_error(
        StatusCode::BAD_REQUEST,
        &format!("the password can have at most {MAX_PASSWORD} characters"),
    )
}

fn too_long(password: &str) -> bool {
    // A byte count above 4x the limit cannot fit in MAX_PASSWORD characters: skip the scan.
    password.len() > MAX_PASSWORD * 4 || password.chars().count() > MAX_PASSWORD
}

/// Runs slow, memory-hungry work on the blocking pool; at most `ARGON_RUNS` run at once, the rest wait.
/// The permit travels into the closure: a blocking task cannot be cancelled, so a client that
/// disconnects must not free its place while the work still runs.
async fn run_limited<T: Send + 'static>(
    gate: &Arc<Semaphore>,
    work: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let permit = gate
        .clone()
        .acquire_owned()
        .await
        .expect("the semaphore is never closed");
    tokio::task::spawn_blocking(move || {
        let out = work();
        drop(permit);
        out
    })
    .await
    .ok()
}

async fn hash_blocking(live: &Live, password: String) -> String {
    run_limited(&live.argon, move || crypto::hash_password(&password))
        .await
        .expect("hashing does not panic")
}

async fn verify_blocking(live: &Live, password: String, hash: String) -> bool {
    run_limited(&live.argon, move || {
        crypto::verify_password(&password, &hash)
    })
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
    let began = live.sessions.lock().expect(LOCK).generation();
    if let Err(e) = live.store.create_session(&hash, now, SESSION_TTL_MS).await {
        return db_error(e);
    }
    live.sessions.lock().expect(LOCK).confirm(
        &hash,
        now + SESSION_TTL_MS,
        now,
        Instant::now(),
        began,
    );
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
    ApiJson(body): ApiJson<SetupBody>,
) -> Response {
    let Some(live) = &s.live else {
        return json_error(StatusCode::NOT_FOUND, "demo mode has no admin");
    };
    if too_long(&body.password) {
        return long_password();
    }
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
    let hash = hash_blocking(live, body.password).await;
    match live.store.create_admin_if_absent(&hash, now).await {
        Ok(true) => {}
        Ok(false) => return json_error(StatusCode::CONFLICT, "the admin already exists"),
        Err(e) => return db_error(e),
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
    ApiJson(body): ApiJson<LoginBody>,
) -> Response {
    let Some(live) = &s.live else {
        return json_error(StatusCode::NOT_FOUND, "demo mode has no admin");
    };
    if too_long(&body.password) {
        return long_password();
    }
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
        let deleted = live.store.delete_session(&hash).await;
        // Again once the row is gone: a check that began in between may have read it.
        live.sessions.lock().expect(LOCK).evict(&hash);
        if let Err(e) = deleted {
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
    ApiJson(body): ApiJson<PasswordBody>,
) -> Response {
    let live = s
        .live
        .as_ref()
        .expect("the admin guard lets only live requests in");
    if too_long(&body.current) || too_long(&body.new) {
        return long_password();
    }
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
    live.sessions.lock().expect(LOCK).evict_all_except(&mine);
    let deleted = live.store.delete_other_sessions(&mine).await;
    // Again once the rows are gone: a check that began in between may have read them.
    live.sessions.lock().expect(LOCK).evict_all_except(&mine);
    if let Err(e) = deleted {
        return db_error(e);
    }
    Json(json!({ "ok": true })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn the_permit_outlives_a_cancelled_request() {
        let gate = Arc::new(Semaphore::new(1));
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let g = gate.clone();
        let request = tokio::spawn(async move {
            run_limited(&g, move || {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            })
            .await
        });
        // Wait until the blocking work really runs, then the client "disconnects".
        tokio::task::spawn_blocking(move || started_rx.recv().unwrap())
            .await
            .unwrap();
        request.abort();
        assert!(request.await.unwrap_err().is_cancelled());
        assert_eq!(
            gate.available_permits(),
            0,
            "the work is still running: its place is not free"
        );
        release_tx.send(()).unwrap();
        let _permit = tokio::time::timeout(Duration::from_secs(5), gate.acquire())
            .await
            .expect("the place is freed once the work ends")
            .unwrap();
    }
}
