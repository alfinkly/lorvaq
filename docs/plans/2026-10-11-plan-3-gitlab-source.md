# lorvaq — план 3: источник GitLab и закрытие хвостов

> **Для агентов-исполнителей:** ОБЯЗАТЕЛЬНЫЙ ПОД-СКИЛЛ: используйте superpowers:subagent-driven-development (рекомендуется) или superpowers:executing-plans, чтобы выполнять план задача за задачей. Шаги размечены чекбоксами (`- [ ]`).

**Цель:** администратор подключает свой GitLab из интерфейса — кнопкой Authorize (OAuth + PKCE) или токеном по готовой ссылке, — и на сцене CI едут настоящие джобы его раннеров; при проблемах с GitLab виден баннер «данные устарели», при отзыве токена — «нужна повторная авторизация». Заодно закрыты хвосты из ревью плана 2.

**Архитектура:** новый модуль `sources::gitlab`: `model` (запросы GraphQL и превращение ответов во входы сцены CI), `client` (HTTP к одному GitLab, ошибки по смыслу), `oauth` (PKCE, обмен кода, обновление токена, хранение подключения с зашифрованными секретами), `poller` (опрос раз в 3 с, раннеры раз в минуту, «устарело» через 10 с сбоев, отступ до 30 с, перечитывание подключения по сигналу), `fake` (поддельный GitLab для тестов). HTTP получает маршруты администратора для подключения и публичный маршрут возврата из OAuth. Сцена убирает пустые строки раннеров без агента. Плюс отдельные задачи по безопасности входа и истории.

**Стек:** добавляется reqwest 0.12 (без настроек по умолчанию; `json`, `rustls-tls` — корневые сертификаты вшиты, работает в образе `scratch`). Проверено: reqwest 0.12.28 собирается на rustc 1.91, ходит на поддельный сервер axum и по TLS на настоящий gitlab.com. Запросы GraphQL этого плана выполнены на настоящем GitLab 19: проходят по сложности, форма ответа совпадает с моделью.

**Spec:** `docs/design/2026-10-10-core-and-ci-scene.md` — §4.1 (источник GitLab), §6.2 (мастер подключения), §6.3 (секреты), §9 (GitLab недоступен, токен отозван). Хвосты — `docs/reviews/2026-10-10-plan-2-final-review.md`, раздел «Статус».

**Отклонение от spec, принятое здесь:** spec допускает подключение без прав администратора GitLab с предупреждением «видно не всё». `Query.jobs` для не-администратора возвращает пустой список, а запрос по проектам упирается в лимит сложности GraphQL — честно показать «часть» в этом плане нельзя. Поэтому мастер **требует** администратора (лучше служебного) и объясняет почему. Подключение без администратора — отдельной задачей позже.

## Глобальные ограничения

- Всё из глобальных ограничений планов 1 и 2 в силе (Rust 1.91, clippy `-D warnings`, fmt, ни следа конкретной студии, UI на английском со строками в `web/src/i18n.ts`, время в мс, коммиты на английском с `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`).
- Все секреты GitLab (токен, access/refresh, client secret, PKCE verifier) хранятся только запечатанными `SecretBox`; ни API, ни логи их не возвращают. В тестах и примерах — только `127.0.0.1`, `gitlab.example.com` и документационные адреса.
- Опрос: джобы раз в **3 с** одним запросом (`active` + `finished`), раннеры раз в **60 с**, до **5** страниц по 100; таймаут HTTP **15 с** (соединение 5 с); «устарело» после **10 с** непрерывных сбоев; отступ при сбоях удваивается до **30 с**; access token OAuth обновляется, когда до истечения меньше **5 мин**; незавершённый OAuth живёт **10 мин**.
- Статусы GitLab: `PENDING` → pending; `PREPARING`, `RUNNING`, `CANCELING` → running; `SUCCESS`, `FAILED`, `CANCELED` → завершён; остальные (`CREATED`, `MANUAL`, `SCHEDULED`, `WAITING_FOR_RESOURCE`, `SKIPPED`…) — не показываются.
- Сеть только к адресу GitLab, который ввёл администратор. Тесты не ходят в интернет — только к поддельному GitLab на `127.0.0.1`.

## Фокус ревью

1. **Незнакомые или «промежуточные» статусы джобов** и пустые поля (`project: null`, `runner: null`, групповой раннер без групп) — пропускаются, а не роняют опрос (задача 1).
2. **Ротация refresh-токена**: GitLab выдал новую пару, а запись в базу не удалась, — следующий опрос получает отказ и переходит в «нужна повторная авторизация», без бесконечного цикла ошибок (задачи 3 и 4).
3. **Возврат из OAuth**: чужой или повторный `state`, нет сессии администратора, GitLab вернул `error` — подключение не создаётся, пользователь возвращается в настройки с причиной (задача 5).
4. **GitLab висит или отвечает 5xx** — таймаут 15 с, баннер «устарело» через 10 с сбоев, отступ не больше 30 с, после восстановления баннер уходит (задача 4).
5. **Отключение GitLab, пока на сцене есть джобы** — джобы уходят через срок пропажи, баннер снимается, строки раннеров без агента пропадают (задачи 4 и 6).

## Структура файлов

```text
Cargo.toml                         + reqwest
migrations/0002_event_runs.sql     run у событий, уникальность, индекс по времени
src/sources/mod.rs
src/sources/gitlab/mod.rs          реэкспорт, handle, spawn
src/sources/gitlab/model.rs        запросы GraphQL, ответы → JobInfo / RunnerInfo
src/sources/gitlab/client.rs       Gitlab, GitlabError, http_client, normalize_url
src/sources/gitlab/oauth.rs        PKCE, OauthClient, Connection, StoredAuth, Pending, bearer()
src/sources/gitlab/poller.rs       опрос, статус, «устарело»
src/sources/gitlab/fake.rs         поддельный GitLab для тестов
src/http/sources.rs                маршруты подключения GitLab
src/http/mod.rs, account.rs        Live: gitlab, http, public_url, auth_epoch; SSE отзыв; argon очередь
src/auth.rs, src/store/mod.rs, src/history.rs, src/world/mod.rs, src/scene.rs, src/scenes/ci/reduce.rs, src/app.rs
tests/gitlab_source.rs  tests/http_gitlab.rs  (+ правки tests/store.rs, tests/http_live.rs, tests/live_smoke.rs)
web/src/api.ts  web/src/settings/forms.ts(+test)  web/src/pages/Settings.tsx  web/src/App.tsx  web/src/i18n.ts
compose.yml  README.md
```

---

### Задача 1: Модель GraphQL GitLab

**Files:**
- Modify: `Cargo.toml` — добавить reqwest (используется с задачи 2)
- Create: `src/sources/mod.rs`, `src/sources/gitlab/mod.rs`, `src/sources/gitlab/model.rs`
- Modify: `src/lib.rs` — `pub mod sources;`

**Interfaces:**
- Produces (`sources::gitlab::model`): константы `WHO_QUERY`, `JOBS_QUERY`, `MORE_JOBS_QUERY`, `RUNNERS_QUERY`; типы ответов `Who { current_user: Option<User> }`, `User { username: String, admin: Option<bool> }`, `PageInfo`, `Nodes<T> { nodes, page_info: Option<PageInfo> }`, `JobsPage { active, finished }`, `MoreJobs { active }`, `RunnersPage { runners }`, `GqlJob`, `GqlRunner`; функции `gid_number(&str) -> Option<u64>`, `job_status(&str) -> Option<JobStatus>`, `to_job(&GqlJob) -> Option<JobInfo>`, `to_runner(&GqlRunner) -> Option<RunnerInfo>`.

- [x] **Step 1: Зависимость и модули**

В `Cargo.toml`, `[dependencies]`:

```toml
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
```

`src/sources/mod.rs`:

```rust
//! Where scenes get their data from.

pub mod gitlab;
```

`src/sources/gitlab/mod.rs` (задачи 2–4 дополнят его):

```rust
//! GitLab as a source for the CI scene: queries, client, credentials and the poller.

pub mod model;
```

Добавить в `src/lib.rs` строку `pub mod sources;`.

- [x] **Step 2: Написать падающие тесты**

`src/sources/gitlab/model.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenes::ci::input::{JobStatus, RunnerScope};
    use serde_json::json;

    fn job(value: serde_json::Value) -> Option<JobInfo> {
        to_job(&serde_json::from_value::<GqlJob>(value).unwrap())
    }

    fn runner(value: serde_json::Value) -> Option<RunnerInfo> {
        to_runner(&serde_json::from_value::<GqlRunner>(value).unwrap())
    }

    #[test]
    fn global_ids_end_in_numbers() {
        assert_eq!(gid_number("gid://gitlab/Ci::Build/31378"), Some(31378));
        assert_eq!(gid_number("gid://gitlab/Ci::Runner/7"), Some(7));
        assert_eq!(gid_number("not-a-gid"), None);
    }

    #[test]
    fn statuses_that_matter_and_the_rest() {
        assert_eq!(job_status("PENDING"), Some(JobStatus::Pending));
        for s in ["PREPARING", "RUNNING", "CANCELING"] {
            assert_eq!(job_status(s), Some(JobStatus::Running), "{s}");
        }
        assert_eq!(job_status("SUCCESS"), Some(JobStatus::Success));
        assert_eq!(job_status("FAILED"), Some(JobStatus::Failed));
        assert_eq!(job_status("CANCELED"), Some(JobStatus::Canceled));
        for s in ["CREATED", "MANUAL", "SCHEDULED", "WAITING_FOR_RESOURCE", "SKIPPED", "NEW_THING"] {
            assert_eq!(job_status(s), None, "{s}");
        }
    }

    #[test]
    fn a_full_job_maps_to_job_info() {
        let j = job(json!({
            "id": "gid://gitlab/Ci::Build/42", "name": "build", "status": "RUNNING",
            "stage": { "name": "build" }, "tags": ["ops"], "runner": { "id": "gid://gitlab/Ci::Runner/7" },
            "project": { "fullPath": "demo/backend" }, "pipeline": { "id": "gid://gitlab/Ci::Pipeline/9" }
        }))
        .unwrap();
        assert_eq!((j.id, j.name.as_str(), j.stage.as_str(), j.status), (42, "build", "build", JobStatus::Running));
        assert_eq!((j.tags, j.project.as_str(), j.pipeline, j.runner_id), (vec!["ops".to_string()], "demo/backend", 9, Some(7)));
    }

    #[test]
    fn sparse_jobs_are_kept_or_skipped_sensibly() {
        let pending = job(json!({ "id": "gid://gitlab/Ci::Build/1", "name": null, "status": "PENDING", "stage": null, "tags": null, "runner": null, "project": { "fullPath": "demo/docs" }, "pipeline": null })).unwrap();
        assert_eq!((pending.name.as_str(), pending.stage.as_str(), pending.pipeline, pending.runner_id), ("", "", 0, None));
        assert!(job(json!({ "id": "gid://gitlab/Ci::Build/2", "status": "PENDING", "project": null })).is_none(), "no project");
        assert!(job(json!({ "id": "gid://gitlab/Ci::Build/3", "status": "MANUAL", "project": { "fullPath": "demo/docs" } })).is_none(), "not shown");
    }

    #[test]
    fn runners_and_their_scopes() {
        let instance = runner(json!({ "id": "gid://gitlab/Ci::Runner/1", "description": "shared", "status": "ONLINE", "paused": false, "runnerType": "INSTANCE_TYPE", "tagList": ["ops"], "runUntagged": true, "groups": null, "projects": null })).unwrap();
        assert_eq!((instance.id, instance.online, instance.run_untagged, instance.scope), (1, true, true, RunnerScope::Instance));
        let group = runner(json!({ "id": "gid://gitlab/Ci::Runner/2", "description": "", "status": "ONLINE", "paused": true, "runnerType": "GROUP_TYPE", "tagList": [], "runUntagged": false, "groups": { "nodes": [{ "fullPath": "demo" }] }, "projects": null })).unwrap();
        assert_eq!(group.scope, RunnerScope::Group { full_path: "demo".into() });
        assert!(!group.online, "paused runners take no jobs");
        assert_eq!(group.description, "runner #2", "empty descriptions get a name");
        let project = runner(json!({ "id": "gid://gitlab/Ci::Runner/3", "status": "OFFLINE", "runnerType": "PROJECT_TYPE", "projects": { "nodes": [{ "fullPath": "demo/a" }, { "fullPath": "demo/b" }] } })).unwrap();
        assert_eq!(project.scope, RunnerScope::Project { full_paths: vec!["demo/a".into(), "demo/b".into()] });
        assert!(!project.online);
        assert!(runner(json!({ "id": "gid://gitlab/Ci::Runner/4", "status": "ONLINE", "runnerType": "GROUP_TYPE", "groups": { "nodes": [] } })).is_none(), "group runner without a group");
        assert!(runner(json!({ "id": "gid://gitlab/Ci::Runner/5", "status": "ONLINE", "runnerType": "MYSTERY" })).is_none());
    }

    #[test]
    fn pages_parse() {
        let page: JobsPage = serde_json::from_value(json!({
            "active": { "nodes": [], "pageInfo": { "hasNextPage": true, "endCursor": "abc" } },
            "finished": { "nodes": [] }
        }))
        .unwrap();
        let info = page.active.page_info.unwrap();
        assert!(info.has_next_page);
        assert_eq!(info.end_cursor.as_deref(), Some("abc"));
        let who: Who = serde_json::from_value(json!({ "currentUser": { "username": "dana", "admin": true } })).unwrap();
        assert_eq!(who.current_user.unwrap().admin, Some(true));
    }

    #[test]
    fn queries_ask_for_what_the_model_reads() {
        for field in ["active:", "finished:", "PREPARING", "CANCELING", "fullPath", "pageInfo", "fragment job on CiJobInterface"] {
            assert!(JOBS_QUERY.contains(field), "{field}");
        }
        assert!(MORE_JOBS_QUERY.contains("$after") && !MORE_JOBS_QUERY.contains("finished:"));
        for field in ["runners(first: 100, after: $after)", "runUntagged", "tagList", "paused", "groups(first: 1)", "projects(first: 20)"] {
            assert!(RUNNERS_QUERY.contains(field), "{field}");
        }
    }
}
```

Run: `cargo test sources::gitlab::model`
Expected: ошибки компиляции `cannot find function 'to_job'`.

- [x] **Step 3: Реализовать**

