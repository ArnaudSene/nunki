//! The flow of a mission (SPEC 4.5) as a state machine.
//!
//! The HQ holds the wheel: it launches the coder, then — according to the
//! mission's shape — the integrator, then the security agent; a red verdict
//! goes back to the coder as a "volet", bounded; harness failures replay
//! without counting. This module decides transitions only. It launches
//! nothing, reads no file: the engine feeds it [`Event`]s derived from the
//! harness, the journal and the gates, and acts on the [`Stage`] it returns.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::harness::{Outcome, Role};

use super::{Header, Verdict};

/// Which piece of work a coder run is for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Work {
    /// A planned lot, by index into the header's list.
    Lot(usize),
    /// A return to the coder after a red verdict; `n` starts at 1.
    Volet { n: u32, cause: String },
}

/// Why the flow stopped and handed over to the human.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Handover {
    /// A lot failed `attempts_per_lot` times; the journals are side by side.
    LotAttemptsExhausted { lot: String, attempts: u32 },
    /// A role failed its own mission `attempts_per_lot` times.
    RoleAttemptsExhausted { role: Role, attempts: u32 },
    /// `max_volets` returns to the coder were used; the verdicts are listed.
    VoletsExhausted { causes: Vec<String> },
}

/// Where the mission is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stage {
    /// A coder run is due or running.
    Coding { work: Work, attempt: u32 },
    /// The final gates (5–7) and the mechanical security gates are due.
    Gates,
    /// The integration mission is due or running.
    Integration { attempt: u32 },
    /// The security agent mission is due or running.
    SecurityAgent { attempt: u32 },
    /// The security agent returned `FINDINGS`: the HQ iterates (back to the
    /// coder) or the human lifts the findings.
    Findings { report: String },
    /// Stopped; the human decides.
    AwaitingHuman(Handover),
    /// Every declared stage is green: ready for human validation and `hq push`.
    Verified,
}

/// What the engine observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The current agent run ended. For the coder, `lot_done` says whether
    /// the journal reports the lot finished and proved; it is ignored for
    /// the other roles.
    RunEnded { outcome: Outcome, lot_done: bool },
    /// A stall was observed by the liveness checks, or a human `kill`; the
    /// run was stopped. Counts as a failed attempt.
    Stalled { reason: String },
    /// The gates of the current stage passed.
    GatesPassed,
    /// A gate failed; the coder gets a run to fix it (not a volet).
    GatesFailed { reason: String },
    /// The integrator or the security agent wrote its verdict.
    Verdict { verdict: Verdict, report: String },
    /// From `Findings`: the HQ chooses to iterate.
    Iterate,
    /// From `Findings`: the human lifted every finding (`hq mission accept`).
    HumanAccepted,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FlowError {
    #[error("event {event:?} is not valid in stage {stage:?}")]
    InvalidTransition { stage: Stage, event: Event },
    #[error("a mission needs at least one lot")]
    NoLots,
}

/// The state machine. Serializable, so the engine persists it at every
/// transition (SPEC 4.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flow {
    header: Header,
    stage: Stage,
    volets: u32,
    volet_causes: Vec<String>,
    /// Attempts used per coder work item, keyed by a stable label.
    attempts: HashMap<String, u32>,
}

impl Flow {
    /// A new flow starts with the first lot, first attempt.
    pub fn new(header: Header) -> Result<Self, FlowError> {
        if header.lots.is_empty() {
            return Err(FlowError::NoLots);
        }
        Ok(Self {
            header,
            stage: Stage::Coding {
                work: Work::Lot(0),
                attempt: 1,
            },
            volets: 0,
            volet_causes: Vec::new(),
            attempts: HashMap::new(),
        })
    }

