//! Missions: the structured header (SPEC 4.1), verdicts (SPEC 4.4) and the
//! flow (SPEC 4.5).

pub mod dir;
pub mod flow;

use serde::{Deserialize, Serialize};

use crate::harness::Role;

/// A lot: a small unit of work with its own proof, done in one run (SPEC
/// 4.3, "un run par lot").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lot {
    pub id: String,
    pub title: String,
}

/// Where a mission's external services live (SPEC 2, "Où vivent les services
/// externes"). Only what the engine needs to build the allowlist is kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Service {
    pub name: String,
    /// Domains or addresses the system profile may reach for this service.
    pub reach: Vec<String>,
}

/// The `integration` field of the mission header (SPEC 2, mission shapes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Integration {
    /// No integration mission; the reason is written at framing.
    None { reason: String },
    /// An integration mission on the system profile, reaching these services.
    Services { services: Vec<Service> },
}

/// The `security` field of the mission header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Security {
    /// Mechanical gates only (dependency audit, secret scan, static analysis).
    Gates,
    /// The gates, plus the security agent mission.
    Agent,
}

/// The three shapes of SPEC 2, derived from the header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    CodeOnly,
    CodeAndSecurity,
    Full,
}

/// Bounds and cadences (SPEC 4.3 and 4.5). Project defaults come from
/// `hq.yaml`; the mission header overrides them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bounds {
    /// Returns to the coder allowed after a red verdict. Zero means the first
    /// red goes to the human.
    pub max_volets: u32,
    /// Attempts allowed per lot before the human is called.
    pub attempts_per_lot: u32,
    /// Minutes between the agent's checkpoint blocks.
    pub checkpoint_minutes: u32,
    /// Minutes between liveness checks.
    pub check_minutes: u32,
    /// Consecutive checks without change that count as a stall.
    pub stall_checks: u32,
    /// Hours on one lot, with progress, after which the human is told.
    pub long_lot_hours: u32,
}

impl Default for Bounds {
    fn default() -> Self {
        Self {
            max_volets: 3,
            attempts_per_lot: 3,
            checkpoint_minutes: 45,
            check_minutes: 15,
            stall_checks: 3,
            long_lot_hours: 8,
        }
    }
}

/// The structured header of `MISSION.md`, frozen into the state at
/// validation (SPEC 4.1). The agent cannot write it and `hq` never re-reads
/// it during the mission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub branch: String,
    pub base: String,
    pub lots: Vec<Lot>,
    pub integration: Integration,
    pub security: Security,
    #[serde(default)]
    pub bounds: Bounds,
}

impl Header {
    pub fn shape(&self) -> Shape {
        match (&self.integration, self.security) {
            (Integration::Services { .. }, _) => Shape::Full,
            (Integration::None { .. }, Security::Agent) => Shape::CodeAndSecurity,
            (Integration::None { .. }, Security::Gates) => Shape::CodeOnly,
        }
    }

    pub fn has_integration(&self) -> bool {
        matches!(self.integration, Integration::Services { .. })
    }

    pub fn has_security_agent(&self) -> bool {
        self.security == Security::Agent
    }
}

/// A role's verdict (SPEC 2 and 4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Verdict {
    Integrated,
    Broken,
    Clear,
    Findings,
}

impl Verdict {
    /// The verdict a stage waits for.
    pub fn is_green(self) -> bool {
        matches!(self, Verdict::Integrated | Verdict::Clear)
    }
}

/// `VERDICT.json`, written by the agent at the end of its last run and
/// validated by `hq` against the real `HEAD` (SPEC 4.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerdictFile {
    pub role: Role,
    pub verdict: Verdict,
    /// Commit the role concluded on.
    pub head: String,
    /// RFC 3339 date.
    pub date: String,
    /// The report, as text; for the security role this is the whole finding
    /// list.
    #[serde(default)]
    pub report: String,
}
