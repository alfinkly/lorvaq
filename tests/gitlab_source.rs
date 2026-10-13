mod common;

use lorvaq::crypto::SecretBox;
use lorvaq::sources::gitlab::client::{http_client, GitlabError};
use lorvaq::sources::gitlab::fake::FakeGitlab;
use lorvaq::sources::gitlab::oauth::{
    self, bearer, pkce_challenge, pkce_verifier, Connection, StoredAuth,
};

fn secrets() -> SecretBox {
    SecretBox::from_base64(&SecretBox::generate_key()).unwrap()
}

fn oauth_connection(
    s: &SecretBox,
    url: &str,
    verifier: &str,
    access: &str,
    refresh: &str,
    expires_ms: i64,
) -> Connection {
    Connection {
        url: url.into(),
        user: "dana".into(),
        auth: StoredAuth::Oauth {
            client_id: "app-id".into(),
            client_secret: None,
            redirect_uri: "http://l/cb".into(),
            verifier: s.seal(verifier.as_bytes()),
            access: s.seal(access.as_bytes()),
            refresh: s.seal(refresh.as_bytes()),
            expires_ms,
        },
        connected_ms: 0,
    }
}

#[tokio::test]
async fn a_token_connection_opens_its_token() {
    let store = common::store().await;
    let s = secrets();
    let mut conn = Connection {
        url: "http://x".into(),
        user: "dana".into(),
        auth: StoredAuth::Token {
            token: s.seal(b"good-token"),
        },
        connected_ms: 0,
    };
    assert_eq!(conn.auth.kind(), "token");
    assert_eq!(
        bearer(&mut conn, &s, &http_client(), &store, 0)
            .await
            .unwrap(),
        "good-token"
    );
    let other = secrets();
    assert!(
        matches!(
            bearer(&mut conn, &other, &http_client(), &store, 0)
                .await
                .unwrap_err(),
            GitlabError::Local(_)
        ),
        "another key cannot open it"
    );
}

#[tokio::test]
async fn an_expiring_oauth_token_is_refreshed_and_the_new_pair_stored() {
    let store = common::store().await;
    let fake = FakeGitlab::start().await;
    let s = secrets();
    let verifier = pkce_verifier();
    fake.state.lock().unwrap().refresh = "refresh-0".into();
    let mut conn = oauth_connection(
        &s,
        &fake.url,
        &verifier,
        "old-access",
        "refresh-0",
        10 * 60 * 1000,
    );
    let http = http_client();

    let still_fresh = bearer(&mut conn, &s, &http, &store, 0).await.unwrap();
    assert_eq!(still_fresh, "old-access", "10 minutes left: no refresh");
    assert_eq!(fake.state.lock().unwrap().token_requests, 0);

    let refreshed = bearer(&mut conn, &s, &http, &store, 6 * 60 * 1000)
        .await
        .unwrap();
    assert_eq!(refreshed, "access-1");
    let stored: Connection = store.setting(oauth::CONNECTION_KEY).await.unwrap().unwrap();
    let StoredAuth::Oauth {
        access, refresh, ..
    } = &stored.auth
    else {
        panic!("oauth expected")
    };
    assert_eq!(s.open(access).unwrap(), b"access-1");
    assert_eq!(s.open(refresh).unwrap(), b"refresh-1");
    let raw: serde_json::Value = store.setting(oauth::CONNECTION_KEY).await.unwrap().unwrap();
    assert!(
        !raw.to_string().contains("access-1") && !raw.to_string().contains("refresh-1"),
        "secrets are sealed: {raw}"
    );
}

#[tokio::test]
async fn a_spent_refresh_token_asks_for_authorization() {
    let store = common::store().await;
    let fake = FakeGitlab::start().await;
    let s = secrets();
    let mut conn = oauth_connection(&s, &fake.url, &pkce_verifier(), "old", "spent", 0);
    assert_eq!(
        bearer(&mut conn, &s, &http_client(), &store, 1)
            .await
            .unwrap_err(),
        GitlabError::Unauthorized
    );
    let _ = pkce_challenge("x");
}

use lorvaq::scenes::ci::input::CiInput;
use lorvaq::sources::gitlab::poller::{self, PollConfig, SourceState};
use std::time::Duration;
use tokio::sync::mpsc;

fn quick() -> PollConfig {
    PollConfig {
        every: Duration::from_millis(50),
        runners_every: Duration::from_millis(300),
        stale_after: Duration::from_millis(150),
        max_backoff: Duration::from_millis(100),
    }
}

async fn next(rx: &mut mpsc::Receiver<CiInput>) -> CiInput {
    tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .expect("an input within 3 s")
        .expect("the poller is alive")
}

/// Skips inputs until one matches.
async fn until(rx: &mut mpsc::Receiver<CiInput>, wanted: impl Fn(&CiInput) -> bool) -> CiInput {
    loop {
        let input = next(rx).await;
        if wanted(&input) {
            return input;
        }
    }
}

