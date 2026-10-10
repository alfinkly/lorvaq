//! The CI scene reducer: turns GitLab polls and agent reports into world changes.

use super::input::{
    AgentReport, CiInput, ContainerExit, JobInfo, JobStatus, JobsUpdate, ProcessReport, RunnerInfo,
    SlotReport,
};
use super::matching::runner_can_take;
use super::settings::{CiSettings, Exit};
use crate::scene::Scene;
use crate::world::{Attrs, Change, Entity, Level, Place, World};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

pub const PROJECTS: &str = "projects";
pub const QUEUE: &str = "queue";
pub const STUCK: &str = "stuck";
pub const HOSTS: &str = "hosts";
pub const NOHOST: &str = "nohost";
pub const EXITS: &str = "exits";
pub const SINK: &str = "sink";
pub const OTHER_PROJECT: &str = "project:~other";

pub fn job_id(id: u64) -> String {
    format!("job:{id}")
}
pub fn project_id(path: &str) -> String {
    format!("project:{path}")
}
pub fn host_id(label: &str) -> String {
    format!("host:{label}")
}
pub fn process_id(agent: &str, key: &str) -> String {
    format!("process:{agent}/{key}")
}
pub fn slot_id(agent: &str, key: &str, n: u32) -> String {
    format!("slot:{agent}/{key}/{n}")
}
pub fn runner_place_id(id: u64) -> String {
    format!("runner:{id}")
}

fn place(
    id: impl Into<String>,
    kind: &str,
    parent: Option<&str>,
    label: impl Into<String>,
    capacity: Option<u32>,
    attrs: Attrs,
) -> Place {
    Place {
        id: id.into(),
        kind: kind.into(),
        parent: parent.map(str::to_string),
        label: label.into(),
        capacity,
        attrs,
    }
}

fn attrs(value: serde_json::Value) -> Attrs {
    serde_json::from_value(value).expect("callers pass JSON objects")
}

fn job_attrs(job: &JobInfo) -> Attrs {
    attrs(
        json!({ "name": job.name, "stage": job.stage, "project": job.project, "pipeline": job.pipeline, "tags": job.tags }),
    )
}

/// The agent of a slot id `slot:<agent>/<key>/<n>`.
fn slot_agent(slot: &str) -> Option<&str> {
    slot.strip_prefix("slot:")?.split('/').next()
}

pub struct CiScene {
    settings: CiSettings,
    exits: Vec<Exit>,
    runners: Vec<RunnerInfo>,
    runners_known: bool,
    active: BTreeMap<u64, JobInfo>,
    /// Projects with their own place → last time they had an active job.
    projects: BTreeMap<String, i64>,
    /// Latest report per agent; offline agents keep theirs so processes stay known.
    agents: BTreeMap<String, AgentReport>,
    offline: BTreeSet<String>,
    oom: BTreeSet<u64>,
    finished: BTreeMap<u64, i64>,
    missing: BTreeMap<u64, i64>,
}

