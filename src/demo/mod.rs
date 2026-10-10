//! Built-in simulator: a fake GitLab with runners and agents, so the CI scene can
//! be shown and developed without any real infrastructure. Names are generic on purpose.

use crate::scenes::ci::input::*;
use crate::scenes::ci::matching::runner_can_take;
use crate::scenes::ci::settings::{CiSettings, ExitRule};
use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};
use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;
use tokio::sync::mpsc;

const GIB: u64 = 1 << 30;
const HOST: &str = "runner-host";
const PROJECTS: [(&str, u32); 5] = [
    ("demo/backend", 30),
    ("demo/frontend", 25),
    ("demo/mobile", 20),
    ("demo/legacy", 10),
    ("demo/docs", 15),
];

struct Proc {
    agent: &'static str,
    key: &'static str,
    concurrent: u32,
    runners: &'static [u64],
    limits: &'static [(u64, u32)],
}

const PROCS: [Proc; 2] = [
    Proc {
        agent: "agent-shared",
        key: "shared",
        concurrent: 5,
        runners: &[1, 2],
        limits: &[(2, 2)],
    },
    Proc {
        agent: "agent-deploy",
        key: "deploy",
        concurrent: 1,
        runners: &[3],
        limits: &[],
    },
];

pub fn settings() -> CiSettings {
    CiSettings {
        exits: vec![
            ExitRule {
                label: "registry".into(),
                stage: Some("build".into()),
                name: None,
            },
            ExitRule {
                label: "prod".into(),
                stage: Some("deploy".into()),
                name: None,
            },
        ],
        ..CiSettings::default()
    }
}

pub fn runners() -> Vec<RunnerInfo> {
    let r = |id: u64, description: &str, tag: &str, untagged: bool| RunnerInfo {
        id,
        description: description.into(),
        online: true,
        tags: vec![tag.into()],
        run_untagged: untagged,
        scope: RunnerScope::Instance,
    };
    vec![
        r(1, "shared", "ops", true),
        r(2, "heavy", "heavy", false),
        r(3, "deploy", "deploy", false),
    ]
}

struct SimJob {
    info: JobInfo,
    slot: Option<(usize, u32)>,
    started: i64,
    ends: i64,
    peak: u64,
    limit: u64,
    oom: bool,
    cancel_at: Option<i64>,
}

pub struct Simulator {
    rng: SmallRng,
    next_id: u64,
    jobs: BTreeMap<u64, SimJob>,
    finished: VecDeque<JobInfo>,
    exits: Vec<ContainerExit>,
    disk: f64,
    steps: u64,
    runners: Vec<RunnerInfo>,
}

