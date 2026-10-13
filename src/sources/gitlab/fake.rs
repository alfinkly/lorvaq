//! A tiny stand-in for GitLab, for tests: the GraphQL endpoint and the OAuth token endpoint.
#![doc(hidden)]

use super::oauth::pkce_challenge;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Form, Json, Router};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub struct FakeState {
    /// The bearer token GraphQL accepts.
    pub token: String,
    pub username: String,
    pub admin: bool,
    pub fail_with: Option<u16>,
    pub graphql_error: Option<String>,
    pub active: Vec<Value>,
    pub finished: Vec<Value>,
    pub runners: Vec<Value>,
    pub graphql_calls: usize,
    // OAuth
    pub client_id: String,
    pub code: String,
    /// The PKCE challenge the "browser" brought to /oauth/authorize.
    pub challenge: Option<String>,
    pub refresh: String,
    pub issued: usize,
    pub expires_in: i64,
    pub token_requests: usize,
}

impl Default for FakeState {
    fn default() -> Self {
        Self {
            token: "good-token".into(),
            username: "dana".into(),
            admin: true,
            fail_with: None,
            graphql_error: None,
            active: vec![],
            finished: vec![],
            runners: vec![],
            graphql_calls: 0,
            client_id: "app-id".into(),
            code: "the-code".into(),
            challenge: None,
            refresh: "refresh-0".into(),
            issued: 0,
            expires_in: 7200,
            token_requests: 0,
        }
    }
}

pub struct FakeGitlab {
    pub url: String,
    pub state: Arc<Mutex<FakeState>>,
}

type Shared = Arc<Mutex<FakeState>>;

impl FakeGitlab {
    pub async fn start() -> Self {
        let state: Shared = Arc::new(Mutex::new(FakeState::default()));
        let app = Router::new()
            .route("/api/graphql", post(graphql))
            .route("/oauth/token", post(token))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a free port");
        let url = format!("http://{}", listener.local_addr().expect("bound"));
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self { url, state }
    }

    pub fn job(id: u64, project: &str, status: &str, runner: Option<u64>) -> Value {
        json!({
            "id": format!("gid://gitlab/Ci::Build/{id}"), "name": format!("job{id}"), "status": status,
            "stage": { "name": "test" }, "tags": [],
            "runner": runner.map(|r| json!({ "id": format!("gid://gitlab/Ci::Runner/{r}") })),
            "project": { "fullPath": project }, "pipeline": { "id": "gid://gitlab/Ci::Pipeline/1" }
        })
    }

    pub fn runner(id: u64, tags: &[&str], untagged: bool) -> Value {
        json!({
            "id": format!("gid://gitlab/Ci::Runner/{id}"), "description": format!("runner{id}"), "status": "ONLINE",
            "paused": false, "runnerType": "INSTANCE_TYPE", "tagList": tags, "runUntagged": untagged,
            "groups": null, "projects": null
        })
    }
}

async fn graphql(
    State(st): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let mut s = st.lock().expect("fake lock");
    s.graphql_calls += 1;
    if let Some(code) = s.fail_with {
        return StatusCode::from_u16(code)
            .expect("a valid status")
            .into_response();
    }
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    if bearer != Some(s.token.as_str()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if let Some(message) = &s.graphql_error {
        return Json(json!({ "errors": [{ "message": message }] })).into_response();
    }
    let query = body["query"].as_str().unwrap_or_default();
    let last_page = json!({ "hasNextPage": false, "endCursor": null });
    let data = if query.contains("currentUser") {
        json!({ "currentUser": { "username": s.username, "admin": s.admin } })
    } else if query.contains("runners(") {
        json!({ "runners": { "nodes": s.runners, "pageInfo": last_page } })
    } else {
        json!({ "active": { "nodes": s.active, "pageInfo": last_page }, "finished": { "nodes": s.finished } })
    };
    Json(json!({ "data": data })).into_response()
}

async fn token(State(st): State<Shared>, Form(form): Form<HashMap<String, String>>) -> Response {
    let mut s = st.lock().expect("fake lock");
    s.token_requests += 1;
    if form.get("client_id") != Some(&s.client_id) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "invalid_client" })),
        )
            .into_response();
    }
    let ok = match form.get("grant_type").map(String::as_str) {
        Some("authorization_code") => {
            form.get("code") == Some(&s.code)
                && matches!((s.challenge.as_deref(), form.get("code_verifier")), (Some(c), Some(v)) if pkce_challenge(v) == c)
        }
        Some("refresh_token") => form.get("refresh_token") == Some(&s.refresh),
        _ => false,
    };
    if !ok {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "invalid_grant" })),
        )
            .into_response();
    }
    s.issued += 1;
    s.token = format!("access-{}", s.issued);
    s.refresh = format!("refresh-{}", s.issued);
    Json(json!({ "access_token": s.token, "token_type": "Bearer", "expires_in": s.expires_in, "refresh_token": s.refresh, "scope": "read_api" }))
        .into_response()
}
