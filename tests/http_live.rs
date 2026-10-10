mod common;

use axum::extract::connect_info::MockConnectInfo;
use axum::http::StatusCode;
use axum::Router;
use common::call;
use lorvaq::auth::{self, Access};
use lorvaq::crypto::{self, SecretBox};
use lorvaq::http::{router, AppState, Live};
use lorvaq::scene;
use lorvaq::scenes::ci::{settings::CiSettings, CiScene};
use lorvaq::store::Store;
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

const PASSWORD: &str = "correct horse battery";

async fn app(access: Access) -> (Router, Store) {
    let store = common::store().await;
    let (ci, _inputs) = scene::spawn(
        CiScene::new(CiSettings::default()).unwrap(),
        Duration::from_secs(1),
    );
    let live = Live::new(
        store.clone(),
        SecretBox::from_base64(&SecretBox::generate_key()).unwrap(),
        false,
        access,
        vec![],
    );
    let router = router(Arc::new(AppState {
        scenes: vec![ci],
        live: Some(live),
    }))
    .layer(MockConnectInfo(SocketAddr::from(([10, 1, 2, 3], 4000))));
    (router, store)
}

async fn signed_in(access: Access) -> (Router, Store, String) {
    let (r, store) = app(access).await;
    store
        .set_admin_hash(&crypto::hash_password(PASSWORD), 0)
        .await
        .unwrap();
    let (status, _, cookie) = call(
        &r,
        "POST",
        "/api/v1/login",
        None,
        Some(json!({ "password": PASSWORD })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    (r, store, cookie.expect("a session cookie"))
}

#[tokio::test]
async fn first_run_setup_creates_the_admin() {
    let (r, store) = app(Access::Admin).await;
    let (_, me, _) = call(&r, "GET", "/api/v1/me", None, None, &[]).await;
    assert_eq!(
        me,
        json!({ "demo": false, "setup_needed": true, "admin": false, "viewer": false })
    );
    let code = auth::issue_setup_code(&store, lorvaq::scene::now_ms())
        .await
        .unwrap();

    let (status, body, _) = call(
        &r,
        "POST",
        "/api/v1/setup",
        None,
        Some(json!({ "code": "WRONG", "password": PASSWORD })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, _, _) = call(
        &r,
        "POST",
        "/api/v1/setup",
        None,
        Some(json!({ "code": code, "password": "short" })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, cookie) = call(
        &r,
        "POST",
        "/api/v1/setup",
        None,
        Some(json!({ "code": code, "password": PASSWORD })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie.expect("setup signs the admin in");
    assert!(cookie.starts_with("lorvaq_session="));

    let (_, me, _) = call(&r, "GET", "/api/v1/me", Some(&cookie), None, &[]).await;
    assert_eq!(
        me,
        json!({ "demo": false, "setup_needed": false, "admin": true, "viewer": true })
    );
    let (status, _, _) = call(
        &r,
        "POST",
        "/api/v1/setup",
        None,
        Some(json!({ "code": code, "password": PASSWORD })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, access, _) =
        call(&r, "GET", "/api/v1/admin/access", Some(&cookie), None, &[]).await;
    assert_eq!(
        (status, access),
        (StatusCode::OK, json!({ "mode": "admin" }))
    );
}

#[tokio::test]
async fn login_logout_and_lockout() {
    let (r, store, cookie) = signed_in(Access::Admin).await;
    let (status, _, _) = call(&r, "GET", "/api/v1/admin/access", Some(&cookie), None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, cleared) = call(&r, "POST", "/api/v1/logout", Some(&cookie), None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cleared.as_deref(), Some("lorvaq_session="));
    let (status, _, _) = call(&r, "GET", "/api/v1/admin/access", Some(&cookie), None, &[]).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "the session is gone");

    for _ in 0..5 {
        let (status, _, _) = call(
            &r,
            "POST",
            "/api/v1/login",
            None,
            Some(json!({ "password": "wrong wrong wrong" })),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let (status, _, _) = call(
        &r,
        "POST",
        "/api/v1/login",
        None,
        Some(json!({ "password": PASSWORD })),
        &[],
    )
    .await;
    assert_eq!(
        status,
        StatusCode::TOO_MANY_REQUESTS,
        "blocked even with the right password"
    );
    drop(store);
}

#[tokio::test]
async fn viewer_modes_guard_the_scenes() {
    let (r, _) = app(Access::Admin).await;
    assert_eq!(
        call(&r, "GET", "/api/v1/scenes/ci/snapshot", None, None, &[])
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let (r, _) = app(Access::Open).await;
    assert_eq!(
        call(&r, "GET", "/api/v1/scenes/ci/snapshot", None, None, &[])
            .await
            .0,
        StatusCode::OK
    );
    let proxy = |net: &str| Access::Proxy {
        header: "x-forwarded-user".into(),
        trusted: vec![net.into()],
    };
    let (r, _) = app(proxy("10.0.0.0/8")).await;
    assert_eq!(
        call(&r, "GET", "/api/v1/scenes", None, None, &[]).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &r,
            "GET",
            "/api/v1/scenes",
            None,
            None,
            &[("x-forwarded-user", "dana")]
        )
        .await
        .0,
        StatusCode::OK
    );
    let (r, _) = app(proxy("192.168.0.0/16")).await;
    assert_eq!(
        call(
            &r,
            "GET",
            "/api/v1/scenes",
            None,
            None,
            &[("x-forwarded-user", "dana")]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED,
        "not our proxy"
    );
}

#[tokio::test]
async fn admin_routes_need_a_session() {
    let (r, _) = app(Access::Open).await;
    for (method, uri) in [
        ("GET", "/api/v1/admin/access"),
        ("GET", "/api/v1/admin/history"),
        ("GET", "/api/v1/admin/scenes/ci/settings"),
    ] {
        assert_eq!(
            call(&r, method, uri, None, None, &[]).await.0,
            StatusCode::UNAUTHORIZED,
            "{uri}"
        );
    }
}

#[tokio::test]
async fn access_and_history_settings_are_validated_and_kept() {
    let (r, store, cookie) = signed_in(Access::Admin).await;
    let bad = json!({ "mode": "proxy", "header": "x-forwarded-user", "trusted": [] });
    let (status, body, _) = call(
        &r,
        "PUT",
        "/api/v1/admin/access",
        Some(&cookie),
        Some(bad),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"].as_str().unwrap().contains("trusted"),
        "{body}"
    );
    let (status, _, _) = call(
        &r,
        "PUT",
        "/api/v1/admin/access",
        Some(&cookie),
        Some(json!({ "mode": "open" })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        store.setting::<Access>(auth::ACCESS_KEY).await.unwrap(),
        Some(Access::Open)
    );
    assert_eq!(
        call(&r, "GET", "/api/v1/scenes", None, None, &[]).await.0,
        StatusCode::OK,
        "applies at once"
    );

    let (status, _, _) = call(
        &r,
        "PUT",
        "/api/v1/admin/history",
        Some(&cookie),
        Some(json!({ "retention_days": 0 })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = call(
        &r,
        "PUT",
        "/api/v1/admin/history",
        Some(&cookie),
        Some(json!({ "retention_days": 30 })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, history, _) = call(&r, "GET", "/api/v1/admin/history", Some(&cookie), None, &[]).await;
    assert_eq!(history["retention_days"], json!(30));
}

#[tokio::test]
async fn scene_settings_apply_live_and_bad_ones_change_nothing() {
    let (r, store, cookie) = signed_in(Access::Admin).await;
    let (status, current, _) = call(
        &r,
        "GET",
        "/api/v1/admin/scenes/ci/settings",
        Some(&cookie),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(current["max_projects"], json!(8));

    let mut next = current.clone();
    next["exits"] = json!([{ "label": "prod", "stage": "deploy" }]);
    let (status, applied, _) = call(
        &r,
        "PUT",
        "/api/v1/admin/scenes/ci/settings",
        Some(&cookie),
        Some(next),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(applied["exits"][0]["label"], json!("prod"));
    assert_eq!(
        store.scene_settings("ci").await.unwrap().unwrap()["exits"][0]["label"],
        json!("prod")
    );

    let mut bad = applied.clone();
    bad["exits"] = json!([{ "label": "x", "stage": "(" }]);
    let (status, body, _) = call(
        &r,
        "PUT",
        "/api/v1/admin/scenes/ci/settings",
        Some(&cookie),
        Some(bad),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        store.scene_settings("ci").await.unwrap().unwrap()["exits"][0]["label"],
        json!("prod"),
        "nothing stored"
    );
    assert_eq!(
        call(
            &r,
            "GET",
            "/api/v1/admin/scenes/nope/settings",
            Some(&cookie),
            None,
            &[]
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_down_database_answers_503_and_me_still_works() {
    let (r, store, _) = signed_in(Access::Open).await;
    store.close().await;
    let (status, body, _) = call(
        &r,
        "POST",
        "/api/v1/login",
        None,
        Some(json!({ "password": PASSWORD })),
        &[],
    )
    .await;
    assert_eq!(
        (status, body),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "error": "the database is unavailable" })
        )
    );
    let (status, me, _) = call(&r, "GET", "/api/v1/me", None, None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(
        call(&r, "GET", "/api/v1/scenes/ci/snapshot", None, None, &[])
            .await
            .0,
        StatusCode::OK,
        "the live scene does not need the database"
    );
}

#[tokio::test]
async fn admin_mode_keeps_the_scenes_up_when_the_database_is_down() {
    let (r, store, cookie) = signed_in(Access::Admin).await;
    store.close().await;
    assert_eq!(
        call(
            &r,
            "GET",
            "/api/v1/scenes/ci/snapshot",
            Some(&cookie),
            None,
            &[]
        )
        .await
        .0,
        StatusCode::OK,
        "a session confirmed before keeps working"
    );
    assert_eq!(
        call(&r, "GET", "/api/v1/scenes/ci/snapshot", None, None, &[])
            .await
            .0,
        StatusCode::UNAUTHORIZED,
        "an anonymous request is still refused"
    );
    assert_eq!(
        call(
            &r,
            "GET",
            "/api/v1/scenes/ci/snapshot",
            Some("lorvaq_session=never-seen"),
            None,
            &[]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED,
        "an unknown session is not trusted"
    );
}

#[tokio::test]
async fn logout_forgets_the_session_at_once() {
    let (r, _, cookie) = signed_in(Access::Admin).await;
    assert_eq!(
        call(&r, "GET", "/api/v1/scenes", Some(&cookie), None, &[])
            .await
            .0,
        StatusCode::OK
    );
    call(&r, "POST", "/api/v1/logout", Some(&cookie), None, &[]).await;
    assert_eq!(
        call(&r, "GET", "/api/v1/scenes", Some(&cookie), None, &[])
            .await
            .0,
        StatusCode::UNAUTHORIZED,
        "not served from the cache"
    );
}

#[tokio::test]
async fn changing_the_password_signs_other_sessions_out() {
    let (r, _, mine) = signed_in(Access::Admin).await;
    let (_, _, other) = call(
        &r,
        "POST",
        "/api/v1/login",
        None,
        Some(json!({ "password": PASSWORD })),
        &[],
    )
    .await;
    let other = other.unwrap();
    let wrong = json!({ "current": "not it at all", "new": "a brand new password" });
    assert_eq!(
        call(
            &r,
            "POST",
            "/api/v1/admin/password",
            Some(&mine),
            Some(wrong),
            &[]
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let good = json!({ "current": PASSWORD, "new": "a brand new password" });
    assert_eq!(
        call(
            &r,
            "POST",
            "/api/v1/admin/password",
            Some(&mine),
            Some(good),
            &[]
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&r, "GET", "/api/v1/admin/access", Some(&mine), None, &[])
            .await
            .0,
        StatusCode::OK,
        "my session stays"
    );
    assert_eq!(
        call(&r, "GET", "/api/v1/admin/access", Some(&other), None, &[])
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let (status, _, _) = call(
        &r,
        "POST",
        "/api/v1/login",
        None,
        Some(json!({ "password": "a brand new password" })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}
