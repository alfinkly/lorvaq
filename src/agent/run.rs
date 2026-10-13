//! The agent's life: connect to the server, say hello, report every 2 seconds, pass on
//! container exits at once; reconnect with a growing pause when anything breaks.

use super::docker::{Docker, DockerEvent};
use super::metrics::{self, RunnerMetrics};
use super::proto::{FromAgent, ToAgent, CONNECT_PATH, REPORT_EVERY};
use super::report;
use super::slots::Slots;
use anyhow::{bail, Context};
use futures::{SinkExt, StreamExt};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;
use tokio_tungstenite::tungstenite::{self, Message};

const METRICS_EVERY: Duration = Duration::from_secs(30);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
/// A connection that lasted this long was healthy: the next pause starts small again.
const HEALTHY: Duration = Duration::from_secs(60);

#[derive(Clone, Debug)]
pub struct AgentConfig {
    /// `https://lorvaq.example.com`
    pub server: String,
    pub join: String,
    pub host_label: String,
    pub process: String,
    pub runner_metrics: Option<String>,
    pub disk_path: PathBuf,
    pub report_every: Duration,
}

impl AgentConfig {
    pub fn new(server: &str, join: &str, host_label: &str) -> Self {
        Self {
            server: server.into(),
            join: join.into(),
            host_label: host_label.into(),
            process: "gitlab-runner".into(),
            runner_metrics: None,
            disk_path: PathBuf::from("/"),
            report_every: REPORT_EVERY,
        }
    }
}

/// `https://lorvaq.example.com/` → `wss://lorvaq.example.com/api/v1/agent/connect`.
pub fn connect_url(server: &str) -> anyhow::Result<String> {
    let mut url = reqwest::Url::parse(server.trim()).with_context(|| {
        format!("{server:?} is not an address; use one like https://lorvaq.example.com")
    })?;
    let scheme = match url.scheme() {
        "https" | "wss" => "wss",
        "http" | "ws" => "ws",
        other => bail!("use an https:// or http:// server address, not {other}://"),
    };
    url.set_scheme(scheme)
        .expect("ws and wss are valid schemes");
    let base = url.path().trim_end_matches('/').to_string();
    url.set_path(&format!("{base}{CONNECT_PATH}"));
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.into())
}

/// The host's name: from /etc/hostname when the agent runs with the host's, else "host".
pub fn default_host_label() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "host".into())
}

/// Runs until the process is stopped.
pub async fn run<D: Docker>(docker: D, cfg: AgentConfig) {
    let mut backoff = Duration::from_secs(1);
    loop {
        let began = Instant::now();
        match session(&docker, &cfg).await {
            Ok(()) => tracing::info!("the server closed the connection; reconnecting"),
            Err(e) => tracing::warn!("{e:#}; reconnecting in {}s", backoff.as_secs()),
        }
        if began.elapsed() >= HEALTHY {
            backoff = Duration::from_secs(1);
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

async fn read_metrics(http: &reqwest::Client, url: &str) -> Option<RunnerMetrics> {
    let res = http.get(url).send().await.ok()?.error_for_status().ok()?;
    Some(metrics::parse(&res.text().await.ok()?))
}

/// One connection, until it closes (Ok) or fails (Err).
pub async fn session<D: Docker>(docker: &D, cfg: &AgentConfig) -> anyhow::Result<()> {
    let mut request = connect_url(&cfg.server)?.into_client_request()?;
    request.headers_mut().insert(
        AUTHORIZATION,
        format!("Bearer {}", cfg.join.trim())
            .parse()
            .context("the join token has characters a header cannot carry")?,
    );
    let (ws, _) = match tokio_tungstenite::connect_async(request).await {
        Ok(ok) => ok,
        Err(tungstenite::Error::Http(res)) if res.status() == 401 => {
            bail!("the server refused the join token; create a new one in Settings → Hosts")
        }
        Err(e) => return Err(anyhow::Error::new(e).context("cannot connect to the server")),
    };
    let (mut tx, mut rx) = ws.split();
    let send = |msg: &FromAgent| {
        Message::Text(serde_json::to_string(msg).expect("frames serialize").into())
    };
    tx.send(send(&FromAgent::Hello {
        host: cfg.host_label.clone(),
        version: env!("CARGO_PKG_VERSION").into(),
    }))
    .await?;

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("a client with static settings builds");
    let mut runner_metrics = RunnerMetrics::default();
    let mut metrics_at: Option<Instant> = None;
    let mut slots = Slots::default();
    let mut oom = BTreeSet::new();
    let mut events = docker.events();
    let mut tick = tokio::time::interval(cfg.report_every);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = tick.tick() => {
                if let Some(url) = &cfg.runner_metrics {
                    if metrics_at.is_none_or(|at| at.elapsed() >= METRICS_EVERY) {
                        metrics_at = Some(Instant::now());
                        match read_metrics(&http, url).await {
                            Some(m) => runner_metrics = m,
                            None => tracing::warn!("cannot read the runner metrics at {url}; capacity comes from the scene settings"),
                        }
                    }
                }
                // No report while docker is unreachable: the server shows the agent as gone.
                match docker.containers().await {
                    Ok(containers) => {
                        let process = report::build(&cfg.process, &containers, &mut slots, &runner_metrics);
                        let disk = super::disk::usage(&cfg.disk_path);
                        tx.send(send(&FromAgent::Report { processes: vec![process], disk })).await?;
                    }
                    Err(e) => tracing::warn!("{e:#}"),
                }
            }
            event = events.next() => match event {
                Some(Ok(DockerEvent::Oom { container, .. })) => {
                    oom.insert(container);
                }
                Some(Ok(DockerEvent::Die { container, job_id, exit_code })) => {
                    let killed = oom.remove(&container) || docker.oom_killed(&container).await;
                    tx.send(send(&FromAgent::Exit { job_id, oom: killed, exit_code })).await?;
                }
                Some(Err(e)) => return Err(e),
                None => bail!("the docker event stream ended"),
            },
            frame = rx.next() => match frame {
                None | Some(Ok(Message::Close(_))) => return Ok(()),
                Some(Ok(Message::Text(text))) => {
                    if let Ok(ToAgent::Welcome { agent }) = serde_json::from_str(&text) {
                        tracing::info!(%agent, "connected to {}", cfg.server);
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(e)) => return Err(anyhow::Error::new(e).context("the connection broke")),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_addresses_become_websocket_urls() {
        assert_eq!(
            connect_url("https://lorvaq.example.com/").unwrap(),
            "wss://lorvaq.example.com/api/v1/agent/connect"
        );
        assert_eq!(
            connect_url("http://127.0.0.1:8080/lorvaq?x=1").unwrap(),
            "ws://127.0.0.1:8080/lorvaq/api/v1/agent/connect"
        );
        assert!(connect_url("lorvaq.example.com").is_err());
        assert!(connect_url("ftp://lorvaq.example.com").is_err());
    }
}
