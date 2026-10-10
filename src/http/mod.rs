//! HTTP: scene list, snapshots, the live event stream (SSE), and the built-in UI.

mod static_files;

use crate::scene::SceneHandle;
use crate::world::bus::Gap;
use crate::world::Event;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tokio_stream::wrappers::BroadcastStream;

pub struct AppState {
    pub scenes: Vec<SceneHandle>,
}

impl AppState {
    fn scene(&self, id: &str) -> Option<&SceneHandle> {
        self.scenes.iter().find(|s| s.id == id)
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/api/v1/scenes", get(list_scenes))
        .route("/api/v1/scenes/{id}/snapshot", get(snapshot))
        .route("/api/v1/scenes/{id}/events", get(events))
        .fallback(static_files::serve)
        .with_state(state)
}

async fn healthz(State(s): State<Arc<AppState>>) -> Json<Value> {
    let scenes: Vec<Value> = s
        .scenes
        .iter()
        .map(|h| json!({ "id": h.id, "stale": h.snapshot().stale }))
        .collect();
    Json(json!({ "status": "ok", "scenes": scenes }))
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
        (
            router(Arc::new(AppState {
                scenes: vec![h.clone()],
            })),
            h,
        )
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
        assert!(body.contains(r#""status":"ok""#), "{body}");
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
