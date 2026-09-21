//! Re-framing a mission, and closing one (SPEC 4.1 rule 3, 4.2 verb table).
//!
//! Two verbs at opposite ends of a mission's life, together because they are
//! the two moments `nunki` touches its framing rather than its work.
//!
//! **`reframe`** exists because `nunki` never re-reads the header during a
//! mission: it froze it when the human validated the framing, and that copy
//! is what generates every Compose file and every allowlist. So a `MISSION.md`
//! edited behind its back changes nothing and looks as though it did. The
//! verb puts the framing back in front of the human — it says what would
//! change and does nothing — and re-freezes only when told to.
//!
//! **`archive`** is the word for what comes after the push. `verify` is the
//! verb for the verification phase and its answer is `VERIFIED`; closing is a
//! separate act, and it is this one.

use std::path::PathBuf;

use crate::mission::Header;
use crate::mission::dir::Paths;
use crate::mission::flow::Stage;
use crate::project::Project;
use crate::state::{MissionState, Store};

#[derive(Debug, thiserror::Error)]
pub enum LifecycleError {
    #[error("mission {0} has not started — edit MISSION.md; nothing is frozen yet")]
    NotStarted(String),
    #[error(
        "a run is going in slot {slot}: reframing now would change the perimeter under an \
         agent that is inside it — `nunki mission stop {mission}` ends its turn first"
    )]
    RunInProgress { mission: String, slot: String },
    #[error(
        "mission {mission} is at {stage:?}. A mission is archived once it is finished — \
         verified and pushed, or handed back to you and left there"
    )]
    NotFinished { mission: String, stage: Stage },
    #[error("{0} already exists: this mission has been archived once")]
    AlreadyArchived(PathBuf),
    #[error(
        "say why: a mission called off without a reason is a puzzle for whoever finds it \
         six months from now"
    )]
    NoReason,
    #[error(
        "say what changed: a retry hands the agent back the work it already failed, on \
         the same tree and the same cause — without that, it spends the budget again \
         reaching the same handover"
    )]
    NoChange,
    #[error(
        "mission {mission} is at {stage:?}, and `retry` takes back a mission that stopped \
         on its own bounds — there is nothing to take back here"
    )]
    NotHandedOver { mission: String, stage: Stage },
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error(transparent)]
    Followup(#[from] crate::followup::FollowupError),
    #[error(transparent)]
    Mission(#[from] crate::mission::dir::MissionDirError),
    #[error(transparent)]
    State(#[from] crate::state::StateError),
    #[error(transparent)]
    Flow(#[from] crate::mission::flow::FlowError),
}

/// One thing the new framing changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub what: &'static str,
    pub from: String,
    pub to: String,
}

/// What `reframe` found, and whether it applied it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reframed {
    pub changes: Vec<Change>,
    /// `false` when the human has not said to apply it yet.
    pub applied: bool,
}

/// Put the framing back in front of the human, and re-freeze it on their word.
pub fn reframe(project: &Project, id: &str, apply: bool) -> Result<Reframed, LifecycleError> {
    let store = Store::open(&project.hq_root)?;
    let mut state = store
        .load(id)
        .map_err(|_| LifecycleError::NotStarted(id.to_string()))?;
    if state.run.is_some() {
        return Err(LifecycleError::RunInProgress {
            mission: id.to_string(),
            slot: state.slot.clone(),
        });
    }

    let fresh = crate::mission::dir::read_header(&project.hq_root, id)?;
    let changes = differences(state.flow.header(), &fresh);
    if !apply || changes.is_empty() {
        return Ok(Reframed {
            changes,
            applied: false,
        });
    }
    state.flow.reframe(fresh)?;
    store.save(&state)?;
    Ok(Reframed {
        changes,
        applied: true,
    })
}

/// What the two framings disagree about, in the order a human reads a header.
///
/// Field by field rather than as a text diff: what matters is which
/// **decision** moved, and a diff of the serialised header would report a
/// reordered list as a change and a renumbered lot as two.
pub fn differences(frozen: &Header, fresh: &Header) -> Vec<Change> {
    let mut changes = Vec::new();
    let mut note = |what: &'static str, from: String, to: String| {
        if from != to {
            changes.push(Change { what, from, to });
        }
    };
    note("branch", frozen.branch.clone(), fresh.branch.clone());
    note("base", frozen.base.clone(), fresh.base.clone());
    note("lots", lots(frozen), lots(fresh));
    note(
        "integration",
        format!("{:?}", frozen.integration),
        format!("{:?}", fresh.integration),
    );
    note(
        "security",
        format!("{:?}", frozen.security),
        format!("{:?}", fresh.security),
    );
    note(
        "arbiter",
        frozen.arbiter.clone().unwrap_or_else(|| "—".into()),
        fresh.arbiter.clone().unwrap_or_else(|| "—".into()),
    );
    note(
        "account",
        frozen.account.clone().unwrap_or_else(|| "—".into()),
        fresh.account.clone().unwrap_or_else(|| "—".into()),
    );
    note(
        "model",
        frozen.model.clone().unwrap_or_else(|| "—".into()),
        fresh.model.clone().unwrap_or_else(|| "—".into()),
    );
    note(
        "run",
        frozen.run.clone().unwrap_or_else(|| "—".into()),
        fresh.run.clone().unwrap_or_else(|| "—".into()),
    );
    note(
        "bounds",
        format!("{:?}", frozen.bounds),
        format!("{:?}", fresh.bounds),
    );
    changes
}

