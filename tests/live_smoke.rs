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
