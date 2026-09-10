//! `hq` — a mission orchestrator for AI coding agents.
//!
//! The specification lives in `SPEC.md` at the repository root. Section
//! numbers in doc comments refer to it. This crate is the engine (SPEC 4.2):
//! it holds the state of missions, slots and verdicts, and talks to a coding
//! harness only through the [`harness::Harness`] trait (SPEC 4.3).

pub mod account;
pub mod check;
pub mod compose;
pub mod engine;
pub mod exec;
pub mod firewall;
pub mod gate;
pub mod git;
pub mod harness;
pub mod human;
pub mod image;
pub mod init;
pub mod mission;
pub mod perimeter;
pub mod probe;
pub mod project;
pub mod role;
pub mod run;
pub mod slot;
pub mod state;
