//! Slot numbers for running jobs. gitlab-runner's `concurrent-N` in container names counts
//! builds of one project on one registration, not slots of the process, so the agent keeps
//! its own: a job keeps its slot while it runs, a new job takes the lowest free one.

use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Default)]
pub struct Slots {
    by_job: BTreeMap<u64, u32>,
}

impl Slots {
    /// The slot of every job in `running`; jobs no longer running free theirs.
    pub fn assign(&mut self, running: &BTreeSet<u64>) -> BTreeMap<u64, u32> {
        self.by_job.retain(|job, _| running.contains(job));
        for job in running {
            if !self.by_job.contains_key(job) {
                let taken: BTreeSet<u32> = self.by_job.values().copied().collect();
                let free = (0..)
                    .find(|n| !taken.contains(n))
                    .expect("a free slot exists");
                self.by_job.insert(*job, free);
            }
        }
        self.by_job.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(jobs: &[u64]) -> BTreeSet<u64> {
        jobs.iter().copied().collect()
    }

    #[test]
    fn jobs_keep_their_slots_and_newcomers_fill_gaps() {
        let mut s = Slots::default();
        assert_eq!(
            s.assign(&set(&[10, 11, 12])),
            BTreeMap::from([(10, 0), (11, 1), (12, 2)])
        );
        assert_eq!(
            s.assign(&set(&[10, 12])),
            BTreeMap::from([(10, 0), (12, 2)])
        );
        assert_eq!(
            s.assign(&set(&[10, 12, 13, 14])),
            BTreeMap::from([(10, 0), (12, 2), (13, 1), (14, 3)])
        );
        assert!(s.assign(&set(&[])).is_empty());
        assert_eq!(s.assign(&set(&[99])), BTreeMap::from([(99, 0)]));
    }
}
