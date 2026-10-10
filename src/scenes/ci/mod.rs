//! The CI scene: GitLab jobs flowing through runner processes and their slots.

pub mod input;
pub mod matching;
mod reduce;
pub mod settings;
#[cfg(test)]
pub(crate) mod testkit;

pub use reduce::*;