Добавить в начало `src/sources/gitlab/model.rs`:

```rust
//! The GraphQL lorvaq sends to GitLab and how the answers become CI scene inputs.
//! Checked against GitLab 19: both queries pass its complexity limit.

use crate::scenes::ci::input::{JobInfo, JobStatus, RunnerInfo, RunnerScope};
use serde::Deserialize;

macro_rules! job_fragment {
    () => {
        " fragment job on CiJobInterface { id name status stage { name } tags runner { id } project { fullPath } pipeline { id } }"
    };
}

pub const WHO_QUERY: &str = "query lorvaqWho { currentUser { username admin } }";

pub const JOBS_QUERY: &str = concat!(
    "query lorvaqJobs { ",
    "active: jobs(statuses: [PENDING, PREPARING, RUNNING, CANCELING], first: 100) { nodes { ...job } pageInfo { hasNextPage endCursor } } ",
    "finished: jobs(statuses: [SUCCESS, FAILED, CANCELED], first: 50) { nodes { ...job } } }",
    job_fragment!()
);

pub const MORE_JOBS_QUERY: &str = concat!(
    "query lorvaqMoreJobs($after: String) { ",
    "active: jobs(statuses: [PENDING, PREPARING, RUNNING, CANCELING], first: 100, after: $after) { nodes { ...job } pageInfo { hasNextPage endCursor } } }",
    job_fragment!()
);

pub const RUNNERS_QUERY: &str = "query lorvaqRunners($after: String) { runners(first: 100, after: $after) { \
     nodes { id description status paused runnerType tagList runUntagged groups(first: 1) { nodes { fullPath } } projects(first: 20) { nodes { fullPath } } } \
     pageInfo { hasNextPage endCursor } } }";

#[derive(Debug, Deserialize)]
pub struct Who {
    #[serde(rename = "currentUser")]
    pub current_user: Option<User>,
}

#[derive(Debug, Deserialize)]
pub struct User {
    pub username: String,
    pub admin: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct PageInfo {
    #[serde(rename = "hasNextPage")]
    pub has_next_page: bool,
    #[serde(rename = "endCursor")]
    pub end_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Nodes<T> {
    pub nodes: Vec<T>,
    #[serde(rename = "pageInfo")]
    pub page_info: Option<PageInfo>,
}

#[derive(Debug, Deserialize)]
pub struct JobsPage {
    pub active: Nodes<GqlJob>,
    pub finished: Option<Nodes<GqlJob>>,
}

#[derive(Debug, Deserialize)]
pub struct MoreJobs {
    pub active: Nodes<GqlJob>,
}

#[derive(Debug, Deserialize)]
pub struct RunnersPage {
    pub runners: Nodes<GqlRunner>,
}

#[derive(Debug, Deserialize)]
pub struct Named {
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Gid {
    pub id: String,
}

#[derive(Debug, Deserialize)]
pub struct FullPath {
    #[serde(rename = "fullPath")]
    pub full_path: String,
}

#[derive(Debug, Deserialize)]
pub struct GqlJob {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub status: String,
    #[serde(default)]
    pub stage: Option<Named>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub runner: Option<Gid>,
    #[serde(default)]
    pub project: Option<FullPath>,
    #[serde(default)]
    pub pipeline: Option<Gid>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GqlRunner {
    pub id: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub paused: Option<bool>,
    pub runner_type: String,
    #[serde(default)]
    pub tag_list: Option<Vec<String>>,
    #[serde(default)]
    pub run_untagged: Option<bool>,
    #[serde(default)]
    pub groups: Option<Nodes<FullPath>>,
    #[serde(default)]
    pub projects: Option<Nodes<FullPath>>,
}

/// The number at the end of a global id: `gid://gitlab/Ci::Build/42` → 42.
pub fn gid_number(gid: &str) -> Option<u64> {
    gid.rsplit('/').next()?.parse().ok()
}

pub fn job_status(status: &str) -> Option<JobStatus> {
    match status {
        "PENDING" => Some(JobStatus::Pending),
        "PREPARING" | "RUNNING" | "CANCELING" => Some(JobStatus::Running),
        "SUCCESS" => Some(JobStatus::Success),
        "FAILED" => Some(JobStatus::Failed),
        "CANCELED" => Some(JobStatus::Canceled),
        _ => None,
    }
}

pub fn to_job(job: &GqlJob) -> Option<JobInfo> {
    Some(JobInfo {
        id: gid_number(&job.id)?,
        name: job.name.clone().unwrap_or_default(),
        stage: job.stage.as_ref().and_then(|s| s.name.clone()).unwrap_or_default(),
        status: job_status(&job.status)?,
        tags: job.tags.clone().unwrap_or_default(),
        project: job.project.as_ref()?.full_path.clone(),
        pipeline: job.pipeline.as_ref().and_then(|p| gid_number(&p.id)).unwrap_or(0),
        runner_id: job.runner.as_ref().and_then(|r| gid_number(&r.id)),
    })
}

pub fn to_runner(runner: &GqlRunner) -> Option<RunnerInfo> {
    let id = gid_number(&runner.id)?;
    let scope = match runner.runner_type.as_str() {
        "INSTANCE_TYPE" => RunnerScope::Instance,
        "GROUP_TYPE" => RunnerScope::Group { full_path: runner.groups.as_ref()?.nodes.first()?.full_path.clone() },
        "PROJECT_TYPE" => RunnerScope::Project {
            full_paths: runner.projects.as_ref().map(|p| p.nodes.iter().map(|n| n.full_path.clone()).collect()).unwrap_or_default(),
        },
        _ => return None,
    };
    Some(RunnerInfo {
        id,
        description: runner.description.clone().filter(|d| !d.trim().is_empty()).unwrap_or_else(|| format!("runner #{id}")),
        online: runner.status.as_deref() == Some("ONLINE") && !runner.paused.unwrap_or(false),
        tags: runner.tag_list.clone().unwrap_or_default(),
        run_untagged: runner.run_untagged.unwrap_or(false),
        scope,
    })
}
```

- [x] **Step 4: Запустить — проходит**

Run: `cargo test sources::gitlab::model && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: `7 passed`.

- [x] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/
git commit -m "feat(gitlab): GraphQL queries and mapping to CI scene inputs

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Задача 2: Клиент GitLab и поддельный GitLab

**Files:**
- Create: `src/sources/gitlab/client.rs`, `src/sources/gitlab/fake.rs`
- Modify: `src/sources/gitlab/mod.rs`

**Interfaces:**
- Consumes: `model::*`.
- Produces:
  - `client::{GitlabError::{Unauthorized, Status(u16), Network(String), Graphql(String), Decode(String), Local(String)}, http_client() -> reqwest::Client, normalize_url(&str) -> Result<String, String>, Gitlab::new(&Client, base, token)}` с методами `query::<T>(query, variables)`, `whoami() -> Result<User, _>`, `jobs() -> Result<JobsUpdate, _>`, `runners() -> Result<Vec<RunnerInfo>, _>`. 401 и 403 → `Unauthorized`.
  - `fake::{FakeGitlab { url, state }, FakeState, FakeGitlab::{start(), job(id, project, status, runner), runner(id, tags, untagged)}}`; `FakeState` поля: `token, username, admin, fail_with: Option<u16>, graphql_error: Option<String>, active, finished, runners, graphql_calls, client_id, code, challenge: Option<String>, refresh, issued, expires_in, token_requests`.

- [x] **Step 1: Поддельный GitLab**

`src/sources/gitlab/fake.rs`:

```rust
//! A tiny stand-in for GitLab, for tests: the GraphQL endpoint and the OAuth token endpoint.
#![doc(hidden)]

use super::oauth::pkce_challenge;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Form, Json, Router};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub struct FakeState {
    /// The bearer token GraphQL accepts.
    pub token: String,
    pub username: String,
    pub admin: bool,
    pub fail_with: Option<u16>,
    pub graphql_error: Option<String>,
    pub active: Vec<Value>,
    pub finished: Vec<Value>,
    pub runners: Vec<Value>,
    pub graphql_calls: usize,
    // OAuth
    pub client_id: String,
    pub code: String,
    /// The PKCE challenge the "browser" brought to /oauth/authorize.
    pub challenge: Option<String>,
    pub refresh: String,
    pub issued: usize,
    pub expires_in: i64,
    pub token_requests: usize,
}

impl Default for FakeState {
    fn default() -> Self {
        Self {
            token: "good-token".into(),
            username: "dana".into(),
            admin: true,
            fail_with: None,
            graphql_error: None,
            active: vec![],
            finished: vec![],
            runners: vec![],
            graphql_calls: 0,
            client_id: "app-id".into(),
            code: "the-code".into(),
            challenge: None,
            refresh: "refresh-0".into(),
            issued: 0,
            expires_in: 7200,
            token_requests: 0,
        }
    }
}

pub struct FakeGitlab {
    pub url: String,
    pub state: Arc<Mutex<FakeState>>,
}

type Shared = Arc<Mutex<FakeState>>;

impl FakeGitlab {
    pub async fn start() -> Self {
        let state: Shared = Arc::new(Mutex::new(FakeState::default()));
        let app = Router::new().route("/api/graphql", post(graphql)).route("/oauth/token", post(token)).with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("a free port");
        let url = format!("http://{}", listener.local_addr().expect("bound"));
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self { url, state }
    }

    pub fn job(id: u64, project: &str, status: &str, runner: Option<u64>) -> Value {
        json!({
            "id": format!("gid://gitlab/Ci::Build/{id}"), "name": format!("job{id}"), "status": status,
            "stage": { "name": "test" }, "tags": [],
            "runner": runner.map(|r| json!({ "id": format!("gid://gitlab/Ci::Runner/{r}") })),
            "project": { "fullPath": project }, "pipeline": { "id": "gid://gitlab/Ci::Pipeline/1" }
        })
    }

    pub fn runner(id: u64, tags: &[&str], untagged: bool) -> Value {
        json!({
            "id": format!("gid://gitlab/Ci::Runner/{id}"), "description": format!("runner{id}"), "status": "ONLINE",
            "paused": false, "runnerType": "INSTANCE_TYPE", "tagList": tags, "runUntagged": untagged,
            "groups": null, "projects": null
        })
    }
}

async fn graphql(State(st): State<Shared>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    let mut s = st.lock().expect("fake lock");
    s.graphql_calls += 1;
    if let Some(code) = s.fail_with {
        return StatusCode::from_u16(code).expect("a valid status").into_response();
    }
    let bearer = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer "));
    if bearer != Some(s.token.as_str()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if let Some(message) = &s.graphql_error {
        return Json(json!({ "errors": [{ "message": message }] })).into_response();
    }
    let query = body["query"].as_str().unwrap_or_default();
    let last_page = json!({ "hasNextPage": false, "endCursor": null });
    let data = if query.contains("currentUser") {
        json!({ "currentUser": { "username": s.username, "admin": s.admin } })
    } else if query.contains("runners(") {
        json!({ "runners": { "nodes": s.runners, "pageInfo": last_page } })
    } else {
        json!({ "active": { "nodes": s.active, "pageInfo": last_page }, "finished": { "nodes": s.finished } })
    };
    Json(json!({ "data": data })).into_response()
}

async fn token(State(st): State<Shared>, Form(form): Form<HashMap<String, String>>) -> Response {
    let mut s = st.lock().expect("fake lock");
    s.token_requests += 1;
    if form.get("client_id") != Some(&s.client_id) {
        return (StatusCode::UNAUTHORIZED, Json(json!({ "error": "invalid_client" }))).into_response();
    }
    let ok = match form.get("grant_type").map(String::as_str) {
        Some("authorization_code") => {
            form.get("code") == Some(&s.code)
                && matches!((s.challenge.as_deref(), form.get("code_verifier")), (Some(c), Some(v)) if pkce_challenge(v) == c)
        }
        Some("refresh_token") => form.get("refresh_token") == Some(&s.refresh),
        _ => false,
    };
    if !ok {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "invalid_grant" }))).into_response();
    }
    s.issued += 1;
    s.token = format!("access-{}", s.issued);
    s.refresh = format!("refresh-{}", s.issued);
    Json(json!({ "access_token": s.token, "token_type": "Bearer", "expires_in": s.expires_in, "refresh_token": s.refresh, "scope": "read_api" }))
        .into_response()
}
```

`fake.rs` ссылается на `oauth::pkce_challenge` из задачи 3. Чтобы задача 2 собиралась сама, в этой задаче добавить в `src/sources/gitlab/mod.rs` только `model`, `client` и `fake`, а функцию `pkce_challenge` создать сразу в `src/sources/gitlab/oauth.rs` с минимумом:

```rust
//! OAuth (authorization code + PKCE) and stored GitLab credentials. Grows in the next task.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};

/// RFC 7636 S256: base64url(sha256(verifier)) without padding.
pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}
```

Заменить `src/sources/gitlab/mod.rs`:

```rust
//! GitLab as a source for the CI scene: queries, client, credentials and the poller.

pub mod client;
pub mod fake;
pub mod model;
pub mod oauth;
```

- [x] **Step 2: Написать падающие тесты клиента**

