//! Job containers → the process report the CI scene reads.

use super::docker::JobContainer;
use super::metrics::RunnerMetrics;
use super::slots::Slots;
use crate::scenes::ci::input::{ProcessReport, SlotReport};
use std::collections::{BTreeMap, BTreeSet};

/// One slot per job: memory summed over its containers; a limit only if each has one.
pub fn build(
    key: &str,
    containers: &[JobContainer],
    slots: &mut Slots,
    metrics: &RunnerMetrics,
) -> ProcessReport {
    let mut jobs: BTreeMap<u64, Vec<&JobContainer>> = BTreeMap::new();
    for c in containers {
        jobs.entry(c.job_id).or_default().push(c);
    }
    let running: BTreeSet<u64> = jobs.keys().copied().collect();
    let numbers = slots.assign(&running);
    let mut out: Vec<SlotReport> = jobs
        .iter()
        .map(|(job, parts)| SlotReport {
            slot: numbers[job],
            job_id: *job,
            runner_id: 0,
            runner_token: parts
                .iter()
                .find(|c| !c.runner_token.is_empty())
                .map(|c| c.runner_token.clone())
                .unwrap_or_default(),
            mem_bytes: parts.iter().map(|c| c.mem_bytes).sum(),
            mem_limit_bytes: parts.iter().map(|c| c.mem_limit_bytes).sum::<Option<u64>>(),
            has_service: parts.iter().any(|c| c.service),
        })
        .collect();
    out.sort_by_key(|s| s.slot);
    ProcessReport {
        key: key.to_string(),
        concurrent: metrics.concurrent,
        token_limits: metrics.limits.clone(),
        slots: out,
        ..ProcessReport::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::docker::fake::FakeDocker;

    #[test]
    fn containers_of_a_job_add_up_in_one_slot() {
        let mut build_ = FakeDocker::container("b", 7, 300, Some(1024));
        build_.runner_token = String::new();
        let mut service = FakeDocker::container("s", 7, 100, Some(512));
        service.service = true;
        let other = FakeDocker::container("o", 9, 50, None);
        let metrics = RunnerMetrics {
            concurrent: Some(4),
            limits: BTreeMap::from([("abcd1234".into(), 2)]),
        };
        let mut slots = Slots::default();
        let p = build(
            "gitlab-runner",
            &[build_, service, other],
            &mut slots,
            &metrics,
        );
        assert_eq!((p.key.as_str(), p.concurrent), ("gitlab-runner", Some(4)));
        assert_eq!(p.token_limits.get("abcd1234"), Some(&2));
        assert_eq!(p.slots.len(), 2);
        let seven = &p.slots[0];
        assert_eq!(
            (
                seven.slot,
                seven.job_id,
                seven.mem_bytes,
                seven.mem_limit_bytes
            ),
            (0, 7, 400 << 20, Some(1536 << 20))
        );
        assert!(seven.has_service);
        assert_eq!(
            seven.runner_token, "abcd1234",
            "taken from the container that has it"
        );
        assert_eq!(seven.runner_id, 0, "the scene fills it in");
        let nine = &p.slots[1];
        assert_eq!((nine.slot, nine.mem_limit_bytes), (1, None));
    }
}
