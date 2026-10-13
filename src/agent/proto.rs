//! What the agent and the server say to each other: JSON in WebSocket text frames.

use crate::scenes::ci::input::{DiskUsage, ProcessReport};
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const CONNECT_PATH: &str = "/api/v1/agent/connect";
/// A larger frame closes the connection.
pub const MAX_FRAME: usize = 256 * 1024;
pub const MAX_PROCESSES: usize = 16;
pub const MAX_SLOTS: usize = 512;
pub const MAX_HOST_LABEL: usize = 64;
/// How often an agent reports.
pub const REPORT_EVERY: Duration = Duration::from_secs(2);
/// An agent this quiet counts as gone.
pub const SILENCE: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FromAgent {
    /// The first frame.
    Hello { host: String, version: String },
    Report {
        processes: Vec<ProcessReport>,
        disk: Option<DiskUsage>,
    },
    /// A job container stopped.
    Exit {
        job_id: u64,
        oom: bool,
        exit_code: i64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToAgent {
    Welcome { agent: String },
}

impl FromAgent {
    /// Bounds a report so one agent cannot flood the scene.
    pub fn check(&self) -> Result<(), &'static str> {
        match self {
            FromAgent::Hello { host, version } => {
                if host.trim().is_empty() || host.chars().count() > MAX_HOST_LABEL {
                    return Err("the host label needs 1 to 64 characters");
                }
                if host.chars().any(char::is_control) || version.len() > 64 {
                    return Err("bad hello");
                }
            }
            FromAgent::Report { processes, .. } => {
                if processes.len() > MAX_PROCESSES {
                    return Err("too many processes");
                }
                if processes
                    .iter()
                    .any(|p| p.slots.len() > MAX_SLOTS || p.key.is_empty() || p.key.len() > 64)
                {
                    return Err("bad process");
                }
            }
            FromAgent::Exit { .. } => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip_and_are_bounded() {
        let hello = FromAgent::Hello {
            host: "build-01".into(),
            version: "0.1.0".into(),
        };
        let text = serde_json::to_string(&hello).unwrap();
        assert!(text.contains(r#""type":"hello""#), "{text}");
        assert_eq!(serde_json::from_str::<FromAgent>(&text).unwrap(), hello);
        assert!(hello.check().is_ok());
        let long = FromAgent::Hello {
            host: "x".repeat(65),
            version: String::new(),
        };
        assert!(long.check().is_err());
        let flood = FromAgent::Report {
            processes: vec![
                ProcessReport {
                    key: "p".into(),
                    ..Default::default()
                };
                17
            ],
            disk: None,
        };
        assert!(flood.check().is_err());
        let report: FromAgent =
            serde_json::from_str(r#"{"type":"report","processes":[{"key":"p","concurrent":null,"slots":[{"slot":0,"job_id":1,"mem_bytes":5,"mem_limit_bytes":null,"has_service":false}]}],"disk":null}"#)
                .unwrap();
        assert!(report.check().is_ok());
    }
}
