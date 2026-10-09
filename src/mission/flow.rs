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
use crate::text::{brief, one_line};

use super::{Header, Rigor, RigorError, Severity, Verdict};

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
    /// The coder's run on `lot`, at `attempt`, said the lot cannot finish
    /// without a ruling it is forbidden to give: the equivalence of the
    /// `survivors` it names, every one an open survivor of the campaign.
    /// Handed over at once, with no further attempt spent; the HQ rules
    /// (`nunki mission mutants --equivalent … --because …`), then `retry`
    /// resumes at the next attempt — or, when the one that asked was the last,
    /// hands over as [`Handover::LotAttemptsExhausted`] rather than go past
    /// the bound.
    ///
    /// A new variant and not a field on an old one, so that every state file
    /// written before it still reads.
    AwaitingRuling {
        lot: String,
        what: String,
        attempt: u32,
        survivors: Vec<String>,
    },
    /// The services file on the slot's `HEAD` renders to `digest`, which no
    /// human approved, so the system profile the `role` needed was not
    /// started (SPEC 4.2). No attempt is spent: none would change it. A
    /// human reads the rendering (`nunki services --show`), approves it, and
    /// `retry` resumes the same role at the same attempt.
    ///
    /// Its own variant, and not [`Handover::AwaitingRuling`]: that one is
    /// the mutation campaign's, reached only while coding, and this one is
    /// reached only where a system profile is lifted.
    ServicesNotApproved {
        role: Role,
        attempt: u32,
        file: String,
        digest: String,
    },
}

impl Handover {
    /// The handover in one line, for `mission status`, `verify` and the
    /// monitor's exit: what stopped, and for a ruling, what the HQ is
    /// expected to do about it. `mission` is the id the verbs are spelled
    /// with.
    pub fn line(&self, mission: &str) -> String {
        format!("{} — {}", self.detail(), self.awaits(mission).1)
    }