fn lots(header: &Header) -> String {
    header
        .lots
        .iter()
        .map(|l| format!("{} {}", l.id, l.title))
        .collect::<Vec<_>>()
        .join("; ")
}

/// Call a mission off before it is verified (SPEC 4.2, verb table).
///
/// The one verb in SPEC's list that the list never explains, and this
/// definition is **derived rather than quoted**: `stop` ends a run, `archive`
/// closes a finished mission, and between them sat a mission a human has
/// given up on — still `Coding`, never to be verified, and impossible to
/// close. `end` is what closes it, and after it `archive` will.
///
/// It refuses on a mission that is already over, because "call it off" is not
/// a thing to say twice, and it takes a reason for the same purpose every
/// other reason in `nunki` is taken for: six months from now, "abandoned" alone
/// says nothing.
pub fn end(project: &Project, id: &str, why: &str) -> Result<MissionState, LifecycleError> {
    if why.trim().is_empty() {
        return Err(LifecycleError::NoReason);
    }
    let store = Store::open(&project.hq_root)?;
    let mut state = store
        .load(id)
        .map_err(|_| LifecycleError::NotStarted(id.to_string()))?;
    if state.run.is_some() {
        return Err(LifecycleError::RunInProgress {
            mission: id.to_string(),
            slot: state.slot.clone(),
        });
    }
    // Written where a human reads it, and before the transition: a mission
    // called off leaves a record of why, or it leaves a puzzle.
    let paths = Paths::of(&project.hq_root, id);
    let who = crate::human::me(&project.nunki_home(), Some(&project.root)).addressed();
    crate::followup::ended(&paths.followup, &who, why.trim())?;
    store.apply(
        &mut state,
        crate::mission::flow::Event::Ended {
            reason: why.trim().to_string(),
        },
    )?;
    Ok(state)
}

/// Take a mission back from a handover, and say what changed.
///
/// A handover is the flow saying "a bound stopped me, and the decision is
/// yours" (SPEC 4.5). Every way of reaching one is a bound running out, and
/// until this verb existed there was no way out of it: `resume` lifts a hold,
/// `iterate` and `accept` only leave `Findings`, and `monitor::wanted` gives
/// up on the stage — so a mission that exhausted its volets was over without
/// being finished.
///
/// The reason is required and it is not paperwork: it is written to
/// `FOLLOWUP_HQ.md`, which every role reads before anything else, because the
/// tree has not changed and neither has the cause. A retry that says nothing
/// buys the same handover a second time.
///
/// **It resumes the work, and does not ask whether the work is already done.**
/// That looks wasteful when the cause was outside the mission: on 2026-09-20,
/// a defect in gate 3 exhausted a lot whose commit was correct all along, and
/// the retry sent a coder to re-examine a tree that already passed every gate.
///
/// It stays that way on purpose. The handover says a bound ran out on *this*
/// work, and nothing on disk distinguishes "the lot is finished and something
/// else was wrong" from "the lot is finished in the journal and is not". The
/// flow could play the gates on entering `Coding` and advance when they are
/// green — gate 3 would catch a stale `done` from an older commit — but that
/// is a change to the mission's state machine to save one cheap run, in a
/// case that arises only when `nunki` itself was wrong.
///
/// What carries the difference instead is the reason above, and it works:
/// measured the same day, the coder read it, checked the tree itself, wrote
/// no code, and closed the volet in 3545 output tokens — the shortest run of
/// that mission by an order of magnitude.
pub fn retry(project: &Project, id: &str, why: &str) -> Result<MissionState, LifecycleError> {
    if why.trim().is_empty() {
        return Err(LifecycleError::NoChange);
    }
    let store = Store::open(&project.hq_root)?;
    let mut state = store
        .load(id)
        .map_err(|_| LifecycleError::NotStarted(id.to_string()))?;
    if state.run.is_some() {
        return Err(LifecycleError::RunInProgress {
            mission: id.to_string(),
            slot: state.slot.clone(),
        });
    }
    // Said before the transition is attempted, so that a mission which is
    // simply not stopped is told that, rather than an invalid transition.
    let Stage::AwaitingHuman(handover) = state.flow.stage().clone() else {
        return Err(LifecycleError::NotHandedOver {
            mission: id.to_string(),
            stage: state.flow.stage().clone(),
        });
    };

    // Refused before anything is written. The record below is read by the next
    // run as a statement of fact — "the bounds are handed back whole" — and a
    // retry the flow refuses leaves the mission exactly where it was. Measured
    // on 2026-09-18 against a real HQ: five records in `FOLLOWUP_HQ.md` for
    // three retries, the other two about a mission that had not moved.
    //
    // On a copy, so that the order the comment below asks for still holds: the
    // record lands before the state does. The transition is a pure function of
    // the flow, so playing it twice on the same flow answers the same thing —
    // and asking the flow beats restating its rules here, which is why the
    // check above stops at "not handed over".
    state
        .flow
        .clone()
        .advance(crate::mission::flow::Event::Retried {
            because: why.trim().to_string(),
        })?;

    // Written where the agent reads, and before the transition: a retry whose
    // record did not land is a retry the next run cannot act on.
    let paths = Paths::of(&project.hq_root, id);
    let who = crate::human::me(&project.nunki_home(), Some(&project.root)).addressed();
    crate::followup::retried(&paths.followup, &who, why.trim(), &what_stopped(&handover))?;
    store.apply(
        &mut state,
        crate::mission::flow::Event::Retried {
            because: why.trim().to_string(),
        },
    )?;
    Ok(state)
}

