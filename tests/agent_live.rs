mod common;

use axum::http::StatusCode;
use common::call;
use lorvaq::agent::docker::fake::FakeDocker;
use lorvaq::agent::run::{self, AgentConfig};
use lorvaq::app;
use lorvaq::crypto::{self, SecretBox};
use lorvaq::http::router;
use lorvaq::scene::SceneHandle;
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

const PASSWORD: &str = "correct horse battery";

/// A live server on a free port, the admin signed in.
async fn server() -> (axum::Router, String, String, SceneHandle) {
    let store = common::store().await;
    let secrets = SecretBox::from_base64(&SecretBox::generate_key()).unwrap();
    store
        .set_admin_hash(&crypto::hash_password(PASSWORD), 0)
        .await
        .unwrap();
    let state = Arc::new(app::live_state(store, secrets, None).await.unwrap());
    let ci = state.scenes[0].clone();
    let r = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let served = r.clone();
    tokio::spawn(async move {
        axum::serve(
            listener,
            served.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let (_, _, cookie) = call(
        &r,
        "POST",
        "/api/v1/login",
        None,
        Some(json!({ "password": PASSWORD })),
        &[],
    )
    .await;
    (r, url, cookie.unwrap(), ci)
}

async fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
    for _ in 0..100 {
        if ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

fn place_attr(ci: &SceneHandle, id: &str, key: &str) -> Option<serde_json::Value> {
    ci.snapshot()
        .places
        .into_iter()
        .find(|p| p.id == id)
        .and_then(|p| p.attrs.get(key).cloned())
}

#[tokio::test]
async fn an_agent_joins_reports_and_is_removed() {
    let (r, url, cookie, ci) = server().await;
    let (status, created, _) = call(
        &r,
        "POST",
        "/api/v1/admin/agents",
        Some(&cookie),
        Some(json!({ "name": "build-01" })),
        &[("host", "lorvaq.local:8080")],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let token = created["token"].as_str().unwrap().to_string();
    assert!(created["command"]
        .as_str()
        .unwrap()
        .contains("LORVAQ_SERVER=http://lorvaq.local:8080"));
    let (status, _, _) = call(
        &r,
        "POST",
        "/api/v1/admin/agents",
        Some(&cookie),
        Some(json!({ "name": "build-01" })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let docker = FakeDocker::default();
    docker
        .containers
        .lock()
        .unwrap()
        .push(FakeDocker::container("c1", 5, 300, Some(1024)));
    let mut cfg = AgentConfig::new(&url, &token, "runner-host");
    cfg.report_every = Duration::from_millis(100);
    let agent = tokio::spawn(run::run(docker.clone(), cfg));

    let process = "process:build-01/gitlab-runner";
    wait_for("the agent's process on the scene", || {
        place_attr(&ci, process, "online") == Some(json!(true))
    })
    .await;
    let snap = ci.snapshot();
    let host = snap
        .places
        .iter()
        .find(|p| p.id == "host:runner-host")
        .unwrap();
    assert_eq!(host.label, "runner-host");
    assert!(
        snap.levels
            .iter()
            .any(|l| l.owner == "host:runner-host" && l.metric == "disk"),
        "the disk level is reported"
    );
    assert!(snap
        .places
        .iter()
        .any(|p| p.id == "slot:build-01/gitlab-runner/0"));

    let (_, list, _) = call(&r, "GET", "/api/v1/admin/agents", Some(&cookie), None, &[]).await;
    assert_eq!(
        (
            list[0]["name"].clone(),
            list[0]["online"].clone(),
            list[0]["host"].clone()
        ),
        (json!("build-01"), json!(true), json!("runner-host"))
    );
    assert!(
        !list.to_string().contains(&token),
        "the token never comes back"
    );

    let (status, _, _) = call(
        &r,
        "DELETE",
        "/api/v1/admin/agents/build-01",
        Some(&cookie),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    wait_for("the removed agent's process to grey out", || {
        place_attr(&ci, process, "online") == Some(json!(false))
    })
    .await;
    let (_, list, _) = call(&r, "GET", "/api/v1/admin/agents", Some(&cookie), None, &[]).await;
    assert_eq!(list, json!([]));
    agent.abort();
}

#[tokio::test]
async fn a_wrong_token_is_refused_before_the_upgrade() {
    let (_, url, _, _) = server().await;
    let cfg = AgentConfig::new(&url, "not-a-token", "h");
    let err = run::session(&FakeDocker::default(), &cfg)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("refused the join token"),
        "{err:#}"
    );
}

#[tokio::test]
async fn agent_routes_need_the_admin() {
    let (r, _, _, _) = server().await;
    let (status, _, _) = call(&r, "GET", "/api/v1/admin/agents", None, None, &[]).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