    pub fn stage(&self) -> &Stage {
        &self.stage
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    /// Returns to the coder used so far.
    pub fn volets(&self) -> u32 {
        self.volets
    }

    /// Apply an event; returns the new stage. An event that makes no sense
    /// in the current stage is an error, never a silent no-op.
    pub fn advance(&mut self, event: Event) -> Result<&Stage, FlowError> {
        let next = match (self.stage.clone(), event.clone()) {
            // --- coder -------------------------------------------------
            (Stage::Coding { work, attempt }, Event::RunEnded { outcome, lot_done }) => {
                match outcome {
                    // Not the mission's fault: replay the same attempt.
                    Outcome::HarnessFailure(_) => Stage::Coding { work, attempt },
                    Outcome::Finished(_) if lot_done => self.after_lot(&work),
                    // Finished without finishing the lot, or failed for a
                    // mission cause: one more attempt, bounded.
                    Outcome::Finished(_) | Outcome::MissionFailure(_) => {
                        self.retry_coder(work, attempt)
                    }
                }
            }
            (Stage::Coding { work, attempt }, Event::Stalled { .. }) => {
                self.retry_coder(work, attempt)
            }
            // Gates 1 to 4 are played at the end of **every** run, not only
            // at the final verification (SPEC 4.4, decided 2026-09-09): a
            // perimeter gate that only falls at the end loses a six-hour
            // mission over a forbidden write in the first lot. A red one
            // here is one more run on the same lot — the work is not done,
            // and it is bounded like any other attempt.
            (Stage::Coding { work, attempt }, Event::GatesFailed { .. }) => {
                self.retry_coder(work, attempt)
            }

            // --- gates -------------------------------------------------
            (Stage::Gates, Event::GatesPassed) => self.after_gates(),
            (Stage::Gates, Event::GatesFailed { reason }) => Stage::Coding {
                work: Work::Volet {
                    n: self.volets,
                    cause: format!("gate: {reason}"),
                },
                attempt: 1,
            },

            // --- integrator --------------------------------------------
            (Stage::Integration { attempt }, Event::RunEnded { outcome, .. }) => {
                self.role_run_ended(Role::Integrator, attempt, outcome)
            }
            (Stage::Integration { attempt }, Event::Stalled { .. }) => {
                self.retry_role(Role::Integrator, attempt)
            }
            (
                Stage::Integration { .. },
                Event::Verdict {
                    verdict: Verdict::Integrated,
                    ..
                },
            ) => self.after_integration(),
            (
                Stage::Integration { .. },
                Event::Verdict {
                    verdict: Verdict::Broken,
                    report,
                },
            ) => self.volet(format!("integrator BROKEN: {report}")),

            // --- security agent ----------------------------------------
            (Stage::SecurityAgent { attempt }, Event::RunEnded { outcome, .. }) => {
                self.role_run_ended(Role::Security, attempt, outcome)
            }
            (Stage::SecurityAgent { attempt }, Event::Stalled { .. }) => {
                self.retry_role(Role::Security, attempt)
            }
            (
                Stage::SecurityAgent { .. },
                Event::Verdict {
                    verdict: Verdict::Clear,
                    ..
                },
            ) => Stage::Verified,
            (
                Stage::SecurityAgent { .. },
                Event::Verdict {
                    verdict: Verdict::Findings,
                    report,
                },
            ) => Stage::Findings { report },
            (Stage::Findings { report }, Event::Iterate) => {
                self.volet(format!("security FINDINGS: {report}"))
            }
            (Stage::Findings { .. }, Event::HumanAccepted) => Stage::Verified,

            (stage, event) => return Err(FlowError::InvalidTransition { stage, event }),
        };
        self.stage = next;
        Ok(&self.stage)
    }

    /// The stage after the coder finished a piece of work: the next planned
    /// lot, or the final gates.
    fn after_lot(&self, work: &Work) -> Stage {
        match work {
            Work::Lot(i) if i + 1 < self.header.lots.len() => Stage::Coding {
                work: Work::Lot(i + 1),
                attempt: 1,
            },
            _ => Stage::Gates,
        }
    }

    /// The stage after the final gates: what the mission's shape declares.
    fn after_gates(&self) -> Stage {
        if self.header.has_integration() {
            Stage::Integration { attempt: 1 }
        } else if self.header.has_security_agent() {
            Stage::SecurityAgent { attempt: 1 }
        } else {
            Stage::Verified
        }
    }

    fn after_integration(&self) -> Stage {
        if self.header.has_security_agent() {
            Stage::SecurityAgent { attempt: 1 }
        } else {
            Stage::Verified
        }
    }

    /// One more attempt on the same coder work, or the human once the bound
    /// is reached.
    fn retry_coder(&mut self, work: Work, attempt: u32) -> Stage {
        let label = match &work {
            Work::Lot(i) => self.header.lots[*i].id.clone(),
            Work::Volet { n, .. } => format!("volet-{n}"),
        };
        *self.attempts.entry(label.clone()).or_insert(0) = attempt;
        if attempt >= self.header.bounds.attempts_per_lot {
            Stage::AwaitingHuman(Handover::LotAttemptsExhausted {
                lot: label,
                attempts: attempt,
            })
        } else {
            Stage::Coding {
                work,
                attempt: attempt + 1,
            }
        }
    }

    fn role_run_ended(&mut self, role: Role, attempt: u32, outcome: Outcome) -> Stage {
        match outcome {
            Outcome::HarnessFailure(_) => self.role_stage(role, attempt),
            // A finished run must be followed by a `Verdict` event; the run
            // ending by itself changes nothing here.
            Outcome::Finished(_) => self.role_stage(role, attempt),
            Outcome::MissionFailure(_) => self.retry_role(role, attempt),
        }
    }

    fn retry_role(&mut self, role: Role, attempt: u32) -> Stage {
        if attempt >= self.header.bounds.attempts_per_lot {
            Stage::AwaitingHuman(Handover::RoleAttemptsExhausted {
                role,
                attempts: attempt,
            })
        } else {
            self.role_stage(role, attempt + 1)
        }
    }

    fn role_stage(&self, role: Role, attempt: u32) -> Stage {
        match role {
            Role::Integrator => Stage::Integration { attempt },
            Role::Security => Stage::SecurityAgent { attempt },
            Role::Coder => unreachable!("coder runs are handled by retry_coder"),
        }
    }

    /// A red verdict: back to the coder, bounded by `max_volets`. The volet
    /// replays the gates and every declared stage after it (SPEC 4.5, "un
    /// verdict vaut pour un HEAD").
    fn volet(&mut self, cause: String) -> Stage {
        self.volet_causes.push(cause.clone());
        if self.volets >= self.header.bounds.max_volets {
            return Stage::AwaitingHuman(Handover::VoletsExhausted {
                causes: self.volet_causes.clone(),
            });
        }
        self.volets += 1;
        Stage::Coding {
            work: Work::Volet {
                n: self.volets,
                cause,
            },
            attempt: 1,
        }
    }
}