`src/sources/gitlab/client.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenes::ci::input::JobStatus;
    use crate::sources::gitlab::fake::FakeGitlab;

    #[test]
    fn urls_are_normalized_and_checked() {
        assert_eq!(normalize_url(" https://gitlab.example.com/ ").unwrap(), "https://gitlab.example.com");
        assert_eq!(normalize_url("https://example.com/gitlab/").unwrap(), "https://example.com/gitlab");
        assert_eq!(normalize_url("http://127.0.0.1:8929").unwrap(), "http://127.0.0.1:8929");
        for bad in ["gitlab.example.com", "ftp://gitlab.example.com", "https://gitlab.example.com/?x=1", "https://user:pw@gitlab.example.com", "https://gitlab.example.com/#a"] {
            assert!(normalize_url(bad).is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn who_am_i_and_bad_tokens() {
        let fake = FakeGitlab::start().await;
        let http = http_client();
        let me = Gitlab::new(&http, &fake.url, "good-token").whoami().await.unwrap();
        assert_eq!((me.username.as_str(), me.admin), ("dana", Some(true)));
        assert_eq!(Gitlab::new(&http, &fake.url, "nope").whoami().await.unwrap_err(), GitlabError::Unauthorized);
    }

    #[tokio::test]
    async fn jobs_and_runners_map_through() {
        let fake = FakeGitlab::start().await;
        {
            let mut s = fake.state.lock().unwrap();
            s.active = vec![FakeGitlab::job(1, "demo/backend", "RUNNING", Some(7)), FakeGitlab::job(2, "demo/docs", "MANUAL", None)];
            s.finished = vec![FakeGitlab::job(3, "demo/backend", "SUCCESS", Some(7))];
            s.runners = vec![FakeGitlab::runner(7, &["ops"], true)];
        }
        let http = http_client();
        let gl = Gitlab::new(&http, &fake.url, "good-token");
        let jobs = gl.jobs().await.unwrap();
        assert_eq!(jobs.active.len(), 1, "MANUAL is not shown");
        assert_eq!((jobs.active[0].id, jobs.active[0].status, jobs.active[0].runner_id), (1, JobStatus::Running, Some(7)));
        assert_eq!(jobs.recent_finished[0].id, 3);
        let runners = gl.runners().await.unwrap();
        assert_eq!((runners[0].id, runners[0].tags.clone()), (7, vec!["ops".to_string()]));
    }

    #[tokio::test]
    async fn failures_are_sorted() {
        let fake = FakeGitlab::start().await;
        let http = http_client();
        fake.state.lock().unwrap().fail_with = Some(502);
        assert_eq!(Gitlab::new(&http, &fake.url, "good-token").jobs().await.unwrap_err(), GitlabError::Status(502));
        fake.state.lock().unwrap().fail_with = Some(403);
        assert_eq!(Gitlab::new(&http, &fake.url, "good-token").jobs().await.unwrap_err(), GitlabError::Unauthorized);
        fake.state.lock().unwrap().fail_with = None;
        fake.state.lock().unwrap().graphql_error = Some("Field 'x' doesn't exist".into());
        let err = Gitlab::new(&http, &fake.url, "good-token").jobs().await.unwrap_err();
        assert!(matches!(err, GitlabError::Graphql(ref m) if m.contains("doesn't exist")), "{err}");
        let closed = Gitlab::new(&http, "http://127.0.0.1:1", "t").whoami().await.unwrap_err();
        assert!(matches!(closed, GitlabError::Network(_)), "{closed}");
    }
}
```

Run: `cargo test sources::gitlab::client`
Expected: ошибки компиляции `cannot find function 'normalize_url'`.

- [x] **Step 3: Реализовать клиент**

Добавить в начало `src/sources/gitlab/client.rs`:

```rust
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
    let url = reqwest::Url::parse(text).map_err(|_| format!("{text:?} is not an address; use one like https://gitlab.example.com"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("use an http:// or https:// address".into());
    }
    if url.host_str().is_none() || url.query().is_some() || url.fragment().is_some() || !url.username().is_empty() || url.password().is_some() {
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

    pub async fn query<T: DeserializeOwned>(&self, query: &str, variables: Value) -> Result<T, GitlabError> {
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
            return Err(GitlabError::Graphql(errors.into_iter().map(|e| e.message).collect::<Vec<_>>().join("; ")));
        }
        body.data.ok_or_else(|| GitlabError::Decode("the answer has no data".into()))
    }

    pub async fn whoami(&self) -> Result<User, GitlabError> {
        let who: Who = self.query(model::WHO_QUERY, json!({})).await?;
        who.current_user.ok_or(GitlabError::Unauthorized)
    }

    pub async fn jobs(&self) -> Result<JobsUpdate, GitlabError> {
        let first: JobsPage = self.query(model::JOBS_QUERY, json!({})).await?;
        let mut active: Vec<JobInfo> = first.active.nodes.iter().filter_map(model::to_job).collect();
        let recent_finished = first.finished.map(|f| f.nodes.iter().filter_map(model::to_job).collect()).unwrap_or_default();
        let mut page = first.active.page_info;
        for _ in 1..MAX_PAGES {
            let Some(cursor) = page.filter(|p| p.has_next_page).and_then(|p| p.end_cursor) else { break };
            let more: MoreJobs = self.query(model::MORE_JOBS_QUERY, json!({ "after": cursor })).await?;
            active.extend(more.active.nodes.iter().filter_map(model::to_job));
            page = more.active.page_info;
        }
        Ok(JobsUpdate { active, recent_finished })
    }

    pub async fn runners(&self) -> Result<Vec<RunnerInfo>, GitlabError> {
        let mut out = Vec::new();
        let mut after: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let page: RunnersPage = self.query(model::RUNNERS_QUERY, json!({ "after": after })).await?;
            out.extend(page.runners.nodes.iter().filter_map(model::to_runner));
            match page.runners.page_info {
                Some(p) if p.has_next_page && p.end_cursor.is_some() => after = p.end_cursor,
                _ => break,
            }
        }
        Ok(out)
    }
}
```

- [x] **Step 4: Запустить — проходит**

Run: `cargo test sources::gitlab && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: `11 passed` (7 модели + 4 клиента).

- [x] **Step 5: Commit**

```bash
git add src/
git commit -m "feat(gitlab): GraphQL client with sorted errors, and a fake GitLab for tests

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Задача 3: OAuth, токен и хранение подключения

**Files:**
- Replace: `src/sources/gitlab/oauth.rs`
- Create: `tests/gitlab_source.rs` (первые тесты; задача 4 дополнит)

**Interfaces:**
- Consumes: `SecretBox`, `Store::{put_setting}`, `crypto::random_token`, `client::{GitlabError, network, decode}`.
- Produces (`sources::gitlab::oauth`):
  - константы `CONNECTION_KEY = "gitlab"`, `PENDING_KEY = "gitlab_oauth"`, `SCOPE = "read_api"`, `PENDING_TTL_MS = 600_000`, `REFRESH_BEFORE_MS = 300_000`
  - `Connection { url, user, auth: StoredAuth, connected_ms }`; `StoredAuth::{Token { token }, Oauth { client_id, client_secret: Option<String>, redirect_uri, verifier, access, refresh, expires_ms }}` (все секретные строки — запечатаны) + `kind() -> &'static str` (`"token"` / `"oauth"`)
  - `Pending { state_hash, url, client_id, client_secret: Option<String>, redirect_uri, verifier, created_ms }`
  - `pkce_verifier()`, `pkce_challenge(&str)`, `authorize_url(base, client_id, redirect_uri, state, challenge) -> String`, `token_page_url(base) -> String`
  - `OauthClient { http, base, client_id, client_secret: Option<&str>, redirect_uri }` с `exchange(code, verifier, now_ms) -> Result<Tokens, GitlabError>` и `refresh(refresh_token, verifier, now_ms)`; `Tokens { access, refresh, expires_ms }`
  - `bearer(&mut Connection, &SecretBox, &reqwest::Client, &Store, now_ms) -> Result<String, GitlabError>` — токен для запросов; OAuth-токен, которому осталось < 5 мин, обновляется, новая пара сохраняется.

- [x] **Step 1: Написать падающие тесты**

Модульные тесты в `src/sources/gitlab/oauth.rs` (добавить в конец файла):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::gitlab::client::http_client;
    use crate::sources::gitlab::fake::FakeGitlab;

    #[test]
    fn pkce_matches_the_rfc_example() {
        assert_eq!(pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        let v = pkce_verifier();
        assert!((43..=128).contains(&v.len()));
        assert_ne!(v, pkce_verifier());
    }

    #[test]
    fn the_authorize_url_carries_everything() {
        let url = authorize_url("https://gitlab.example.com", "app id", "https://lorvaq.example.com/api/v1/oauth/gitlab/callback", "st", "ch");
        let parsed = reqwest::Url::parse(&url).unwrap();
        assert_eq!(parsed.path(), "/oauth/authorize");
        let q: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(q["client_id"], "app id");
        assert_eq!(q["response_type"], "code");
        assert_eq!(q["scope"], "read_api");
        assert_eq!((q["state"].as_str(), q["code_challenge"].as_str(), q["code_challenge_method"].as_str()), ("st", "ch", "S256"));
        assert_eq!(q["redirect_uri"], "https://lorvaq.example.com/api/v1/oauth/gitlab/callback");
        assert_eq!(token_page_url("https://gitlab.example.com"), "https://gitlab.example.com/-/user_settings/personal_access_tokens?name=lorvaq&scopes=read_api");
    }

    #[tokio::test]
    async fn code_exchange_needs_the_right_verifier_and_refresh_rotates() {
        let fake = FakeGitlab::start().await;
        let verifier = pkce_verifier();
        fake.state.lock().unwrap().challenge = Some(pkce_challenge(&verifier));
        let http = http_client();
        let client = OauthClient { http: &http, base: &fake.url, client_id: "app-id", client_secret: None, redirect_uri: "http://l/cb" };
        assert_eq!(client.exchange("the-code", "wrong verifier", 0).await.unwrap_err(), GitlabError::Unauthorized);
        let first = client.exchange("the-code", &verifier, 1_000).await.unwrap();
        assert_eq!((first.access.as_str(), first.refresh.as_str(), first.expires_ms), ("access-1", "refresh-1", 1_000 + 7_200_000));
        let second = client.refresh(&first.refresh, &verifier, 2_000).await.unwrap();
        assert_eq!(second.access, "access-2");
        assert_eq!(client.refresh(&first.refresh, &verifier, 3_000).await.unwrap_err(), GitlabError::Unauthorized, "the old refresh token is spent");
    }
}
```

Интеграционные тесты `tests/gitlab_source.rs`:

```rust
mod common;

use lorvaq::crypto::SecretBox;
use lorvaq::sources::gitlab::client::{http_client, GitlabError};
use lorvaq::sources::gitlab::fake::FakeGitlab;
use lorvaq::sources::gitlab::oauth::{self, bearer, pkce_challenge, pkce_verifier, Connection, StoredAuth};

fn secrets() -> SecretBox {
    SecretBox::from_base64(&SecretBox::generate_key()).unwrap()
}

fn oauth_connection(s: &SecretBox, url: &str, verifier: &str, access: &str, refresh: &str, expires_ms: i64) -> Connection {
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
    let mut conn = Connection { url: "http://x".into(), user: "dana".into(), auth: StoredAuth::Token { token: s.seal(b"good-token") }, connected_ms: 0 };
    assert_eq!(conn.auth.kind(), "token");
    assert_eq!(bearer(&mut conn, &s, &http_client(), &store, 0).await.unwrap(), "good-token");
    let other = secrets();
    assert!(matches!(bearer(&mut conn, &other, &http_client(), &store, 0).await.unwrap_err(), GitlabError::Local(_)), "another key cannot open it");
}

#[tokio::test]
async fn an_expiring_oauth_token_is_refreshed_and_the_new_pair_stored() {
    let store = common::store().await;
    let fake = FakeGitlab::start().await;
    let s = secrets();
    let verifier = pkce_verifier();
    fake.state.lock().unwrap().refresh = "refresh-0".into();
    let mut conn = oauth_connection(&s, &fake.url, &verifier, "old-access", "refresh-0", 10 * 60 * 1000);
    let http = http_client();

    let still_fresh = bearer(&mut conn, &s, &http, &store, 0).await.unwrap();
    assert_eq!(still_fresh, "old-access", "10 minutes left: no refresh");
    assert_eq!(fake.state.lock().unwrap().token_requests, 0);

    let refreshed = bearer(&mut conn, &s, &http, &store, 6 * 60 * 1000).await.unwrap();
    assert_eq!(refreshed, "access-1");
    let stored: Connection = store.setting(oauth::CONNECTION_KEY).await.unwrap().unwrap();
    let StoredAuth::Oauth { access, refresh, .. } = &stored.auth else { panic!("oauth expected") };
    assert_eq!(s.open(access).unwrap(), b"access-1");
    assert_eq!(s.open(refresh).unwrap(), b"refresh-1");
    let raw: serde_json::Value = store.setting(oauth::CONNECTION_KEY).await.unwrap().unwrap();
    assert!(!raw.to_string().contains("access-1") && !raw.to_string().contains("refresh-1"), "secrets are sealed: {raw}");
}

#[tokio::test]
async fn a_spent_refresh_token_asks_for_authorization() {
    let store = common::store().await;
    let fake = FakeGitlab::start().await;
    let s = secrets();
    let mut conn = oauth_connection(&s, &fake.url, &pkce_verifier(), "old", "spent", 0);
    assert_eq!(bearer(&mut conn, &s, &http_client(), &store, 1).await.unwrap_err(), GitlabError::Unauthorized);
    let _ = pkce_challenge("x");
}
```

Run: `cargo test sources::gitlab::oauth && cargo test --test gitlab_source`
Expected: ошибки компиляции `cannot find function 'pkce_verifier'`.

- [x] **Step 2: Реализовать**

Заменить `src/sources/gitlab/oauth.rs` целиком (тесты из шага 1 остаются в конце файла):

```rust
//! How lorvaq holds a GitLab connection: a pasted token, or OAuth (authorization
//! code + PKCE) with refresh. Every secret is sealed before it reaches the database.

use super::client::{decode, network, GitlabError};
use crate::crypto::{self, SecretBox};
use crate::store::Store;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const CONNECTION_KEY: &str = "gitlab";
pub const PENDING_KEY: &str = "gitlab_oauth";
pub const SCOPE: &str = "read_api";
/// An OAuth round trip that took longer is abandoned.
pub const PENDING_TTL_MS: i64 = 10 * 60 * 1000;
/// An access token with less left than this is refreshed first.
pub const REFRESH_BEFORE_MS: i64 = 5 * 60 * 1000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Connection {
    pub url: String,
    pub user: String,
    pub auth: StoredAuth,
    pub connected_ms: i64,
}

/// Every string here except `client_id` and `redirect_uri` is sealed with `SecretBox`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StoredAuth {
    Token {
        token: String,
    },
    Oauth {
        client_id: String,
        client_secret: Option<String>,
        redirect_uri: String,
        verifier: String,
        access: String,
        refresh: String,
        expires_ms: i64,
    },
}

impl StoredAuth {
    pub fn kind(&self) -> &'static str {
        match self {
            StoredAuth::Token { .. } => "token",
            StoredAuth::Oauth { .. } => "oauth",
        }
    }
}

/// An OAuth round trip in progress: what the callback needs to finish it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pending {
    pub state_hash: String,
    pub url: String,
    pub client_id: String,
    /// Sealed.
    pub client_secret: Option<String>,
    pub redirect_uri: String,
    /// Sealed.
    pub verifier: String,
    pub created_ms: i64,
}

