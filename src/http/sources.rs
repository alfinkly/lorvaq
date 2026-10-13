//! Admin: connect GitLab with a token or with OAuth, see the connection, disconnect.
//! The OAuth callback is public (the browser comes back from GitLab) but checks the admin session.

use super::guard::{db_error, has_session, json_error};
use super::json::ApiJson;
use super::{AppState, Live};
use crate::crypto;
use crate::scene::{now_ms, LOCK};
use crate::sources::gitlab::client::{normalize_url, Gitlab, GitlabError};
use crate::sources::gitlab::oauth::{
    self, Connection, OauthClient, Pending, StoredAuth, CONNECTION_KEY, PENDING_KEY, PENDING_TTL_MS,
};
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

#[derive(Deserialize)]
pub(super) struct TokenBody {
    url: String,
    token: String,
}

#[derive(Deserialize)]
pub(super) struct OauthBody {
    url: String,
    client_id: String,
    #[serde(default)]
    client_secret: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

fn live(s: &AppState) -> &Live {
    s.live
        .as_ref()
        .expect("the admin guard lets only live requests in")
}

fn public_base(live: &Live, headers: &HeaderMap) -> Option<String> {
    if let Some(url) = &live.public_url {
        return Some(url.clone());
    }
    let host = headers.get(header::HOST)?.to_str().ok()?;
    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .filter(|p| *p == "https")
        .unwrap_or("http");
    Some(format!("{proto}://{host}"))
}

fn redirect_uri(base: &str) -> String {
    format!("{base}/api/v1/oauth/gitlab/callback")
}

fn not_admin(user: &str) -> String {
    format!("{user} is not a GitLab administrator; lorvaq needs an administrator (a dedicated service user is best) to see the jobs of every runner")
}

fn gitlab_error(e: GitlabError) -> Response {
    match e {
        GitlabError::Unauthorized => {
            json_error(StatusCode::BAD_REQUEST, "GitLab did not accept the token")
        }
        other => json_error(StatusCode::BAD_GATEWAY, &other.to_string()),
    }
}

async fn save(live: &Live, conn: &Connection) -> Response {
    if let Err(e) = live.store.put_setting(CONNECTION_KEY, conn).await {
        return db_error(e);
    }
    if let Err(e) = live.store.delete_setting(PENDING_KEY).await {
        tracing::warn!("an unused GitLab OAuth start stays until it expires: {e:#}");
    }
    if let Some(gitlab) = &live.gitlab {
        gitlab.reload.notify_one();
    }
    Json(json!({ "connected": true, "user": conn.user, "url": conn.url })).into_response()
}

pub(super) async fn status(State(s): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let live = live(&s);
    let conn = match live.store.setting::<Connection>(CONNECTION_KEY).await {
        Ok(conn) => conn,
        Err(e) => return db_error(e),
    };
    let st = live
        .gitlab
        .as_ref()
        .map(|g| g.status.lock().expect(LOCK).clone())
        .unwrap_or_default();
    Json(json!({
        "connected": conn.is_some(),
        "url": conn.as_ref().map(|c| c.url.clone()),
        "user": conn.as_ref().map(|c| c.user.clone()),
        "kind": conn.as_ref().map(|c| c.auth.kind()),
        "state": st.state,
        "last_ok_ms": st.last_ok_ms,
        "last_error": st.last_error,
        "redirect_uri": public_base(live, &headers).map(|b| redirect_uri(&b)),
    }))
    .into_response()
}

pub(super) async fn connect_token(
    State(s): State<Arc<AppState>>,
    ApiJson(body): ApiJson<TokenBody>,
) -> Response {
    let live = live(&s);
    let url = match normalize_url(&body.url) {
        Ok(url) => url,
        Err(message) => return json_error(StatusCode::BAD_REQUEST, &message),
    };
    let token = body.token.trim();
    if token.is_empty() || token.len() > 4096 {
        return json_error(StatusCode::BAD_REQUEST, "paste the token GitLab showed you");
    }
    let user = match Gitlab::new(&live.http, &url, token).whoami().await {
        Ok(user) => user,
        Err(e) => return gitlab_error(e),
    };
    if !user.admin.unwrap_or(false) {
        return json_error(StatusCode::BAD_REQUEST, &not_admin(&user.username));
    }
    let conn = Connection {
        url,
        user: user.username,
        auth: StoredAuth::Token {
            token: live.secrets.seal(token.as_bytes()),
        },
        connected_ms: now_ms(),
    };
    save(live, &conn).await
}

pub(super) async fn oauth_start(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<OauthBody>,
) -> Response {
    let live = live(&s);
    let url = match normalize_url(&body.url) {
        Ok(url) => url,
        Err(message) => return json_error(StatusCode::BAD_REQUEST, &message),
    };
    let client_id = body.client_id.trim();
    if client_id.is_empty() || client_id.len() > 256 {
        return json_error(
            StatusCode::BAD_REQUEST,
            "paste the Application ID from GitLab",
        );
    }
    let Some(base) = public_base(live, &headers) else {
        return json_error(
            StatusCode::BAD_REQUEST,
            "set LORVAQ_PUBLIC_URL so GitLab knows where to send you back",
        );
    };
    let redirect = redirect_uri(&base);
    let state = crypto::random_token();
    let verifier = oauth::pkce_verifier();
    let pending = Pending {
        state_hash: crypto::token_hash(&state),
        url: url.clone(),
        client_id: client_id.to_string(),
        client_secret: body
            .client_secret
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| live.secrets.seal(s.as_bytes())),
        redirect_uri: redirect.clone(),
        verifier: live.secrets.seal(verifier.as_bytes()),
        created_ms: now_ms(),
    };
    if let Err(e) = live.store.put_setting(PENDING_KEY, &pending).await {
        return db_error(e);
    }
    Json(json!({ "authorize_url": oauth::authorize_url(&url, client_id, &redirect, &state, &oauth::pkce_challenge(&verifier)) })).into_response()
}

