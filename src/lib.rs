//! `nunki` — a mission orchestrator for AI coding agents.
//!
//! The specification lives in `SPEC.md` at the repository root. Section
//! numbers in doc comments refer to it. This crate is the engine (SPEC 4.2):
//! it holds the state of missions, slots and verdicts, and talks to a coding
//! harness only through the [`harness::Harness`] trait (SPEC 4.3).

pub mod account;
pub mod backoff;
pub mod check;
pub mod compose;
pub mod consumption;
pub mod engine;
pub mod exec;
pub mod findings;
pub mod firewall;
pub mod followup;
pub mod forge;
pub mod gate;
pub mod gesture;
pub mod git;
pub mod harness;
pub mod human;
pub mod image;
pub mod init;
pub mod launch;
pub mod lifecycle;
pub mod logs;
pub mod mission;
pub mod monitor;
pub mod mutants;
pub mod perimeter;
pub mod probe;
pub mod project;
pub mod provider;
pub mod push;
pub mod role;
pub mod run;
pub mod sessions;
pub mod slot;
pub mod state;
pub mod verify;
