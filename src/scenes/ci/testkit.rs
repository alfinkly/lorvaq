//! Test harness for the CI scene: feeds inputs, applies changes to a world.

use super::input::*;
use super::reduce::{job_id, CiScene};
use super::settings::CiSettings;
use crate::scene::Scene;
use crate::world::{Change, World};
use std::collections::BTreeMap;

const GIB: f64 = (1u64 << 30) as f64;

pub struct Harness {
    pub scene: CiScene,
    pub world: World,
    pub now: i64,
}

impl Harness {
    pub fn new(settings: CiSettings) -> Self {
        Self {
            scene: CiScene::new(settings).unwrap(),
            world: World::default(),
            now: 1_000_000,
        }
    }

    /// Reduces the input and returns the changes that were actually applied.
    pub fn feed(&mut self, input: CiInput) -> Vec<Change> {
        let changes = self.scene.reduce(&self.world, input, self.now);
        self.apply(changes)
    }

    pub fn tick(&mut self, advance_ms: i64) -> Vec<Change> {
        self.now += advance_ms;
        let changes = self.scene.tick(&self.world, self.now);
        self.apply(changes)
    }

    fn apply(&mut self, changes: Vec<Change>) -> Vec<Change> {
        changes
            .into_iter()
            .filter(|c| self.world.apply(c, self.now))
            .collect()
    }

    pub fn place_of(&self, job: u64) -> Option<String> {
        self.world.entity(&job_id(job)).map(|e| e.place.clone())
    }

    pub fn state_of(&self, job: u64) -> Option<String> {
        self.world.entity(&job_id(job)).map(|e| e.state.clone())
    }
}

pub fn job(id: u64, project: &str, status: JobStatus, tags: &[&str]) -> JobInfo {
    JobInfo {
        id,
        name: format!("job{id}"),
        stage: "test".into(),
        status,
        tags: tags.iter().map(|t| t.to_string()).collect(),
        project: project.into(),
        pipeline: 1,
        runner_id: None,
    }
}

pub fn running(id: u64, project: &str, runner: u64) -> JobInfo {
    JobInfo {
        runner_id: Some(runner),
        ..job(id, project, JobStatus::Running, &[])
    }
}

pub fn runner(id: u64, tags: &[&str], untagged: bool) -> RunnerInfo {
    RunnerInfo {
        id,
        description: format!("runner{id}"),
        online: true,
        tags: tags.iter().map(|t| t.to_string()).collect(),
        run_untagged: untagged,
        scope: RunnerScope::Instance,
    }
}

pub fn jobs(active: Vec<JobInfo>, recent_finished: Vec<JobInfo>) -> CiInput {
    CiInput::Jobs(JobsUpdate {
        active,
        recent_finished,
    })
}

pub fn agent(name: &str, host: &str, processes: Vec<ProcessReport>) -> CiInput {
    CiInput::Agent(AgentReport {
        agent: name.into(),
        host_label: host.into(),
        processes,
        disk: None,
    })
}

pub fn process(key: &str, concurrent: Option<u32>, slots: Vec<SlotReport>) -> ProcessReport {
    ProcessReport {
        key: key.into(),
        concurrent,
        runner_ids: vec![],
        limits: BTreeMap::new(),
        slots,
    }
}

pub fn slot(n: u32, job: u64, runner: u64, mem_gib: f64, limit_gib: f64) -> SlotReport {
    SlotReport {
        slot: n,
        job_id: job,
        runner_id: runner,
        mem_bytes: (mem_gib * GIB) as u64,
        mem_limit_bytes: Some((limit_gib * GIB) as u64),
        has_service: false,
    }
}
