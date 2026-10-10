//! What the CI scene is fed with: GitLab polls and agent reports.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Pending,
    Running,
    Success,
    Failed,
    Canceled,
}

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            JobStatus::Pending => "pending",
            JobStatus::Running => "running",
            JobStatus::Success => "success",
            JobStatus::Failed => "failed",
            JobStatus::Canceled => "canceled",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JobInfo {
    pub id: u64,
    pub name: String,
    pub stage: String,
    pub status: JobStatus,
    pub tags: Vec<String>,
    /// Full path, e.g. `group/project`.
    pub project: String,
    pub pipeline: u64,
    pub runner_id: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JobsUpdate {
    /// Every pending and running job.
    pub active: Vec<JobInfo>,
    /// The newest finished jobs, to catch finishes between polls.
    pub recent_finished: Vec<JobInfo>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunnerScope {
    Instance,
    Group { full_path: String },
    Project { full_paths: Vec<String> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunnerInfo {
    pub id: u64,
    pub description: String,
    pub online: bool,
    pub tags: Vec<String>,
    pub run_untagged: bool,
    pub scope: RunnerScope,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentReport {
    pub agent: String,
    pub host_label: String,
    pub processes: Vec<ProcessReport>,
    pub disk: Option<DiskUsage>,
}

/// One gitlab-runner process: its registrations share its slots.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProcessReport {
    pub key: String,
    /// `concurrent` from the runner's metrics, when known.
    pub concurrent: Option<u32>,
    #[serde(default)]
    pub runner_ids: Vec<u64>,
    /// Per-registration `limit`, when known.
    #[serde(default)]
    pub limits: BTreeMap<u64, u32>,
    /// Occupied slots only.
    pub slots: Vec<SlotReport>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlotReport {
    pub slot: u32,
    pub job_id: u64,
    pub runner_id: u64,
    pub mem_bytes: u64,
    pub mem_limit_bytes: Option<u64>,
    pub has_service: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiskUsage {
    pub used_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContainerExit {
    pub agent: String,
    pub job_id: u64,
    pub oom: bool,
    pub exit_code: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum CiInput {
    Jobs(JobsUpdate),
    Runners(Vec<RunnerInfo>),
    Agent(AgentReport),
    AgentExit(ContainerExit),
    AgentGone { agent: String },
    SourceStale { source: String, since_ms: i64 },
    SourceFresh { source: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_report_round_trips_with_numeric_map_keys() {
        let report = CiInput::Agent(AgentReport {
            agent: "a".into(),
            host_label: "h".into(),
            processes: vec![ProcessReport {
                key: "p".into(),
                concurrent: Some(5),
                runner_ids: vec![1, 2],
                limits: BTreeMap::from([(2, 2)]),
                slots: vec![SlotReport {
                    slot: 0,
                    job_id: 9,
                    runner_id: 1,
                    mem_bytes: 10,
                    mem_limit_bytes: Some(20),
                    has_service: true,
                }],
            }],
            disk: Some(DiskUsage {
                used_bytes: 1,
                total_bytes: 2,
            }),
        });
        let text = serde_json::to_string(&report).unwrap();
        assert!(text.contains(r#""type":"agent""#), "{text}");
        assert_eq!(serde_json::from_str::<CiInput>(&text).unwrap(), report);
    }

    #[test]
    fn status_names() {
        assert_eq!(JobStatus::Pending.as_str(), "pending");
        assert_eq!(JobStatus::Canceled.as_str(), "canceled");
        assert_eq!(
            serde_json::to_string(&JobStatus::Running).unwrap(),
            r#""running""#
        );
    }
}
