mod common;

use lorvaq::store::Store;
use lorvaq::world::{Change, Event};
use serde_json::json;

#[tokio::test]
async fn migrations_can_run_again() {
    let (_, url) = common::store_with_url().await;
    Store::connect(&url)
        .await
        .expect("second start on the same database");
}

#[tokio::test]
async fn settings_round_trip() {
    let s = common::store().await;
    assert!(s.ping().await);
    assert_eq!(s.setting::<i64>("x").await.unwrap(), None);
    s.put_setting("x", &5i64).await.unwrap();
    s.put_setting("x", &7i64).await.unwrap();
    assert_eq!(s.setting::<i64>("x").await.unwrap(), Some(7));
    assert!(
        s.setting::<String>("x").await.is_err(),
        "wrong shape is an error, not a silent default"
    );
    s.delete_setting("x").await.unwrap();
    assert_eq!(s.setting::<i64>("x").await.unwrap(), None);
}

#[tokio::test]
async fn admin_and_sessions() {
    let s = common::store().await;
    assert_eq!(s.admin_hash().await.unwrap(), None);
    s.set_admin_hash("h1", 1).await.unwrap();
    s.set_admin_hash("h2", 2).await.unwrap();
    assert_eq!(s.admin_hash().await.unwrap().as_deref(), Some("h2"));

    s.create_session("a", 100, 50).await.unwrap();
    s.create_session("b", 100, 50).await.unwrap();
    assert!(s.session_valid("a", 149).await.unwrap());
    assert!(!s.session_valid("a", 150).await.unwrap(), "expired");
    assert!(!s.session_valid("nope", 120).await.unwrap());

    s.delete_other_sessions("a").await.unwrap();
    assert!(s.session_valid("a", 120).await.unwrap());
    assert!(!s.session_valid("b", 120).await.unwrap());
    s.delete_session("a").await.unwrap();
    assert!(!s.session_valid("a", 120).await.unwrap());

    s.create_session("c", 100, 50).await.unwrap();
    s.reset_admin().await.unwrap();
    assert_eq!(s.admin_hash().await.unwrap(), None);
    assert!(
        !s.session_valid("c", 120).await.unwrap(),
        "reset signs everybody out"
    );
}

#[tokio::test]
async fn expired_sessions_are_pruned() {
    let s = common::store().await;
    s.create_session("old", 0, 10).await.unwrap();
    s.create_session("new", 0, 1_000).await.unwrap();
    assert_eq!(s.prune_sessions(500).await.unwrap(), 1);
    assert!(s.session_valid("new", 500).await.unwrap());
}

#[tokio::test]
async fn scene_settings_round_trip() {
    let s = common::store().await;
    assert_eq!(s.scene_settings("ci").await.unwrap(), None);
    s.put_scene_settings("ci", &json!({"max_projects": 3}), 1)
        .await
        .unwrap();
    s.put_scene_settings("ci", &json!({"max_projects": 4}), 2)
        .await
        .unwrap();
    assert_eq!(
        s.scene_settings("ci").await.unwrap(),
        Some(json!({"max_projects": 4}))
    );
}

#[tokio::test]
async fn events_are_written_counted_and_pruned() {
    let s = common::store().await;
    s.insert_events("ci", &[]).await.unwrap();
    let ev = |seq: u64, ts_ms: i64| Event {
        seq,
        ts_ms,
        change: Change::Gone {
            id: format!("job:{seq}"),
        },
    };
    s.insert_events("ci", &[ev(1, 100), ev(2, 200), ev(3, 300)])
        .await
        .unwrap();
    s.insert_events("other", &[ev(1, 100)]).await.unwrap();
    assert_eq!(s.event_count("ci").await.unwrap(), 3);
    assert_eq!(
        s.prune_events(250).await.unwrap(),
        3,
        "two of ci and one of other are older"
    );
    assert_eq!(s.event_count("ci").await.unwrap(), 1);
}
