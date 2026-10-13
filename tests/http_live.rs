mod common;

use axum::extract::connect_info::MockConnectInfo;
use axum::http::StatusCode;
use axum::Router;
use common::{call, call_raw};
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

#[tokio::test]
async fn overlong_passwords_are_refused_before_any_work() {
    let (r, _, cookie) = signed_in(Access::Admin).await;
    let long = "x".repeat(1025);
    let exact = "y".repeat(1024);
    // Six attempts would use the limiter up (and a setup would say 409) if they were counted.
    for _ in 0..6 {
        let (status, body, _) = call(
            &r,
            "POST",
            "/api/v1/login",
            None,
            Some(json!({ "password": long })),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "login: {body}");
        let (status, body, _) = call(
            &r,
            "POST",
            "/api/v1/setup",
            None,
            Some(json!({ "code": "WRONG", "password": long })),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "setup: {body}");
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
    assert_eq!(status, StatusCode::OK, "the limiter was not touched");
    for body in [
        json!({ "current": PASSWORD, "new": long }),
        json!({ "current": long, "new": exact }),
    ] {
        let (status, _, _) = call(
            &r,
            "POST",
            "/api/v1/admin/password",
            Some(&cookie),
            Some(body),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    let (status, _, _) = call(
        &r,
        "POST",
        "/api/v1/admin/password",
        Some(&cookie),
        Some(json!({ "current": PASSWORD, "new": exact })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "1024 characters are fine");
}

#[tokio::test]
async fn parallel_wrong_logins_are_counted_before_they_are_checked() {
    let (r, _, _) = signed_in(Access::Admin).await;
    let attempts = (0..10).map(|_| {
        call(
            &r,
            "POST",
            "/api/v1/login",
            None,
            Some(json!({ "password": "wrong wrong wrong" })),
            &[],
        )
    });
    let statuses: Vec<StatusCode> = futures::future::join_all(attempts)
        .await
        .into_iter()
        .map(|(status, _, _)| status)
        .collect();
    let count = |code| statuses.iter().filter(|s| **s == code).count();
    assert!(count(StatusCode::UNAUTHORIZED) <= 5, "{statuses:?}");
    assert_eq!(
        count(StatusCode::UNAUTHORIZED) + count(StatusCode::TOO_MANY_REQUESTS),
        10,
        "{statuses:?}"
    );
}

#[tokio::test]
async fn body_rejections_answer_with_an_error_object() {
    let (r, _, cookie) = signed_in(Access::Admin).await;
    let json = Some("application/json");
    for (status, uri, ct, body) in [
        (StatusCode::BAD_REQUEST, "/api/v1/login", json, "{not json"),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            "/api/v1/login",
            json,
            "{}",
        ),
        (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "/api/v1/login",
            None,
            r#"{"password":"x"}"#,
        ),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            "/api/v1/admin/access",
            json,
            r#"{"mode":"proxy"}"#,
        ),
    ] {
        let method = if uri.contains("admin") { "PUT" } else { "POST" };
        let (got, answer) = call_raw(&r, method, uri, Some(&cookie), ct, body).await;
        assert_eq!(got, status, "{uri} {body}: {answer}");
        assert!(
            answer["error"].as_str().is_some_and(|m| !m.is_empty()),
            "{uri} {body}: {answer}"
        );
    }
}

#[tokio::test]
async fn nonsense_numbers_in_scene_settings_are_refused() {
    let (r, store, cookie) = signed_in(Access::Admin).await;
    let (_, mut next, _) = call(
        &r,
        "GET",
        "/api/v1/admin/scenes/ci/settings",
        Some(&cookie),
        None,
        &[],
    )
    .await;
    next["max_projects"] = json!(0);
    let (status, body, _) = call(
        &r,
        "PUT",
        "/api/v1/admin/scenes/ci/settings",
        Some(&cookie),
        Some(next),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("max_projects"));
    assert_eq!(store.scene_settings("ci").await.unwrap(), None);
}

#[tokio::test]
async fn a_failed_store_puts_the_scene_settings_back() {
    let (r, store, cookie) = signed_in(Access::Admin).await;
    let (_, mut next, _) = call(
        &r,
        "GET",
        "/api/v1/admin/scenes/ci/settings",
        Some(&cookie),
        None,
        &[],
    )
    .await;
    let before = next.clone();
    next["max_projects"] = json!(3);
    store.close().await; // the cached session still lets the admin in
    let (status, _, _) = call(
        &r,
        "PUT",
        "/api/v1/admin/scenes/ci/settings",
        Some(&cookie),
        Some(next),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let (_, now, _) = call(
        &r,
        "GET",
        "/api/v1/admin/scenes/ci/settings",
        Some(&cookie),
        None,
        &[],
    )
    .await;
    assert_eq!(now, before, "the scene is back on its old settings");
}

#[tokio::test]
async fn scene_settings_are_written_one_request_at_a_time() {
    let store = common::store().await;
    store
        .set_admin_hash(&crypto::hash_password(PASSWORD), 0)
        .await
        .unwrap();
    let (ci, _inputs) = scene::spawn(
        CiScene::new(CiSettings::default()).unwrap(),
        Duration::from_secs(1),
    );
    let live = Live::new(
        store,
        SecretBox::from_base64(&SecretBox::generate_key()).unwrap(),
        false,
        Access::Admin,
        vec![],
    );
    let state = Arc::new(AppState {
        scenes: vec![ci],
        live: Some(live),
    });
    let r = router(state.clone()).layer(MockConnectInfo(SocketAddr::from(([10, 1, 2, 3], 4000))));
    let (_, _, cookie) = call(
        &r,
        "POST",
        "/api/v1/login",
        None,
        Some(json!({ "password": PASSWORD })),
        &[],
    )
    .await;
    let cookie = cookie.unwrap();
    let uri = "/api/v1/admin/scenes/ci/settings";
    let (_, mut next, _) = call(&r, "GET", uri, Some(&cookie), None, &[]).await;
    next["max_projects"] = json!(3);

    // Another settings write is in progress: this one waits for it.
    let busy = state.live.as_ref().unwrap().scene_write.lock().await;
    let waiting = tokio::spawn({
        let (r, cookie) = (r.clone(), cookie.clone());
        async move { call(&r, "PUT", uri, Some(&cookie), Some(next), &[]).await }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!waiting.is_finished(), "it must not run in parallel");
    let (_, now, _) = call(&r, "GET", uri, Some(&cookie), None, &[]).await;
    assert_ne!(now["max_projects"], json!(3), "nothing applied yet");
    drop(busy);
    let (status, _, _) = waiting.await.unwrap();
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn signing_out_ends_open_event_streams() {
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let (r, _, cookie) = signed_in(Access::Admin).await;
    let req = axum::http::Request::get("/api/v1/scenes/ci/events?since=0")
        .header(axum::http::header::COOKIE, &cookie)
        .body(axum::body::Body::empty())
        .unwrap();
    let res = r.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let mut body = res.into_body();
    let (status, _, _) = call(&r, "POST", "/api/v1/logout", Some(&cookie), None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    let ended = tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(frame) = body.frame().await {
            frame.unwrap();
        }
    })
    .await;
    assert!(ended.is_ok(), "the stream closed after sign-out");
}

#[tokio::test]
async fn sign_out_clears_the_cookie_even_with_the_database_down() {
    let (r, store, cookie) = signed_in(Access::Admin).await;
    store.close().await;
    let (status, _, cleared) = call(&r, "POST", "/api/v1/logout", Some(&cookie), None, &[]).await;
    assert_eq!(
        (status, cleared.as_deref()),
        (StatusCode::OK, Some("lorvaq_session="))
    );
}
