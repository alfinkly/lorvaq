//! `lorvaq agent`: runs next to gitlab-runner, reads the docker daemon (never changes it)
//! and reports job containers, their memory, the disk and OOM kills to the server.

pub mod bollard_docker;
pub mod disk;
pub mod docker;
pub mod labels;
pub mod metrics;
pub mod proto;
pub mod report;
pub mod run;
pub mod slots;
