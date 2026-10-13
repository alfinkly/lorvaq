//! HTTP: scenes (behind the viewer guard), sign-in, admin settings (behind the
//! admin guard), and the built-in UI.

mod account;
mod admin;
mod guard;
mod json;
mod sessions;
mod sources;
mod static_files;

use crate::auth::{Access, LoginLimiter};
use crate::crypto::SecretBox;
use crate::history::HistoryStatus;
use crate::scene::SceneHandle;
use crate::store::Store;
use crate::world::bus::Gap;
use crate::world::Event;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::convert::Infallible;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio_stream::wrappers::BroadcastStream;

pub use sessions::SessionCache;

/// How many password hash/verify runs may go at once.
const ARGON_RUNS: usize = 2;

pub struct AppState {
    pub scenes: Vec<SceneHandle>,
    /// None in demo mode: no database, no admin, everything open.
    pub live: Option<Live>,
}

pub struct Live {
    pub store: Store,
    /// Encrypts source credentials (used from plan 3 on).
    pub secrets: SecretBox,
    pub cookie_secure: bool,
    pub access: RwLock<Access>,
    pub limiter: Mutex<LoginLimiter>,
    pub history: Vec<(&'static str, Arc<Mutex<HistoryStatus>>)>,
    /// Admin sessions the database confirmed lately.
    pub sessions: Mutex<SessionCache>,
    /// Caps concurrent password hash/verify runs (each takes about 19 MiB).
    pub argon: Arc<Semaphore>,
    /// One scene-settings write at a time: configure, store and revert must not interleave.
    pub scene_write: tokio::sync::Mutex<()>,
    /// The GitLab poller, when a scene uses GitLab.
    pub gitlab: Option<crate::sources::gitlab::GitlabHandle>,
    /// LORVAQ_PUBLIC_URL without the trailing slash: OAuth sends the browser back here.
    pub public_url: Option<String>,
    /// For calls to GitLab.
    pub http: reqwest::Client,
}

impl Live {
    pub fn new(
        store: Store,
        secrets: SecretBox,
        cookie_secure: bool,
        access: Access,
        history: Vec<(&'static str, Arc<Mutex<HistoryStatus>>)>,
    ) -> Self {
        Self {
            store,
            secrets,
            cookie_secure,
            access: RwLock::new(access),
            limiter: Mutex::new(LoginLimiter::default()),
            history,
            sessions: Mutex::new(SessionCache::default()),
            argon: Arc::new(Semaphore::new(ARGON_RUNS)),
            scene_write: tokio::sync::Mutex::new(()),
            gitlab: None,
            public_url: None,
            http: crate::sources::gitlab::client::http_client(),
        }
    }
}

impl AppState {
    pub fn demo(scenes: Vec<SceneHandle>) -> Self {
        Self { scenes, live: None }
    }

