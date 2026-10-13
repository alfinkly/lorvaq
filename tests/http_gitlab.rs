mod common;

use axum::http::{header, StatusCode};
use axum::Router;
use common::call;
use lorvaq::auth::Access;
use lorvaq::crypto::{self, SecretBox};
use lorvaq::http::{router, AppState, Live};
use lorvaq::scene;
use lorvaq::scenes::ci::{settings::CiSettings, CiScene};
use lorvaq::sources::gitlab::fake::FakeGitlab;
use lorvaq::sources::gitlab::oauth::{self, Connection, StoredAuth};
use lorvaq::store::Store;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

const PASSWORD: &str = "correct horse battery";

async fn app(public_url: Option<&str>) -> (Router, Store, String, SecretBox) {
    let store = common::store().await;
    let secrets = SecretBox::from_base64(&SecretBox::generate_key()).unwrap();
    let (ci, _inputs) = scene::spawn(
        CiScene::new(CiSettings::default()).unwrap(),
        Duration::from_secs(1),
    );
    let mut live = Live::new(store.clone(), secrets.clone(), false, Access::Admin, vec![]);
    live.public_url = public_url.map(String::from);
    let r = router(Arc::new(AppState {
        scenes: vec![ci],
        live: Some(live),
    }));
    store
        .set_admin_hash(&crypto::hash_password(PASSWORD), 0)
        .await
        .unwrap();
    let (_, _, cookie) = call(
        &r,
        "POST",
        "/api/v1/login",
        None,
        Some(json!({ "password": PASSWORD })),
        &[],
    )
    .await;
    (r, store, cookie.unwrap(), secrets)
}