pub fn pkce_verifier() -> String {
    crypto::random_token()
}

/// RFC 7636 S256: base64url(sha256(verifier)) without padding.
pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

pub fn authorize_url(base: &str, client_id: &str, redirect_uri: &str, state: &str, challenge: &str) -> String {
    let mut url = reqwest::Url::parse(&format!("{base}/oauth/authorize")).expect("the base address was validated");
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", SCOPE)
        .append_pair("state", state)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256");
    url.into()
}

/// GitLab's "new personal access token" page with the name and scope filled in.
pub fn token_page_url(base: &str) -> String {
    format!("{base}/-/user_settings/personal_access_tokens?name=lorvaq&scopes={SCOPE}")
}

pub struct Tokens {
    pub access: String,
    pub refresh: String,
    pub expires_ms: i64,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
}

pub struct OauthClient<'a> {
    pub http: &'a reqwest::Client,
    pub base: &'a str,
    pub client_id: &'a str,
    pub client_secret: Option<&'a str>,
    pub redirect_uri: &'a str,
}

impl OauthClient<'_> {
    pub async fn exchange(&self, code: &str, verifier: &str, now_ms: i64) -> Result<Tokens, GitlabError> {
        self.token(&[("grant_type", "authorization_code"), ("code", code), ("code_verifier", verifier)], now_ms).await
    }

    pub async fn refresh(&self, refresh_token: &str, verifier: &str, now_ms: i64) -> Result<Tokens, GitlabError> {
        self.token(&[("grant_type", "refresh_token"), ("refresh_token", refresh_token), ("code_verifier", verifier)], now_ms).await
    }

    async fn token(&self, grant: &[(&str, &str)], now_ms: i64) -> Result<Tokens, GitlabError> {
        let mut form: Vec<(&str, &str)> = vec![("client_id", self.client_id), ("redirect_uri", self.redirect_uri)];
        form.extend_from_slice(grant);
        if let Some(secret) = self.client_secret {
            form.push(("client_secret", secret));
        }
        let res = self.http.post(format!("{}/oauth/token", self.base)).form(&form).send().await.map_err(network)?;
        match res.status().as_u16() {
            400 | 401 => return Err(GitlabError::Unauthorized),
            code if !(200..300).contains(&code) => return Err(GitlabError::Status(code)),
            _ => {}
        }
        let t: TokenResponse = res.json().await.map_err(decode)?;
        Ok(Tokens { access: t.access_token, refresh: t.refresh_token.unwrap_or_default(), expires_ms: now_ms + t.expires_in.unwrap_or(7200) * 1000 })
    }
}

fn open(secrets: &SecretBox, sealed: &str) -> Result<String, GitlabError> {
    let bytes = secrets
        .open(sealed)
        .map_err(|_| GitlabError::Local("the stored GitLab credentials cannot be decrypted; was LORVAQ_SECRET_KEY changed? Connect GitLab again".into()))?;
    String::from_utf8(bytes).map_err(|_| GitlabError::Local("the stored GitLab credentials are damaged; connect GitLab again".into()))
}

/// The token to call GitLab with. An OAuth token about to expire is refreshed and
/// the new pair stored before it is used (GitLab rotates refresh tokens).
pub async fn bearer(conn: &mut Connection, secrets: &SecretBox, http: &reqwest::Client, store: &Store, now_ms: i64) -> Result<String, GitlabError> {
    let url = conn.url.clone();
    match &mut conn.auth {
        StoredAuth::Token { token } => open(secrets, token),
        StoredAuth::Oauth { client_id, client_secret, redirect_uri, verifier, access, refresh, expires_ms } => {
            if *expires_ms - now_ms > REFRESH_BEFORE_MS {
                return open(secrets, access);
            }
            let secret = client_secret.as_deref().map(|s| open(secrets, s)).transpose()?;
            let client = OauthClient { http, base: &url, client_id, client_secret: secret.as_deref(), redirect_uri };
            let tokens = client.refresh(&open(secrets, refresh)?, &open(secrets, verifier)?, now_ms).await?;
            *access = secrets.seal(tokens.access.as_bytes());
            *refresh = secrets.seal(tokens.refresh.as_bytes());
            *expires_ms = tokens.expires_ms;
            store
                .put_setting(CONNECTION_KEY, &*conn)
                .await
                .map_err(|e| GitlabError::Local(format!("cannot save the refreshed GitLab token: {e:#}")))?;
            Ok(tokens.access)
        }
    }
}
```

Если `store.put_setting(CONNECTION_KEY, &*conn)` не компилируется из-за живых заимствований `conn.auth`, перенести сохранение за пределы `match`: запомнить в `match` флаг «обновили» и `tokens.access`, а после `match` сохранить `conn`.

- [x] **Step 3: Запустить — проходит**

Run: `cargo test sources::gitlab && cargo test --test gitlab_source && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: `14 passed` в `sources::gitlab`, `3 passed` в `gitlab_source`.

- [x] **Step 4: Commit**

```bash
git add src/ tests/
git commit -m "feat(gitlab): OAuth with PKCE, token refresh and sealed stored credentials

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Задача 4: Опрос GitLab

**Files:**
- Create: `src/sources/gitlab/poller.rs`
- Modify: `src/sources/gitlab/mod.rs` — `pub mod poller;` и реэкспорт; `tests/gitlab_source.rs` — тесты опроса

**Interfaces:**
- Consumes: `oauth::{bearer, Connection, CONNECTION_KEY}`, `client::Gitlab`, `CiInput`, `Store::setting`, `scene::{now_ms, LOCK}`.
- Produces:
  - `poller::{PollConfig { every, runners_every, stale_after, max_backoff } (Default: 3 s, 60 s, 10 s, 30 s), SourceState::{Disconnected, Ok, Failing, Reauthorize} (serde snake_case, Default Disconnected), GitlabStatus { state, last_ok_ms, last_error }, GitlabHandle { status: Arc<Mutex<GitlabStatus>>, reload: Arc<Notify> }, spawn(Store, SecretBox, reqwest::Client, mpsc::Sender<CiInput>, PollConfig) -> GitlabHandle}`
  - поведение: без подключения — ждёт `reload.notify_one()`; с подключением — раннеры, затем джобы; после `stale_after` сбоев подряд — `CiInput::SourceStale { source: "gitlab", since_ms: <последний успех или начало сбоев> }`, после восстановления — `SourceFresh`; отказ в доступе → `Reauthorize`, повтор раз в `max_backoff`; при переходе в «отключено» шлёт пустой `Jobs` (джобы уходят по сроку пропажи) и снимает «устарело».

- [x] **Step 1: Написать падающие тесты**

Дописать в `tests/gitlab_source.rs`:

```rust
use lorvaq::scenes::ci::input::CiInput;
use lorvaq::sources::gitlab::poller::{self, PollConfig, SourceState};
use std::time::Duration;
use tokio::sync::mpsc;

fn quick() -> PollConfig {
    PollConfig { every: Duration::from_millis(50), runners_every: Duration::from_millis(300), stale_after: Duration::from_millis(150), max_backoff: Duration::from_millis(100) }
}

async fn next(rx: &mut mpsc::Receiver<CiInput>) -> CiInput {
    tokio::time::timeout(Duration::from_secs(3), rx.recv()).await.expect("an input within 3 s").expect("the poller is alive")
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
    let conn = Connection { url: url.into(), user: "dana".into(), auth: StoredAuth::Token { token: s.seal(b"good-token") }, connected_ms: 0 };
    store.put_setting(oauth::CONNECTION_KEY, &conn).await.unwrap();
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
    assert_eq!(handle.status.lock().unwrap().state, SourceState::Disconnected);

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
    assert!(handle.status.lock().unwrap().last_error.as_deref().unwrap().contains("502"));

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
    assert_eq!(handle.status.lock().unwrap().state, SourceState::Reauthorize);
}

#[tokio::test]
async fn disconnecting_empties_the_scene_and_clears_staleness() {
    let store = common::store().await;
    let fake = FakeGitlab::start().await;
    fake.state.lock().unwrap().active = vec![FakeGitlab::job(1, "demo/backend", "RUNNING", Some(7))];
    let s = secrets();
    connect_token(&store, &s, &fake.url).await;
    let (tx, mut rx) = mpsc::channel(64);
    let handle = poller::spawn(store.clone(), s, http_client(), tx, quick());
    until(&mut rx, |i| matches!(i, CiInput::Jobs(j) if !j.active.is_empty())).await;
    fake.state.lock().unwrap().fail_with = Some(500);
    until(&mut rx, |i| matches!(i, CiInput::SourceStale { .. })).await;

    store.delete_setting(oauth::CONNECTION_KEY).await.unwrap();
    handle.reload.notify_one();
    until(&mut rx, |i| matches!(i, CiInput::Jobs(j) if j.active.is_empty() && j.recent_finished.is_empty())).await;
    until(&mut rx, |i| matches!(i, CiInput::SourceFresh { .. })).await;
    assert_eq!(handle.status.lock().unwrap().state, SourceState::Disconnected);
}
```

Run: `cargo test --test gitlab_source`
Expected: ошибки компиляции `could not find 'poller'`.

- [x] **Step 2: Реализовать**

`src/sources/gitlab/poller.rs`:

```rust
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
        Self { every: Duration::from_secs(3), runners_every: Duration::from_secs(60), stale_after: Duration::from_secs(10), max_backoff: Duration::from_secs(30) }
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