    /// What stopped, in one line: the lot and its attempts, the role, the
    /// causes of the volets, the reason it was called off, the survivors.
    /// Agent-written as most of it is (causes, survivors), it is made
    /// printable as a whole ([`crate::text`]).
    pub fn detail(&self) -> String {
        let raw = match self {
            Handover::LotAttemptsExhausted { lot, attempts } => {
                format!("lot {lot} failed {attempts} attempt(s)")
            }
            Handover::RoleAttemptsExhausted { role, attempts } => {
                format!("the {role:?} failed {attempts} attempt(s)")
            }
            // The last cause is the one that found no volet left: it is
            // listed, and not counted as a volet taken.
            Handover::VoletsExhausted { causes } => format!(
                "{} return(s) to the coder were taken, and the last cause found none left: {}",
                causes.len().saturating_sub(1),
                causes
                    .iter()
                    .map(|cause| brief(cause, CAUSE_CHARS))
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
            Handover::Abandoned { reason } => format!("called off: {reason}"),
            Handover::AwaitingRuling {
                lot,
                attempt,
                survivors,
                ..
            } => format!(
                "lot {lot}, attempt {attempt}, awaits the HQ's ruling on {}",
                survivors
                    .iter()
                    .map(|id| format!("`{id}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Handover::ServicesNotApproved {
                role, file, digest, ..
            } => format!(
                "the {role:?} needs the project's services, and {file} on the slot's HEAD \
                 renders to {digest}, which no human approved: nothing was started"
            ),
        };
        one_line(&raw)
    }

    /// Who the handover waits on, and what they are expected to do, with the
    /// verbs spelled for `mission`. A ruling is the HQ's; the rest are the
    /// human's, whose word ends a bound.
    pub fn awaits(&self, mission: &str) -> (&'static str, String) {
        let retry = format!("`nunki mission retry {mission} --because <what changed>`");
        match self {
            Handover::LotAttemptsExhausted { .. } => {
                ("the human", format!("read the journals, then {retry}"))
            }
            Handover::RoleAttemptsExhausted { .. } => ("the human", retry),
            Handover::VoletsExhausted { .. } => (
                "the human",
                format!(
                    "`nunki mission retry {mission} --because <why>` grants one more volet, \
                     written in FOLLOWUP_HQ.md"
                ),
            ),
            Handover::Abandoned { .. } => (
                "the human",
                format!("`nunki mission archive {mission}` closes it"),
            ),
            Handover::AwaitingRuling { .. } => (
                "the HQ",
                format!(
                    "rule each (`nunki mission mutants {mission} --equivalent <survivor> \
                     --because <why>`), then `nunki mission retry {mission} --because <what \
                     was ruled>`"
                ),
            ),
            Handover::ServicesNotApproved { digest, .. } => (
                "the human",
                format!(
                    "read it (`nunki services --show {mission}`), approve it (`nunki services \
                     --approve {digest}`), then {retry}"
                ),
            ),
        }
    }
}

/// How much of one volet's cause a handover's line carries: the start says
/// which verdict or gate it was, and the journals hold the rest.
const CAUSE_CHARS: usize = 120;

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
    /// A `critical` mission whose every other stage is green: gate 7 is
    /// played again, alone, and the mission is `Verified` only on a full
    /// campaign at `HEAD` that passes (SPEC 4.4, 4.5). One the chain left
    /// partial owes the final full campaign first.
    FinalCampaign,
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
    /// From `Findings`: the human lifted every finding (`nunki mission
    /// accept`) — or `nunki` did, every finding being `LOW` or `INFO`
    /// ([`crate::findings::lift_by_nunki`]). One transition for both, so a
    /// lift by `nunki` moves the flow exactly as a human's does.
    HumanAccepted,
    /// From `Verified`: the HQ read the verified branch before pushing it and
    /// sends it back to the coder, saying why (`nunki mission iterate
    /// --because`). SPEC 4.5: the HQ reads the code, and its answer is the
    /// iteration loop.
    Reviewed { because: String },
    /// The human takes a mission back from a handover (`nunki mission
    /// retry`), saying what changed. Only from [`Stage::AwaitingHuman`], and
    /// never from a mission the human called off themselves.
    Retried { because: String },
    /// The coder's run said its lot awaits a ruling on `survivors`, and the
    /// engine checked that each is an open survivor of the campaign. Only
    /// while coding; what the run wrote is `what`.
    RulingAwaited {
        what: String,
        survivors: Vec<String>,
    },
    /// A system profile was due, and the services file on the slot's
    /// `HEAD` renders to `digest`, which no human approved: the engine
    /// started nothing. Only where a system profile is lifted, before the
    /// role's run is launched.
    ServicesUnapproved { file: String, digest: String },
    /// The human called the mission off (`nunki mission end`), with a reason.
    /// Valid wherever a mission can still be worked on: what it says is
    /// "stop asking me about this", and there is no stage where that is not
    /// a thing a human may say.
    Ended { reason: String },
    /// The HQ sends the mission back although its rigor says not to, and says
    /// why (`nunki mission iterate --override --because`): from `Findings`
    /// on a report the rigor does not count as blocking, or from `Verified`
    /// on a review the rigor does not allow. Recorded as a departure from
    /// the rigor, dated `date`, then a volet like any other. Refused where
    /// the rigor would let the plain verb through: there is nothing to set
    /// aside there.
    Overridden { because: String, date: String },
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FlowError {
    #[error("event {event:?} is not valid in stage {stage:?}")]
    InvalidTransition {
        stage: Box<Stage>,
        event: Box<Event>,
    },
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
    /// The rigor does not send the mission back from here: said with what
    /// it would set aside. The caller names the verbs.
    #[error("the mission's rigor does not send it back: {0}")]
    RigorRefuses(Departure),
    /// `--override` where the rigor already lets the verb through.
    #[error(
        "the rigor already lets this through, so there is nothing to override — drop \
         --override"
    )]
    NothingToOverride,
    /// The header asks for a role its rigor does not run. Said where a header
    /// is frozen — at the start of a mission and at a reframe — and not
    /// only by `mission new`, which a header written by hand never went
    /// through.
    #[error(transparent)]
    Rigor(#[from] RigorError),
}

/// The state machine. Serializable, so the engine persists it at every
/// transition (SPEC 4.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flow {
    header: Header,
    stage: Stage,
    /// The cause of every return to the coder over the mission's life, in
    /// order — and, while the flow stands handed over on
    /// [`Handover::VoletsExhausted`], last of all the cause that found no
    /// volet left. It is the count ([`Flow::volets`]): there is no counter
    /// beside it for a verb to reset, so a retry cannot hand the cap back
    /// whole. State written while a `volets` counter stood beside it reads
    /// the same way — that field is ignored, and the causes are counted.
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
    /// Verdicts the security agent has concluded, `CLEAR` or `FINDINGS`:
    /// its rounds, bounded by the mission's rigor
    /// ([`super::Rigor::max_security_rounds`]). `serde(default)` because
    /// state written before the count reads it as zero.
    #[serde(default)]
    security_rounds: u32,
    /// Set when the flow went past the security stage without the agent
    /// because its rounds were spent, and taken once by whoever records it
    /// in the follow-up ([`Flow::take_security_cap`]). This module writes no
    /// file, so it leaves the fact here rather than losing it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    security_cap: Option<SecurityCap>,
    /// The report of the last `FINDINGS` the security agent concluded, while
    /// no `CLEAR` has followed it and no human has lifted it. A spent round cap brings the mission back
    /// to it rather than verifying a branch whose last verdict was red
    /// (SPEC 4.5). `serde(default)` because state written before it reads
    /// as "no findings held".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_findings: Option<String>,
    /// The worst severity the last `FINDINGS` concluded ranks, when every
    /// finding carries one ([`super::VerdictFile::worst`]); `None` when it
    /// ranks nothing, which blocks as a report always did. `serde(default)`
    /// because state written before it reads as unranked, hence blocking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    findings_worst: Option<Severity>,
    /// The rank the caller read for the verdict it is about to apply
    /// ([`Flow::rank`]), taken by that verdict and by nothing else, so a
    /// verdict nobody ranked is unranked rather than ranked as the last one.
    #[serde(skip)]
    ranked: Option<Severity>,
    /// Every time the HQ set the rigor aside, in order, with its reason.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    overrides: Vec<Override>,
}

/// The security agent was not launched because its rounds were spent:
/// `rounds` played, `max` allowed by the rigor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityCap {
    pub rounds: u32,
    pub max: u32,
}

