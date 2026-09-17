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
    /// The human called it off before it was verified, and said why.
    Abandoned { reason: String },
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
    /// Every declared stage is green: ready for human validation and `nunki push`.
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
    /// From `Findings`: the human lifted every finding (`nunki mission accept`).
    HumanAccepted,
    /// The human takes a mission back from a handover (`nunki mission
    /// retry`), saying what changed. Only from [`Stage::AwaitingHuman`], and
    /// never from a mission the human called off themselves.
    Retried { because: String },
    /// The human called the mission off (`nunki mission end`), with a reason.
    /// Valid wherever a mission can still be worked on: what it says is
    /// "stop asking me about this", and there is no stage where that is not
    /// a thing a human may say.
    Ended { reason: String },
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FlowError {
    #[error("event {event:?} is not valid in stage {stage:?}")]
    InvalidTransition { stage: Stage, event: Event },
    #[error("a mission needs at least one lot")]
    NoLots,
    #[error(
        "the mission is working on lot {lot}, and the new framing declares only {lots} \
         lot(s) — finish the lot, or stop the mission before reframing it"
    )]
    LotGone { lot: String, lots: usize },
    #[error("the mission is already over; `nunki mission archive` closes it")]
    AlreadyOver,
    #[error(
        "this mission was called off, and that is a decision rather than a bound it ran \
         into — `nunki mission retry` takes back a mission the bounds stopped, never \
         one you stopped yourself ({0})"
    )]
    CalledOff(String),
    #[error(
        "this mission handed over before `nunki mission retry` existed, so nothing \
         recorded which work it stopped on — reframe it, or start it again"
    )]
    NothingToResume,
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
    /// The coder work a [`Event::Retried`] picks up, kept from the moment the
    /// flow handed over.
    ///
    /// It is held here and not read back from [`Handover`], whose `lot` is a
    /// label: [`Flow::label`] is not invertible — it folds a volet's cause
    /// away, and nothing stops a human naming a lot `volet-2`. A label is
    /// what a human reads; this is what the machine resumes with.
    ///
    /// `serde(default)` because missions handed over before this existed are
    /// on disk: they read back as `None`, and a retry says so rather than
    /// guessing which work it was.
    #[serde(default)]
    resume_with: Option<Work>,
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
            resume_with: None,
        })
    }

    pub fn stage(&self) -> &Stage {
        &self.stage
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    /// Re-freeze the framing (SPEC 4.1, rule 3).
    ///
    /// Changing the shape of a running mission is a gesture of the HQ,
    /// through a verb, never an edit of the file: `nunki` never re-reads the
    /// header during a mission, so a file edited behind its back changes
    /// nothing and looks as though it did.
    ///
    /// Refused when the flow is standing on a lot the new framing does not
    /// have. Re-numbering under a running mission would leave the state
    /// pointing at work nobody described, and there is no honest guess to
    /// make about which lot was meant.
    pub fn reframe(&mut self, header: Header) -> Result<(), FlowError> {
        if header.lots.is_empty() {
            return Err(FlowError::NoLots);
        }
        if let Stage::Coding {
            work: Work::Lot(i), ..
        } = &self.stage
            && *i >= header.lots.len()
        {
            return Err(FlowError::LotGone {
                lot: self.header.lots[*i].id.clone(),
                lots: header.lots.len(),
            });
        }
        self.header = header;
        Ok(())
    }

    /// What a piece of coder work is called — in the run's prompt, in the
    /// journal's `Lot:` line, in the attempts' count. One spelling, because
    /// the line the agent writes is compared with the one it was given.
    pub fn label(&self, work: &Work) -> String {
        match work {
            Work::Lot(i) => self.header.lots[*i].id.clone(),
            Work::Volet { n, .. } => format!("volet-{n}"),
        }
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
            // Through `volet`, like a red verdict: it counts, and the count
            // is what ends a mission the coder cannot fix.
            //
            // Opened by hand here until 2026-09-17, with `n: self.volets` and
            // no increment, so every return was volet 0 and `max_volets`
            // never came round. Measured on `notes-3`: a gate 7 red on
            // survivors that no test could kill sent the coder back
            // **twenty-eight times** in half an hour — the agent declared its
            // volet done, the gates were played again, the same gate was red
            // again, and the same volet 0 opened again. 34 million tokens,
            // and the only thing that would have stopped it was the
            // account's weekly cap.
            (Stage::Gates, Event::GatesFailed { reason }) => self.volet(format!("gate: {reason}")),

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

            // --- the human takes it back -------------------------------
            //
            // A handover is the flow saying "a bound stopped me, and the
            // decision is yours". `Retried` is that decision, and it hands
            // the budget back whole: resetting the counter **is** the verb,
            // not a side effect of it. Without that the mission would land
            // in the same handover on the very next event.
            //
            // `Abandoned` is not a bound. The human said stop, and a verb
            // that undid that would make `nunki mission end` something they
            // could not rely on.
            (Stage::AwaitingHuman(Handover::Abandoned { reason }), Event::Retried { .. }) => {
                return Err(FlowError::CalledOff(reason));
            }
            // A role's attempts: the role is typed, so the stage to go back
            // to is read straight from the handover.
            (
                Stage::AwaitingHuman(Handover::RoleAttemptsExhausted { role, .. }),
                Event::Retried { .. },
            ) => self.role_stage(role, 1),
            (Stage::AwaitingHuman(_), Event::Retried { .. }) => {
                let Some(work) = self.resume_with.clone() else {
                    return Err(FlowError::NothingToResume);
                };
                // A volet resumes on the budget it was given: 1 when the
                // returns ran out, its own number when it was that volet's
                // attempts that did.
                //
                // Nothing resets `attempts`: the bound is read from the
                // stage's own `attempt`, which starts at 1 below, and the map
                // is written but never read (measured — removing a reset here
                // changed no test, because there is nothing to change).
                if let Work::Volet { n, .. } = &work {
                    self.volets = *n;
                }
                self.resume_with = None;
                Stage::Coding { work, attempt: 1 }
            }

            // Wherever it is, and last in the match so that no stage can
            // claim it first: a mission a human has called off is over, and
            // a verb that worked in five stages out of seven would be a verb
            // the human cannot rely on when they want out.
            (Stage::Verified, Event::Ended { .. })
            | (Stage::AwaitingHuman(_), Event::Ended { .. }) => {
                return Err(FlowError::AlreadyOver);
            }
            (_, Event::Ended { reason }) => Stage::AwaitingHuman(Handover::Abandoned { reason }),

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
        let label = self.label(&work);
        *self.attempts.entry(label.clone()).or_insert(0) = attempt;
        if attempt >= self.header.bounds.attempts_per_lot {
            // Kept whole, so a retry resumes this work and not a label.
            self.resume_with = Some(work);
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
            // The cause that arrived with no budget left to open a volet for
            // it: the one a retry has to start from, numbered 1 because a
            // retry hands back the whole budget.
            self.resume_with = Some(Work::Volet { n: 1, cause });
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