pub fn spawn(store: Store, secrets: SecretBox, http: reqwest::Client, tx: mpsc::Sender<CiInput>, cfg: PollConfig) -> GitlabHandle {
    let handle = GitlabHandle::default();
    let poller = Poller { store, secrets, http, tx, cfg, handle: handle.clone(), stale: false, failing_since: None, last_runners: None, had_connection: false };
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
            GitlabError::Unauthorized => PollError::Reauthorize("GitLab refused lorvaq's credentials; authorize again in Settings".into()),
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
                    self.set(SourceState::Failing, Some(format!("cannot read the GitLab connection: {e:#}")));
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
        if self.last_runners.is_none_or(|at| at.elapsed() >= self.cfg.runners_every) {
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
            self.send(CiInput::SourceFresh { source: SOURCE.into() }).await?;
        }
        Ok(())
    }

    /// After `stale_after` of failures in a row, the scene learns its GitLab data is old.
    async fn mark_failing(&mut self, now: i64) -> Result<(), PollError> {
        let since = *self.failing_since.get_or_insert(now);
        if !self.stale && now - since >= self.cfg.stale_after.as_millis() as i64 {
            self.stale = true;
            let since_ms = self.handle.status.lock().expect(LOCK).last_ok_ms.unwrap_or(since);
            self.send(CiInput::SourceStale { source: SOURCE.into(), since_ms }).await?;
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
            self.send(CiInput::Jobs(JobsUpdate { active: vec![], recent_finished: vec![] })).await?;
        }
        if self.stale {
            self.stale = false;
            self.send(CiInput::SourceFresh { source: SOURCE.into() }).await?;
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
```

Заменить `src/sources/gitlab/mod.rs`:

```rust
//! GitLab as a source for the CI scene: queries, client, credentials and the poller.

pub mod client;
pub mod fake;
pub mod model;
pub mod oauth;
pub mod poller;

pub use poller::{spawn, GitlabHandle, GitlabStatus, PollConfig, SourceState};
```

- [x] **Step 3: Запустить — проходит**

Run: `cargo test --test gitlab_source && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: `7 passed`.

- [x] **Step 4: Commit**

```bash
git add src/ tests/
git commit -m "feat(gitlab): poller with staleness, backoff and reauthorization

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Задача 5: Подключение GitLab через HTTP

**Files:**
- Create: `src/http/sources.rs`, `tests/http_gitlab.rs`
- Modify: `src/http/mod.rs` — поля `Live`, маршруты

**Interfaces:**
- Consumes: задачи 2–4; `guard::{has_session, json_error, db_error}`, `json::ApiJson`, `crypto::{random_token, token_hash}`.
- Produces:
  - `Live` получает публичные поля `gitlab: Option<GitlabHandle>` (по умолчанию `None`), `public_url: Option<String>` (`None`), `http: reqwest::Client` (`http_client()`); `Live::new` сигнатуру не меняет.
  - маршруты (контракт для фронта, задача 10):
    - администратора: `GET /api/v1/admin/sources/gitlab` → `{"connected","url","user","kind","state","last_ok_ms","last_error","redirect_uri"}`; `DELETE` → `{"ok":true}`; `POST /api/v1/admin/sources/gitlab/token {url,token}` → `{"connected":true,"user","url"}`; `POST /api/v1/admin/sources/gitlab/oauth {url,client_id,client_secret?}` → `{"authorize_url"}`
    - публичный: `GET /api/v1/oauth/gitlab/callback?code&state[&error]` → 303 на `/#/settings?gitlab=<итог>`, итог ∈ `connected | signin | denied | expired | failed | not_admin | unreachable | unavailable | demo`
  - ошибки: 400 — неверный адрес, пустой токен, отказ GitLab, не администратор (`"<user> is not a GitLab administrator; …"`), нет публичного адреса для OAuth; 502 — GitLab недоступен или ответил странно; 503 — база.
  - публичный адрес lorvaq: `LORVAQ_PUBLIC_URL`, иначе `<X-Forwarded-Proto|http>://<Host>`; redirect URI = `<адрес>/api/v1/oauth/gitlab/callback`.

- [x] **Step 1: Написать падающие тесты**

`tests/http_gitlab.rs`:

```rust
mod common;

use axum::http::{header, StatusCode};
use axum::Router;
use common::call;
use lorvaq::auth::Access;
use lorvaq::crypto::{self, SecretBox};
use lorvaq::http::{router, AppState, Live};
use lorvaq::scene;
use lorvaq::scenes::ci::{settings::CiSettings, CiScene};
use lorvaq::sources::gitlab::fake::FakeGitlab;
use lorvaq::sources::gitlab::oauth::{self, Connection, StoredAuth};
use lorvaq::store::Store;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

const PASSWORD: &str = "correct horse battery";

async fn app(public_url: Option<&str>) -> (Router, Store, String, SecretBox) {
    let store = common::store().await;
    let secrets = SecretBox::from_base64(&SecretBox::generate_key()).unwrap();
    let (ci, _inputs) = scene::spawn(CiScene::new(CiSettings::default()).unwrap(), Duration::from_secs(1));
    let mut live = Live::new(store.clone(), secrets.clone(), false, Access::Admin, vec![]);
    live.public_url = public_url.map(String::from);
    let r = router(Arc::new(AppState { scenes: vec![ci], live: Some(live) }));
    store.set_admin_hash(&crypto::hash_password(PASSWORD), 0).await.unwrap();
    let (_, _, cookie) = call(&r, "POST", "/api/v1/login", None, Some(json!({ "password": PASSWORD })), &[]).await;
    (r, store, cookie.unwrap(), secrets)
}

/// Follows nothing: returns the Location of a redirect.
async fn location(r: &Router, uri: &str, cookie: Option<&str>) -> String {
    use tower::ServiceExt;
    let mut req = axum::http::Request::get(uri);
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    let res = r.clone().oneshot(req.body(axum::body::Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    res.headers()[header::LOCATION].to_str().unwrap().to_string()
}

#[tokio::test]
async fn connecting_with_a_token_checks_it_and_seals_it() {
    let (r, store, cookie, _) = app(None).await;
    let fake = FakeGitlab::start().await;
    let (status, body, _) = call(&r, "POST", "/api/v1/admin/sources/gitlab/token", Some(&cookie), Some(json!({ "url": "not a url", "token": "x" })), &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body, _) = call(&r, "POST", "/api/v1/admin/sources/gitlab/token", Some(&cookie), Some(json!({ "url": fake.url, "token": "wrong" })), &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    fake.state.lock().unwrap().admin = false;
    let (status, body, _) = call(&r, "POST", "/api/v1/admin/sources/gitlab/token", Some(&cookie), Some(json!({ "url": fake.url, "token": "good-token" })), &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("not a GitLab administrator"), "{body}");
    fake.state.lock().unwrap().admin = true;
    let (status, body, _) = call(&r, "POST", "/api/v1/admin/sources/gitlab/token", Some(&cookie), Some(json!({ "url": format!("{}/", fake.url), "token": "good-token" })), &[]).await;
    assert_eq!((status, body["user"].as_str()), (StatusCode::OK, Some("dana")));

    let raw: serde_json::Value = store.setting(oauth::CONNECTION_KEY).await.unwrap().unwrap();
    assert!(!raw.to_string().contains("good-token"), "the token is sealed: {raw}");
    assert_eq!(raw["url"], json!(fake.url), "the trailing slash is gone");
    let (_, st, _) = call(&r, "GET", "/api/v1/admin/sources/gitlab", Some(&cookie), None, &[]).await;
    assert_eq!((st["connected"].clone(), st["kind"].clone(), st["user"].clone()), (json!(true), json!("token"), json!("dana")));
    assert!(!st.to_string().contains("good-token"));

    let (status, _, _) = call(&r, "DELETE", "/api/v1/admin/sources/gitlab", Some(&cookie), None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    let (_, st, _) = call(&r, "GET", "/api/v1/admin/sources/gitlab", Some(&cookie), None, &[]).await;
    assert_eq!(st["connected"], json!(false));
}

#[tokio::test]
async fn the_oauth_round_trip_connects_once() {
    let (r, store, cookie, secrets) = app(Some("https://lorvaq.example.com")).await;
    let fake = FakeGitlab::start().await;
    let (status, body, _) = call(&r, "POST", "/api/v1/admin/sources/gitlab/oauth", Some(&cookie), Some(json!({ "url": fake.url, "client_id": "app-id" })), &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let authorize = reqwest::Url::parse(body["authorize_url"].as_str().unwrap()).unwrap();
    let q: std::collections::HashMap<_, _> = authorize.query_pairs().into_owned().collect();
    assert_eq!(q["redirect_uri"], "https://lorvaq.example.com/api/v1/oauth/gitlab/callback");
    fake.state.lock().unwrap().challenge = Some(q["code_challenge"].clone());
    let state = q["state"].clone();

    let cb = |code: &str, st: &str| format!("/api/v1/oauth/gitlab/callback?code={code}&state={st}");
    assert_eq!(location(&r, &cb("the-code", &state), None).await, "/#/settings?gitlab=signin", "no admin session");
    assert_eq!(location(&r, &cb("the-code", "forged"), Some(&cookie)).await, "/#/settings?gitlab=expired");
    assert_eq!(location(&r, &cb("the-code", &state), Some(&cookie)).await, "/#/settings?gitlab=connected");
    assert_eq!(location(&r, &cb("the-code", &state), Some(&cookie)).await, "/#/settings?gitlab=expired", "a state works once");

    let conn: Connection = store.setting(oauth::CONNECTION_KEY).await.unwrap().unwrap();
    let StoredAuth::Oauth { access, .. } = &conn.auth else { panic!("oauth expected") };
    assert_eq!(secrets.open(access).unwrap(), b"access-1");
}

#[tokio::test]
async fn oauth_needs_a_public_address_and_reports_denials() {
    let (r, _, cookie, _) = app(None).await;
    let fake = FakeGitlab::start().await;
    let (status, body, _) = call(&r, "POST", "/api/v1/admin/sources/gitlab/oauth", Some(&cookie), Some(json!({ "url": fake.url, "client_id": "app-id" })), &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "no LORVAQ_PUBLIC_URL and no Host: {body}");
    let (status, body, _) = call(&r, "POST", "/api/v1/admin/sources/gitlab/oauth", Some(&cookie), Some(json!({ "url": fake.url, "client_id": "app-id" })), &[("host", "lorvaq.local:8080")]).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["authorize_url"].as_str().unwrap().contains("lorvaq.local%3A8080"), "{body}");
    assert_eq!(location(&r, "/api/v1/oauth/gitlab/callback?error=access_denied&state=x", Some(&cookie)).await, "/#/settings?gitlab=denied");
}

#[tokio::test]
async fn gitlab_routes_need_the_admin() {
    let (r, _, _, _) = app(None).await;
    assert_eq!(call(&r, "GET", "/api/v1/admin/sources/gitlab", None, None, &[]).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(call(&r, "POST", "/api/v1/admin/sources/gitlab/token", None, Some(json!({ "url": "http://x", "token": "t" })), &[]).await.0, StatusCode::UNAUTHORIZED);
}
```

Run: `cargo test --test http_gitlab`
Expected: ошибки компиляции: нет поля `public_url` у `Live`.

- [x] **Step 2: Поля `Live` и маршруты**

В `src/http/mod.rs`:
- добавить `mod sources;` к объявлениям модулей;
- в `struct Live` добавить поля:

```rust
    /// The GitLab poller, when a scene uses GitLab.
    pub gitlab: Option<crate::sources::gitlab::GitlabHandle>,
    /// LORVAQ_PUBLIC_URL without the trailing slash: OAuth sends the browser back here.
    pub public_url: Option<String>,
    /// For calls to GitLab.
    pub http: reqwest::Client,
```

- в `Live::new` в литерал `Self { … }` добавить `gitlab: None, public_url: None, http: crate::sources::gitlab::client::http_client(),`;
- в роутер администратора (`let admin = Router::new()…`, до `.route_layer`) добавить:

```rust
        .route("/api/v1/admin/sources/gitlab", get(sources::status).delete(sources::disconnect))
        .route("/api/v1/admin/sources/gitlab/token", post(sources::connect_token))
        .route("/api/v1/admin/sources/gitlab/oauth", post(sources::oauth_start))
```

- в общий роутер (рядом с `/api/v1/logout`) добавить `.route("/api/v1/oauth/gitlab/callback", get(sources::oauth_callback))`.

`src/http/sources.rs`:

```rust
//! Admin: connect GitLab with a token or with OAuth, see the connection, disconnect.
//! The OAuth callback is public (the browser comes back from GitLab) but checks the admin session.

use super::guard::{db_error, has_session, json_error};
use super::json::ApiJson;
use super::{AppState, Live};
use crate::crypto;
use crate::scene::{now_ms, LOCK};
use crate::sources::gitlab::client::{normalize_url, Gitlab, GitlabError};
use crate::sources::gitlab::oauth::{self, Connection, OauthClient, Pending, StoredAuth, CONNECTION_KEY, PENDING_KEY, PENDING_TTL_MS};
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

#[derive(Deserialize)]
pub(super) struct TokenBody {
    url: String,
    token: String,
}

#[derive(Deserialize)]
pub(super) struct OauthBody {
    url: String,
    client_id: String,
    #[serde(default)]
    client_secret: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

fn live(s: &AppState) -> &Live {
    s.live.as_ref().expect("the admin guard lets only live requests in")
}

fn public_base(live: &Live, headers: &HeaderMap) -> Option<String> {
    if let Some(url) = &live.public_url {
        return Some(url.clone());
    }
    let host = headers.get(header::HOST)?.to_str().ok()?;
    let proto = headers.get("x-forwarded-proto").and_then(|v| v.to_str().ok()).filter(|p| *p == "https").unwrap_or("http");
    Some(format!("{proto}://{host}"))
}

fn redirect_uri(base: &str) -> String {
    format!("{base}/api/v1/oauth/gitlab/callback")
}

fn not_admin(user: &str) -> String {
    format!("{user} is not a GitLab administrator; lorvaq needs an administrator (a dedicated service user is best) to see the jobs of every runner")
}

fn gitlab_error(e: GitlabError) -> Response {
    match e {
        GitlabError::Unauthorized => json_error(StatusCode::BAD_REQUEST, "GitLab did not accept the token"),
        other => json_error(StatusCode::BAD_GATEWAY, &other.to_string()),
    }
}

async fn save(live: &Live, conn: &Connection) -> Response {
    if let Err(e) = live.store.put_setting(CONNECTION_KEY, conn).await {
        return db_error(e);
    }
    if let Err(e) = live.store.delete_setting(PENDING_KEY).await {
        tracing::warn!("an unused GitLab OAuth start stays until it expires: {e:#}");
    }
    if let Some(gitlab) = &live.gitlab {
        gitlab.reload.notify_one();
    }
    Json(json!({ "connected": true, "user": conn.user, "url": conn.url })).into_response()
}

pub(super) async fn status(State(s): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let live = live(&s);
    let conn = match live.store.setting::<Connection>(CONNECTION_KEY).await {
        Ok(conn) => conn,
        Err(e) => return db_error(e),
    };
    let st = live.gitlab.as_ref().map(|g| g.status.lock().expect(LOCK).clone()).unwrap_or_default();
    Json(json!({
        "connected": conn.is_some(),
        "url": conn.as_ref().map(|c| c.url.clone()),
        "user": conn.as_ref().map(|c| c.user.clone()),
        "kind": conn.as_ref().map(|c| c.auth.kind()),
        "state": st.state,
        "last_ok_ms": st.last_ok_ms,
        "last_error": st.last_error,
        "redirect_uri": public_base(live, &headers).map(|b| redirect_uri(&b)),
    }))
    .into_response()
}

pub(super) async fn connect_token(State(s): State<Arc<AppState>>, ApiJson(body): ApiJson<TokenBody>) -> Response {
    let live = live(&s);
    let url = match normalize_url(&body.url) {
        Ok(url) => url,
        Err(message) => return json_error(StatusCode::BAD_REQUEST, &message),
    };
    let token = body.token.trim();
    if token.is_empty() || token.len() > 4096 {
        return json_error(StatusCode::BAD_REQUEST, "paste the token GitLab showed you");
    }
    let user = match Gitlab::new(&live.http, &url, token).whoami().await {
        Ok(user) => user,
        Err(e) => return gitlab_error(e),
    };
    if !user.admin.unwrap_or(false) {
        return json_error(StatusCode::BAD_REQUEST, &not_admin(&user.username));
    }
    let conn = Connection { url, user: user.username, auth: StoredAuth::Token { token: live.secrets.seal(token.as_bytes()) }, connected_ms: now_ms() };
    save(live, &conn).await
}

pub(super) async fn oauth_start(State(s): State<Arc<AppState>>, headers: HeaderMap, ApiJson(body): ApiJson<OauthBody>) -> Response {
    let live = live(&s);
    let url = match normalize_url(&body.url) {
        Ok(url) => url,
        Err(message) => return json_error(StatusCode::BAD_REQUEST, &message),
    };
    let client_id = body.client_id.trim();
    if client_id.is_empty() || client_id.len() > 256 {
        return json_error(StatusCode::BAD_REQUEST, "paste the Application ID from GitLab");
    }
    let Some(base) = public_base(live, &headers) else {
        return json_error(StatusCode::BAD_REQUEST, "set LORVAQ_PUBLIC_URL so GitLab knows where to send you back");
    };
    let redirect = redirect_uri(&base);
    let state = crypto::random_token();
    let verifier = oauth::pkce_verifier();
    let pending = Pending {
        state_hash: crypto::token_hash(&state),
        url: url.clone(),
        client_id: client_id.to_string(),
        client_secret: body.client_secret.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(|s| live.secrets.seal(s.as_bytes())),
        redirect_uri: redirect.clone(),
        verifier: live.secrets.seal(verifier.as_bytes()),
        created_ms: now_ms(),
    };
    if let Err(e) = live.store.put_setting(PENDING_KEY, &pending).await {
        return db_error(e);
    }
    Json(json!({ "authorize_url": oauth::authorize_url(&url, client_id, &redirect, &state, &oauth::pkce_challenge(&verifier)) })).into_response()
}

fn back(outcome: &str) -> Response {
    Redirect::to(&format!("/#/settings?gitlab={outcome}")).into_response()
}

pub(super) async fn oauth_callback(State(s): State<Arc<AppState>>, jar: CookieJar, Query(q): Query<CallbackQuery>) -> Response {
    let Some(live) = &s.live else { return back("demo") };
    if !has_session(&s, &jar).await {
        return back("signin");
    }
    if q.error.is_some() {
        return back("denied");
    }
    let (Some(code), Some(state)) = (q.code, q.state) else { return back("denied") };
    let now = now_ms();
    let pending = match live.store.setting::<Pending>(PENDING_KEY).await {
        Ok(Some(p)) => p,
        Ok(None) => return back("expired"),
        Err(_) => return back("unavailable"),
    };
    if now - pending.created_ms > PENDING_TTL_MS || crypto::token_hash(&state) != pending.state_hash {
        return back("expired");
    }
    // One use: a replayed callback finds nothing.
    if live.store.delete_setting(PENDING_KEY).await.is_err() {
        return back("unavailable");
    }
    let open = |sealed: &str| live.secrets.open(sealed).ok().and_then(|b| String::from_utf8(b).ok());
    let Some(verifier) = open(&pending.verifier) else { return back("failed") };
    let secret = match pending.client_secret.as_deref().map(open) {
        Some(None) => return back("failed"),
        Some(Some(secret)) => Some(secret),
        None => None,
    };
    let client = OauthClient { http: &live.http, base: &pending.url, client_id: &pending.client_id, client_secret: secret.as_deref(), redirect_uri: &pending.redirect_uri };
    let tokens = match client.exchange(&code, &verifier, now).await {
        Ok(tokens) => tokens,
        Err(GitlabError::Network(_)) => return back("unreachable"),
        Err(_) => return back("failed"),
    };
    let user = match Gitlab::new(&live.http, &pending.url, &tokens.access).whoami().await {
        Ok(user) => user,
        Err(_) => return back("failed"),
    };
    if !user.admin.unwrap_or(false) {
        return back("not_admin");
    }
    let conn = Connection {
        url: pending.url,
        user: user.username,
        auth: StoredAuth::Oauth {
            client_id: pending.client_id,
            client_secret: pending.client_secret,
            redirect_uri: pending.redirect_uri,
            verifier: pending.verifier,
            access: live.secrets.seal(tokens.access.as_bytes()),
            refresh: live.secrets.seal(tokens.refresh.as_bytes()),
            expires_ms: tokens.expires_ms,
        },
        connected_ms: now,
    };
    if live.store.put_setting(CONNECTION_KEY, &conn).await.is_err() {
        return back("unavailable");
    }
    if let Some(gitlab) = &live.gitlab {
        gitlab.reload.notify_one();
    }
    back("connected")
}

pub(super) async fn disconnect(State(s): State<Arc<AppState>>) -> Response {
    let live = live(&s);
    if let Err(e) = live.store.delete_setting(CONNECTION_KEY).await {
        return db_error(e);
    }
    if let Err(e) = live.store.delete_setting(PENDING_KEY).await {
        tracing::warn!("an unused GitLab OAuth start stays until it expires: {e:#}");
    }
    if let Some(gitlab) = &live.gitlab {
        gitlab.reload.notify_one();
    }
    Json(json!({ "ok": true })).into_response()
}
```

- [x] **Step 3: Запустить — проходит**

Run: `cargo test --test http_gitlab && cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: `4 passed` в `http_gitlab`, всё остальное зелёное.

- [x] **Step 4: Commit**

```bash
git add src/ tests/
git commit -m "feat(http): connect GitLab with a token or OAuth, see and remove the connection

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Задача 6: Сборка — GitLab питает сцену, уборка строк раннеров, дешёвый healthz

**Files:**
- Modify: `src/app.rs`, `src/world/mod.rs`, `src/scene.rs`, `src/http/mod.rs` (healthz), `src/scenes/ci/reduce.rs`, `tests/live_smoke.rs`

**Interfaces:**
- Consumes: `sources::gitlab::{spawn, PollConfig}`.
- Produces: `World::stale() -> &BTreeMap<String, i64>`; `SceneHandle::stale() -> BTreeMap<String, i64>`; живой сервер запускает опрос GitLab в сцену CI; `CiScene::tick` убирает пустые места `runner:*` в `nohost`.

- [x] **Step 1: Написать падающие тесты**

Дописать в модуль `tests` файла `src/scenes/ci/reduce.rs`:

```rust
    #[test]
    fn empty_rows_of_runners_without_an_agent_go_away() {
        let mut h = Harness::new(CiSettings::default());
        h.feed(jobs(vec![running(1, "g/app", 7)], vec![]));
        assert!(h.world.place("runner:7").is_some());
        h.tick(0);
        assert!(h.world.place("runner:7").is_some(), "kept while a job runs there");
        h.feed(jobs(vec![], vec![JobInfo { status: JobStatus::Success, ..running(1, "g/app", 7) }]));
        assert!(h.world.place("runner:7").is_some(), "the finished job just left for the sink");
        let ch = h.tick(0);
        assert!(ch.contains(&Change::RemovePlace { id: "runner:7".into() }), "the empty row goes on the next tick: {ch:?}");
    }
```

Дописать в `tests/live_smoke.rs`:

```rust
#[tokio::test]
async fn a_connected_gitlab_puts_its_jobs_on_the_scene() {
    use lorvaq::sources::gitlab::fake::FakeGitlab;
    use lorvaq::sources::gitlab::oauth::{Connection, StoredAuth, CONNECTION_KEY};
    let store = common::store().await;
    let fake = FakeGitlab::start().await;
    fake.state.lock().unwrap().active = vec![FakeGitlab::job(42, "demo/backend", "PENDING", None)];
    let secrets = SecretBox::from_base64(&SecretBox::generate_key()).unwrap();
    let conn = Connection { url: fake.url.clone(), user: "dana".into(), auth: StoredAuth::Token { token: secrets.seal(b"good-token") }, connected_ms: 0 };
    store.put_setting(CONNECTION_KEY, &conn).await.unwrap();
    let state = app::live_state(store, secrets, None).await.unwrap();
    let ci = state.scenes[0].clone();
    let mut seen = false;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if ci.snapshot().entities.iter().any(|e| e.id == "job:42") {
            seen = true;
            break;
        }
    }
    assert!(seen, "job 42 from GitLab reached the scene");
}
```

Run: `cargo test scenes::ci && cargo test --test live_smoke`
Expected: тест про строки раннеров падает (нет `RemovePlace`), смоук — не находит `job:42`.

- [x] **Step 2: Реализовать**

В `src/scenes/ci/reduce.rs`, в `fn tick` у `impl Scene for CiScene`, перед последней строкой `out` добавить:

```rust
        // Rows of runners without an agent exist only while they hold a job.
        for row in world.places().filter(|p| p.kind == "process" && p.parent.as_deref() == Some(NOHOST)) {
            if !world.entities().any(|e| e.place == row.id) {
                out.push(Change::RemovePlace { id: row.id.clone() });
            }
        }
```

В `src/world/mod.rs`, в `impl World`, добавить:

```rust
    pub fn stale(&self) -> &BTreeMap<String, i64> {
        &self.stale
    }
```

В `src/scene.rs`, в `impl SceneHandle`, добавить:

```rust
    /// The sources whose data is old, without copying the whole world.
    pub fn stale(&self) -> std::collections::BTreeMap<String, i64> {
        self.state.read().expect(LOCK).world.stale().clone()
    }
```

В `src/http/mod.rs`, в `healthz`, заменить `h.snapshot().stale` на `h.stale()`.

В `src/app.rs`:
- импорт: `use crate::sources::gitlab::{self as gitlab_source, PollConfig};` и `use crate::scenes::ci::input::CiInput;` и `use tokio::sync::mpsc;`
- `live_scenes` возвращает ещё и вход сцены CI: заменить сигнатуру на `async fn live_scenes(store: &Store) -> anyhow::Result<(Vec<SceneHandle>, mpsc::Sender<CiInput>)>`, строку `let (handle, _inputs) = scene::spawn(ci, Duration::from_secs(1));` — на `let (handle, inputs) = scene::spawn(ci, Duration::from_secs(1));`, комментарий над ней удалить, последнюю строку — на `Ok((vec![handle], inputs))`;
- в `live_state`: `let (scenes, ci_inputs) = live_scenes(&store).await?;`; перед `let live = Live::new(…)` добавить

```rust
    let http = crate::sources::gitlab::client::http_client();
    let gitlab = gitlab_source::spawn(store.clone(), secrets.clone(), http.clone(), ci_inputs, PollConfig::default());
```

  и после `let live = Live::new(…);` заменить его на изменяемый и дописать:

```rust
    let mut live = Live::new(store, secrets, cookie_secure, access, history);
    live.gitlab = Some(gitlab);
    live.http = http;
    live.public_url = public_url.map(|u| u.trim_end_matches('/').to_string()).filter(|u| !u.is_empty());
```

- [x] **Step 3: Запустить — всё проходит**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: всё зелёное, `live_smoke` — `3 passed`.

- [x] **Step 4: Commit**

```bash
git add src/ tests/
git commit -m "feat: the live server polls GitLab into the CI scene; tidy runner rows; cheap healthz

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Задача 7: Вход — отзыв потоков, очередь argon2, ограничитель, атомарная смена пароля

**Files:**
- Modify: `src/http/mod.rs`, `src/http/account.rs`, `src/http/admin.rs`, `src/auth.rs`, `src/store/mod.rs`, `tests/http_live.rs`, `tests/store.rs`

**Interfaces:**
- Produces:
  - `Live::auth_epoch: tokio::sync::watch::Sender<u64>` и `Live::bump_auth_epoch()`; эпоха растёт при выходе, смене пароля и смене режима доступа. Поток событий, открытый в режиме не `open`, закрывается при смене эпохи и не живёт дольше **10 мин** (клиент переподключается и проходит проверку заново).
  - `Live::argon_waiting: AtomicUsize`; больше **16** ожидающих проверок пароля → 503 `{"error":"lorvaq is busy checking passwords; try again in a moment"}`, попытка не засчитывается.
  - `LoginLimiter`: не больше **20 000** адресов (при переполнении выбрасываются те, чья последняя попытка старше всех); `release(key, at_ms)` снимает именно свою отметку.
  - `Store::replace_admin_password(hash, keep_session_hash, now_ms)` — в одной транзакции меняет пароль и удаляет чужие сессии.
  - выход всегда снимает cookie, даже если база недоступна (сессия удаляется из кэша, предупреждение в лог).
  - `announce_setup_code` не перевыпускает код, если действующий уже есть.

- [x] **Step 1: Написать падающие тесты**

Дописать в `tests/http_live.rs`:

```rust
#[tokio::test]
async fn signing_out_ends_open_event_streams() {
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let (r, _, cookie) = signed_in(Access::Admin).await;
    let req = axum::http::Request::get("/api/v1/scenes/ci/events?since=0").header(axum::http::header::COOKIE, &cookie).body(axum::body::Body::empty()).unwrap();
    let res = r.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let mut body = res.into_body();
    let (status, _, _) = call(&r, "POST", "/api/v1/logout", Some(&cookie), None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    let ended = tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(frame) = body.frame().await {
            frame.unwrap();
        }
    })
    .await;
    assert!(ended.is_ok(), "the stream closed after sign-out");
}

#[tokio::test]
async fn sign_out_clears_the_cookie_even_with_the_database_down() {
    let (r, store, cookie) = signed_in(Access::Admin).await;
    store.close().await;
    let (status, _, cleared) = call(&r, "POST", "/api/v1/logout", Some(&cookie), None, &[]).await;
    assert_eq!((status, cleared.as_deref()), (StatusCode::OK, Some("lorvaq_session=")));
}
```

Дописать в `tests/store.rs`:

```rust
#[tokio::test]
async fn replacing_the_password_drops_other_sessions_atomically() {
    let s = common::store().await;
    s.set_admin_hash("old", 0).await.unwrap();
    s.create_session("mine", 0, 1_000).await.unwrap();
    s.create_session("theirs", 0, 1_000).await.unwrap();
    s.replace_admin_password("new", "mine", 1).await.unwrap();
    assert_eq!(s.admin_hash().await.unwrap().as_deref(), Some("new"));
    assert!(s.session_valid("mine", 10).await.unwrap());
    assert!(!s.session_valid("theirs", 10).await.unwrap());
}

#[tokio::test]
async fn a_valid_setup_code_survives_a_restart() {
    let s = common::store().await;
    let code = lorvaq::auth::issue_setup_code(&s, 1_000).await.unwrap();
    lorvaq::auth::announce_setup_code(&s, 2_000).await.unwrap();
    assert!(lorvaq::auth::check_setup_code(&s, &code, 3_000).await.unwrap(), "the code from reset-password still works");
}
```

Дописать в модуль `tests` файла `src/auth.rs`:

```rust
    #[test]
    fn the_limiter_stays_bounded_and_releases_its_own_mark() {
        let mut l = LoginLimiter::default();
        for i in 0..(MAX_TRACKED as u32 + 10) {
            assert!(l.try_begin(IpAddr::from(std::net::Ipv4Addr::from(i)), i64::from(i)));
        }
        assert!(l.len() <= MAX_TRACKED, "{}", l.len());
        let key: IpAddr = "10.9.9.9".parse().unwrap();
        assert!(l.try_begin(key, 100));
        assert!(l.try_begin(key, 200));
        l.release(key, 100);
        assert_eq!(l.marks(key), vec![200]);
    }
```

Дописать в модуль `tests` файла `src/http/account.rs`:

```rust
    #[tokio::test]
    async fn a_full_queue_answers_busy_without_running_the_work() {
        let gate = Arc::new(Semaphore::new(0));
        let waiting = AtomicUsize::new(ARGON_QUEUE);
        let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let r = ran.clone();
        let result = run_limited(&gate, &waiting, move || r.store(true, Ordering::SeqCst)).await;
        assert!(result.is_err());
        assert!(!ran.load(Ordering::SeqCst));
        assert_eq!(waiting.load(Ordering::SeqCst), ARGON_QUEUE, "the counter is restored");
    }
```

В том же модуле в существующем тесте `the_permit_outlives_a_cancelled_request` вызов `run_limited(&g, move || { … })` заменить на `run_limited(&g, &AtomicUsize::new(0), move || { … })`.

Run: `cargo test`
Expected: ошибки компиляции (`replace_admin_password`, `len`, `marks`, новая сигнатура `run_limited`), тест потока падает по таймауту.

- [x] **Step 2: Хранилище, ограничитель, код настройки**

В `src/store/mod.rs`, в `impl Store`:

```rust
    /// A new admin password and every other session gone, or neither.
    pub async fn replace_admin_password(&self, hash: &str, keep_session_hash: &str, now_ms: i64) -> anyhow::Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE admin SET password_hash = $1, updated_ms = $2 WHERE id = 1").bind(hash).bind(now_ms).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM sessions WHERE token_hash <> $1").bind(keep_session_hash).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
