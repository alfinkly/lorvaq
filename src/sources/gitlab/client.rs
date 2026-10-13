//! One GitLab: GraphQL with a bearer token, and errors sorted by what to do next.

use super::model::{self, JobsPage, MoreJobs, RunnersPage, User, Who};
use crate::scenes::ci::input::{JobInfo, JobsUpdate, RunnerInfo};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use std::fmt;
use std::time::Duration;

/// Pages of 100 read per poll at most.
const MAX_PAGES: usize = 5;

#[derive(Debug, Clone, PartialEq)]
pub enum GitlabError {
    /// The credentials were refused (401/403, or the token endpoint said no): authorize again.
    Unauthorized,
    Status(u16),
    Network(String),
    Graphql(String),
    Decode(String),
    /// Our side: stored credentials could not be decrypted or saved.
    Local(String),
}

impl fmt::Display for GitlabError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GitlabError::Unauthorized => write!(f, "GitLab refused the credentials"),
            GitlabError::Status(code) => write!(f, "GitLab answered HTTP {code}"),
            GitlabError::Network(m) => write!(f, "cannot reach GitLab: {m}"),
            GitlabError::Graphql(m) => write!(f, "GitLab GraphQL error: {m}"),
            GitlabError::Decode(m) => write!(f, "unexpected answer from GitLab: {m}"),
            GitlabError::Local(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for GitlabError {}

pub(super) fn network(e: reqwest::Error) -> GitlabError {
    GitlabError::Network(e.without_url().to_string())
}

pub(super) fn decode(e: reqwest::Error) -> GitlabError {
    GitlabError::Decode(e.without_url().to_string())
}

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(5))
        .user_agent(concat!("lorvaq/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("a client with static settings builds")
}

/// `https://gitlab.example.com/` → `https://gitlab.example.com`; paths are kept for GitLab under a prefix.
pub fn normalize_url(input: &str) -> Result<String, String> {
    let text = input.trim();
    let url = reqwest::Url::parse(text).map_err(|_| {
        format!("{text:?} is not an address; use one like https://gitlab.example.com")
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("use an http:// or https:// address".into());
    }
    if url.host_str().is_none()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("use the plain address of GitLab, like https://gitlab.example.com".into());
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

#[derive(Deserialize)]
struct GraphqlResponse<T> {
    data: Option<T>,
    errors: Option<Vec<GraphqlMessage>>,
}

#[derive(Deserialize)]
struct GraphqlMessage {
    message: String,
}

pub struct Gitlab<'a> {
    http: &'a reqwest::Client,
    base: &'a str,
    token: &'a str,
}

impl<'a> Gitlab<'a> {
    pub fn new(http: &'a reqwest::Client, base: &'a str, token: &'a str) -> Self {
        Self { http, base, token }
    }

    pub async fn query<T: DeserializeOwned>(
        &self,
        query: &str,
        variables: Value,
    ) -> Result<T, GitlabError> {
        let res = self
            .http
            .post(format!("{}/api/graphql", self.base))
            .bearer_auth(self.token)
            .json(&json!({ "query": query, "variables": variables }))
            .send()
            .await
            .map_err(network)?;
        match res.status().as_u16() {
            401 | 403 => return Err(GitlabError::Unauthorized),
            code if !(200..300).contains(&code) => return Err(GitlabError::Status(code)),
            _ => {}
        }
        let body: GraphqlResponse<T> = res.json().await.map_err(decode)?;
        if let Some(errors) = body.errors.filter(|e| !e.is_empty()) {
            return Err(GitlabError::Graphql(
                errors
                    .into_iter()
                    .map(|e| e.message)
                    .collect::<Vec<_>>()
                    .join("; "),
            ));
        }
        body.data
            .ok_or_else(|| GitlabError::Decode("the answer has no data".into()))
    }

    pub async fn whoami(&self) -> Result<User, GitlabError> {
        let who: Who = self.query(model::WHO_QUERY, json!({})).await?;
        who.current_user.ok_or(GitlabError::Unauthorized)
    }

    pub async fn jobs(&self) -> Result<JobsUpdate, GitlabError> {
        let first: JobsPage = self.query(model::JOBS_QUERY, json!({})).await?;
        let mut active: Vec<JobInfo> = first
            .active
            .nodes
            .iter()
            .filter_map(model::to_job)
            .collect();
        let recent_finished = first
            .finished
            .map(|f| f.nodes.iter().filter_map(model::to_job).collect())
            .unwrap_or_default();
        let mut page = first.active.page_info;
        for _ in 1..MAX_PAGES {
            let Some(cursor) = page.filter(|p| p.has_next_page).and_then(|p| p.end_cursor) else {
                break;
            };
            let more: MoreJobs = self
                .query(model::MORE_JOBS_QUERY, json!({ "after": cursor }))
                .await?;
            active.extend(more.active.nodes.iter().filter_map(model::to_job));
            page = more.active.page_info;
        }
        Ok(JobsUpdate {
            active,
            recent_finished,
        })
    }

    pub async fn runners(&self) -> Result<Vec<RunnerInfo>, GitlabError> {
        let mut out = Vec::new();
        let mut after: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let page: RunnersPage = self
                .query(model::RUNNERS_QUERY, json!({ "after": after }))
                .await?;
            out.extend(page.runners.nodes.iter().filter_map(model::to_runner));
            match page.runners.page_info {
                Some(p) if p.has_next_page && p.end_cursor.is_some() => after = p.end_cursor,
                _ => break,
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenes::ci::input::JobStatus;
    use crate::sources::gitlab::fake::FakeGitlab;

    #[test]
    fn urls_are_normalized_and_checked() {
        assert_eq!(
            normalize_url(" https://gitlab.example.com/ ").unwrap(),
            "https://gitlab.example.com"
        );
        assert_eq!(
            normalize_url("https://example.com/gitlab/").unwrap(),
            "https://example.com/gitlab"
        );
        assert_eq!(
            normalize_url("http://127.0.0.1:8929").unwrap(),
            "http://127.0.0.1:8929"
        );
        for bad in [
            "gitlab.example.com",
            "ftp://gitlab.example.com",
            "https://gitlab.example.com/?x=1",
            "https://user:pw@gitlab.example.com",
            "https://gitlab.example.com/#a",
        ] {
            assert!(normalize_url(bad).is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn who_am_i_and_bad_tokens() {
        let fake = FakeGitlab::start().await;
        let http = http_client();
        let me = Gitlab::new(&http, &fake.url, "good-token")
            .whoami()
            .await
            .unwrap();
        assert_eq!((me.username.as_str(), me.admin), ("dana", Some(true)));
        assert_eq!(
            Gitlab::new(&http, &fake.url, "nope")
                .whoami()
                .await
                .unwrap_err(),
            GitlabError::Unauthorized
        );
    }

    #[tokio::test]
    async fn jobs_and_runners_map_through() {
        let fake = FakeGitlab::start().await;
        {
            let mut s = fake.state.lock().unwrap();
            s.active = vec![
                FakeGitlab::job(1, "demo/backend", "RUNNING", Some(7)),
                FakeGitlab::job(2, "demo/docs", "MANUAL", None),
            ];
            s.finished = vec![FakeGitlab::job(3, "demo/backend", "SUCCESS", Some(7))];
            s.runners = vec![FakeGitlab::runner(7, &["ops"], true)];
        }
        let http = http_client();
        let gl = Gitlab::new(&http, &fake.url, "good-token");
        let jobs = gl.jobs().await.unwrap();
        assert_eq!(jobs.active.len(), 1, "MANUAL is not shown");
        assert_eq!(
            (
                jobs.active[0].id,
                jobs.active[0].status,
                jobs.active[0].runner_id
            ),
            (1, JobStatus::Running, Some(7))
        );
        assert_eq!(jobs.recent_finished[0].id, 3);
        let runners = gl.runners().await.unwrap();
        assert_eq!(
            (runners[0].id, runners[0].tags.clone()),
            (7, vec!["ops".to_string()])
        );
    }

    #[tokio::test]
    async fn failures_are_sorted() {
        let fake = FakeGitlab::start().await;
        let http = http_client();
        fake.state.lock().unwrap().fail_with = Some(502);
        assert_eq!(
            Gitlab::new(&http, &fake.url, "good-token")
                .jobs()
                .await
                .unwrap_err(),
            GitlabError::Status(502)
        );
        fake.state.lock().unwrap().fail_with = Some(403);
        assert_eq!(
            Gitlab::new(&http, &fake.url, "good-token")
                .jobs()
                .await
                .unwrap_err(),
            GitlabError::Unauthorized
        );
        fake.state.lock().unwrap().fail_with = None;
        fake.state.lock().unwrap().graphql_error = Some("Field 'x' doesn't exist".into());
        let err = Gitlab::new(&http, &fake.url, "good-token")
            .jobs()
            .await
            .unwrap_err();
        assert!(
            matches!(err, GitlabError::Graphql(ref m) if m.contains("doesn't exist")),
            "{err}"
        );
        let closed = Gitlab::new(&http, "http://127.0.0.1:1", "t")
            .whoami()
            .await
            .unwrap_err();
        assert!(matches!(closed, GitlabError::Network(_)), "{closed}");
    }
}
