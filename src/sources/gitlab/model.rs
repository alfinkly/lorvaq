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
        stage: job
            .stage
            .as_ref()
            .and_then(|s| s.name.clone())
            .unwrap_or_default(),
        status: job_status(&job.status)?,
        tags: job.tags.clone().unwrap_or_default(),
        project: job.project.as_ref()?.full_path.clone(),
        pipeline: job
            .pipeline
            .as_ref()
            .and_then(|p| gid_number(&p.id))
            .unwrap_or(0),
        runner_id: job.runner.as_ref().and_then(|r| gid_number(&r.id)),
    })
}

pub fn to_runner(runner: &GqlRunner) -> Option<RunnerInfo> {
    let id = gid_number(&runner.id)?;
    let scope = match runner.runner_type.as_str() {
        "INSTANCE_TYPE" => RunnerScope::Instance,
        "GROUP_TYPE" => RunnerScope::Group {
            full_path: runner.groups.as_ref()?.nodes.first()?.full_path.clone(),
        },
        "PROJECT_TYPE" => RunnerScope::Project {
            full_paths: runner
                .projects
                .as_ref()
                .map(|p| p.nodes.iter().map(|n| n.full_path.clone()).collect())
                .unwrap_or_default(),
        },
        _ => return None,
    };
    Some(RunnerInfo {
        id,
        description: runner
            .description
            .clone()
            .filter(|d| !d.trim().is_empty())
            .unwrap_or_else(|| format!("runner #{id}")),
        online: runner.status.as_deref() == Some("ONLINE") && !runner.paused.unwrap_or(false),
        tags: runner.tag_list.clone().unwrap_or_default(),
        run_untagged: runner.run_untagged.unwrap_or(false),
        scope,
    })
}

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
        for s in [
            "CREATED",
            "MANUAL",
            "SCHEDULED",
            "WAITING_FOR_RESOURCE",
            "SKIPPED",
            "NEW_THING",
        ] {
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
        assert_eq!(
            (j.id, j.name.as_str(), j.stage.as_str(), j.status),
            (42, "build", "build", JobStatus::Running)
        );
        assert_eq!(
            (j.tags, j.project.as_str(), j.pipeline, j.runner_id),
            (vec!["ops".to_string()], "demo/backend", 9, Some(7))
        );
    }

    #[test]
    fn sparse_jobs_are_kept_or_skipped_sensibly() {
        let pending = job(json!({ "id": "gid://gitlab/Ci::Build/1", "name": null, "status": "PENDING", "stage": null, "tags": null, "runner": null, "project": { "fullPath": "demo/docs" }, "pipeline": null })).unwrap();
        assert_eq!(
            (
                pending.name.as_str(),
                pending.stage.as_str(),
                pending.pipeline,
                pending.runner_id
            ),
            ("", "", 0, None)
        );
        assert!(
            job(json!({ "id": "gid://gitlab/Ci::Build/2", "status": "PENDING", "project": null }))
                .is_none(),
            "no project"
        );
        assert!(job(json!({ "id": "gid://gitlab/Ci::Build/3", "status": "MANUAL", "project": { "fullPath": "demo/docs" } })).is_none(), "not shown");
    }

    #[test]
    fn runners_and_their_scopes() {
        let instance = runner(json!({ "id": "gid://gitlab/Ci::Runner/1", "description": "shared", "status": "ONLINE", "paused": false, "runnerType": "INSTANCE_TYPE", "tagList": ["ops"], "runUntagged": true, "groups": null, "projects": null })).unwrap();
        assert_eq!(
            (
                instance.id,
                instance.online,
                instance.run_untagged,
                instance.scope
            ),
            (1, true, true, RunnerScope::Instance)
        );
        let group = runner(json!({ "id": "gid://gitlab/Ci::Runner/2", "description": "", "status": "ONLINE", "paused": true, "runnerType": "GROUP_TYPE", "tagList": [], "runUntagged": false, "groups": { "nodes": [{ "fullPath": "demo" }] }, "projects": null })).unwrap();
        assert_eq!(
            group.scope,
            RunnerScope::Group {
                full_path: "demo".into()
            }
        );
        assert!(!group.online, "paused runners take no jobs");
        assert_eq!(
            group.description, "runner #2",
            "empty descriptions get a name"
        );
        let project = runner(json!({ "id": "gid://gitlab/Ci::Runner/3", "status": "OFFLINE", "runnerType": "PROJECT_TYPE", "projects": { "nodes": [{ "fullPath": "demo/a" }, { "fullPath": "demo/b" }] } })).unwrap();
        assert_eq!(
            project.scope,
            RunnerScope::Project {
                full_paths: vec!["demo/a".into(), "demo/b".into()]
            }
        );
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
        let who: Who =
            serde_json::from_value(json!({ "currentUser": { "username": "dana", "admin": true } }))
                .unwrap();
        assert_eq!(who.current_user.unwrap().admin, Some(true));
    }

    #[test]
    fn queries_ask_for_what_the_model_reads() {
        for field in [
            "active:",
            "finished:",
            "PREPARING",
            "CANCELING",
            "fullPath",
            "pageInfo",
            "fragment job on CiJobInterface",
        ] {
            assert!(JOBS_QUERY.contains(field), "{field}");
        }
        assert!(MORE_JOBS_QUERY.contains("$after") && !MORE_JOBS_QUERY.contains("finished:"));
        for field in [
            "runners(first: 100, after: $after)",
            "runUntagged",
            "tagList",
            "paused",
            "groups(first: 1)",
            "projects(first: 20)",
        ] {
            assert!(RUNNERS_QUERY.contains(field), "{field}");
        }
    }
}
