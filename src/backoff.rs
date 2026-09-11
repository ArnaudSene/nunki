//! Waiting out the harness (SPEC 4.3, "les causes du harnais restent à part").
//!
//! A run that fell for a harness cause — quota, network, a harness crash —
//! is replayed without costing an attempt, but not at once: each failure in
//! a row doubles the wait before the next launch, from two minutes up to an
//! hour. Once waiting again would run past the mission's ceiling
//! (`harness_wait_hours`), `hq` holds the mission and hands it to the human.
//! An authentication failure goes to the human at once: no wait mends a
//! revoked token.
//!
//! `hq verify` does not sleep — it launches and returns. The wait is a "not
//! before" kept in the mission's state and honoured at the launch site, so it
//! survives a machine asleep; the mission's monitor ([`crate::monitor`]) is
//! what calls `verify` again once it is over.
//!
//! Pure: every function takes the time it is asked about.

use serde::{Deserialize, Serialize};

use crate::harness::Fault;

/// The wait after the first failure.
pub const FIRST_WAIT_MINUTES: u64 = 2;
/// No single wait is longer than this.
pub const LONGEST_WAIT_MINUTES: u64 = 60;

/// Harness failures in a row, kept with the mission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessDown {
    /// Failures in a row since the harness last carried a run through.
    pub failures: u32,
    /// Epoch seconds of the first of them.
    pub since: u64,
    /// Epoch seconds before which no run is launched.
    pub not_before: u64,
    /// What the harness said the last time.
    pub last: String,
}

/// What follows a harness failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// Launch nothing before this epoch second.
    Wait { until: u64 },
    /// Hold the mission for the human, and say why.
    Human { why: String },
}

/// The wait after the `failures`-th failure in a row: 2, 4, 8, 16, 32, then
/// 60 minutes every time.
pub fn wait_minutes(failures: u32) -> u64 {
    let doublings = failures.saturating_sub(1).min(16);
    (FIRST_WAIT_MINUTES << doublings).min(LONGEST_WAIT_MINUTES)
}

/// Count one more failure, and say what follows it.
pub fn after(
    previous: Option<&HarnessDown>,
    fault: &Fault,
    now: u64,
    ceiling_hours: u32,
) -> (HarnessDown, Next) {
    let failures = previous.map_or(0, |p| p.failures) + 1;
    let since = previous.map_or(now, |p| p.since);
    let until = now + wait_minutes(failures) * 60;
    let record = HarnessDown {
        failures,
        since,
        not_before: until,
        last: fault.why.clone(),
    };
    let next = if fault.authentication {
        Next::Human {
            why: format!(
                "the harness could not authenticate ({}) — no wait mends a token: log in \
                 again, then lift the hold",
                fault.why
            ),
        }
    } else if until.saturating_sub(since) > u64::from(ceiling_hours) * 3600 {
        Next::Human {
            why: format!(
                "the harness has failed {failures} time(s) in a row since {}, and waiting \
                 again would pass the {ceiling_hours}-hour ceiling — last: {}",
                crate::state::rfc3339(since),
                fault.why
            ),
        }
    } else {
        Next::Wait { until }
    };
    (record, next)
}

/// Whether a run may be launched at `now`.
pub fn due(record: Option<&HarnessDown>, now: u64) -> bool {
    record.is_none_or(|r| now >= r.not_before)
}