/// What the rigor says not to do with the mission where it stands: the
/// rule an override sets aside (SPEC 4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Departure {
    /// A `FINDINGS` whose worst finding is `worst`, below `HIGH`, at a
    /// `rigor` and `round` where such a report does not block.
    ReportNotBlocking {
        rigor: Rigor,
        round: u32,
        worst: Severity,
    },
    /// A verified mission sent back on review once more, at a `rigor` that
    /// allows one review: `reviews` were already sent back.
    AnotherReview { rigor: Rigor, reviews: u32 },
}

impl std::fmt::Display for Departure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Departure::ReportNotBlocking {
                rigor,
                round,
                worst,
            } => write!(
                f,
                "at {rigor}, security round {round}, a report whose worst finding is {worst} \
                 does not block — only a HIGH does"
            ),
            Departure::AnotherReview { rigor, reviews } => write!(
                f,
                "at {rigor}, a verified mission is sent back on review once, and this one \
                 already was {reviews} time(s)"
            ),
        }
    }
}

/// The HQ set the rigor aside: what it set aside, why, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Override {
    pub departure: Departure,
    pub because: String,
    pub date: String,
}

/// How an HQ review's volet cause starts: what counts the reviews a
/// mission was sent back on, in state written before and after the count.
const REVIEW_CAUSE: &str = "HQ review: ";