type SlotMap<'a> = BTreeMap<u64, (String, &'a SlotReport)>;
/// A job, where it goes, its state there, and its (used, limit) memory from the agent.
type PlannedJob = (JobInfo, String, &'static str, Option<(u64, Option<u64>)>);

impl CiScene {
    pub fn new(settings: CiSettings) -> anyhow::Result<Self> {
        let exits = settings.compile_exits()?;
        Ok(Self {
            settings,
            exits,
            runners: Vec::new(),
            runners_known: false,
            active: BTreeMap::new(),
            projects: BTreeMap::new(),
            agents: BTreeMap::new(),
            offline: BTreeSet::new(),
            oom: BTreeSet::new(),
            finished: BTreeMap::new(),
            missing: BTreeMap::new(),
        })
    }

    fn ensure_static(&self, world: &World) -> Vec<Change> {
        let mut zones = vec![
            place(PROJECTS, "zone", None, "projects", None, Attrs::new()),
            place(QUEUE, "queue", None, "queue", None, Attrs::new()),
            place(STUCK, "stuck", None, "no runner", None, Attrs::new()),
            place(HOSTS, "zone", None, "runners", None, Attrs::new()),
            place(
                NOHOST,
                "zone",
                None,
                "runners without agent",
                None,
                Attrs::new(),
            ),
            place(EXITS, "zone", None, "exits", None, Attrs::new()),
            place(SINK, "sink", None, "", None, Attrs::new()),
        ];
        for exit in &self.exits {
            zones.push(place(
                exit.id.clone(),
                "exit",
                Some(EXITS),
                exit.label.clone(),
                None,
                Attrs::new(),
            ));
        }
        zones
            .into_iter()
            .filter(|p| world.place(&p.id).is_none())
            .map(|place| Change::UpsertPlace { place })
            .collect()
    }

    /// The place a project's jobs are born in; folds overflow into "other".
    fn project_place(
        &mut self,
        world: &World,
        path: &str,
        now: i64,
        out: &mut Vec<Change>,
    ) -> String {
        if !self.projects.contains_key(path) && self.projects.len() >= self.settings.max_projects {
            // Full: the quietest empty project gives its place to this newcomer.
            let quiet = self.settings.project_quiet_ms;
            let victim = self
                .projects
                .iter()
                .filter(|(p, last)| {
                    now - **last >= quiet && !world.entities().any(|e| e.place == project_id(p))
                })
                .min_by_key(|(_, last)| **last)
                .map(|(p, _)| p.clone());
            if let Some(victim) = victim {
                self.projects.remove(&victim);
                out.push(Change::RemovePlace {
                    id: project_id(&victim),
                });
            }
        }
        if self.projects.contains_key(path) || self.projects.len() < self.settings.max_projects {
            self.projects.insert(path.to_string(), now);
            let id = project_id(path);
            if world.place(&id).is_none() {
                let label = path.rsplit('/').next().unwrap_or(path);
                out.push(Change::UpsertPlace {
                    place: place(
                        id.clone(),
                        "project",
                        Some(PROJECTS),
                        label,
                        None,
                        attrs(json!({ "path": path })),
                    ),
                });
            }
            id
        } else {
            if world.place(OTHER_PROJECT).is_none() {
                out.push(Change::UpsertPlace {
                    place: place(
                        OTHER_PROJECT,
                        "project",
                        Some(PROJECTS),
                        "other",
                        None,
                        attrs(json!({ "path": "~other" })),
                    ),
                });
            }
            OTHER_PROJECT.to_string()
        }
    }

    fn capacity(&self, p: &ProcessReport) -> u32 {
        let declared = p
            .concurrent
            .or_else(|| self.settings.capacities.get(&p.key).copied())
            .unwrap_or(self.settings.default_capacity);
        let seen = p.slots.iter().map(|s| s.slot + 1).max().unwrap_or(0);
        declared.max(seen)
    }

    fn process_of_runner(&self, runner: u64) -> Option<(&str, &ProcessReport)> {
        self.agents.iter().find_map(|(agent, report)| {
            report
                .processes
                .iter()
                .find(|p| {
                    p.runner_ids.contains(&runner) || p.slots.iter().any(|s| s.runner_id == runner)
                })
                .map(|p| (agent.as_str(), p))
        })
    }

    fn slot_map(&self) -> SlotMap<'_> {
        let mut map = BTreeMap::new();
        for (agent, report) in self
            .agents
            .iter()
            .filter(|(agent, _)| !self.offline.contains(*agent))
        {
            for p in &report.processes {
                for s in &p.slots {
                    map.insert(s.job_id, (slot_id(agent, &p.key, s.slot), s));
                }
            }
        }
        map
    }

    fn slot_online(&self, slot: &str) -> bool {
        slot_agent(slot)
            .is_some_and(|agent| self.agents.contains_key(agent) && !self.offline.contains(agent))
    }

    /// Every candidate runner is at its `limit` while its process still has a free slot.
    fn limited(&self, runner: &RunnerInfo, running: &BTreeMap<u64, u32>) -> bool {
        let Some((_, p)) = self.process_of_runner(runner.id) else {
            return false;
        };
        let Some(limit) = p.limits.get(&runner.id) else {
            return false;
        };
        let busy = running.get(&runner.id).copied().unwrap_or(0);
        busy >= *limit && (p.slots.len() as u32) < self.capacity(p)
    }

    fn target(
        &self,
        world: &World,
        job: &JobInfo,
        slots: &SlotMap<'_>,
        running: &BTreeMap<u64, u32>,
    ) -> (String, &'static str) {
        match job.status {
            JobStatus::Pending => {
                if !self.runners_known {
                    return (QUEUE.into(), "pending");
                }
                let candidates: Vec<&RunnerInfo> = self
                    .runners
                    .iter()
                    .filter(|r| r.online && runner_can_take(job, r))
                    .collect();
                if candidates.is_empty() {
                    (STUCK.into(), "stuck")
                } else if candidates.iter().all(|r| self.limited(r, running)) {
                    (QUEUE.into(), "waiting_limit")
                } else {
                    (QUEUE.into(), "pending")
                }
            }
            JobStatus::Running => {
                if let Some((slot, _)) = slots.get(&job.id) {
                    return (slot.clone(), "running");
                }
                if let Some(e) = world.entity(&job_id(job.id)) {
                    let taken = slots
                        .values()
                        .any(|(s, r)| *s == e.place && r.job_id != job.id);
                    if e.place.starts_with("slot:") && self.slot_online(&e.place) && !taken {
                        return (e.place.clone(), "running");
                    }
                }
                match job.runner_id {
                    Some(r) => match self.process_of_runner(r) {
                        Some((agent, p)) => (process_id(agent, &p.key), "running"),
                        None => (runner_place_id(r), "running"),
                    },
                    None => (runner_place_id(0), "running"),
                }
            }
            JobStatus::Success | JobStatus::Failed | JobStatus::Canceled => {
                (SINK.into(), job.status.as_str())
            }
        }
    }

    /// Puts every active job where it belongs: appear, move, state, memory.
    fn place_jobs(&mut self, world: &World, now: i64, out: &mut Vec<Change>) {
        let mut running: BTreeMap<u64, u32> = BTreeMap::new();
        for j in self
            .active
            .values()
            .filter(|j| j.status == JobStatus::Running)
        {
            if let Some(r) = j.runner_id {
                *running.entry(r).or_default() += 1;
            }
        }
        let plan: Vec<PlannedJob> = {
            let slots = self.slot_map();
            self.active
                .values()
                .map(|job| {
                    let (to, state) = self.target(world, job, &slots, &running);
                    let mem = slots
                        .get(&job.id)
                        .map(|(_, s)| (s.mem_bytes, s.mem_limit_bytes));
                    (job.clone(), to, state, mem)
                })
                .collect()
        };
        for (job, to, state, mem) in plan {
            if to.starts_with("runner:") && world.place(&to).is_none() {
                let label = job
                    .runner_id
                    .and_then(|r| self.runners.iter().find(|x| x.id == r))
                    .map_or_else(|| "unknown runner".to_string(), |r| r.description.clone());
                out.push(Change::UpsertPlace {
                    place: place(
                        to.clone(),
                        "process",
                        Some(NOHOST),
                        label,
                        None,
                        Attrs::new(),
                    ),
                });
            }
            let eid = job_id(job.id);
            if let Some(last) = self.projects.get_mut(&job.project) {
                *last = now;
            }
            match world.entity(&eid) {
                None => {
                    let born_at = self.project_place(world, &job.project, now, out);
                    let entity = Entity {
                        id: eid.clone(),
                        kind: "job".into(),
                        place: born_at.clone(),
                        state: state.into(),
                        attrs: job_attrs(&job),
                        since_ms: now,
                    };
                    out.push(Change::Appear { entity });
                    if born_at != to {
                        out.push(Change::Move {
                            id: eid.clone(),
                            to,
                        });
                    }
                }
                Some(e) => {
                    if e.place != to {
                        out.push(Change::Move {
                            id: eid.clone(),
                            to,
                        });
                    }
                    if e.state != state && !self.oom.contains(&job.id) {
                        out.push(Change::SetState {
                            id: eid.clone(),
                            state: state.into(),
                        });
                    }
                }
            }
            if let Some((used, limit)) = mem {
                out.push(Change::SetLevel {
                    level: Level {
                        owner: eid,
                        metric: "mem".into(),
                        value: used as f64,
                        max: limit.map(|v| v as f64),
                    },
                });
            }
        }
    }

    fn finish(&mut self, job: &JobInfo, now: i64, out: &mut Vec<Change>) {
        let eid = job_id(job.id);
        let state = if self.oom.contains(&job.id) {
            "oom"
        } else {
            job.status.as_str()
        };
        out.push(Change::SetState {
            id: eid.clone(),
            state: state.into(),
        });
        let exit = if job.status == JobStatus::Success {
            self.exits
                .iter()
                .find(|x| x.matches(job))
                .map(|x| x.id.clone())
        } else {
            None
        };
        out.push(Change::Move {
            id: eid.clone(),
            to: exit.unwrap_or_else(|| SINK.into()),
        });
        out.push(Change::ClearLevel {
            owner: eid,
            metric: "mem".into(),
        });
        self.finished.insert(job.id, now);
        self.missing.remove(&job.id);
    }

    fn on_jobs(&mut self, world: &World, update: JobsUpdate, now: i64, out: &mut Vec<Change>) {
        self.active = update
            .active
            .into_iter()
            .filter(|j| matches!(j.status, JobStatus::Pending | JobStatus::Running))
            .map(|j| (j.id, j))
            .collect();
        let active: Vec<u64> = self.active.keys().copied().collect();
        for id in active {
            self.missing.remove(&id);
        }
        self.place_jobs(world, now, out);
        let shown: Vec<u64> = world
            .entities()
            .filter(|e| e.kind == "job")
            .filter_map(|e| e.id.strip_prefix("job:")?.parse().ok())
            .collect();
        for id in shown {
            if self.active.contains_key(&id) || self.finished.contains_key(&id) {
                continue;
            }
            match update.recent_finished.iter().find(|f| f.id == id) {
                Some(f) => self.finish(f, now, out),
                None => {
                    self.missing.entry(id).or_insert(now);
                }
            }
        }
    }

    fn on_agent(&mut self, world: &World, report: AgentReport, now: i64, out: &mut Vec<Change>) {
        let host = host_id(&report.host_label);
        out.push(Change::UpsertPlace {
            place: place(
                host.clone(),
                "host",
                Some(HOSTS),
                report.host_label.clone(),
                None,
                Attrs::new(),
            ),
        });
        for p in &report.processes {
            let cap = self.capacity(p);
            let pid = process_id(&report.agent, &p.key);
            let process_attrs = attrs(json!({ "agent": report.agent, "online": true }));
            out.push(Change::UpsertPlace {
                place: place(
                    pid.clone(),
                    "process",
                    Some(&host),
                    p.key.clone(),
                    Some(cap),
                    process_attrs,
                ),
            });
            for n in 0..cap {
                out.push(Change::UpsertPlace {
                    place: place(
                        slot_id(&report.agent, &p.key, n),
                        "slot",
                        Some(&pid),
                        format!("#{n}"),
                        None,
                        Attrs::new(),
                    ),
                });
            }
            for extra in world
                .places()
                .filter(|s| s.kind == "slot" && s.parent.as_deref() == Some(pid.as_str()))
            {
                let n: u32 = extra
                    .id
                    .rsplit('/')
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                if n >= cap {
                    out.push(Change::RemovePlace {
                        id: extra.id.clone(),
                    });
                }
            }
        }
        if let Some(d) = &report.disk {
            out.push(Change::SetLevel {
                level: Level {
                    owner: host,
                    metric: "disk".into(),
                    value: d.used_bytes as f64,
                    max: Some(d.total_bytes as f64),
                },
            });
        }
        self.offline.remove(&report.agent);
        self.agents.insert(report.agent.clone(), report);
        self.place_jobs(world, now, out);
    }

    fn on_agent_gone(&mut self, world: &World, agent: &str, now: i64, out: &mut Vec<Change>) {
        self.offline.insert(agent.to_string());
        let of_agent = |p: &&Place| {
            p.kind == "process" && p.attrs.get("agent").and_then(|v| v.as_str()) == Some(agent)
        };
        for p in world.places().filter(of_agent) {
            let mut grey = p.clone();
            grey.attrs.insert("online".into(), json!(false));
            out.push(Change::UpsertPlace { place: grey });
        }
        self.place_jobs(world, now, out);
    }

    fn on_exit(&mut self, world: &World, exit: ContainerExit, out: &mut Vec<Change>) {
        if !exit.oom {
            return;
        }
        self.oom.insert(exit.job_id);
        let eid = job_id(exit.job_id);
        if let Some(e) = world.entity(&eid) {
            out.push(Change::Flash {
                at: e.place.clone(),
                kind: "oom".into(),
            });
            out.push(Change::SetState {
                id: eid,
                state: "oom".into(),
            });
        }
    }
}

