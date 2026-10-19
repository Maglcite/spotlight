//! Portable core of Spotlight: configuration, secrets, search, launching,
//! single-instance IPC and the read-only AI harness. The UI lives elsewhere and
//! only talks to these modules.

pub mod ai;
pub mod cli;
pub mod config;
pub mod ipc;
pub mod launch;
pub mod search;
pub mod secrets;
mod tools;
