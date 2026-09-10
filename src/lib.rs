//! `hq` — a mission orchestrator for AI coding agents.
//!
//! The specification lives in `SPEC.md` at the repository root. Section
//! numbers in doc comments refer to it. This crate is the engine (SPEC 4.2):
//! it holds the state of missions, slots and verdicts, and talks to a coding
//! harness only through the [`harness::Harness`] trait (SPEC 4.3).

pub mod compose;
pub mod firewall;
pub mod harness;
pub mod mission;
pub mod perimeter;
pub mod state;