    fn scene(&self, id: &str) -> Option<&SceneHandle> {
        self.scenes.iter().find(|s| s.id == id)
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    let viewer = Router::new()
        .route("/api/v1/scenes", get(list_scenes))
        .route("/api/v1/scenes/{id}/snapshot", get(snapshot))
        .route("/api/v1/scenes/{id}/events", get(events))
        .route_layer(middleware::from_fn_with_state(state.clone(), guard::viewer));
    let admin = Router::new()
        .route(
            "/api/v1/admin/access",
            get(admin::get_access).put(admin::put_access),
        )
        .route(
            "/api/v1/admin/history",
            get(admin::get_history).put(admin::put_history),
        )
        .route(
            "/api/v1/admin/scenes/{id}/settings",
            get(admin::get_scene).put(admin::put_scene),
        )
        .route("/api/v1/admin/password", post(account::change_password))
        .route(
            "/api/v1/admin/sources/gitlab",
            get(sources::status).delete(sources::disconnect),
        )
        .route(
            "/api/v1/admin/sources/gitlab/token",
            post(sources::connect_token),
        )
        .route(
            "/api/v1/admin/sources/gitlab/oauth",
            post(sources::oauth_start),
        )
        .route_layer(middleware::from_fn_with_state(state.clone(), guard::admin));
    Router::new()
        .route("/healthz", get(healthz))
        .route("/api/v1/me", get(account::me))
        .route("/api/v1/setup", post(account::setup))
        .route("/api/v1/login", post(account::login))
        .route("/api/v1/logout", post(account::logout))
        .route(
            "/api/v1/oauth/gitlab/callback",
            get(sources::oauth_callback),
        )
        .merge(viewer)
        .merge(admin)
        .fallback(static_files::serve)
        .with_state(state)
}

async fn healthz(State(s): State<Arc<AppState>>) -> Json<Value> {
    let scenes: Vec<Value> = s
        .scenes
        .iter()
        .map(|h| json!({ "id": h.id, "stale": h.stale() }))
        .collect();
    let db = match &s.live {
        Some(live) => json!(live.store.ping().await),
        None => Value::Null,
    };
    Json(json!({ "status": "ok", "db": db, "scenes": scenes }))
}

async fn list_scenes(State(s): State<Arc<AppState>>) -> Json<Value> {
    Json(json!(s
        .scenes
        .iter()
        .map(|h| json!({ "id": h.id, "title": h.title }))
        .collect::<Vec<_>>()))
}

async fn snapshot(State(s): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match s.scene(&id) {
        Some(h) => Json(h.snapshot()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Deserialize)]
struct EventsQuery {
    since: Option<u64>,
}

/// Backlog after `since` (or `Last-Event-ID`), then live events. A gap is 410:
/// the client takes a fresh snapshot. A lagging client is cut off and reconnects.
async fn events(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<EventsQuery>,
    headers: HeaderMap,
) -> Response {
    let Some(h) = s.scene(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let from_header = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    // Subscribe before reading the backlog, so nothing falls between the two.
    let rx = h.bus.subscribe();
    let after = from_header.or(q.since).unwrap_or_else(|| h.bus.last_seq());
    let backlog = match h.bus.since(after) {
        Ok(events) => events,
        Err(Gap) => return StatusCode::GONE.into_response(),
    };
    let sent_up_to = backlog.last().map_or(after, |e| e.seq);
    let live = BroadcastStream::new(rx)
        .take_while(|r| futures::future::ready(r.is_ok()))
        .filter_map(|r| futures::future::ready(r.ok()))
        .filter(move |e| futures::future::ready(e.seq > sent_up_to));
    let stream = futures::stream::iter(backlog).chain(live).map(to_sse);
    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response()
}

fn to_sse(ev: Event) -> Result<SseEvent, Infallible> {
    Ok(SseEvent::default()
        .id(ev.seq.to_string())
        .json_data(&ev)
        .expect("events always serialize"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{Attrs, Change, Place};
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn app(cap: usize) -> (Router, SceneHandle) {
        let h = SceneHandle::new("test", "Test", cap);
        (router(Arc::new(AppState::demo(vec![h.clone()]))), h)
    }

    fn zone(id: &str) -> Change {
        Change::UpsertPlace {
            place: Place {
                id: id.into(),
                kind: "zone".into(),
                parent: None,
                label: id.into(),
                capacity: None,
                attrs: Attrs::new(),
            },
        }
    }

    async fn get(app: &Router, uri: &str) -> (StatusCode, String) {
        let res = app
            .clone()
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let body = res.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&body).into())
    }

    /// Event ids in an SSE text, in order.
    fn ids(text: &str) -> Vec<u64> {
        text.lines()
            .filter_map(|l| l.strip_prefix("id:")?.trim().parse().ok())
            .collect()
    }

    /// Reads SSE frames until `want` events arrived; fails after 2 s of silence.
    async fn read_events(res: Response, want: usize) -> String {
        assert_eq!(res.status(), StatusCode::OK);
        let mut body = res.into_body();
        let mut text = String::new();
        while ids(&text).len() < want {
            let frame = tokio::time::timeout(Duration::from_secs(2), body.frame())
                .await
                .expect("an event within 2s")
                .expect("stream open")
                .unwrap();
            if let Ok(data) = frame.into_data() {
                text.push_str(&String::from_utf8_lossy(&data));
            }
        }
        text
    }

    async fn open(app: &Router, uri: &str, last_event_id: Option<&str>) -> Response {
        let mut req = Request::get(uri);
        if let Some(id) = last_event_id {
            req = req.header("last-event-id", id);
        }
        app.clone()
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn lists_scenes_and_health() {
        let (app, _) = app(10);
        let (status, body) = get(&app, "/api/v1/scenes").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            serde_json::from_str::<Value>(&body).unwrap(),
            json!([{"id": "test", "title": "Test"}])
        );
        let (status, body) = get(&app, "/healthz").await;
        assert_eq!(status, StatusCode::OK);
        let health: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            (health["status"].as_str(), health["db"].clone()),
            (Some("ok"), Value::Null)
        );
    }

    #[tokio::test]
    async fn demo_mode_is_open_and_has_no_admin() {
        let (app, _) = app(10);
        let (status, body) = get(&app, "/api/v1/me").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            serde_json::from_str::<Value>(&body).unwrap(),
            json!({"demo": true, "setup_needed": false, "admin": false, "viewer": true})
        );
        assert_eq!(
            get(&app, "/api/v1/admin/access").await.0,
            StatusCode::NOT_FOUND
        );
        let req = Request::post("/api/v1/login")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"password":"x"}"#))
            .unwrap();
        assert_eq!(
            app.clone().oneshot(req).await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn snapshot_of_known_and_unknown_scenes() {
        let (app, h) = app(10);
        h.commit(vec![zone("a"), zone("b")], 1);
        let (status, body) = get(&app, "/api/v1/scenes/test/snapshot").await;
        assert_eq!(status, StatusCode::OK);
        let snap: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            (
                snap["seq"].as_u64(),
                snap["places"].as_array().map(Vec::len)
            ),
            (Some(2), Some(2))
        );
        assert_eq!(
            get(&app, "/api/v1/scenes/nope/snapshot").await.0,
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn events_replay_the_backlog() {
        let (app, h) = app(10);
        h.commit(vec![zone("a"), zone("b")], 1);
        let text = read_events(
            open(&app, "/api/v1/scenes/test/events?since=0", None).await,
            2,
        )
        .await;
        assert_eq!(ids(&text), vec![1, 2], "{text}");
        assert!(text.contains(r#""op":"upsert_place""#), "{text}");
    }

    #[tokio::test]
    async fn events_stream_live_changes_without_duplicates() {
        let (app, h) = app(10);
        h.commit(vec![zone("a")], 1);
        let res = open(&app, "/api/v1/scenes/test/events?since=0", None).await;
        h.commit(vec![zone("b")], 2);
        let text = read_events(res, 2).await;
        assert_eq!(ids(&text), vec![1, 2], "each event exactly once: {text}");
    }

    #[tokio::test]
    async fn last_event_id_wins_over_the_query() {
        let (app, h) = app(10);
        h.commit(vec![zone("a"), zone("b")], 1);
        let text = read_events(
            open(&app, "/api/v1/scenes/test/events?since=0", Some("1")).await,
            1,
        )
        .await;
        assert_eq!(ids(&text), vec![2], "{text}");
    }

    #[tokio::test]
    async fn gaps_and_future_ids_are_gone() {
        let (app, h) = app(2);
        h.commit(vec![zone("a"), zone("b"), zone("c"), zone("d")], 1);
        assert_eq!(
            open(&app, "/api/v1/scenes/test/events?since=1", None)
                .await
                .status(),
            StatusCode::GONE
        );
        assert_eq!(
            open(&app, "/api/v1/scenes/test/events?since=9", None)
                .await
                .status(),
            StatusCode::GONE
        );
        assert_eq!(
            open(&app, "/api/v1/scenes/test/events?since=2", None)
                .await
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            open(&app, "/api/v1/scenes/nope/events", None)
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn ui_is_served_and_unknown_api_is_404() {
        let (app, _) = app(10);
        let (status, body) = get(&app, "/").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("lorvaq"), "{body}");
        assert_eq!(get(&app, "/api/v1/nothing").await.0, StatusCode::NOT_FOUND);
    }
}