```

В `src/auth.rs`:
- константу `MAX_TRACKED` сделать `pub(crate) const MAX_TRACKED: usize = 20_000;` (если она уже есть с другим значением — заменить);
- в `LoginLimiter::try_begin` сразу после существующей чистки устаревших записей добавить вытеснение:

```rust
        if self.attempts.len() >= MAX_TRACKED && !self.attempts.contains_key(&key) {
            // Still full of recent addresses: forget the one that tried longest ago.
            if let Some(oldest) = self.attempts.iter().min_by_key(|(_, times)| times.last().copied().unwrap_or(i64::MIN)).map(|(k, _)| *k) {
                self.attempts.remove(&oldest);
            }
        }
```

- заменить `release` и добавить два помощника для тестов:

```rust
    /// Takes back the attempt counted at `at_ms`: it was not a password guess.
    pub fn release(&mut self, key: IpAddr, at_ms: i64) {
        if let Some(times) = self.attempts.get_mut(&key) {
            if let Some(i) = times.iter().rposition(|t| *t == at_ms) {
                times.remove(i);
            }
            if times.is_empty() {
                self.attempts.remove(&key);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.attempts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.attempts.is_empty()
    }

    pub fn marks(&self, key: IpAddr) -> Vec<i64> {
        self.attempts.get(&key).cloned().unwrap_or_default()
    }
```

- в `src/http/account.rs` все вызовы `release(key)` заменить на `release(key, now)` (в `setup` — в замыкании `let release = || …release(key, now);`, в `login` — там же).
- заменить `announce_setup_code`:

```rust
/// On a fresh install there is no admin: print a one-time code to create one. A code
/// that is still valid (from a first start or `reset-password`) is kept, not replaced.
pub async fn announce_setup_code(store: &Store, now_ms: i64) -> anyhow::Result<()> {
    if store.admin_hash().await?.is_some() {
        return Ok(());
    }
    if store.setting::<SetupCode>(SETUP_CODE_KEY).await?.is_some_and(|c| c.expires_ms > now_ms) {
        tracing::warn!("no admin yet: use the setup code printed earlier, or run `lorvaq admin reset-password` for a new one");
        return Ok(());
    }
    let code = issue_setup_code(store, now_ms).await?;
    tracing::warn!("no admin yet: open lorvaq and enter the one-time setup code {code} (valid for 24 hours)");
    Ok(())
}
```

Тест плана 2 `a_setup_code_is_announced_only_without_an_admin` остаётся зелёным: без кода он выпускается, при администраторе — нет.

- [x] **Step 3: Очередь argon2, выход, смена пароля, эпоха**

В `src/http/mod.rs`:
- в `struct Live` добавить:

```rust
    /// Bumped when sign-in state or the access mode changes: guarded event streams close.
    pub auth_epoch: tokio::sync::watch::Sender<u64>,
    /// Password checks waiting for an argon2 slot.
    pub argon_waiting: std::sync::atomic::AtomicUsize,
```

- в `Live::new` добавить `auth_epoch: tokio::sync::watch::channel(0).0, argon_waiting: std::sync::atomic::AtomicUsize::new(0),`;
- добавить в `impl Live`:

```rust
    pub fn bump_auth_epoch(&self) {
        self.auth_epoch.send_modify(|epoch| *epoch += 1);
    }
```

- в обработчике `events` перед `Sse::new(stream)` заменить формирование ответа на:

```rust
    let stream = futures::stream::iter(backlog).chain(live).map(to_sse);
    // Streams that needed a sign-in or a trusted proxy end when that may have changed;
    // the client reconnects and the guard checks again.
    let guarded = s.live.as_ref().map(|l| (*l.access.read().expect(LOCK) != Access::Open, l.auth_epoch.subscribe()));
    let stream: futures::stream::BoxStream<'static, Result<SseEvent, Infallible>> = match guarded {
        Some((true, mut epoch)) => {
            let revoked = async move {
                tokio::select! {
                    _ = epoch.changed() => {}
                    _ = tokio::time::sleep(GUARDED_STREAM_MAX) => {}
                }
            };
            stream.take_until(revoked).boxed()
        }
        _ => stream.boxed(),
    };
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))).into_response()
```

  (строку `let stream = futures::stream::iter(backlog)…` из текущего кода заменить этой же), добавить константу `const GUARDED_STREAM_MAX: Duration = Duration::from_secs(600);` и импорт `use crate::scene::LOCK;`, если его нет.

В `src/http/admin.rs`, в `put_access`, после `*live.access.write().expect(LOCK) = access.clone();` добавить `live.bump_auth_epoch();`.

В `src/http/account.rs`:
- импорт `use std::sync::atomic::{AtomicUsize, Ordering};`;
- заменить `run_limited`, `hash_blocking`, `verify_blocking`:

```rust
/// More password checks than this already waiting: answer 503 instead of queueing.
const ARGON_QUEUE: usize = 16;

