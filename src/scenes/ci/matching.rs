//! Could this runner ever pick this job up?

use super::input::{JobInfo, RunnerInfo, RunnerScope};

pub fn runner_can_take(job: &JobInfo, runner: &RunnerInfo) -> bool {
    let tags_ok = if job.tags.is_empty() {
        runner.run_untagged
    } else {
        job.tags.iter().all(|t| runner.tags.contains(t))
    };
    let scope_ok = match &runner.scope {
        RunnerScope::Instance => true,
        RunnerScope::Group { full_path } => job.project.starts_with(&format!("{full_path}/")),
        RunnerScope::Project { full_paths } => full_paths.iter().any(|p| p == &job.project),
    };
    tags_ok && scope_ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenes::ci::input::{JobStatus, RunnerScope};

    fn job(project: &str, tags: &[&str]) -> JobInfo {
        JobInfo {
            id: 1,
            name: "test".into(),
            stage: "test".into(),
            status: JobStatus::Pending,
            tags: tags.iter().map(|t| t.to_string()).collect(),
            project: project.into(),
            pipeline: 1,
            runner_id: None,
        }
    }
    fn runner(tags: &[&str], untagged: bool, scope: RunnerScope) -> RunnerInfo {
        RunnerInfo {
            id: 1,
            description: "r".into(),
            online: true,
            tags: tags.iter().map(|t| t.to_string()).collect(),
            run_untagged: untagged,
            scope,
        }
    }

    #[test]
    fn job_tags_must_all_be_on_the_runner() {
        let r = runner(&["ops", "heavy"], false, RunnerScope::Instance);
        assert!(runner_can_take(&job("a/b", &["ops"]), &r));
        assert!(runner_can_take(&job("a/b", &["ops", "heavy"]), &r));
        assert!(!runner_can_take(&job("a/b", &["ops", "gpu"]), &r));
    }

    #[test]
    fn untagged_jobs_need_run_untagged() {
        assert!(runner_can_take(
            &job("a/b", &[]),
            &runner(&["ops"], true, RunnerScope::Instance)
        ));
        assert!(!runner_can_take(
            &job("a/b", &[]),
            &runner(&["ops"], false, RunnerScope::Instance)
        ));
    }

    #[test]
    fn group_runner_takes_its_group_and_subgroups_only() {
        let r = runner(
            &[],
            true,
            RunnerScope::Group {
                full_path: "studio".into(),
            },
        );
        assert!(runner_can_take(&job("studio/app", &[]), &r));
        assert!(runner_can_take(&job("studio/web/app", &[]), &r));
        assert!(
            !runner_can_take(&job("studio-old/app", &[]), &r),
            "prefix is not a group"
        );
        assert!(!runner_can_take(&job("other/app", &[]), &r));
    }

    #[test]
    fn project_runner_takes_listed_projects_only() {
        let r = runner(
            &[],
            true,
            RunnerScope::Project {
                full_paths: vec!["a/x".into()],
            },
        );
        assert!(runner_can_take(&job("a/x", &[]), &r));
        assert!(!runner_can_take(&job("a/y", &[]), &r));
    }
}