impl Flow {
    /// A new flow starts with the first lot, first attempt.
    pub fn new(header: Header) -> Result<Self, FlowError> {
        if header.lots.is_empty() {
            return Err(FlowError::NoLots);
        }
        admitted(&header)?;
        Ok(Self {
            header,
            stage: Stage::Coding {
                work: Work::Lot(0),
                attempt: 1,
            },
            volet_causes: Vec::new(),
            attempts: HashMap::new(),
            resume_with: None,
            security_rounds: 0,
            security_cap: None,
            last_findings: None,
            findings_worst: None,
            ranked: None,
            overrides: Vec::new(),
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
        admitted(&header)?;
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

    /// Returns to the coder taken over the mission's life: the causes
    /// recorded, less the one still waiting for a volet while the flow is
    /// handed over on [`Handover::VoletsExhausted`].
    pub fn volets(&self) -> u32 {
        let recorded = u32::try_from(self.volet_causes.len()).unwrap_or(u32::MAX);
        match self.stage {
            Stage::AwaitingHuman(Handover::VoletsExhausted { .. }) => recorded.saturating_sub(1),
            _ => recorded,
        }
    }

    /// The volets taken past the cap: each one granted by the HQ, one
    /// `retry` at a time.
    pub fn volets_granted(&self) -> u32 {
        self.volets().saturating_sub(self.header.bounds.max_volets)
    }

    /// `n / cap`, and past the cap how many the HQ granted — the count as
    /// `mission status` and `mission wait` print it.
    pub fn volets_said(&self) -> String {
        let (taken, cap) = (self.volets(), self.header.bounds.max_volets);
        match self.volets_granted() {
            0 => format!("{taken} / {cap}"),
            granted => format!("{taken} / {cap} ({granted} granted by the HQ)"),
        }
    }

    /// Rounds the security agent has played: verdicts it concluded.
    pub fn security_rounds(&self) -> u32 {
        self.security_rounds
    }

    /// Rounds the security agent may play on this mission, by its rigor.
    pub fn max_security_rounds(&self) -> u32 {
        self.header.rigor.max_security_rounds()
    }

    /// The round cap the flow ran into past the security stage, once: the
    /// caller records it in the follow-up, and a second call says nothing.
    pub fn take_security_cap(&mut self) -> Option<SecurityCap> {
        self.security_cap.take()
    }

    /// Rank the security verdict about to be applied: the worst severity
    /// its findings carry, or `None` when they carry none `nunki` can read
    /// ([`super::VerdictFile::worst`]). Taken by that verdict only.
    pub fn rank(&mut self, worst: Option<Severity>) {
        self.ranked = worst;
    }

    /// The worst severity of the report the mission stands on, if it was
    /// ranked.
    pub fn findings_worst(&self) -> Option<Severity> {
        self.findings_worst
    }

    /// Every departure from the rigor the HQ made, in order.
    pub fn overrides(&self) -> &[Override] {
        &self.overrides
    }

    /// Verified missions the HQ sent back on review: the volet causes a
    /// review opened, over the mission's life.
    pub fn reviews(&self) -> u32 {
        let reviews = self
            .volet_causes
            .iter()
            .filter(|cause| cause.starts_with(REVIEW_CAUSE))
            .count();
        u32::try_from(reviews).unwrap_or(u32::MAX)
    }

    /// What the rigor refuses about sending the mission back from where it
    /// stands, or `None` when it lets it be sent back (SPEC 4.5).
    ///
    /// On `Findings`: a report blocks when it ranks a `HIGH`, or ranks
    /// nothing (its findings carry no severity, as before severities
    /// existed); otherwise it blocks only on a `critical` mission's first
    /// round. On `Verified`: a `standard` mission is sent back on review
    /// once. Anywhere else there is nothing to send back, and the flow says
    /// so on its own.
    pub fn rigor_refuses(&self) -> Option<Departure> {
        let rigor = self.header.rigor;
        match &self.stage {
            Stage::Findings { .. } => {
                let worst = self.findings_worst?;
                let round = self.security_rounds;
                let blocks = worst >= Severity::High
                    || match rigor {
                        Rigor::Standard => false,
                        Rigor::Critical => round <= 1,
                        // No security agent ever concludes there; nothing
                        // to relax.
                        Rigor::Prototype => true,
                    };
                (!blocks).then_some(Departure::ReportNotBlocking {
                    rigor,
                    round,
                    worst,
                })
            }
            Stage::Verified => {
                let reviews = self.reviews();
                (rigor == Rigor::Standard && reviews >= 1)
                    .then_some(Departure::AnotherReview { rigor, reviews })
            }
            _ => None,
        }
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
            // A ruling the coder may not give: no attempt would change it, so
            // none is spent. The attempt that asked is the one recorded, and
            // the work is kept whole for the retry, as for any handover.
            (Stage::Coding { work, attempt }, Event::RulingAwaited { what, survivors }) => {
                let lot = self.label(&work);
                *self.attempts.entry(lot.clone()).or_insert(0) = attempt;
                self.resume_with = Some(work);
                Stage::AwaitingHuman(Handover::AwaitingRuling {
                    lot,
                    what,
                    attempt,
                    survivors,
                })
            }
            // --- a system profile nobody approved -------------------------
            //
            // Nothing was started and no run was spent, so the attempt is
            // kept: the human's approval is what changes, not the agent's
            // work.
            (Stage::Integration { attempt }, Event::ServicesUnapproved { file, digest }) => {
                Stage::AwaitingHuman(Handover::ServicesNotApproved {
                    role: Role::Integrator,
                    attempt,
                    file,
                    digest,
                })
            }
            (Stage::SecurityAgent { attempt }, Event::ServicesUnapproved { file, digest }) => {
                Stage::AwaitingHuman(Handover::ServicesNotApproved {
                    role: Role::Security,
                    attempt,
                    file,
                    digest,
                })
            }
            // Gates 1 to 4 are played at the end of **every** run, not only
            // at the final verification (SPEC 4.4): a
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
            // Opened by hand here, with `n: self.volets` and no increment,
            // every return would be volet 0 and `max_volets` would never come
            // round: a gate 7 red on survivors that no test can kill would
            // send the coder back again and again — the agent declares its
            // volet done, the gates are played again, the same gate is red
            // again, and the same volet 0 opens again, until the only thing
            // left to stop it is the account's weekly cap.
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
            ) => {
                self.security_rounds += 1;
                self.last_findings = None;
                self.findings_worst = None;
                self.ranked = None;
                self.verified_or_final()
            }
            (
                Stage::SecurityAgent { .. },
                Event::Verdict {
                    verdict: Verdict::Findings,
                    report,
                },
            ) => {
                self.security_rounds += 1;
                self.last_findings = Some(report.clone());
                self.findings_worst = self.ranked.take();
                Stage::Findings { report }
            }
            // The rigor binds the HQ's verb, not only the agents: a report
            // it does not count as blocking is not sent back by the plain
            // verb, and `Overridden` is the way past it that leaves a
            // record (SPEC 4.5).
            (Stage::Findings { report }, Event::Iterate) => {
                if let Some(departure) = self.rigor_refuses() {
                    return Err(FlowError::RigorRefuses(departure));
                }
                self.volet(format!("security FINDINGS: {report}"))
            }
            (Stage::Findings { report }, Event::Overridden { because, date }) => {
                self.overridden(because, date)?;
                self.volet(format!("security FINDINGS: {report}"))
            }
            // A report a human lifted is no longer held: kept, it would come
            // back at the cap after a later HQ review, as if nobody had
            // lifted it, and ask for the same lift on every volet (SPEC 4.5).
            (Stage::Findings { .. }, Event::HumanAccepted) => {
                self.last_findings = None;
                self.verified_or_final()
            }
            // --- the final full campaign, at `critical` ----------------
            //
            // Green on a full campaign at `HEAD`: verified. Red: its
            // survivors go back to the coder as a red gate 7 at the final
            // gates sends them — a volet, bounded like the others — and the
            // full campaign is owed again before `Verified`.
            (Stage::FinalCampaign, Event::GatesPassed) => Stage::Verified,
            (Stage::FinalCampaign, Event::GatesFailed { reason }) => {
                self.volet(format!("final campaign: {reason}"))
            }
            // A verified branch is not a pushed one: the HQ reads it first,
            // and what it refuses goes back as a volet, bounded like the
            // others. With none left, the handover says so and `retry`
            // grants one more. At `standard`, once: a second review sets the
            // rigor aside, and only `Overridden` says so.
            (Stage::Verified, Event::Reviewed { because }) => {
                if let Some(departure) = self.rigor_refuses() {
                    return Err(FlowError::RigorRefuses(departure));
                }
                self.volet(format!("{REVIEW_CAUSE}{because}"))
            }
            (Stage::Verified, Event::Overridden { because, date }) => {
                let cause = format!("{REVIEW_CAUSE}{because}");
                self.overridden(because, date)?;
                self.volet(cause)
            }

            // --- the human takes it back -------------------------------
            //
            // A handover is the flow saying "a bound stopped me, and the
            // decision is yours". `Retried` is that decision. A lot's or a
            // role's attempts it hands back whole — without that the mission
            // would land in the same handover on the very next event. The
            // volets it hands back one at a time: they are counted over the
            // mission's life, so a retry past the cap is one volet the HQ
            // grants, and the next red verdict hands the mission back again.
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
            // Approved since, or not: the same role at the same attempt, and
            // a profile still unapproved hands over again on the next launch.
            (
                Stage::AwaitingHuman(Handover::ServicesNotApproved { role, attempt, .. }),
                Event::Retried { .. },
            ) => self.role_stage(role, attempt),
            // A ruling is not a bound running out, so it hands no budget
            // back: the lot resumes at the attempt after the one that asked.
            // Never past the bound: when the attempt that asked was the last
            // one, the retry hands over as exhausted instead, and the next
            // `retry` — from a bound, this time — hands the attempts back
            // whole. Without that, a coder that asked on every attempt would
            // go on past `attempts_per_lot` one human retry at a time.
            (
                Stage::AwaitingHuman(Handover::AwaitingRuling { lot, attempt, .. }),
                Event::Retried { .. },
            ) => {
                let Some(work) = self.resume_with.clone() else {
                    return Err(FlowError::NothingToResume);
                };
                if attempt >= self.header.bounds.attempts_per_lot {
                    Stage::AwaitingHuman(Handover::LotAttemptsExhausted {
                        lot,
                        attempts: attempt,
                    })
                } else {
                    self.resume_with = None;
                    Stage::Coding {
                        work,
                        attempt: attempt + 1,
                    }
                }
            }
            // The cause that found no volet left gets one, numbered by the
            // mission's whole count: the causes recorded, that one included.
            // Renumbered here rather than read from `resume_with`, where
            // state written before the count was the causes' holds volet 1.
            (Stage::AwaitingHuman(Handover::VoletsExhausted { .. }), Event::Retried { .. }) => {
                let Some(Work::Volet { cause, .. }) = self.resume_with.clone() else {
                    return Err(FlowError::NothingToResume);
                };
                self.resume_with = None;
                Stage::Coding {
                    work: Work::Volet {
                        n: u32::try_from(self.volet_causes.len()).unwrap_or(u32::MAX),
                        cause,
                    },
                    attempt: 1,
                }
            }
            (Stage::AwaitingHuman(_), Event::Retried { .. }) => {
                let Some(work) = self.resume_with.clone() else {
                    return Err(FlowError::NothingToResume);
                };
                // A lot's attempts, or a volet's: the work resumes as it was,
                // and the volet count — the causes — is left alone.
                //
                // Nothing resets `attempts`: the bound is read from the
                // stage's own `attempt`, which starts at 1 below, and the map
                // is written but never read (measured — removing a reset here
                // changed no test, because there is nothing to change).
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

            (stage, event) => {
                return Err(FlowError::InvalidTransition {
                    stage: Box::new(stage),
                    event: Box::new(event),
                });
            }
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
    fn after_gates(&mut self) -> Stage {
        if self.header.has_integration() {
            Stage::Integration { attempt: 1 }
        } else {
            self.security_or_verified()
        }
    }

    fn after_integration(&mut self) -> Stage {
        self.security_or_verified()
    }

    /// The security agent, when the mission declares one and it has a round
    /// left; otherwise `Verified`, or the last findings. An `iterate` after
    /// the last round still runs and is gated as usual; only the next round
    /// is not played, and the cap is left for the follow-up to record
    /// (SPEC 4.5).
    ///
    /// A spent cap never turns a red verdict green. When the last verdict
    /// concluded is `FINDINGS`, the mission goes back to it, with its
    /// report, so that the verbs a `FINDINGS` already has decide: `accept`
    /// lifts it, `iterate` spends a volet. Verifying it here would make
    /// `Verified` reachable with no `CLEAR` and no human lift, and the
    /// volet that answered the findings has not been attacked again. The
    /// same holds whichever way the coder got here — an iterate, a review,
    /// a retry — because the rule reads the verdict, not the way back.
    fn security_or_verified(&mut self) -> Stage {
        if !self.header.has_security_agent() {
            return self.verified_or_final();
        }
        let max = self.max_security_rounds();
        if self.security_rounds >= max {
            self.security_cap = Some(SecurityCap {
                rounds: self.security_rounds,
                max,
            });
            return match &self.last_findings {
                Some(report) => Stage::Findings {
                    report: report.clone(),
                },
                None => self.verified_or_final(),
            };
        }
        Stage::SecurityAgent { attempt: 1 }
    }

    /// Where a mission whose every other stage is green goes: `Verified`,
    /// or at `critical` the final full campaign first (SPEC 4.4, 4.5), which
    /// the flow cannot judge itself — whether the campaign on file is full
    /// is the file's to say, and `verify` reads it there.
    fn verified_or_final(&self) -> Stage {
        if self.header.rigor == super::Rigor::Critical {
            Stage::FinalCampaign
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

    /// Record the HQ setting the rigor aside where it stands, or refuse
    /// when the rigor sets nothing in the way: an override that departs
    /// from nothing would be a record of a departure that never happened.
    fn overridden(&mut self, because: String, date: String) -> Result<(), FlowError> {
        let departure = self.rigor_refuses().ok_or(FlowError::NothingToOverride)?;
        self.overrides.push(Override {
            departure,
            because,
            date,
        });
        Ok(())
    }

    /// A red verdict: back to the coder, bounded by `max_volets`. The volet
    /// replays the gates and every declared stage after it (SPEC 4.5, "un
    /// verdict vaut pour un HEAD").
    ///
    /// Bounded over the mission's life: once `max_volets` are taken, every
    /// further cause hands the mission back, so a volet past the cap is only
    /// ever one that a `retry` granted.
    fn volet(&mut self, cause: String) -> Stage {
        let taken = self.volets();
        self.volet_causes.push(cause.clone());
        let n = taken.saturating_add(1);
        if taken >= self.header.bounds.max_volets {
            // The cause that arrived with no volet left to open for it: the
            // one a retry starts from.
            self.resume_with = Some(Work::Volet { n, cause });
            return Stage::AwaitingHuman(Handover::VoletsExhausted {
                causes: self.volet_causes.clone(),
            });
        }
        Stage::Coding {
            work: Work::Volet { n, cause },
            attempt: 1,
        }
    }
}

/// Refuse a header whose rigor does not run the roles it declares
/// ([`super::Rigor::admits`]): a prototype with services or a security
/// agent. `mission new` already says it, but a header can be written by
/// hand and frozen without going through it.
fn admitted(header: &Header) -> Result<(), FlowError> {
    header
        .rigor
        .admits(header.has_integration(), header.has_security_agent())?;
    Ok(())
}