pub(super) struct Busy;

fn busy() -> Response {
    json_error(StatusCode::SERVICE_UNAVAILABLE, "lorvaq is busy checking passwords; try again in a moment")
}

/// Leaves the waiting count when dropped, even if the request is cancelled while waiting.
struct Waiting<'a>(&'a AtomicUsize);

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Runs slow, memory-hungry work on the blocking pool; at most `ARGON_RUNS` run at once and
/// at most `ARGON_QUEUE` wait. The permit travels into the closure: a blocking task cannot be
/// cancelled, so a client that disconnects must not free its place while the work still runs.
async fn run_limited<T: Send + 'static>(gate: &Arc<Semaphore>, waiting: &AtomicUsize, work: impl FnOnce() -> T + Send + 'static) -> Result<T, Busy> {
    let queued = waiting.fetch_add(1, Ordering::SeqCst);
    let in_line = Waiting(waiting);
    if queued >= ARGON_QUEUE {
        return Err(Busy);
    }
    let permit = gate.clone().acquire_owned().await.expect("the semaphore is never closed");
    drop(in_line);
    tokio::task::spawn_blocking(move || {
        let out = work();
        drop(permit);
        out
    })
    .await
    .map_err(|_| Busy)
}

async fn hash_blocking(live: &Live, password: String) -> Result<String, Busy> {
    run_limited(&live.argon, &live.argon_waiting, move || crypto::hash_password(&password)).await
}

async fn verify_blocking(live: &Live, password: String, hash: String) -> Result<bool, Busy> {
    run_limited(&live.argon, &live.argon_waiting, move || crypto::verify_password(&password, &hash)).await
}
```

- в `setup` заменить `let hash = hash_blocking(live, body.password).await;` на:

```rust
    let Ok(hash) = hash_blocking(live, body.password).await else { return busy() };
```

- в `login` заменить блок `if !verify_blocking(live, body.password, hash).await { … }` на:

```rust
    match verify_blocking(live, body.password, hash).await {
        Ok(true) => {}
        Ok(false) => return json_error(StatusCode::UNAUTHORIZED, "wrong password"),
        Err(Busy) => {
            live.limiter.lock().expect(LOCK).release(key, now);
            return busy();
        }
    }
```

- заменить `logout` целиком:

```rust
pub(super) async fn logout(State(s): State<Arc<AppState>>, jar: CookieJar) -> Response {
    if let (Some(live), Some(cookie)) = (&s.live, jar.get(SESSION_COOKIE)) {
        let hash = crypto::token_hash(cookie.value());
        live.sessions.lock().expect(LOCK).evict(&hash);
        let deleted = live.store.delete_session(&hash).await;
        // Again once the row is gone: a check that began in between may have read it.
        live.sessions.lock().expect(LOCK).evict(&hash);
        live.bump_auth_epoch();
        if let Err(e) = deleted {
            // The cookie goes anyway; the row expires on its own.
            tracing::warn!("signed out, but the session row stays until it expires: {e:#}");
        }
    }
    let mut gone = Cookie::from(SESSION_COOKIE);
    gone.set_path("/");
    (jar.remove(gone), Json(json!({ "ok": true }))).into_response()
}
```

- в `change_password` заменить всё после проверки длины нового пароля на:

```rust
    let hash = match live.store.admin_hash().await {
        Ok(Some(hash)) => hash,
        Ok(None) => return json_error(StatusCode::CONFLICT, "no admin yet"),
        Err(e) => return db_error(e),
    };
    match verify_blocking(live, body.current, hash).await {
        Ok(true) => {}
        Ok(false) => return json_error(StatusCode::FORBIDDEN, "the current password is wrong"),
        Err(Busy) => return busy(),
    }
    let Ok(new_hash) = hash_blocking(live, body.new).await else { return busy() };
    let mine = jar.get(SESSION_COOKIE).map(|c| crypto::token_hash(c.value())).expect("the admin guard checked the cookie");
    live.sessions.lock().expect(LOCK).evict_all_except(&mine);
    let replaced = live.store.replace_admin_password(&new_hash, &mine, now_ms()).await;
    // Again once the rows are gone: a check that began in between may have read them.
    live.sessions.lock().expect(LOCK).evict_all_except(&mine);
    live.bump_auth_epoch();
    if let Err(e) = replaced {
        return db_error(e);
    }
    Json(json!({ "ok": true })).into_response()
```

- [x] **Step 4: Запустить — всё проходит**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: всё зелёное, новые тесты проходят.

- [x] **Step 5: Commit**

```bash
git add src/ tests/
git commit -m "fix(auth): revoke guarded streams, bound the argon2 queue and the limiter, atomic password change

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Задача 8: История без дубликатов

**Files:**
- Create: `migrations/0002_event_runs.sql`
- Modify: `src/store/mod.rs`, `src/history.rs`, `tests/store.rs`

**Interfaces:**
- Produces: `Store::insert_events(scene, run: i64, events)` — повтор того же `(scene, run, seq)` ничего не добавляет; `history::HistorySink { store, run }` реализует `EventSink`; `spawn_writer` пишет с `run = now_ms()` на момент запуска.

- [x] **Step 1: Миграция**

`migrations/0002_event_runs.sql`:

```sql
-- A run is one server start: seq restarts with it. (scene, run, seq) is unique, so a
-- batch written twice (a timed-out write that still committed) keeps one copy.
ALTER TABLE events ADD COLUMN run BIGINT;
-- Rows from before runs existed: each its own run, so the unique index can be built.
UPDATE events SET run = -id;
ALTER TABLE events ALTER COLUMN run SET NOT NULL;
CREATE UNIQUE INDEX events_scene_run_seq ON events (scene, run, seq);
-- Pruning by age across all scenes.
CREATE INDEX events_ts ON events (ts_ms);
```

- [x] **Step 2: Написать падающий тест**

В `tests/store.rs` в тесте `events_are_written_counted_and_pruned` заменить вызовы `s.insert_events("ci", …)` и `s.insert_events("other", …)` на `s.insert_events("ci", 1, …)` и `s.insert_events("other", 1, …)`, и дописать тест:

```rust
#[tokio::test]
async fn a_batch_written_twice_keeps_one_copy() {
    let s = common::store().await;
    let ev = |seq: u64| Event { seq, ts_ms: 10, change: Change::Gone { id: "j".into() } };
    s.insert_events("ci", 5, &[ev(1), ev(2)]).await.unwrap();
    s.insert_events("ci", 5, &[ev(1), ev(2), ev(3)]).await.unwrap();
    assert_eq!(s.event_count("ci").await.unwrap(), 3);
    s.insert_events("ci", 6, &[ev(1)]).await.unwrap();
    assert_eq!(s.event_count("ci").await.unwrap(), 4, "a new run starts its seq again");
}
```

Run: `cargo test --test store`
Expected: ошибка компиляции — `insert_events` принимает 2 аргумента.

- [x] **Step 3: Реализовать**

В `src/store/mod.rs` заменить `insert_events`:

```rust
    /// Writes events of one server run; an event already stored for the run is skipped.
    pub async fn insert_events(&self, scene: &str, run: i64, events: &[Event]) -> anyhow::Result<()> {
        if events.is_empty() {
            return Ok(());
        }
        let mut query = QueryBuilder::<Postgres>::new("INSERT INTO events (scene, run, seq, ts_ms, change) ");
        query.push_values(events, |mut row, ev| {
            row.push_bind(scene)
                .push_bind(run)
                .push_bind(ev.seq as i64)
                .push_bind(ev.ts_ms)
                .push_bind(serde_json::to_value(&ev.change).expect("changes always serialize"));
        });
        query.push(" ON CONFLICT (scene, run, seq) DO NOTHING");
        query.build().execute(&self.pool).await?;
        Ok(())
    }
```

В `src/history.rs` заменить `impl EventSink for Store { … }` на:

```rust
/// The store, writing for one server run.
#[derive(Clone)]
pub struct HistorySink {
    pub store: Store,
    pub run: i64,
}

impl EventSink for HistorySink {
    async fn write(&self, scene: &str, events: &[Event]) -> anyhow::Result<()> {
        self.store.insert_events(scene, self.run, events).await
    }
}
```

и в `spawn_writer` там, где `Writer` создаётся из `store`, передавать `HistorySink { store, run: now_ms() }` вместо `store` (`now_ms` уже импортирован из `crate::scene`).

- [x] **Step 4: Запустить — всё проходит**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: всё зелёное; `store` — на 1 тест больше, чем было.

- [x] **Step 5: Commit**

```bash
git add migrations/ src/ tests/
git commit -m "fix(history): one copy per event per server run; index for pruning

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Задача 9: Веб — подключение GitLab

**Files:**
- Modify: `web/src/api.ts`, `web/src/settings/forms.ts`, `web/src/settings/forms.test.ts`, `web/src/i18n.ts`, `web/src/pages/Settings.tsx`, `web/src/App.tsx`, `web/src/index.css`

**Interfaces:**
- Consumes: HTTP-контракт задачи 5.
- Produces:
  - `api.ts`: `GitlabStatus`, `getGitlab()`, `connectGitlabToken(url, token)`, `startGitlabOauth(url, clientId, clientSecret)`, `disconnectGitlab()`.
  - `forms.ts`: `gitlabUrlProblem(url)`, `normalizeGitlabUrl(url)`, `tokenPageUrl(base)`, `oauthOutcome(hash) -> string | null` (текст по итогу из `#/settings?gitlab=…`).
  - раздел «GitLab» первым на странице настроек; App понимает `#/settings?…`; при недоступном сервере на старте App повторяет запрос раз в 3 с.

- [x] **Step 1: Написать падающие тесты**

Дописать в `web/src/settings/forms.test.ts`:

```ts
import { gitlabUrlProblem, normalizeGitlabUrl, oauthOutcome, tokenPageUrl } from "./forms";

describe("gitlab", () => {
  test("addresses", () => {
    expect(gitlabUrlProblem("https://gitlab.example.com")).toBeNull();
    expect(gitlabUrlProblem("http://127.0.0.1:8929/gitlab/")).toBeNull();
    expect(gitlabUrlProblem("gitlab.example.com")).not.toBeNull();
    expect(gitlabUrlProblem("ftp://gitlab.example.com")).not.toBeNull();
    expect(gitlabUrlProblem("https://gitlab.example.com/?a=1")).not.toBeNull();
    expect(normalizeGitlabUrl(" https://gitlab.example.com/// ")).toBe("https://gitlab.example.com");
  });

  test("the token page is prefilled", () => {
    expect(tokenPageUrl("https://gitlab.example.com")).toBe("https://gitlab.example.com/-/user_settings/personal_access_tokens?name=lorvaq&scopes=read_api");
  });

  test("oauth outcomes come back in the hash", () => {
    expect(oauthOutcome("#/settings?gitlab=connected")).toMatch(/connected/i);
    expect(oauthOutcome("#/settings?gitlab=not_admin")).toMatch(/administrator/);
    expect(oauthOutcome("#/settings?gitlab=whatever")).toMatch(/did not finish/);
    expect(oauthOutcome("#/settings")).toBeNull();
  });
});
```

Run: `cd web && npm test`
Expected: FAIL — нет экспортов `gitlabUrlProblem` и др.

- [x] **Step 2: Реализовать**

Дописать в `web/src/i18n.ts` (в объект `t`, перед закрывающей `}`):

```ts
  // gitlab
  gitlabTitle: "GitLab",
  gitlabUrl: "GitLab address",
  gitlabConnectedAs: (user: string, url: string) => `Connected as ${user} to ${url}`,
  gitlabState: { disconnected: "not connected", ok: "polling", failing: "cannot reach GitLab", reauthorize: "authorize again" } as Record<string, string>,
  gitlabLastOk: (seconds: number) => `last answer ${seconds}s ago`,
  gitlabDisconnect: "Disconnect",
  gitlabByOauth: "Authorize with an OAuth application",
  gitlabByToken: "Paste a token",
  gitlabOauthHint: "In GitLab open Admin → Applications (or User settings → Applications) and add an application named lorvaq with this redirect URI and the read_api scope. Then paste its Application ID here.",
  gitlabRedirectUri: "Redirect URI",
  gitlabNoRedirect: "Set LORVAQ_PUBLIC_URL to use OAuth",
  gitlabClientId: "Application ID",
  gitlabClientSecret: "Secret (only for a confidential application)",
  gitlabAuthorize: "Authorize",
  gitlabTokenHint: "Sign in to GitLab as an administrator (a dedicated service user is best), create a token with the read_api scope and paste it here.",
  gitlabCreateToken: "Create a token in GitLab",
  gitlabToken: "Token",
  gitlabConnect: "Connect",
  gitlabBadUrl: "Use the address of GitLab, like https://gitlab.example.com",
  gitlabOutcome: {
    connected: "GitLab is connected",
    signin: "Sign in as the admin first, then authorize again",
    denied: "GitLab did not grant access",
    expired: "The authorization took too long or was already used; start again",
    not_admin: "That GitLab user is not an administrator; lorvaq needs one to see every runner's jobs",
    unreachable: "lorvaq could not reach GitLab to finish the authorization",
    unavailable: "lorvaq's database is unavailable; try again",
  } as Record<string, string>,
  gitlabOutcomeOther: "The authorization did not finish; start again",
```

Дописать в `web/src/settings/forms.ts`:

```ts
export function normalizeGitlabUrl(url: string): string {
  return url.trim().replace(/\/+$/, "");
}

export function gitlabUrlProblem(url: string): string | null {
  try {
    const u = new URL(url.trim());
    if (!["http:", "https:"].includes(u.protocol) || u.search || u.hash || u.username || u.password) return t.gitlabBadUrl;
    return null;
  } catch {
    return t.gitlabBadUrl;
  }
}

export function tokenPageUrl(base: string): string {
  return `${normalizeGitlabUrl(base)}/-/user_settings/personal_access_tokens?name=lorvaq&scopes=read_api`;
}

/** The result of an OAuth round trip, from `#/settings?gitlab=<outcome>`. */
export function oauthOutcome(hash: string): string | null {
  const query = hash.split("?")[1];
  if (!query) return null;
  const outcome = new URLSearchParams(query).get("gitlab");
  if (!outcome) return null;
  return t.gitlabOutcome[outcome] ?? t.gitlabOutcomeOther;
}
```

Дописать в `web/src/api.ts` (после `putSceneSettings`):

```ts
export interface GitlabStatus {
  connected: boolean;
  url: string | null;
  user: string | null;
  kind: "oauth" | "token" | null;
  state: "disconnected" | "ok" | "failing" | "reauthorize";
  last_ok_ms: number | null;
  last_error: string | null;
  redirect_uri: string | null;
}

export const getGitlab = () => getJson<GitlabStatus>("/api/v1/admin/sources/gitlab");
export const connectGitlabToken = (url: string, token: string) => request<{ connected: true; user: string; url: string }>("POST", "/api/v1/admin/sources/gitlab/token", { url, token });
export const startGitlabOauth = (url: string, client_id: string, client_secret: string | null) =>
  request<{ authorize_url: string }>("POST", "/api/v1/admin/sources/gitlab/oauth", { url, client_id, client_secret });
export const disconnectGitlab = () => request<{ ok: true }>("DELETE", "/api/v1/admin/sources/gitlab");
```

В `web/src/pages/Settings.tsx`:
- в импорт из `"../api"` добавить `connectGitlabToken, disconnectGitlab, getGitlab, startGitlabOauth, type GitlabStatus`;
- в импорт из `"../settings/forms"` добавить `gitlabUrlProblem, normalizeGitlabUrl, oauthOutcome, tokenPageUrl`;
- добавить компонент перед `export function Settings`:

```tsx
function GitlabSection({ onAuthLost }: Props) {
  const [st, setSt] = useState<GitlabStatus | null>(null);
  const [url, setUrl] = useState("");
  const [mode, setMode] = useState<"oauth" | "token">("oauth");
  const [clientId, setClientId] = useState("");
  const [secret, setSecret] = useState("");
  const [token, setToken] = useState("");
  const [status, setStatus] = useState<string | null>(() => oauthOutcome(location.hash));

  const load = () =>
    getGitlab()
      .then((g) => {
        setSt(g);
        if (g.url) setUrl(g.url);
      })
      .catch((e) => setStatus(failure(e, onAuthLost)));

  useEffect(() => {
    void load();
    if (location.hash.includes("?")) history.replaceState(null, "", "#/settings");
    const id = setInterval(() => void load(), 5000);
    return () => clearInterval(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function connect(e: FormEvent) {
    e.preventDefault();
    const problem = gitlabUrlProblem(url);
    if (problem) return setStatus(problem);
    try {
      if (mode === "token") {
        await connectGitlabToken(normalizeGitlabUrl(url), token.trim());
        setToken("");
        setStatus(t.gitlabOutcome.connected);
        void load();
      } else {
        const { authorize_url } = await startGitlabOauth(normalizeGitlabUrl(url), clientId.trim(), secret.trim() || null);
        location.href = authorize_url;
      }
    } catch (err) {
      setStatus(failure(err, onAuthLost));
    }
  }

  async function disconnect() {
    try {
      await disconnectGitlab();
      void load();
    } catch (err) {
      setStatus(failure(err, onAuthLost));
    }
  }

  const seconds = st?.last_ok_ms ? Math.max(0, Math.round((Date.now() - st.last_ok_ms) / 1000)) : null;
  return (
    <form className="card" onSubmit={connect}>
      <h2>{t.gitlabTitle}</h2>
      {st?.connected && (
        <div className="row">
          <span className={st.state === "ok" ? "ok" : "error"}>
            {t.gitlabConnectedAs(st.user ?? "?", st.url ?? "?")} — {t.gitlabState[st.state]}
            {seconds !== null ? `, ${t.gitlabLastOk(seconds)}` : ""}
          </span>
          <button type="button" onClick={disconnect}>{t.gitlabDisconnect}</button>
        </div>
      )}
      {st?.last_error && <p className="error">{st.last_error}</p>}
      <label>
        {t.gitlabUrl}
        <input value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://gitlab.example.com" />
      </label>
      <label className="choice">
        <input type="radio" name="gitlab-mode" checked={mode === "oauth"} onChange={() => setMode("oauth")} />
        {t.gitlabByOauth}
      </label>
      <label className="choice">
        <input type="radio" name="gitlab-mode" checked={mode === "token"} onChange={() => setMode("token")} />
        {t.gitlabByToken}
      </label>
      {mode === "oauth" ? (
        <>
          <p className="hint">{t.gitlabOauthHint}</p>
          <p className="hint">
            {t.gitlabRedirectUri}: {st?.redirect_uri ? <code>{st.redirect_uri}</code> : t.gitlabNoRedirect}
          </p>
          <label>
            {t.gitlabClientId}
            <input value={clientId} onChange={(e) => setClientId(e.target.value)} autoComplete="off" />
          </label>
          <label>
            {t.gitlabClientSecret}
            <input type="password" value={secret} onChange={(e) => setSecret(e.target.value)} autoComplete="off" />
          </label>
          <button type="submit">{t.gitlabAuthorize}</button>
        </>
      ) : (
        <>
          <p className="hint">{t.gitlabTokenHint}</p>
          {!gitlabUrlProblem(url) && (
            <a href={tokenPageUrl(url)} target="_blank" rel="noopener noreferrer">
              {t.gitlabCreateToken}
            </a>
          )}
          <label>
            {t.gitlabToken}
            <input type="password" value={token} onChange={(e) => setToken(e.target.value)} autoComplete="off" />
          </label>
          <button type="submit">{t.gitlabConnect}</button>
        </>
      )}
      {status && <p className={status === t.gitlabOutcome.connected ? "ok" : "error"}>{status}</p>}
    </form>
  );
}
```

- в `export function Settings` первым элементом внутри `<div className="settings">` добавить `<GitlabSection onAuthLost={onAuthLost} />`.

В `web/src/App.tsx`:
- заменить `routeOf` на `const routeOf = (hash: string): Route => (hash.startsWith("#/settings") ? "settings" : "scenes");`
- в `refresh` в обоих `.catch(() => setStatus("offline"))` заменить на `.catch(() => { setStatus("offline"); setTimeout(refresh, 3000); })` (для `fetchMe`; для `fetchScenes` — так же). `refresh` объявлен через `useCallback` до использования; рекурсивная ссылка через замыкание допустима.

Дописать в `web/src/index.css`:

```css
.card code { font-size: 12px; background: var(--bg); border: 1px solid var(--border); border-radius: 4px; padding: 1px 4px; user-select: all; }
.card a { color: var(--text); font-size: 13px; }
```

- [x] **Step 3: Тесты и сборка**

Run: `cd web && npm test && npm run build`
Expected: все тесты зелёные (+3 в `forms.test.ts`), сборка без ошибок.

- [x] **Step 4: Commit**

```bash
git add web/src/
git commit -m "feat(web): connect GitLab with OAuth or a token from Settings

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Задача 10: Compose, README и проверка целиком

**Files:**
- Modify: `compose.yml`, `README.md`

- [x] **Step 1: Файлы**

В `compose.yml`, у сервиса `lorvaq`, заменить блок портов на:

```yaml
    ports:
      # Local only by default: put your reverse proxy (with https) in front.
      # To expose lorvaq directly, use "8080:8080".
      - "127.0.0.1:8080:8080"
```

В `README.md` после раздела `## Run it` добавить:

```markdown
## Connect GitLab

lorvaq reads jobs and runners through GitLab's GraphQL API with the `read_api`
scope. It needs a GitLab **administrator** to see the jobs of every runner; a
dedicated service user is best, so the connection does not depend on a person.

In Settings → GitLab, either:

- **Authorize** — in GitLab create an application (Admin → Applications, or
  User settings → Applications) with the redirect URI lorvaq shows and the
  `read_api` scope, paste its Application ID and press Authorize. This needs
  `LORVAQ_PUBLIC_URL`. Tokens are refreshed automatically.
- **Paste a token** — the link opens GitLab's token page with the name and scope
  filled in.

Tokens are stored encrypted with `LORVAQ_SECRET_KEY`. lorvaq polls jobs every 3
seconds and runners every minute. If GitLab stops answering, the scene shows a
"data is old" banner after 10 seconds.

GitLab with a certificate from a private CA is not supported yet.

## Behind a reverse proxy

Put https in front of lorvaq and set `LORVAQ_PUBLIC_URL` to its https address.

To let your proxy decide who sees the scenes, choose "People your reverse proxy
signed in" in Settings: name the header your proxy sets with the user name and
list the proxy's addresses. lorvaq then trusts that header — and the client
address in `X-Forwarded-For` for sign-in limits — only from those addresses.
Anything that can reach lorvaq from a listed address can set them, so list only
the proxy itself, not a whole network other services share.

In the other modes, all sign-in attempts through a proxy share the proxy's
address: a few wrong passwords lock sign-in for everyone for 15 minutes.
```

- [x] **Step 2: Проверить целиком**

```bash
cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check
(cd web && npm test && npm run build)
git grep -n -i -E -f ~/.config/lorvaq/denylist -- ':!web/src/scenes/ci/renderer.ts' || echo "clean"
```

Expected: всё зелёное, `clean`.

Затем вживую (`compose.yml` + `.env` как в плане 2, `LORVAQ_PUBLIC_URL=http://localhost:8080`):

1. Создать администратора, открыть Settings → GitLab.
2. **Проверка на настоящем GitLab требует GitLab, где у владельца есть права администратора, и токена или OAuth-приложения в нём. Исполнитель сам ничего в GitLab не создаёт — просит владельца выдать токен `read_api` (или создать приложение) и вставить его сам.** С выданным токеном: «Paste a token» → Connect → статус «Connected as … — polling»; через несколько секунд на сцене — настоящие джобы и раннеры.
3. С OAuth-приложением (если владелец его создал): Authorize → страница GitLab → разрешить → возврат в Settings с «GitLab is connected».
4. Остановить сеть до GitLab (например, неверный адрес — подключить `http://127.0.0.1:9`) — через ~10 с баннер «gitlab data is … old»; вернуть верное подключение — баннер уходит.
5. Disconnect → джобы уходят со сцены за ~2 мин, баннера нет.
6. Убрать стенд: `docker compose down -v`, удалить `.env`. Владельцу напомнить отозвать выданный для проверки токен.

- [x] **Step 3: Commit**

```bash
git add compose.yml README.md
git commit -m "docs: connecting GitLab and running behind a reverse proxy; local-only port by default

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```
