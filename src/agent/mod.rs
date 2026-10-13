//! `lorvaq agent`: runs next to gitlab-runner, reads the docker daemon (never changes it)
//! and reports job containers, their memory, the disk and OOM kills to the server.

pub mod docker;
pub mod labels;
pub mod metrics;
pub mod report;
pub mod slots;
