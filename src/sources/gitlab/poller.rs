//! Polls the connected GitLab and feeds the CI scene. A failing GitLab marks the
//! scene stale after a while; a refused token asks the admin to authorize again.

use super::client::{Gitlab, GitlabError};
use super::oauth::{bearer, Connection, CONNECTION_KEY};
use crate::crypto::SecretBox;
use crate::scene::{now_ms, LOCK};
use crate::scenes::ci::input::{CiInput, JobsUpdate};
use crate::store::Store;
use serde::Serialize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Notify};

pub const SOURCE: &str = "gitlab";

#[derive(Clone, Copy, Debug)]
pub struct PollConfig {
    pub every: Duration,
    pub runners_every: Duration,
    pub stale_after: Duration,
    pub max_backoff: Duration,
}

impl Default for PollConfig {
    fn default() -> Self {
        Self {
            every: Duration::from_secs(3),
            runners_every: Duration::from_secs(60),
            stale_after: Duration::from_secs(10),
            max_backoff: Duration::from_secs(30),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceState {
    #[default]
    Disconnected,
    Ok,
    Failing,
    Reauthorize,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct GitlabStatus {
    pub state: SourceState,
    pub last_ok_ms: Option<i64>,
    pub last_error: Option<String>,
}

/// What HTTP needs: the status to show, and a way to make the poller re-read the connection.
#[derive(Clone, Default)]
pub struct GitlabHandle {
    pub status: Arc<Mutex<GitlabStatus>>,
    pub reload: Arc<Notify>,
}

pub fn spawn(
    store: Store,
    secrets: SecretBox,
    http: reqwest::Client,
    tx: mpsc::Sender<CiInput>,
    cfg: PollConfig,
) -> GitlabHandle {
    let handle = GitlabHandle::default();
    let poller = Poller {
        store,
        secrets,
        http,
        tx,
        cfg,
        handle: handle.clone(),
        stale: false,
        failing_since: None,
        last_runners: None,
        had_connection: false,
    };
    tokio::spawn(poller.run());
    handle
}

enum PollError {
    SceneGone,
    Reauthorize(String),
    Failed(String),
}

impl From<GitlabError> for PollError {
    fn from(e: GitlabError) -> Self {
        match e {
            GitlabError::Unauthorized => PollError::Reauthorize(
                "GitLab refused lorvaq's credentials; authorize again in Settings".into(),
            ),
            other => PollError::Failed(other.to_string()),
        }
    }
}

struct Poller {
    store: Store,
    secrets: SecretBox,
    http: reqwest::Client,
    tx: mpsc::Sender<CiInput>,
    cfg: PollConfig,
    handle: GitlabHandle,
    stale: bool,
    failing_since: Option<i64>,
    last_runners: Option<Instant>,
    had_connection: bool,
}

impl Poller {
    async fn run(mut self) {
        loop {
            let conn = match self.store.setting::<Connection>(CONNECTION_KEY).await {
                Ok(Some(conn)) => conn,
                Ok(None) => {
                    if self.disconnected().await.is_err() {
                        return;
                    }
                    self.handle.reload.notified().await;
                    continue;
                }
                Err(e) => {
                    self.set(
                        SourceState::Failing,
                        Some(format!("cannot read the GitLab connection: {e:#}")),
                    );
                    tokio::select! {
                        _ = tokio::time::sleep(self.cfg.max_backoff) => {}
                        _ = self.handle.reload.notified() => {}
                    }
                    continue;
                }
            };
            self.had_connection = true;
            if !self.poll_until_reload(conn).await {
                return;
            }
        }
    }

    /// Polls one connection until it is replaced (true) or the scene is gone (false).
    async fn poll_until_reload(&mut self, mut conn: Connection) -> bool {
        self.last_runners = None;
        let mut backoff = self.cfg.every;
        loop {
            let now = now_ms();
            let delay = match self.poll_once(&mut conn, now).await {
                Ok(()) => {
                    backoff = self.cfg.every;
                    self.cfg.every
                }
                Err(PollError::SceneGone) => return false,
                Err(PollError::Reauthorize(message)) => {
                    self.set(SourceState::Reauthorize, Some(message));
                    if self.mark_failing(now).await.is_err() {
                        return false;
                    }
                    self.cfg.max_backoff
                }
                Err(PollError::Failed(message)) => {
                    self.set(SourceState::Failing, Some(message));
                    if self.mark_failing(now).await.is_err() {
                        return false;
                    }
                    backoff = (backoff * 2).min(self.cfg.max_backoff);
                    backoff
                }
            };
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = self.handle.reload.notified() => return true,
            }
        }
    }

    async fn poll_once(&mut self, conn: &mut Connection, now: i64) -> Result<(), PollError> {
        let token = bearer(conn, &self.secrets, &self.http, &self.store, now).await?;
        let gitlab = Gitlab::new(&self.http, &conn.url, &token);
        if self
            .last_runners
            .is_none_or(|at| at.elapsed() >= self.cfg.runners_every)
        {
            let runners = gitlab.runners().await?;
            self.send(CiInput::Runners(runners)).await?;
            self.last_runners = Some(Instant::now());
        }
        let jobs = gitlab.jobs().await?;
        self.send(CiInput::Jobs(jobs)).await?;
        {
            let mut st = self.handle.status.lock().expect(LOCK);
            st.state = SourceState::Ok;
            st.last_ok_ms = Some(now);
            st.last_error = None;
        }
        self.failing_since = None;
        if self.stale {
            self.stale = false;
            self.send(CiInput::SourceFresh {
                source: SOURCE.into(),
            })
            .await?;
        }
        Ok(())
    }

    /// After `stale_after` of failures in a row, the scene learns its GitLab data is old.
    async fn mark_failing(&mut self, now: i64) -> Result<(), PollError> {
        let since = *self.failing_since.get_or_insert(now);
        if !self.stale && now - since >= self.cfg.stale_after.as_millis() as i64 {
            self.stale = true;
            let since_ms = self
                .handle
                .status
                .lock()
                .expect(LOCK)
                .last_ok_ms
                .unwrap_or(since);
            self.send(CiInput::SourceStale {
                source: SOURCE.into(),
                since_ms,
            })
            .await?;
        }
        Ok(())
    }

    /// No connection: jobs leave the scene through the usual grace, staleness is lifted.
    async fn disconnected(&mut self) -> Result<(), PollError> {
        self.set(SourceState::Disconnected, None);
        self.handle.status.lock().expect(LOCK).last_ok_ms = None;
        self.failing_since = None;
        if self.had_connection {
            self.had_connection = false;
            self.send(CiInput::Jobs(JobsUpdate {
                active: vec![],
                recent_finished: vec![],
            }))
            .await?;
        }
        if self.stale {
            self.stale = false;
            self.send(CiInput::SourceFresh {
                source: SOURCE.into(),
            })
            .await?;
        }
        Ok(())
    }

    async fn send(&self, input: CiInput) -> Result<(), PollError> {
        self.tx.send(input).await.map_err(|_| PollError::SceneGone)
    }

    fn set(&self, state: SourceState, error: Option<String>) {
        let mut st = self.handle.status.lock().expect(LOCK);
        st.state = state;
        st.last_error = error;
    }
}
