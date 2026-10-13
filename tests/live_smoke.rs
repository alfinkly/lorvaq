mod common;

use axum::http::StatusCode;
use lorvaq::app;
use lorvaq::crypto::SecretBox;
use lorvaq::http::router;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
async fn live_server_writes_history_and_asks_for_setup() {
    let store = common::store().await;
    store
        .put_scene_settings(
            "ci",
            &json!({ "exits": [{ "label": "prod", "stage": "deploy" }] }),
            0,
        )
        .await
        .unwrap();
    let state = app::live_state(
        store.clone(),
        SecretBox::from_base64(&SecretBox::generate_key()).unwrap(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        state.scenes[0].settings()["exits"][0]["label"],
        json!("prod"),
        "stored settings are loaded"
    );
    let r = router(Arc::new(state));

    tokio::time::sleep(Duration::from_millis(2_500)).await;
    assert!(
        store.event_count("ci").await.unwrap() > 0,
        "the scene's first events reached the history"
    );
    let (status, me, _) = common::call(&r, "GET", "/api/v1/me", None, None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        me,
        json!({ "demo": false, "setup_needed": true, "admin": false, "viewer": false })
    );
    assert!(
        store
            .setting::<serde_json::Value>(lorvaq::auth::SETUP_CODE_KEY)
            .await
            .unwrap()
            .is_some(),
        "a setup code was issued"
    );
    let (_, health, _) = common::call(&r, "GET", "/healthz", None, None, &[]).await;
    assert_eq!(health["db"], json!(true));
}

#[tokio::test]
async fn broken_stored_settings_fall_back_to_defaults() {
    let store = common::store().await;
    store
        .put_scene_settings(
            "ci",
            &json!({ "exits": [{ "label": "x", "stage": "(" }] }),
            0,
        )
        .await
        .unwrap();
    let state = app::live_state(
        store,
        SecretBox::from_base64(&SecretBox::generate_key()).unwrap(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(state.scenes[0].settings()["exits"], json!([]));
}

#[tokio::test]
async fn a_connected_gitlab_puts_its_jobs_on_the_scene() {
    use lorvaq::sources::gitlab::fake::FakeGitlab;
    use lorvaq::sources::gitlab::oauth::{Connection, StoredAuth, CONNECTION_KEY};
    let store = common::store().await;
    let fake = FakeGitlab::start().await;
    fake.state.lock().unwrap().active = vec![FakeGitlab::job(42, "demo/backend", "PENDING", None)];
    let secrets = SecretBox::from_base64(&SecretBox::generate_key()).unwrap();
    let conn = Connection {
        url: fake.url.clone(),
        user: "dana".into(),
        auth: StoredAuth::Token {
            token: secrets.seal(b"good-token"),
        },
        connected_ms: 0,
    };
    store.put_setting(CONNECTION_KEY, &conn).await.unwrap();
    let state = app::live_state(store, secrets, None).await.unwrap();
    let ci = state.scenes[0].clone();
    let mut seen = false;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if ci.snapshot().entities.iter().any(|e| e.id == "job:42") {
            seen = true;
            break;
        }
    }
    assert!(seen, "job 42 from GitLab reached the scene");
}
