//! GitLab as a source for the CI scene: queries, client, credentials and the poller.

pub mod client;
pub mod fake;
pub mod model;
pub mod oauth;
pub mod poller;

pub use poller::{spawn, GitlabHandle, GitlabStatus, PollConfig, SourceState};
