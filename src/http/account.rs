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
use std::sync::atomic::{AtomicUsize, Ordering};
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

/// More password checks than this already waiting: answer 503 instead of queueing.
const ARGON_QUEUE: usize = 16;

#[derive(Debug)]
pub(super) struct Busy;

fn busy() -> Response {
    json_error(
        StatusCode::SERVICE_UNAVAILABLE,
        "lorvaq is busy checking passwords; try again in a moment",
    )
}

/// Leaves the waiting count when dropped, even if the request is cancelled while waiting.
struct Waiting<'a>(&'a AtomicUsize);

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Runs slow, memory-hungry work on the blocking pool; at most `ARGON_RUNS` run at once and
/// at most `ARGON_QUEUE` wait. The permit travels into the closure: a blocking task cannot be
/// cancelled, so a client that disconnects must not free its place while the work still runs.
async fn run_limited<T: Send + 'static>(
    gate: &Arc<Semaphore>,
    waiting: &AtomicUsize,
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, Busy> {
    let queued = waiting.fetch_add(1, Ordering::SeqCst);
    let in_line = Waiting(waiting);
    if queued >= ARGON_QUEUE {
        return Err(Busy);
    }
    let permit = gate
        .clone()
        .acquire_owned()
        .await
        .expect("the semaphore is never closed");
    drop(in_line);
    tokio::task::spawn_blocking(move || {
        let out = work();
        drop(permit);
        out
    })
    .await
    .map_err(|_| Busy)
}

async fn hash_blocking(live: &Live, password: String) -> Result<String, Busy> {
    run_limited(&live.argon, &live.argon_waiting, move || {
        crypto::hash_password(&password)
    })
    .await
}

async fn verify_blocking(live: &Live, password: String, hash: String) -> Result<bool, Busy> {
    run_limited(&live.argon, &live.argon_waiting, move || {
        crypto::verify_password(&password, &hash)
    })
    .await
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
    let release = || live.limiter.lock().expect(LOCK).release(key, now);
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
    let Ok(hash) = hash_blocking(live, body.password).await else {
        return busy();
    };
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
            live.limiter.lock().expect(LOCK).release(key, now);
            return match other {
                Ok(_) => json_error(StatusCode::CONFLICT, "no admin yet: finish the setup first"),
                Err(e) => db_error(e),
            };
        }
    };
    match verify_blocking(live, body.password, hash).await {
        Ok(true) => {}
        Ok(false) => return json_error(StatusCode::UNAUTHORIZED, "wrong password"),
        Err(Busy) => {
            live.limiter.lock().expect(LOCK).release(key, now);
            return busy();
        }
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
        live.bump_auth_epoch();
        if let Err(e) = deleted {
            // The cookie goes anyway; the row expires on its own.
            tracing::warn!("signed out, but the session row stays until it expires: {e:#}");
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
    match verify_blocking(live, body.current, hash).await {
        Ok(true) => {}
        Ok(false) => return json_error(StatusCode::FORBIDDEN, "the current password is wrong"),
        Err(Busy) => return busy(),
    }
    let Ok(new_hash) = hash_blocking(live, body.new).await else {
        return busy();
    };
    let mine = jar
        .get(SESSION_COOKIE)
        .map(|c| crypto::token_hash(c.value()))
        .expect("the admin guard checked the cookie");
    live.sessions.lock().expect(LOCK).evict_all_except(&mine);
    let replaced = live
        .store
        .replace_admin_password(&new_hash, &mine, now_ms())
        .await;
    // Again once the rows are gone: a check that began in between may have read them.
    live.sessions.lock().expect(LOCK).evict_all_except(&mine);
    live.bump_auth_epoch();
    if let Err(e) = replaced {
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
            run_limited(&g, &AtomicUsize::new(0), move || {
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

    #[tokio::test]
    async fn a_full_queue_answers_busy_without_running_the_work() {
        let gate = Arc::new(Semaphore::new(0));
        let waiting = AtomicUsize::new(ARGON_QUEUE);
        let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let r = ran.clone();
        let result = run_limited(&gate, &waiting, move || r.store(true, Ordering::SeqCst)).await;
        assert!(result.is_err());
        assert!(!ran.load(Ordering::SeqCst));
        assert_eq!(
            waiting.load(Ordering::SeqCst),
            ARGON_QUEUE,
            "the counter is restored"
        );
    }
}
