//! The real docker daemon, through its API (`DOCKER_HOST`, or the local socket).
//! Only list, stats, inspect and events: the agent never changes anything.

use super::docker::{Docker, DockerEvent, JobContainer};
use super::labels::{self, JOB_ID};
use anyhow::Context;
use bollard::query_parameters::{EventsOptions, ListContainersOptions, StatsOptions};
use futures::stream::BoxStream;
use futures::StreamExt;
use std::collections::HashMap;

pub struct BollardDocker {
    docker: bollard::Docker,
    /// All memory of the host: a container "limited" to this has no limit.
    mem_total: u64,
}

impl BollardDocker {
    pub async fn connect() -> anyhow::Result<Self> {
        let docker = bollard::Docker::connect_with_defaults()
            .context("cannot find the docker daemon (DOCKER_HOST or /var/run/docker.sock)")?;
        let info = docker
            .info()
            .await
            .context("cannot talk to the docker daemon; is its socket mounted and readable?")?;
        Ok(Self {
            docker,
            mem_total: info.mem_total.unwrap_or(0).max(0) as u64,
        })
    }

    async fn memory(&self, id: &str) -> Option<(u64, Option<u64>)> {
        let options = StatsOptions {
            stream: false,
            one_shot: true,
        };
        let stats = self.docker.stats(id, Some(options)).next().await?.ok()?;
        let mem = stats.memory_stats?;
        let usage = mem.usage.unwrap_or(0);
        // As `docker stats`: page cache that can be dropped is not counted.
        let cache = mem
            .stats
            .as_ref()
            .and_then(|s| {
                s.get("inactive_file")
                    .or_else(|| s.get("total_inactive_file"))
            })
            .copied()
            .unwrap_or(0);
        let limit = mem
            .limit
            .filter(|l| *l > 0 && (self.mem_total == 0 || *l < self.mem_total));
        Some((usage.saturating_sub(cache), limit))
    }
}

fn job_filter(events: Option<&[&str]>) -> HashMap<String, Vec<String>> {
    let mut filters = HashMap::from([("label".to_string(), vec![JOB_ID.to_string()])]);
    if let Some(events) = events {
        filters.insert("type".into(), vec!["container".into()]);
        filters.insert(
            "event".into(),
            events.iter().map(|e| e.to_string()).collect(),
        );
    }
    filters
}

impl Docker for BollardDocker {
    async fn containers(&self) -> anyhow::Result<Vec<JobContainer>> {
        let options = ListContainersOptions {
            filters: Some(job_filter(None)),
            ..Default::default()
        };
        let list = self
            .docker
            .list_containers(Some(options))
            .await
            .context("cannot list containers")?;
        let found = list.into_iter().filter_map(|c| {
            let job = labels::parse(c.labels.as_ref()?)?;
            Some((c.id?, job))
        });
        let measured = futures::future::join_all(found.map(|(id, job)| async move {
            // A container that ended between the list and its stats is left out.
            let (mem_bytes, mem_limit_bytes) = self.memory(&id).await?;
            Some(JobContainer {
                id,
                job_id: job.job_id,
                runner_token: job.runner_token,
                service: job.service,
                mem_bytes,
                mem_limit_bytes,
            })
        }))
        .await;
        Ok(measured.into_iter().flatten().collect())
    }

    fn events(&self) -> BoxStream<'static, anyhow::Result<DockerEvent>> {
        let options = EventsOptions {
            filters: Some(job_filter(Some(&["oom", "die"]))),
            ..Default::default()
        };
        self.docker
            .events(Some(options))
            .filter_map(|event| async move {
                let event = match event {
                    Ok(e) => e,
                    Err(e) => return Some(Err(anyhow::Error::new(e).context("docker events"))),
                };
                let actor = event.actor?;
                let attrs = actor.attributes.unwrap_or_default();
                let job_id = labels::parse(&attrs)?.job_id;
                let container = actor.id?;
                match event.action.as_deref()? {
                    "oom" => Some(Ok(DockerEvent::Oom { container, job_id })),
                    "die" => Some(Ok(DockerEvent::Die {
                        container,
                        job_id,
                        exit_code: attrs
                            .get("exitCode")
                            .and_then(|c| c.parse().ok())
                            .unwrap_or(-1),
                    })),
                    _ => None,
                }
            })
            .boxed()
    }

    async fn oom_killed(&self, container: &str) -> bool {
        self.docker
            .inspect_container(
                container,
                None::<bollard::query_parameters::InspectContainerOptions>,
            )
            .await
            .ok()
            .and_then(|c| c.state)
            .and_then(|s| s.oom_killed)
            .unwrap_or(false)
    }
}
