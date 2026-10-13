//! What the agent asks the docker daemon. A trait, so tests run without docker.

use futures::stream::BoxStream;
use std::future::Future;

/// One container of a running job, with its memory now.
#[derive(Clone, Debug, PartialEq)]
pub struct JobContainer {
    pub id: String,
    pub job_id: u64,
    pub runner_token: String,
    pub service: bool,
    pub mem_bytes: u64,
    /// None when the container has no memory limit.
    pub mem_limit_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DockerEvent {
    /// The kernel killed a process of the container for memory.
    Oom { container: String, job_id: u64 },
    /// The container stopped.
    Die {
        container: String,
        job_id: u64,
        exit_code: i64,
    },
}

pub trait Docker: Send + Sync + 'static {
    /// The containers of running jobs.
    fn containers(&self) -> impl Future<Output = anyhow::Result<Vec<JobContainer>>> + Send;
    /// `oom` and `die` of job containers, from now on; ends when the daemon goes away.
    fn events(&self) -> BoxStream<'static, anyhow::Result<DockerEvent>>;
    /// Whether docker recorded the container as killed for memory (false if it is gone).
    fn oom_killed(&self, container: &str) -> impl Future<Output = bool> + Send;
}

#[doc(hidden)]
pub mod fake {
    use super::*;
    use std::collections::BTreeSet;
    use std::sync::{Arc, Mutex};
    use tokio::sync::broadcast;

    /// Containers and events set by a test.
    #[derive(Clone)]
    pub struct FakeDocker {
        pub containers: Arc<Mutex<Vec<JobContainer>>>,
        pub oom_killed: Arc<Mutex<BTreeSet<String>>>,
        events: broadcast::Sender<DockerEvent>,
    }

    impl Default for FakeDocker {
        fn default() -> Self {
            Self {
                containers: Arc::default(),
                oom_killed: Arc::default(),
                events: broadcast::channel(64).0,
            }
        }
    }

    impl FakeDocker {
        pub fn emit(&self, event: DockerEvent) {
            let _ = self.events.send(event);
        }

        pub fn container(
            id: &str,
            job_id: u64,
            mem_mib: u64,
            limit_mib: Option<u64>,
        ) -> JobContainer {
            JobContainer {
                id: id.into(),
                job_id,
                runner_token: "abcd1234".into(),
                service: false,
                mem_bytes: mem_mib << 20,
                mem_limit_bytes: limit_mib.map(|l| l << 20),
            }
        }
    }

    impl Docker for FakeDocker {
        async fn containers(&self) -> anyhow::Result<Vec<JobContainer>> {
            Ok(self.containers.lock().expect("fake lock").clone())
        }

        fn events(&self) -> BoxStream<'static, anyhow::Result<DockerEvent>> {
            use futures::StreamExt;
            tokio_stream::wrappers::BroadcastStream::new(self.events.subscribe())
                .filter_map(|e| futures::future::ready(e.ok().map(Ok)))
                .boxed()
        }

        async fn oom_killed(&self, container: &str) -> bool {
            self.oom_killed
                .lock()
                .expect("fake lock")
                .contains(container)
        }
    }
}
