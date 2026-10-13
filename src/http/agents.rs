//! Agents: the WebSocket they connect to (join token), and the admin's list of hosts.

use super::guard::{db_error, json_error};
use super::json::ApiJson;
use super::sources::public_base;
use super::{AppState, Live};
use crate::agent::proto::{FromAgent, ToAgent, MAX_FRAME, SILENCE};
use crate::crypto;
use crate::hub::{Hub, Online};
use crate::scene::now_ms;
use crate::scenes::ci::input::{AgentReport, CiInput, ContainerExit};
use axum::extract::ws::{close_code, CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

/// `last_seen_ms` is written at most this often per connection.
const TOUCH_EVERY_MS: i64 = 60_000;
pub const IMAGE: &str = "ghcr.io/alfinkly/lorvaq:latest";

#[derive(Deserialize)]
pub(super) struct NewAgent {
    name: String,
}

fn live(s: &AppState) -> &Live {
    s.live
        .as_ref()
        .expect("the admin guard lets only live requests in")
}

fn hub(live: &Live) -> Option<&Arc<Hub>> {
    live.agents.as_ref()
}

pub fn name_problem(name: &str) -> Option<&'static str> {
    let ok = !name.is_empty()
        && name.len() <= 40
        && name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'));
    (!ok).then_some(
        "use 1 to 40 lowercase letters, digits, '.', '_' or '-', starting with a letter or digit",
    )
}

/// The command to start an agent on a runner host.
pub fn docker_command(server: &str, token: &str) -> String {
    format!(
        "docker run -d --name lorvaq-agent --restart unless-stopped \\\n  \
         --group-add \"$(stat -c %g /var/run/docker.sock)\" \\\n  \
         -v /var/run/docker.sock:/var/run/docker.sock:ro \\\n  \
         -e LORVAQ_SERVER={server} \\\n  \
         -e LORVAQ_JOIN={token} \\\n  \
         -e LORVAQ_HOST_LABEL=\"$(hostname)\" \\\n  \
         {IMAGE} agent"
    )
}

pub(super) async fn list(State(s): State<Arc<AppState>>) -> Response {
    let live = live(&s);
    let rows = match live.store.agents().await {
        Ok(rows) => rows,
        Err(e) => return db_error(e),
    };
    let list: Vec<_> = rows
        .into_iter()
        .map(|r| {
            let online = hub(live).and_then(|h| h.online(&r.name));
            json!({
                "name": r.name,
                "online": online.is_some(),
                "host": online.as_ref().map(|o| o.host.clone()).or(r.host),
                "version": online.as_ref().map(|o| o.version.clone()).or(r.version),
                "last_seen_ms": if online.is_some() { Some(now_ms()) } else { r.last_seen_ms },
                "created_ms": r.created_ms,
            })
        })
        .collect();
    Json(list).into_response()
}

pub(super) async fn create(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<NewAgent>,
) -> Response {
    let live = live(&s);
    let name = body.name.trim();
    if let Some(problem) = name_problem(name) {
        return json_error(StatusCode::BAD_REQUEST, problem);
    }
    let token = crypto::random_token();
    match live
        .store
        .create_agent(name, &crypto::token_hash(&token), now_ms())
        .await
    {
        Ok(true) => {}
        Ok(false) => return json_error(StatusCode::CONFLICT, "a host with this name exists"),
        Err(e) => return db_error(e),
    }
    let server = public_base(live, &headers).unwrap_or_else(|| "<lorvaq address>".into());
    Json(json!({ "name": name, "token": token, "command": docker_command(&server, &token) }))
        .into_response()
}

pub(super) async fn remove(State(s): State<Arc<AppState>>, Path(name): Path<String>) -> Response {
    let live = live(&s);
    match live.store.delete_agent(&name).await {
        Ok(true) => {}
        Ok(false) => return json_error(StatusCode::NOT_FOUND, "no such host"),
        Err(e) => return db_error(e),
    }
    if let Some(hub) = hub(live) {
        hub.revoke(&name);
    }
    Json(json!({ "ok": true })).into_response()
}

/// `GET /api/v1/agent/connect` with `Authorization: Bearer <join token>`.
pub(super) async fn connect(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let Some(live) = &s.live else {
        return json_error(StatusCode::NOT_FOUND, "demo mode takes no agents");
    };
    if hub(live).is_none() {
        return json_error(StatusCode::NOT_FOUND, "no scene takes agents");
    }
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|t| !t.is_empty() && t.len() <= 256);
    let Some(token) = token else {
        return json_error(StatusCode::UNAUTHORIZED, "a join token is needed");
    };
    let name = match live.store.agent_by_token(&crypto::token_hash(token)).await {
        Ok(Some(name)) => name,
        Ok(None) => return json_error(StatusCode::UNAUTHORIZED, "unknown join token"),
        Err(e) => return db_error(e),
    };
    ws.max_message_size(MAX_FRAME)
        .max_frame_size(MAX_FRAME)
        .on_upgrade(move |socket| serve(s, name, socket))
}