/// A handover in one clause, for the record the agent reads.
fn what_stopped(handover: &crate::mission::flow::Handover) -> String {
    use crate::mission::flow::Handover;
    match handover {
        Handover::LotAttemptsExhausted { lot, attempts } => {
            format!("{lot} failed {attempts} attempt(s)")
        }
        Handover::RoleAttemptsExhausted { role, attempts } => {
            format!("the {role:?} failed {attempts} attempt(s)")
        }
        Handover::VoletsExhausted { causes } => {
            format!("{} return(s) to the coder were used", causes.len())
        }
        Handover::Abandoned { reason } => format!("it was called off ({reason})"),
    }
}

/// Where an archived mission goes.
pub fn archive_dir(project: &Project) -> PathBuf {
    project.hq_root.join("archive")
}

/// What was archived, for the caller to print.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Archived {
    pub id: String,
    pub at: PathBuf,
}

/// Close a mission: the folder and its state move under `archive/`.
///
/// Moved, never deleted — the journals, the pull request text and the
/// verdicts are the record of what was done, and `nunki` deletes nothing it did
/// not create (SPEC 3.3). The slot is left alone: `nunki slot reset` and
/// `nunki slot rm` are the verbs for a slot, and archiving a mission is not one
/// of them.
pub fn archive(project: &Project, id: &str) -> Result<Archived, LifecycleError> {
    let store = Store::open(&project.hq_root)?;
    let state = store
        .load(id)
        .map_err(|_| LifecycleError::NotStarted(id.to_string()))?;
    if state.run.is_some() {
        return Err(LifecycleError::RunInProgress {
            mission: id.to_string(),
            slot: state.slot.clone(),
        });
    }
    if !finished(&state) {
        return Err(LifecycleError::NotFinished {
            mission: id.to_string(),
            stage: state.flow.stage().clone(),
        });
    }

    let at = archive_dir(project).join(id);
    if at.exists() {
        return Err(LifecycleError::AlreadyArchived(at));
    }
    std::fs::create_dir_all(archive_dir(project))
        .map_err(|e| LifecycleError::Io(archive_dir(project), e))?;

    let paths = Paths::of(&project.hq_root, id);
    std::fs::rename(&paths.dir, &at).map_err(|e| LifecycleError::Io(paths.dir.clone(), e))?;
    // The state goes with the folder, in the folder: an archived mission is
    // one thing to keep or to move, not two that have to be found again.
    let from = store.root().join("missions").join(format!("{id}.json"));
    if from.is_file() {
        let to = at.join("state.json");
        std::fs::rename(&from, &to).map_err(|e| LifecycleError::Io(from, e))?;
    }
    Ok(Archived {
        id: id.to_string(),
        at,
    })
}

/// The two places a mission ends: verified, or handed back to the human and
/// left there. Anything else is a mission still being worked on, and
/// archiving it would hide it rather than close it.
fn finished(state: &MissionState) -> bool {
    matches!(
        state.flow.stage(),
        Stage::Verified | Stage::AwaitingHuman(_)
    )
}
