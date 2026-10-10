//! Admin settings: viewer access, history retention, scene settings.

use super::guard::{db_error, json_error};
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
    Json(access): Json<Access>,
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
    Json(settings): Json<HistorySettings>,
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
pub(super) async fn put_scene(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(value): Json<Value>,
) -> Response {
    let Some(h) = s.scene(&id) else {
        return json_error(StatusCode::NOT_FOUND, "no such scene");
    };
    match h.configure(value).await {
        Ok(applied) => match live(&s)
            .store
            .put_scene_settings(&id, &applied, now_ms())
            .await
        {
            Ok(()) => Json(applied).into_response(),
            Err(e) => db_error(e),
        },
        Err(e) => json_error(StatusCode::BAD_REQUEST, &format!("{e:#}")),
    }
}