impl Scene for CiScene {
    type Input = CiInput;

    fn id(&self) -> &'static str {
        "ci"
    }

    fn title(&self) -> &'static str {
        "CI"
    }

    fn reduce(&mut self, world: &World, input: CiInput, now_ms: i64) -> Vec<Change> {
        let mut out = self.ensure_static(world);
        match input {
            CiInput::Jobs(update) => self.on_jobs(world, update, now_ms, &mut out),
            CiInput::Runners(runners) => {
                self.runners = runners;
                self.runners_known = true;
                self.place_jobs(world, now_ms, &mut out);
            }
            CiInput::Agent(report) => self.on_agent(world, report, now_ms, &mut out),
            CiInput::AgentExit(exit) => self.on_exit(world, exit, &mut out),
            CiInput::AgentGone { agent } => self.on_agent_gone(world, &agent, now_ms, &mut out),
            CiInput::SourceStale { source, since_ms } => {
                out.push(Change::Stale { source, since_ms })
            }
            CiInput::SourceFresh { source } => out.push(Change::Fresh { source }),
        }
        out
    }

    fn tick(&mut self, world: &World, now_ms: i64) -> Vec<Change> {
        let mut out = self.ensure_static(world);
        let linger = self.settings.finished_linger_ms;
        let done: Vec<u64> = self
            .finished
            .iter()
            .filter(|(_, at)| now_ms - **at >= linger)
            .map(|(id, _)| *id)
            .collect();
        for id in done {
            self.finished.remove(&id);
            self.oom.remove(&id);
            out.push(Change::Gone { id: job_id(id) });
        }
        let grace = self.settings.missing_grace_ms;
        let lost: Vec<u64> = self
            .missing
            .iter()
            .filter(|(_, at)| now_ms - **at >= grace)
            .map(|(id, _)| *id)
            .collect();
        for id in lost {
            self.missing.remove(&id);
            self.oom.remove(&id);
            out.push(Change::Gone { id: job_id(id) });
        }
        let idle = self.settings.project_idle_ms;
        let quiet: Vec<String> = self
            .projects
            .iter()
            .filter(|(_, last)| now_ms - **last >= idle)
            .map(|(p, _)| p.clone())
            .collect();
        for path in quiet {
            let id = project_id(&path);
            if world.entities().any(|e| e.place == id) {
                continue;
            }
            self.projects.remove(&path);
            out.push(Change::RemovePlace { id });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenes::ci::input::*;
    use crate::scenes::ci::settings::{CiSettings, ExitRule};
    use crate::scenes::ci::testkit::*;
    use crate::world::Change;

    fn with_exits() -> CiSettings {
        CiSettings {
            exits: vec![ExitRule {
                label: "registry".into(),
                stage: Some("build".into()),
                name: None,
            }],
            ..CiSettings::default()
        }
    }

    #[test]
    fn new_pending_job_appears_at_its_project_then_queues() {
        let mut h = Harness::new(CiSettings::default());
        let ch = h.feed(jobs(vec![job(1, "g/app", JobStatus::Pending, &[])], vec![]));
        let appear = ch.iter().find_map(|c| match c {
            Change::Appear { entity } => Some(entity.place.clone()),
            _ => None,
        });
        assert_eq!(appear.as_deref(), Some("project:g/app"));
        assert!(ch.contains(&Change::Move {
            id: "job:1".into(),
            to: QUEUE.into()
        }));
        assert_eq!(h.state_of(1).as_deref(), Some("pending"));
        assert_eq!(h.world.place("project:g/app").unwrap().label, "app");
    }

    #[test]
    fn job_without_a_matching_runner_is_stuck_until_one_shows_up() {
        let mut h = Harness::new(CiSettings::default());
        h.feed(CiInput::Runners(vec![runner(1, &["ops"], false)]));
        h.feed(jobs(
            vec![job(1, "g/app", JobStatus::Pending, &["docker"])],
            vec![],
        ));
        assert_eq!(h.place_of(1).as_deref(), Some(STUCK));
        assert_eq!(h.state_of(1).as_deref(), Some("stuck"));
        h.feed(CiInput::Runners(vec![
            runner(1, &["ops"], false),
            runner(2, &["docker"], false),
        ]));
        assert_eq!(h.place_of(1).as_deref(), Some(QUEUE));
        assert_eq!(h.state_of(1).as_deref(), Some("pending"));
    }

    #[test]
    fn offline_runners_and_untagged_rules_count() {
        let mut h = Harness::new(CiSettings::default());
        let mut offline = runner(1, &["docker"], true);
        offline.online = false;
        h.feed(CiInput::Runners(vec![offline, runner(2, &["ops"], false)]));
        h.feed(jobs(
            vec![
                job(1, "g/a", JobStatus::Pending, &["docker"]),
                job(2, "g/a", JobStatus::Pending, &[]),
            ],
            vec![],
        ));
        assert_eq!(
            h.place_of(1).as_deref(),
            Some(STUCK),
            "only an offline runner has the tag"
        );
        assert_eq!(
            h.place_of(2).as_deref(),
            Some(STUCK),
            "nobody runs untagged jobs"
        );
    }

    #[test]
    fn running_job_without_agent_sits_on_its_runner() {
        let mut h = Harness::new(CiSettings::default());
        h.feed(CiInput::Runners(vec![runner(7, &[], true)]));
        h.feed(jobs(vec![running(1, "g/app", 7)], vec![]));
        assert_eq!(h.place_of(1).as_deref(), Some("runner:7"));
        let place = h.world.place("runner:7").unwrap();
        assert_eq!(
            (
                place.kind.as_str(),
                place.parent.as_deref(),
                place.label.as_str()
            ),
            ("process", Some(NOHOST), "runner7")
        );
    }

    #[test]
    fn finished_build_flies_to_its_exit_then_goes() {
        let mut h = Harness::new(with_exits());
        let mut build = running(1, "g/app", 7);
        build.stage = "build".into();
        h.feed(jobs(vec![build.clone()], vec![]));
        build.status = JobStatus::Success;
        h.feed(jobs(vec![], vec![build]));
        assert_eq!(h.place_of(1).as_deref(), Some("exit:registry"));
        assert_eq!(h.state_of(1).as_deref(), Some("success"));
        h.tick(2_999);
        assert!(h.place_of(1).is_some(), "still visible for the linger time");
        h.tick(1);
        assert!(h.place_of(1).is_none());
    }

    #[test]
    fn failed_job_goes_to_the_sink() {
        let mut h = Harness::new(with_exits());
        let mut build = running(1, "g/app", 7);
        build.stage = "build".into();
        h.feed(jobs(vec![build.clone()], vec![]));
        build.status = JobStatus::Failed;
        h.feed(jobs(vec![], vec![build]));
        assert_eq!(h.place_of(1).as_deref(), Some(SINK));
        assert_eq!(h.state_of(1).as_deref(), Some("failed"));
    }

    #[test]
    fn job_finished_between_polls_is_closed_without_running() {
        let mut h = Harness::new(CiSettings::default());
        h.feed(jobs(vec![job(1, "g/app", JobStatus::Pending, &[])], vec![]));
        h.feed(jobs(vec![], vec![job(1, "g/app", JobStatus::Success, &[])]));
        assert_eq!(h.place_of(1).as_deref(), Some(SINK));
        assert_eq!(h.state_of(1).as_deref(), Some("success"));
    }

    #[test]
    fn job_first_seen_running_flies_from_its_project() {
        let mut h = Harness::new(CiSettings::default());
        let ch = h.feed(jobs(vec![running(1, "g/app", 7)], vec![]));
        assert!(ch
            .iter()
            .any(|c| matches!(c, Change::Appear { entity } if entity.place == "project:g/app")));
        assert!(ch.contains(&Change::Move {
            id: "job:1".into(),
            to: "runner:7".into()
        }));
    }

    #[test]
    fn job_finished_never_seen_is_not_shown() {
        let mut h = Harness::new(CiSettings::default());
        h.feed(jobs(vec![], vec![job(1, "g/app", JobStatus::Success, &[])]));
        assert!(h.place_of(1).is_none());
    }

    #[test]
    fn vanished_job_is_kept_for_the_grace_period() {
        let mut h = Harness::new(CiSettings::default());
        h.feed(jobs(vec![job(1, "g/app", JobStatus::Pending, &[])], vec![]));
        h.feed(jobs(vec![], vec![]));
        h.tick(119_999);
        assert!(h.place_of(1).is_some());
        h.tick(1);
        assert!(h.place_of(1).is_none());
    }

    #[test]
    fn projects_beyond_the_limit_fold_into_other() {
        let mut h = Harness::new(CiSettings {
            max_projects: 2,
            ..CiSettings::default()
        });
        let all = vec![
            job(1, "g/a", JobStatus::Pending, &[]),
            job(2, "g/b", JobStatus::Pending, &[]),
            job(3, "g/c", JobStatus::Pending, &[]),
        ];
        let ch = h.feed(jobs(all, vec![]));
        let born: Vec<(String, String)> = ch
            .iter()
            .filter_map(|c| match c {
                Change::Appear { entity } => Some((entity.id.clone(), entity.place.clone())),
                _ => None,
            })
            .collect();
        assert!(
            born.contains(&("job:3".into(), OTHER_PROJECT.into())),
            "{born:?}"
        );
        assert_eq!(h.world.place(OTHER_PROJECT).unwrap().label, "other");
    }

    #[test]
    fn busy_newcomer_gets_its_own_place_once_an_old_project_goes_quiet() {
        let mut h = Harness::new(CiSettings {
            max_projects: 2,
            project_quiet_ms: 10_000,
            finished_linger_ms: 1_000,
            ..CiSettings::default()
        });
        h.feed(jobs(
            vec![
                job(1, "g/a", JobStatus::Pending, &[]),
                job(2, "g/b", JobStatus::Pending, &[]),
                job(3, "g/c", JobStatus::Pending, &[]),
            ],
            vec![],
        ));
        assert!(h.world.place(OTHER_PROJECT).is_some(), "g/c starts folded");
        // g/a finishes and goes quiet; g/b stays busy, g/c keeps running jobs.
        h.feed(jobs(
            vec![
                job(2, "g/b", JobStatus::Pending, &[]),
                job(3, "g/c", JobStatus::Pending, &[]),
            ],
            vec![job(1, "g/a", JobStatus::Canceled, &[])],
        ));
        h.tick(1_000);
        // Not quiet long enough yet: the newcomer is still folded.
        let ch = h.feed(jobs(
            vec![
                job(2, "g/b", JobStatus::Pending, &[]),
                job(3, "g/c", JobStatus::Pending, &[]),
                job(4, "g/c", JobStatus::Pending, &[]),
            ],
            vec![],
        ));
        assert!(
            ch.iter()
                .any(|c| matches!(c, Change::Appear { entity } if entity.id == "job:4" && entity.place == OTHER_PROJECT)),
            "{ch:?}"
        );
        h.tick(10_000);
        let ch = h.feed(jobs(
            vec![
                job(2, "g/b", JobStatus::Pending, &[]),
                job(3, "g/c", JobStatus::Pending, &[]),
                job(4, "g/c", JobStatus::Pending, &[]),
                job(5, "g/c", JobStatus::Pending, &[]),
            ],
            vec![],
        ));
        assert!(
            ch.iter()
                .any(|c| matches!(c, Change::Appear { entity } if entity.id == "job:5" && entity.place == "project:g/c")),
            "{ch:?}"
        );
        assert!(h.world.place("project:g/a").is_none());
        assert!(h.world.place("project:g/b").is_some());
        assert!(
            !ch.iter()
                .any(|c| matches!(c, Change::Move { id, .. } if id == "job:3" || id == "job:4")),
            "existing tokens must not jump: {ch:?}"
        );
    }

    #[test]
    fn idle_empty_project_is_removed() {
        let mut h = Harness::new(CiSettings {
            project_idle_ms: 10_000,
            finished_linger_ms: 1_000,
            ..CiSettings::default()
        });
        h.feed(jobs(vec![job(1, "g/app", JobStatus::Pending, &[])], vec![]));
        h.feed(jobs(
            vec![],
            vec![job(1, "g/app", JobStatus::Canceled, &[])],
        ));
        h.tick(1_000);
        assert!(h.world.place("project:g/app").is_some());
        let ch = h.tick(9_000);
        assert!(ch.contains(&Change::RemovePlace {
            id: "project:g/app".into()
        }));
    }

    #[test]
    fn stale_and_fresh_pass_through() {
        let mut h = Harness::new(CiSettings::default());
        h.feed(CiInput::SourceStale {
            source: "gitlab".into(),
            since_ms: 5,
        });
        assert_eq!(h.world.snapshot(0).stale.get("gitlab"), Some(&5));
        h.feed(CiInput::SourceFresh {
            source: "gitlab".into(),
        });
        assert!(h.world.snapshot(0).stale.is_empty());
    }

    #[test]
    fn static_zones_exist_after_any_input() {
        let mut h = Harness::new(with_exits());
        h.tick(0);
        for id in [
            PROJECTS,
            QUEUE,
            STUCK,
            HOSTS,
            NOHOST,
            EXITS,
            SINK,
            "exit:registry",
        ] {
            assert!(h.world.place(id).is_some(), "{id}");
        }
        assert_eq!(
            h.world.place("exit:registry").unwrap().parent.as_deref(),
            Some(EXITS)
        );
    }

    fn heavy(id: u64) -> JobInfo {
        JobInfo {
            tags: vec!["heavy".into()],
            ..running(id, "g/app", 2)
        }
    }

    fn shared(slots: Vec<SlotReport>) -> CiInput {
        let mut p = process("p", Some(5), slots);
        p.limits.insert(2, 2);
        agent("a", "host", vec![p])
    }

    #[test]
    fn agent_report_builds_host_process_and_slots() {
        let mut h = Harness::new(CiSettings::default());
        h.feed(agent("a", "host", vec![process("p", Some(3), vec![])]));
        assert_eq!(
            h.world.place("host:host").unwrap().parent.as_deref(),
            Some(HOSTS)
        );
        let p = h.world.place("process:a/p").unwrap();
        assert_eq!(
            (p.capacity, p.parent.as_deref()),
            (Some(3), Some("host:host"))
        );
        assert_eq!(p.attrs.get("agent"), Some(&serde_json::json!("a")));
        assert_eq!(p.attrs.get("online"), Some(&serde_json::json!(true)));
        for n in 0..3 {
            assert_eq!(
                h.world
                    .place(&format!("slot:a/p/{n}"))
                    .unwrap()
                    .parent
                    .as_deref(),
                Some("process:a/p")
            );
        }
        assert!(h.world.place("slot:a/p/3").is_none());
    }

    #[test]
    fn running_job_lands_in_its_slot_with_memory() {
        let mut h = Harness::new(CiSettings::default());
        h.feed(agent(
            "a",
            "host",
            vec![process("p", Some(2), vec![slot(1, 5, 1, 1.5, 3.0)])],
        ));
        h.feed(jobs(vec![running(5, "g/app", 1)], vec![]));
        assert_eq!(h.place_of(5).as_deref(), Some("slot:a/p/1"));
        let mem = h.world.level("job:5", "mem").unwrap();
        assert_eq!(mem.max, Some(3.0 * (1u64 << 30) as f64));
        assert!((mem.value / mem.max.unwrap() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn agent_ahead_of_gitlab_creates_no_phantom_job() {
        let mut h = Harness::new(CiSettings::default());
        h.feed(agent(
            "a",
            "host",
            vec![process("p", Some(2), vec![slot(1, 99, 1, 0.5, 3.0)])],
        ));
        assert!(h.world.entity("job:99").is_none());
        let ch = h.feed(jobs(vec![running(99, "g/app", 1)], vec![]));
        assert!(ch
            .iter()
            .any(|c| matches!(c, Change::Appear { entity } if entity.place == "project:g/app")));
        assert_eq!(h.place_of(99).as_deref(), Some("slot:a/p/1"));
    }

    #[test]
    fn capacity_comes_from_settings_and_grows_to_fit_seen_slots() {
        let mut settings = CiSettings::default();
        settings.capacities.insert("q".into(), 6);
        let mut h = Harness::new(settings);
        h.feed(agent(
            "a",
            "host",
            vec![process("p", None, vec![]), process("q", None, vec![])],
        ));
        assert_eq!(
            h.world.place("process:a/p").unwrap().capacity,
            Some(4),
            "default"
        );
        assert_eq!(
            h.world.place("process:a/q").unwrap().capacity,
            Some(6),
            "from settings"
        );
        h.feed(agent(
            "a",
            "host",
            vec![process("p", Some(2), vec![slot(3, 8, 1, 1.0, 3.0)])],
        ));
        assert_eq!(
            h.world.place("process:a/p").unwrap().capacity,
            Some(4),
            "slot #3 needs 4 slots"
        );
        h.feed(jobs(vec![running(8, "g/app", 1)], vec![]));
        assert_eq!(h.place_of(8).as_deref(), Some("slot:a/p/3"));
    }

    #[test]
    fn waits_for_the_runner_limit_while_a_slot_is_free() {
        let mut h = Harness::new(CiSettings::default());
        h.feed(CiInput::Runners(vec![runner(2, &["heavy"], false)]));
        h.feed(shared(vec![
            slot(0, 1, 2, 1.0, 2.0),
            slot(1, 2, 2, 1.0, 2.0),
        ]));
        let waiting = job(3, "g/app", JobStatus::Pending, &["heavy"]);
        h.feed(jobs(vec![heavy(1), heavy(2), waiting.clone()], vec![]));
        assert_eq!(h.state_of(3).as_deref(), Some("waiting_limit"));
        assert_eq!(h.place_of(3).as_deref(), Some(QUEUE));
        let done = JobInfo {
            status: JobStatus::Success,
            ..heavy(1)
        };
        h.feed(shared(vec![slot(1, 2, 2, 1.0, 2.0)]));
        h.feed(jobs(vec![heavy(2), waiting], vec![done]));
        assert_eq!(h.state_of(3).as_deref(), Some("pending"));
    }

    #[test]
    fn oom_flashes_the_slot_and_the_state_sticks() {
        let mut h = Harness::new(CiSettings::default());
        h.feed(agent(
            "a",
            "host",
            vec![process("p", Some(2), vec![slot(0, 7, 1, 2.9, 3.0)])],
        ));
        h.feed(jobs(vec![running(7, "g/app", 1)], vec![]));
        let ch = h.feed(CiInput::AgentExit(ContainerExit {
            agent: "a".into(),
            job_id: 7,
            oom: true,
            exit_code: 137,
        }));
        assert!(ch.contains(&Change::Flash {
            at: "slot:a/p/0".into(),
            kind: "oom".into()
        }));
        assert_eq!(h.state_of(7).as_deref(), Some("oom"));
        let failed = JobInfo {
            status: JobStatus::Failed,
            ..running(7, "g/app", 1)
        };
        h.feed(jobs(vec![], vec![failed]));
        assert_eq!(h.state_of(7).as_deref(), Some("oom"));
        assert_eq!(h.place_of(7).as_deref(), Some(SINK));
        assert!(h.world.level("job:7", "mem").is_none());
    }

    #[test]
    fn job_dropped_by_the_agent_keeps_its_slot_until_gitlab_finishes_it() {
        let mut h = Harness::new(CiSettings::default());
        h.feed(agent(
            "a",
            "host",
            vec![process("p", Some(2), vec![slot(0, 7, 1, 1.0, 3.0)])],
        ));
        h.feed(jobs(vec![running(7, "g/app", 1)], vec![]));
        h.feed(agent("a", "host", vec![process("p", Some(2), vec![])]));
        assert_eq!(
            h.place_of(7).as_deref(),
            Some("slot:a/p/0"),
            "no flicker to the process row"
        );
    }

    #[test]
    fn job_does_not_keep_a_slot_the_agent_gives_to_another_job() {
        let mut h = Harness::new(CiSettings::default());
        h.feed(agent(
            "a",
            "host",
            vec![process("p", Some(2), vec![slot(0, 1, 1, 1.0, 3.0)])],
        ));
        h.feed(jobs(vec![running(1, "g/app", 1)], vec![]));
        assert_eq!(h.place_of(1).as_deref(), Some("slot:a/p/0"));
        // Job 1's container is gone and job 2 took slot 0, but GitLab still says both run.
        h.feed(agent(
            "a",
            "host",
            vec![process("p", Some(2), vec![slot(0, 2, 1, 1.0, 3.0)])],
        ));
        h.feed(jobs(
            vec![running(1, "g/app", 1), running(2, "g/app", 1)],
            vec![],
        ));
        assert_eq!(h.place_of(2).as_deref(), Some("slot:a/p/0"));
        assert_eq!(
            h.place_of(1).as_deref(),
            Some("process:a/p"),
            "the old job must leave the slot the agent reports as taken"
        );
    }

    #[test]
    fn agent_gone_greys_the_process_and_moves_its_jobs_there() {
        let mut h = Harness::new(CiSettings::default());
        h.feed(agent(
            "a",
            "host",
            vec![process("p", Some(2), vec![slot(0, 7, 1, 1.0, 3.0)])],
        ));
        h.feed(jobs(vec![running(7, "g/app", 1)], vec![]));
        h.feed(CiInput::AgentGone { agent: "a".into() });
        assert_eq!(
            h.world.place("process:a/p").unwrap().attrs.get("online"),
            Some(&serde_json::json!(false))
        );
        assert_eq!(h.place_of(7).as_deref(), Some("process:a/p"));
        h.feed(agent(
            "a",
            "host",
            vec![process("p", Some(2), vec![slot(0, 7, 1, 1.0, 3.0)])],
        ));
        assert_eq!(
            h.world.place("process:a/p").unwrap().attrs.get("online"),
            Some(&serde_json::json!(true))
        );
        assert_eq!(h.place_of(7).as_deref(), Some("slot:a/p/0"));
    }

    #[test]
    fn two_agents_on_one_host_share_the_host() {
        let mut h = Harness::new(CiSettings::default());
        h.feed(agent("a", "box", vec![process("p", Some(1), vec![])]));
        h.feed(agent("b", "box", vec![process("q", Some(1), vec![])]));
        assert_eq!(h.world.places().filter(|p| p.kind == "host").count(), 1);
        assert_eq!(
            h.world.place("process:b/q").unwrap().parent.as_deref(),
            Some("host:box")
        );
    }

    #[test]
    fn disk_level_lands_on_the_host() {
        let mut h = Harness::new(CiSettings::default());
        let report = AgentReport {
            agent: "a".into(),
            host_label: "box".into(),
            processes: vec![],
            disk: Some(DiskUsage {
                used_bytes: 30,
                total_bytes: 100,
            }),
        };
        h.feed(CiInput::Agent(report));
        let disk = h.world.level("host:box", "disk").unwrap();
        assert_eq!((disk.value, disk.max), (30.0, Some(100.0)));
    }
}