fn close(code: u16, reason: &'static str) -> Message {
    Message::Close(Some(CloseFrame {
        code,
        reason: reason.into(),
    }))
}

/// The next frame as a message from the agent; None when the connection should end.
async fn next(socket: &mut WebSocket, wait: Duration) -> Result<Option<FromAgent>, &'static str> {
    loop {
        let frame = match tokio::time::timeout(wait, socket.recv()).await {
            Err(_) => return Err("silent for too long"),
            Ok(None) | Ok(Some(Err(_))) | Ok(Some(Ok(Message::Close(_)))) => return Ok(None),
            Ok(Some(Ok(frame))) => frame,
        };
        match frame {
            Message::Text(text) => {
                let msg: FromAgent = serde_json::from_str(&text).map_err(|_| "unreadable frame")?;
                msg.check()?;
                return Ok(Some(msg));
            }
            Message::Ping(_) | Message::Pong(_) => continue,
            _ => return Err("text frames only"),
        }
    }
}

async fn serve(state: Arc<AppState>, name: String, mut socket: WebSocket) {
    let live = state.live.as_ref().expect("checked before the upgrade");
    let hub = hub(live).expect("checked before the upgrade").clone();
    let (host, version) = match next(&mut socket, SILENCE).await {
        Ok(Some(FromAgent::Hello { host, version })) => (host.trim().to_string(), version),
        Ok(None) => return,
        Ok(Some(_)) => {
            let _ = socket.send(close(close_code::POLICY, "hello first")).await;
            return;
        }
        Err(reason) => {
            let _ = socket.send(close(close_code::POLICY, reason)).await;
            return;
        }
    };
    let now = now_ms();
    let mut reg = hub.register(
        &name,
        Online {
            host: host.clone(),
            version: version.clone(),
            since_ms: now,
        },
    );
    tracing::info!(agent = %name, %host, %version, "agent connected");
    let mut touched = now;
    if let Err(e) = live.store.touch_agent(&name, &host, &version, now).await {
        tracing::warn!("cannot note the agent's visit: {e:#}");
    }
    let welcome = serde_json::to_string(&ToAgent::Welcome {
        agent: name.clone(),
    })
    .expect("frames serialize");
    let mut open = socket.send(Message::Text(welcome.into())).await.is_ok();
    while open {
        let msg = tokio::select! {
            _ = reg.stop.changed() => {
                let _ = socket.send(close(close_code::NORMAL, "replaced or removed")).await;
                break;
            }
            msg = next(&mut socket, SILENCE) => msg,
        };
        let input = match msg {
            Ok(Some(FromAgent::Report { processes, disk })) => CiInput::Agent(AgentReport {
                agent: name.clone(),
                host_label: host.clone(),
                processes,
                disk,
            }),
            Ok(Some(FromAgent::Exit {
                job_id,
                oom,
                exit_code,
            })) => CiInput::AgentExit(ContainerExit {
                agent: name.clone(),
                job_id,
                oom,
                exit_code,
            }),
            Ok(Some(FromAgent::Hello { .. })) => continue,
            Ok(None) => break,
            Err(reason) => {
                let _ = socket.send(close(close_code::POLICY, reason)).await;
                break;
            }
        };
        open = hub.inputs().send(input).await.is_ok();
        let now = now_ms();
        if now - touched >= TOUCH_EVERY_MS {
            touched = now;
            if let Err(e) = live.store.touch_agent(&name, &host, &version, now).await {
                tracing::warn!("cannot note the agent's visit: {e:#}");
            }
        }
    }
    hub.unregister(&name, reg.id).await;
    tracing::info!(agent = %name, "agent disconnected");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        for good in ["build-01", "a", "runner.example_2"] {
            assert!(name_problem(good).is_none(), "{good}");
        }
        for bad in ["", "-x", "Build", "a b", &"x".repeat(41), "ä"] {
            assert!(name_problem(bad).is_some(), "{bad}");
        }
    }

    #[test]
    fn the_command_carries_server_and_token() {
        let c = docker_command("https://lorvaq.example.com", "tok");
        assert!(
            c.contains("LORVAQ_SERVER=https://lorvaq.example.com"),
            "{c}"
        );
        assert!(
            c.contains("LORVAQ_JOIN=tok") && c.ends_with(" agent"),
            "{c}"
        );
        assert!(c.contains("/var/run/docker.sock:/var/run/docker.sock:ro"));
    }
}
