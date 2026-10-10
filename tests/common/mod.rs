//! Shared helpers for tests that need Postgres (docker must be running).
#![allow(dead_code)]

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use lorvaq::store::Store;
use serde_json::Value;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::ContainerAsync;
use tokio::sync::OnceCell;
use tower::ServiceExt;

static PG: OnceCell<(ContainerAsync<Postgres>, String)> = OnceCell::const_new();

/// A fresh, migrated database; one Postgres container per test binary.
pub async fn store_with_url() -> (Store, String) {
    let (_, base) = PG
        .get_or_init(|| async {
            let node = Postgres::default()
                .start()
                .await
                .expect("docker is needed for database tests");
            let host = node.get_host().await.expect("container host");
            let port = node.get_host_port_ipv4(5432).await.expect("container port");
            (node, format!("postgres://postgres:postgres@{host}:{port}"))
        })
        .await;
    let name: String = format!(
        "t{}",
        lorvaq::crypto::token_hash(&lorvaq::crypto::random_token())
    )
    .chars()
    .take(16)
    .collect();
    let admin = sqlx::PgPool::connect(&format!("{base}/postgres"))
        .await
        .expect("postgres is up");
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .expect("create a test database");
    admin.close().await;
    let url = format!("{base}/{name}");
    (Store::connect(&url).await.expect("migrations run"), url)
}

pub async fn store() -> Store {
    store_with_url().await.0
}

/// One request through the router: status, JSON body (or null), the session cookie it set (`name=value`).
pub async fn call(
    router: &Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    body: Option<Value>,
    headers: &[(&str, &str)],
) -> (StatusCode, Value, Option<String>) {
    let mut req = Request::builder().method(method).uri(uri);
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let req = match body {
        Some(b) => req
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(b.to_string()))
            .unwrap(),
        None => req.body(Body::empty()).unwrap(),
    };
    let res = router.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let cookie = res
        .headers()
        .get(header::SET_COOKIE)
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string());
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        cookie,
    )
}
