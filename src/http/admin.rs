//! Admin settings: viewer access, history retention, scene settings.

use super::guard::{db_error, json_error};
use super::json::ApiJson;
use super::AppState;
use crate::auth::{self, Access};
use crate::history::{HistorySettings, HISTORY_KEY};
use crate::scene::{now_ms, LOCK};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use std::sync::Arc;

fn live(s: &AppState) -> &super::Live {
    s.live
        .as_ref()
        .expect("the admin guard lets only live requests in")
}

pub(super) async fn get_access(State(s): State<Arc<AppState>>) -> Json<Access> {
    Json(live(&s).access.read().expect(LOCK).clone())
}

pub(super) async fn put_access(
    State(s): State<Arc<AppState>>,
    ApiJson(access): ApiJson<Access>,
) -> Response {
    if let Err(message) = access.validate() {
        return json_error(StatusCode::BAD_REQUEST, &message);
    }
    let live = live(&s);
    if let Err(e) = live.store.put_setting(auth::ACCESS_KEY, &access).await {
        return db_error(e);
    }
    *live.access.write().expect(LOCK) = access.clone();
    Json(access).into_response()
}

pub(super) async fn get_history(State(s): State<Arc<AppState>>) -> Response {
    let live = live(&s);
    let settings = match live.store.setting::<HistorySettings>(HISTORY_KEY).await {
        Ok(found) => found.unwrap_or_default(),
        Err(e) => return db_error(e),
    };
    let scenes: Vec<Value> = live
        .history
        .iter()
        .map(|(id, status)| {
            let st = status.lock().expect(LOCK).clone();
            json!({ "id": id, "queued": st.queued, "dropped": st.dropped, "written": st.written, "last_error": st.last_error })
        })
        .collect();
    Json(json!({ "retention_days": settings.retention_days, "scenes": scenes })).into_response()
}

pub(super) async fn put_history(
    State(s): State<Arc<AppState>>,
    ApiJson(settings): ApiJson<HistorySettings>,
) -> Response {
    if let Err(message) = settings.validate() {
        return json_error(StatusCode::BAD_REQUEST, &message);
    }
    if let Err(e) = live(&s).store.put_setting(HISTORY_KEY, &settings).await {
        return db_error(e);
    }
    Json(settings).into_response()
}

pub(super) async fn get_scene(State(s): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match s.scene(&id) {
        Some(h) => Json(h.settings()).into_response(),
        None => json_error(StatusCode::NOT_FOUND, "no such scene"),
    }
}

/// Applies the settings in the running scene first; only what it accepted is stored.
/// When storing fails, the scene goes back to its previous settings.
pub(super) async fn put_scene(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    ApiJson(value): ApiJson<Value>,
) -> Response {
    let Some(h) = s.scene(&id) else {
        return json_error(StatusCode::NOT_FOUND, "no such scene");
    };
    let previous = h.settings();
    let applied = match h.configure(value).await {
        Ok(applied) => applied,
        Err(e) => return json_error(StatusCode::BAD_REQUEST, &format!("{e:#}")),
    };
    if let Err(e) = live(&s)
        .store
        .put_scene_settings(&id, &applied, now_ms())
        .await
    {
        if let Err(revert) = h.configure(previous).await {
            tracing::error!("cannot put the scene settings back: {revert:#}");
        }
        return db_error(e);
    }
    Json(applied).into_response()
}