impl Simulator {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: SmallRng::seed_from_u64(seed),
            next_id: 1,
            jobs: BTreeMap::new(),
            finished: VecDeque::new(),
            exits: Vec::new(),
            disk: 0.55,
            steps: 0,
            runners: runners(),
        }
    }

    /// One second of the fake world. Order matters: agent reports and container
    /// exits come before the GitLab poll, as they would in real life.
    pub fn step(&mut self, now: i64) -> Vec<CiInput> {
        self.spawn(now);
        self.schedule(now);
        self.progress(now);
        let mut out = Vec::new();
        if self.steps.is_multiple_of(30) {
            out.push(CiInput::Runners(self.runners.clone()));
        }
        for (i, p) in PROCS.iter().enumerate() {
            out.push(CiInput::Agent(self.report(i, p, now)));
        }
        out.extend(self.exits.drain(..).map(CiInput::AgentExit));
        out.push(CiInput::Jobs(JobsUpdate {
            active: self.jobs.values().map(|j| j.info.clone()).collect(),
            recent_finished: self.finished.iter().cloned().collect(),
        }));
        self.steps += 1;
        out
    }

    fn spawn(&mut self, now: i64) {
        if !self.rng.random_bool(0.55) {
            return;
        }
        let total: u32 = PROJECTS.iter().map(|(_, w)| w).sum();
        let mut roll = self.rng.random_range(0..total);
        let mut project = PROJECTS[0].0;
        for (name, weight) in PROJECTS {
            if roll < weight {
                project = name;
                break;
            }
            roll -= weight;
        }
        let (name, stage, tags): (&str, &str, Vec<&str>) = match project {
            "demo/mobile" => ("e2e", "test", vec!["heavy"]),
            "demo/legacy" if self.rng.random_bool(0.4) => ("build-image", "build", vec!["docker"]),
            "demo/legacy" | "demo/docs" => ("build", "build", vec![]),
            _ => match self.rng.random_range(0..10) {
                0 => ("deploy", "deploy", vec!["deploy"]),
                1..=3 => ("lint", "lint", vec!["ops"]),
                4..=6 => ("test", "test", vec!["ops"]),
                _ => ("build", "build", vec!["ops"]),
            },
        };
        let id = self.next_id;
        self.next_id += 1;
        let cancel_at = (tags == ["docker"]).then_some(now + 40_000);
        let info = JobInfo {
            id,
            name: name.into(),
            stage: stage.into(),
            status: JobStatus::Pending,
            tags: tags.into_iter().map(String::from).collect(),
            project: project.into(),
            pipeline: id / 3 + 1,
            runner_id: None,
        };
        self.jobs.insert(
            id,
            SimJob {
                info,
                slot: None,
                started: 0,
                ends: 0,
                peak: 0,
                limit: 0,
                oom: false,
                cancel_at,
            },
        );
    }

    fn free_slot(&self, pi: usize) -> Option<u32> {
        (0..PROCS[pi].concurrent).find(|n| !self.jobs.values().any(|j| j.slot == Some((pi, *n))))
    }

    fn pick(&self, job: &JobInfo) -> Option<(usize, u64)> {
        for r in &self.runners {
            if !runner_can_take(job, r) {
                continue;
            }
            let Some(pi) = PROCS.iter().position(|p| p.runners.contains(&r.id)) else {
                continue;
            };
            if self.free_slot(pi).is_none() {
                continue;
            }
            if let Some((_, limit)) = PROCS[pi].limits.iter().find(|(rid, _)| *rid == r.id) {
                let busy = self
                    .jobs
                    .values()
                    .filter(|j| {
                        j.info.status == JobStatus::Running && j.info.runner_id == Some(r.id)
                    })
                    .count() as u32;
                if busy >= *limit {
                    continue;
                }
            }
            return Some((pi, r.id));
        }
        None
    }

    fn schedule(&mut self, now: i64) {
        let pending: Vec<u64> = self
            .jobs
            .values()
            .filter(|j| j.info.status == JobStatus::Pending)
            .map(|j| j.info.id)
            .collect();
        for id in pending {
            let info = self.jobs[&id].info.clone();
            let Some((pi, runner)) = self.pick(&info) else {
                continue;
            };
            let slot = self
                .free_slot(pi)
                .expect("pick() checked that a slot is free");
            let heavy = info.tags.iter().any(|t| t == "heavy");
            let deploy = info.stage == "deploy";
            let duration: i64 = if deploy {
                self.rng.random_range(3_000..6_000)
            } else if heavy {
                self.rng.random_range(15_000..30_000)
            } else {
                self.rng.random_range(5_000..15_000)
            };
            let limit = if heavy { 2 * GIB } else { 3 * GIB };
            let oom = !deploy && self.rng.random_bool(0.07);
            let peak = if oom {
                limit
            } else {
                (limit as f64 * self.rng.random_range(0.35..0.95)) as u64
            };
            let j = self.jobs.get_mut(&id).expect("id came from the map");
            j.info.status = JobStatus::Running;
            j.info.runner_id = Some(runner);
            j.slot = Some((pi, slot));
            j.started = now;
            j.ends = now + duration;
            j.limit = limit;
            j.peak = peak;
            j.oom = oom;
        }
    }

    fn progress(&mut self, now: i64) {
        let ids: Vec<u64> = self.jobs.keys().copied().collect();
        for id in ids {
            let j = &self.jobs[&id];
            let status = match j.info.status {
                JobStatus::Running if j.oom && now >= j.started + (j.ends - j.started) * 7 / 10 => {
                    let agent = PROCS[j.slot.expect("running jobs have a slot").0].agent;
                    self.exits.push(ContainerExit {
                        agent: agent.into(),
                        job_id: id,
                        oom: true,
                        exit_code: 137,
                    });
                    Some(JobStatus::Failed)
                }
                JobStatus::Running if now >= j.ends => Some(if self.rng.random_bool(0.05) {
                    JobStatus::Failed
                } else {
                    JobStatus::Success
                }),
                JobStatus::Pending if j.cancel_at.is_some_and(|t| now >= t) => {
                    Some(JobStatus::Canceled)
                }
                _ => None,
            };
            if let Some(status) = status {
                let mut j = self.jobs.remove(&id).expect("id came from the map");
                j.info.status = status;
                self.finished.push_front(j.info);
                self.finished.truncate(30);
                self.disk += 0.004;
                if self.disk > 0.85 {
                    self.disk = 0.45;
                }
            }
        }
    }

    fn report(&self, pi: usize, p: &Proc, now: i64) -> AgentReport {
        let slots = self
            .jobs
            .values()
            .filter_map(|j| {
                let (jp, n) = j.slot?;
                if jp != pi {
                    return None;
                }
                let progress =
                    ((now - j.started) as f64 / (j.ends - j.started).max(1) as f64).clamp(0.0, 1.0);
                let mem = (j.peak as f64 * (0.3 + 1.4 * progress).min(1.0)) as u64;
                Some(SlotReport {
                    slot: n,
                    job_id: j.info.id,
                    runner_id: j.info.runner_id.unwrap_or(0),
                    mem_bytes: mem,
                    mem_limit_bytes: Some(j.limit),
                    has_service: j.info.stage == "build"
                        || j.info.tags.iter().any(|t| t == "heavy"),
                })
            })
            .collect();
        AgentReport {
            agent: p.agent.into(),
            host_label: HOST.into(),
            processes: vec![ProcessReport {
                key: p.key.into(),
                concurrent: Some(p.concurrent),
                runner_ids: p.runners.to_vec(),
                limits: p.limits.iter().copied().collect(),
                slots,
            }],
            disk: Some(DiskUsage {
                used_bytes: (self.disk * 80.0 * GIB as f64) as u64,
                total_bytes: 80 * GIB,
            }),
        }
    }
}

