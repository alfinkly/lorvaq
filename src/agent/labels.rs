//! The labels gitlab-runner puts on the containers of a job: the only ones the agent reads.

use std::collections::HashMap;

pub const JOB_ID: &str = "com.gitlab.gitlab-runner.job.id";
pub const TYPE: &str = "com.gitlab.gitlab-runner.type";
pub const RUNNER_TOKEN: &str = "com.gitlab.gitlab-runner.runner.id";

/// What a container's labels say about the job it belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobLabels {
    pub job_id: u64,
    /// The short token of the runner registration; empty when the label is missing.
    pub runner_token: String,
    /// A service container (`services:` of the job), not the build itself.
    pub service: bool,
}

/// None for containers that are not part of a job.
pub fn parse(labels: &HashMap<String, String>) -> Option<JobLabels> {
    let job_id = labels
        .get(JOB_ID)?
        .trim()
        .parse()
        .ok()
        .filter(|id| *id > 0)?;
    Some(JobLabels {
        job_id,
        runner_token: labels
            .get(RUNNER_TOKEN)
            .map(|t| t.trim().to_string())
            .unwrap_or_default(),
        service: labels.get(TYPE).is_some_and(|t| t == "service"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn build_service_and_foreign_containers() {
        let build = parse(&labels(&[
            (JOB_ID, "42"),
            (TYPE, "build"),
            (RUNNER_TOKEN, "abcd1234"),
        ]))
        .unwrap();
        assert_eq!(
            build,
            JobLabels {
                job_id: 42,
                runner_token: "abcd1234".into(),
                service: false
            }
        );
        assert!(
            parse(&labels(&[(JOB_ID, "42"), (TYPE, "service")]))
                .unwrap()
                .service
        );
        let helper = parse(&labels(&[(JOB_ID, "42"), (TYPE, "predefined")])).unwrap();
        assert!(!helper.service, "helpers count as the build");
        assert_eq!(helper.runner_token, "");
        assert!(parse(&labels(&[(TYPE, "build")])).is_none(), "no job id");
        assert!(parse(&labels(&[(JOB_ID, "x")])).is_none());
        assert!(parse(&labels(&[(JOB_ID, "0")])).is_none());
        assert!(parse(&HashMap::new()).is_none());
    }
}
