//! CI scene settings: exits, folding, capacities, timings.

use super::input::JobInfo;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CiSettings {
    pub exits: Vec<ExitRule>,
    /// Projects with their own place; the rest fold into "other".
    pub max_projects: usize,
    /// Slots of a runner process whose `concurrent` is unknown.
    pub default_capacity: u32,
    /// Per process key, overrides `default_capacity`.
    pub capacities: BTreeMap<String, u32>,
    pub finished_linger_ms: i64,
    pub missing_grace_ms: i64,
    pub project_idle_ms: i64,
    /// A project without active jobs this long gives its own place to a newcomer.
    pub project_quiet_ms: i64,
}

impl Default for CiSettings {
    fn default() -> Self {
        Self {
            exits: Vec::new(),
            max_projects: 8,
            default_capacity: 4,
            capacities: BTreeMap::new(),
            finished_linger_ms: 3_000,
            missing_grace_ms: 120_000,
            project_idle_ms: 6 * 3600 * 1000,
            project_quiet_ms: 15 * 60 * 1000,
        }
    }
}

/// A successful job matching the rule flies to the exit. Patterns match whole values.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExitRule {
    pub label: String,
    #[serde(default)]
    pub stage: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

pub struct Exit {
    pub id: String,
    pub label: String,
    stage: Option<Regex>,
    name: Option<Regex>,
}

impl Exit {
    pub fn matches(&self, job: &JobInfo) -> bool {
        if self.stage.is_none() && self.name.is_none() {
            return false;
        }
        self.stage.as_ref().is_none_or(|r| r.is_match(&job.stage))
            && self.name.as_ref().is_none_or(|r| r.is_match(&job.name))
    }
}

impl CiSettings {
    /// Rejects numbers that would break the scene (no projects, no slots, negative times).
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.max_projects >= 1, "max_projects must be at least 1");
        anyhow::ensure!(
            (1..=1000).contains(&self.default_capacity),
            "default_capacity must be between 1 and 1000"
        );
        if let Some((key, _)) = self.capacities.iter().find(|(_, c)| **c < 1) {
            anyhow::bail!("the capacity of {key:?} must be at least 1");
        }
        for (name, ms) in [
            ("finished_linger_ms", self.finished_linger_ms),
            ("missing_grace_ms", self.missing_grace_ms),
            ("project_idle_ms", self.project_idle_ms),
            ("project_quiet_ms", self.project_quiet_ms),
        ] {
            anyhow::ensure!(ms >= 0, "{name} must not be negative");
        }
        Ok(())
    }

    pub fn compile_exits(&self) -> anyhow::Result<Vec<Exit>> {
        self.exits
            .iter()
            .map(|rule| {
                Ok(Exit {
                    id: format!("exit:{}", rule.label),
                    label: rule.label.clone(),
                    stage: rule.stage.as_deref().map(whole).transpose()?,
                    name: rule.name.as_deref().map(whole).transpose()?,
                })
            })
            .collect()
    }
}

fn whole(pattern: &str) -> anyhow::Result<Regex> {
    Ok(Regex::new(&format!("^(?:{pattern})$"))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenes::ci::input::JobStatus;

    fn job(stage: &str, name: &str) -> JobInfo {
        JobInfo {
            id: 1,
            name: name.into(),
            stage: stage.into(),
            status: JobStatus::Success,
            tags: vec![],
            project: "a/b".into(),
            pipeline: 1,
            runner_id: None,
        }
    }

    #[test]
    fn defaults_come_from_an_empty_object() {
        let s: CiSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.max_projects, 8);
        assert_eq!(s.default_capacity, 4);
        assert_eq!(s.finished_linger_ms, 3_000);
        assert_eq!(s.missing_grace_ms, 120_000);
        assert_eq!(s.project_idle_ms, 6 * 3600 * 1000);
        assert_eq!(s.project_quiet_ms, 15 * 60 * 1000);
        assert!(s.exits.is_empty());
    }

    #[test]
    fn exit_patterns_match_whole_values() {
        let s = CiSettings {
            exits: vec![
                ExitRule {
                    label: "registry".into(),
                    stage: Some("build".into()),
                    name: None,
                },
                ExitRule {
                    label: "prod".into(),
                    stage: Some("deploy".into()),
                    name: Some("deploy:prod|release".into()),
                },
            ],
            ..CiSettings::default()
        };
        let exits = s.compile_exits().unwrap();
        assert_eq!(exits[0].id, "exit:registry");
        assert!(exits[0].matches(&job("build", "anything")));
        assert!(
            !exits[0].matches(&job("prebuild", "x")),
            "whole value, not a substring"
        );
        assert!(exits[1].matches(&job("deploy", "release")));
        assert!(!exits[1].matches(&job("deploy", "deploy:stage")));
    }

    #[test]
    fn a_rule_without_patterns_never_matches() {
        let s = CiSettings {
            exits: vec![ExitRule {
                label: "x".into(),
                stage: None,
                name: None,
            }],
            ..CiSettings::default()
        };
        assert!(!s.compile_exits().unwrap()[0].matches(&job("build", "build")));
    }

    #[test]
    fn defaults_are_valid_and_odd_numbers_are_not() {
        assert!(CiSettings::default().validate().is_ok());
        let bad = |edit: fn(&mut CiSettings)| {
            let mut s = CiSettings::default();
            edit(&mut s);
            s.validate().is_err()
        };
        assert!(bad(|s| s.max_projects = 0));
        assert!(bad(|s| s.default_capacity = 0));
        assert!(bad(|s| s.default_capacity = 1001));
        assert!(bad(|s| {
            s.capacities.insert("host-1".into(), 0);
        }));
        assert!(bad(|s| s.finished_linger_ms = -1));
        assert!(bad(|s| s.missing_grace_ms = -1));
        assert!(bad(|s| s.project_idle_ms = -1));
        assert!(bad(|s| s.project_quiet_ms = -1));
        let mut ok = CiSettings {
            default_capacity: 1000,
            max_projects: 1,
            finished_linger_ms: 0,
            ..CiSettings::default()
        };
        ok.capacities.insert("host-1".into(), 1);
        assert!(ok.validate().is_ok(), "the edges are allowed");
    }

    #[test]
    fn a_broken_pattern_is_an_error() {
        let s = CiSettings {
            exits: vec![ExitRule {
                label: "x".into(),
                stage: Some("(".into()),
                name: None,
            }],
            ..CiSettings::default()
        };
        assert!(s.compile_exits().is_err());
    }
}