/// Feeds the scene one simulated second per real second until the scene stops.
pub async fn run(tx: mpsc::Sender<CiInput>, seed: u64) {
    let mut sim = Simulator::new(seed);
    let mut every = tokio::time::interval(Duration::from_secs(1));
    loop {
        every.tick().await;
        for input in sim.step(crate::scene::now_ms()) {
            if tx.send(input).await.is_err() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenes::ci::testkit::Harness;
    use crate::world::Change;

    #[test]
    fn fifteen_minutes_of_demo_keep_the_world_consistent() {
        let mut sim = Simulator::new(42);
        let mut h = Harness::new(settings());
        let (mut stuck, mut oom, mut exits) = (false, false, false);
        for _ in 0..900 {
            h.now += 1_000;
            for input in sim.step(h.now) {
                for change in h.feed(input) {
                    match &change {
                        Change::Flash { kind, .. } if kind == "oom" => oom = true,
                        Change::Move { to, .. } if to.starts_with("exit:") => exits = true,
                        _ => {}
                    }
                }
            }
            h.tick(0);
            let mut per_slot: BTreeMap<String, u32> = BTreeMap::new();
            for e in h.world.entities() {
                assert!(
                    h.world.place(&e.place).is_some(),
                    "{} sits in a missing place {}",
                    e.id,
                    e.place
                );
                stuck |= e.state == "stuck";
                if e.place.starts_with("slot:") && e.state == "running" {
                    *per_slot.entry(e.place.clone()).or_default() += 1;
                }
            }
            assert!(
                per_slot.values().all(|n| *n <= 1),
                "two running jobs in one slot: {per_slot:?}"
            );
        }
        assert!(
            stuck && oom && exits,
            "stuck={stuck} oom={oom} exits={exits}"
        );
    }

    #[test]
    fn same_seed_same_story() {
        let (mut a, mut b) = (Simulator::new(7), Simulator::new(7));
        for t in 0..60 {
            let now = 1_000 * t;
            assert_eq!(a.step(now), b.step(now));
        }
    }

    #[test]
    fn demo_names_are_generic() {
        let text = serde_json::to_string(&Simulator::new(1).step(0)).unwrap();
        assert!(text.contains("runner-host") && text.contains("agent-shared"));
    }
}