async fn connect_token(store: &lorvaq::store::Store, s: &SecretBox, url: &str) {
    let conn = Connection {
        url: url.into(),
        user: "dana".into(),
        auth: StoredAuth::Token {
            token: s.seal(b"good-token"),
        },
        connected_ms: 0,
    };
    store
        .put_setting(oauth::CONNECTION_KEY, &conn)
        .await
        .unwrap();
}

#[tokio::test]
async fn polls_runners_then_jobs_once_connected() {
    let store = common::store().await;
    let fake = FakeGitlab::start().await;
    {
        let mut st = fake.state.lock().unwrap();
        st.active = vec![FakeGitlab::job(1, "demo/backend", "PENDING", None)];
        st.runners = vec![FakeGitlab::runner(7, &["ops"], true)];
    }
    let s = secrets();
    let (tx, mut rx) = mpsc::channel(64);
    let handle = poller::spawn(store.clone(), s.clone(), http_client(), tx, quick());
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(rx.try_recv().is_err(), "nothing while disconnected");
    assert_eq!(
        handle.status.lock().unwrap().state,
        SourceState::Disconnected
    );

    connect_token(&store, &s, &fake.url).await;
    handle.reload.notify_one();
    assert!(matches!(next(&mut rx).await, CiInput::Runners(r) if r[0].id == 7));
    assert!(matches!(next(&mut rx).await, CiInput::Jobs(j) if j.active[0].id == 1));
    let st = handle.status.lock().unwrap().clone();
    assert_eq!(st.state, SourceState::Ok);
    assert!(st.last_ok_ms.is_some());
}

#[tokio::test]
async fn a_failing_gitlab_goes_stale_and_comes_back() {
    let store = common::store().await;
    let fake = FakeGitlab::start().await;
    let s = secrets();
    connect_token(&store, &s, &fake.url).await;
    let (tx, mut rx) = mpsc::channel(64);
    let handle = poller::spawn(store, s, http_client(), tx, quick());
    until(&mut rx, |i| matches!(i, CiInput::Jobs(_))).await;

    fake.state.lock().unwrap().fail_with = Some(502);
    let stale = until(&mut rx, |i| matches!(i, CiInput::SourceStale { .. })).await;
    assert!(matches!(stale, CiInput::SourceStale { source, .. } if source == "gitlab"));
    assert_eq!(handle.status.lock().unwrap().state, SourceState::Failing);
    assert!(handle
        .status
        .lock()
        .unwrap()
        .last_error
        .as_deref()
        .unwrap()
        .contains("502"));

    fake.state.lock().unwrap().fail_with = None;
    until(&mut rx, |i| matches!(i, CiInput::SourceFresh { .. })).await;
    assert_eq!(handle.status.lock().unwrap().state, SourceState::Ok);
}

#[tokio::test]
async fn a_revoked_token_asks_for_authorization() {
    let store = common::store().await;
    let fake = FakeGitlab::start().await;
    let s = secrets();
    connect_token(&store, &s, &fake.url).await;
    let (tx, mut rx) = mpsc::channel(64);
    let handle = poller::spawn(store, s, http_client(), tx, quick());
    until(&mut rx, |i| matches!(i, CiInput::Jobs(_))).await;
    fake.state.lock().unwrap().token = "rotated-elsewhere".into();
    until(&mut rx, |i| matches!(i, CiInput::SourceStale { .. })).await;
    assert_eq!(
        handle.status.lock().unwrap().state,
        SourceState::Reauthorize
    );
}

#[tokio::test]
async fn disconnecting_empties_the_scene_and_clears_staleness() {
    let store = common::store().await;
    let fake = FakeGitlab::start().await;
    fake.state.lock().unwrap().active =
        vec![FakeGitlab::job(1, "demo/backend", "RUNNING", Some(7))];
    let s = secrets();
    connect_token(&store, &s, &fake.url).await;
    let (tx, mut rx) = mpsc::channel(64);
    let handle = poller::spawn(store.clone(), s, http_client(), tx, quick());
    until(
        &mut rx,
        |i| matches!(i, CiInput::Jobs(j) if !j.active.is_empty()),
    )
    .await;
    fake.state.lock().unwrap().fail_with = Some(500);
    until(&mut rx, |i| matches!(i, CiInput::SourceStale { .. })).await;

    store.delete_setting(oauth::CONNECTION_KEY).await.unwrap();
    handle.reload.notify_one();
    until(
        &mut rx,
        |i| matches!(i, CiInput::Jobs(j) if j.active.is_empty() && j.recent_finished.is_empty()),
    )
    .await;
    until(&mut rx, |i| matches!(i, CiInput::SourceFresh { .. })).await;
    assert_eq!(
        handle.status.lock().unwrap().state,
        SourceState::Disconnected
    );
}