fn back(outcome: &str) -> Response {
    Redirect::to(&format!("/#/settings?gitlab={outcome}")).into_response()
}

pub(super) async fn oauth_callback(
    State(s): State<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<CallbackQuery>,
) -> Response {
    let Some(live) = &s.live else {
        return back("demo");
    };
    if !has_session(&s, &jar).await {
        return back("signin");
    }
    if q.error.is_some() {
        return back("denied");
    }
    let (Some(code), Some(state)) = (q.code, q.state) else {
        return back("denied");
    };
    let now = now_ms();
    let pending = match live.store.setting::<Pending>(PENDING_KEY).await {
        Ok(Some(p)) => p,
        Ok(None) => return back("expired"),
        Err(_) => return back("unavailable"),
    };
    if now - pending.created_ms > PENDING_TTL_MS || crypto::token_hash(&state) != pending.state_hash
    {
        return back("expired");
    }
    // One use: a replayed callback finds nothing.
    if live.store.delete_setting(PENDING_KEY).await.is_err() {
        return back("unavailable");
    }
    let open = |sealed: &str| {
        live.secrets
            .open(sealed)
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
    };
    let Some(verifier) = open(&pending.verifier) else {
        return back("failed");
    };
    let secret = match pending.client_secret.as_deref().map(open) {
        Some(None) => return back("failed"),
        Some(Some(secret)) => Some(secret),
        None => None,
    };
    let client = OauthClient {
        http: &live.http,
        base: &pending.url,
        client_id: &pending.client_id,
        client_secret: secret.as_deref(),
        redirect_uri: &pending.redirect_uri,
    };
    let tokens = match client.exchange(&code, &verifier, now).await {
        Ok(tokens) => tokens,
        Err(GitlabError::Network(_)) => return back("unreachable"),
        Err(_) => return back("failed"),
    };
    let user = match Gitlab::new(&live.http, &pending.url, &tokens.access)
        .whoami()
        .await
    {
        Ok(user) => user,
        Err(_) => return back("failed"),
    };
    if !user.admin.unwrap_or(false) {
        return back("not_admin");
    }
    let conn = Connection {
        url: pending.url,
        user: user.username,
        auth: StoredAuth::Oauth {
            client_id: pending.client_id,
            client_secret: pending.client_secret,
            redirect_uri: pending.redirect_uri,
            verifier: pending.verifier,
            access: live.secrets.seal(tokens.access.as_bytes()),
            refresh: live.secrets.seal(tokens.refresh.as_bytes()),
            expires_ms: tokens.expires_ms,
        },
        connected_ms: now,
    };
    if live.store.put_setting(CONNECTION_KEY, &conn).await.is_err() {
        return back("unavailable");
    }
    if let Some(gitlab) = &live.gitlab {
        gitlab.reload.notify_one();
    }
    back("connected")
}

pub(super) async fn disconnect(State(s): State<Arc<AppState>>) -> Response {
    let live = live(&s);
    if let Err(e) = live.store.delete_setting(CONNECTION_KEY).await {
        return db_error(e);
    }
    if let Err(e) = live.store.delete_setting(PENDING_KEY).await {
        tracing::warn!("an unused GitLab OAuth start stays until it expires: {e:#}");
    }
    if let Some(gitlab) = &live.gitlab {
        gitlab.reload.notify_one();
    }
    Json(json!({ "ok": true })).into_response()
}