/// Follows nothing: returns the Location of a redirect.
async fn location(r: &Router, uri: &str, cookie: Option<&str>) -> String {
    use tower::ServiceExt;
    let mut req = axum::http::Request::get(uri);
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    let res = r
        .clone()
        .oneshot(req.body(axum::body::Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    res.headers()[header::LOCATION]
        .to_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn connecting_with_a_token_checks_it_and_seals_it() {
    let (r, store, cookie, _) = app(None).await;
    let fake = FakeGitlab::start().await;
    let (status, body, _) = call(
        &r,
        "POST",
        "/api/v1/admin/sources/gitlab/token",
        Some(&cookie),
        Some(json!({ "url": "not a url", "token": "x" })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body, _) = call(
        &r,
        "POST",
        "/api/v1/admin/sources/gitlab/token",
        Some(&cookie),
        Some(json!({ "url": fake.url, "token": "wrong" })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    fake.state.lock().unwrap().admin = false;
    let (status, body, _) = call(
        &r,
        "POST",
        "/api/v1/admin/sources/gitlab/token",
        Some(&cookie),
        Some(json!({ "url": fake.url, "token": "good-token" })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("not a GitLab administrator"),
        "{body}"
    );
    fake.state.lock().unwrap().admin = true;
    let (status, body, _) = call(
        &r,
        "POST",
        "/api/v1/admin/sources/gitlab/token",
        Some(&cookie),
        Some(json!({ "url": format!("{}/", fake.url), "token": "good-token" })),
        &[],
    )
    .await;
    assert_eq!(
        (status, body["user"].as_str()),
        (StatusCode::OK, Some("dana"))
    );

    let raw: serde_json::Value = store.setting(oauth::CONNECTION_KEY).await.unwrap().unwrap();
    assert!(
        !raw.to_string().contains("good-token"),
        "the token is sealed: {raw}"
    );
    assert_eq!(raw["url"], json!(fake.url), "the trailing slash is gone");
    let (_, st, _) = call(
        &r,
        "GET",
        "/api/v1/admin/sources/gitlab",
        Some(&cookie),
        None,
        &[],
    )
    .await;
    assert_eq!(
        (
            st["connected"].clone(),
            st["kind"].clone(),
            st["user"].clone()
        ),
        (json!(true), json!("token"), json!("dana"))
    );
    assert!(!st.to_string().contains("good-token"));

    let (status, _, _) = call(
        &r,
        "DELETE",
        "/api/v1/admin/sources/gitlab",
        Some(&cookie),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, st, _) = call(
        &r,
        "GET",
        "/api/v1/admin/sources/gitlab",
        Some(&cookie),
        None,
        &[],
    )
    .await;
    assert_eq!(st["connected"], json!(false));
}

#[tokio::test]
async fn the_oauth_round_trip_connects_once() {
    let (r, store, cookie, secrets) = app(Some("https://lorvaq.example.com")).await;
    let fake = FakeGitlab::start().await;
    let (status, body, _) = call(
        &r,
        "POST",
        "/api/v1/admin/sources/gitlab/oauth",
        Some(&cookie),
        Some(json!({ "url": fake.url, "client_id": "app-id" })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let authorize = reqwest::Url::parse(body["authorize_url"].as_str().unwrap()).unwrap();
    let q: std::collections::HashMap<_, _> = authorize.query_pairs().into_owned().collect();
    assert_eq!(
        q["redirect_uri"],
        "https://lorvaq.example.com/api/v1/oauth/gitlab/callback"
    );
    fake.state.lock().unwrap().challenge = Some(q["code_challenge"].clone());
    let state = q["state"].clone();

    let cb = |code: &str, st: &str| format!("/api/v1/oauth/gitlab/callback?code={code}&state={st}");
    assert_eq!(
        location(&r, &cb("the-code", &state), None).await,
        "/#/settings?gitlab=signin",
        "no admin session"
    );
    assert_eq!(
        location(&r, &cb("the-code", "forged"), Some(&cookie)).await,
        "/#/settings?gitlab=expired"
    );
    assert_eq!(
        location(&r, &cb("the-code", &state), Some(&cookie)).await,
        "/#/settings?gitlab=connected"
    );
    assert_eq!(
        location(&r, &cb("the-code", &state), Some(&cookie)).await,
        "/#/settings?gitlab=expired",
        "a state works once"
    );

    let conn: Connection = store.setting(oauth::CONNECTION_KEY).await.unwrap().unwrap();
    let StoredAuth::Oauth { access, .. } = &conn.auth else {
        panic!("oauth expected")
    };
    assert_eq!(secrets.open(access).unwrap(), b"access-1");
}

#[tokio::test]
async fn oauth_needs_a_public_address_and_reports_denials() {
    let (r, _, cookie, _) = app(None).await;
    let fake = FakeGitlab::start().await;
    let (status, body, _) = call(
        &r,
        "POST",
        "/api/v1/admin/sources/gitlab/oauth",
        Some(&cookie),
        Some(json!({ "url": fake.url, "client_id": "app-id" })),
        &[],
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "no LORVAQ_PUBLIC_URL and no Host: {body}"
    );
    let (status, body, _) = call(
        &r,
        "POST",
        "/api/v1/admin/sources/gitlab/oauth",
        Some(&cookie),
        Some(json!({ "url": fake.url, "client_id": "app-id" })),
        &[("host", "lorvaq.local:8080")],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body["authorize_url"]
            .as_str()
            .unwrap()
            .contains("lorvaq.local%3A8080"),
        "{body}"
    );
    assert_eq!(
        location(
            &r,
            "/api/v1/oauth/gitlab/callback?error=access_denied&state=x",
            Some(&cookie)
        )
        .await,
        "/#/settings?gitlab=denied"
    );
}

#[tokio::test]
async fn gitlab_routes_need_the_admin() {
    let (r, _, _, _) = app(None).await;
    assert_eq!(
        call(&r, "GET", "/api/v1/admin/sources/gitlab", None, None, &[])
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &r,
            "POST",
            "/api/v1/admin/sources/gitlab/token",
            None,
            Some(json!({ "url": "http://x", "token": "t" })),
            &[]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}
