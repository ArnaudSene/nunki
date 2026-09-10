//! Lifting a security verdict, and sending it back to the coder (SPEC 4.5).
//!
//! "Rien ne se pousse en rouge. Il n'existe pas de drapeau pour passer
//! outre." A `FINDINGS` verdict closes one of three ways, and two of them are
//! gestures a human makes:
//!
//! - **corrected** — the mission goes back to the coder as a volet, bounded
//!   by `max_volets`. That is [`iterate`], and it is a gesture rather than a
//!   default because a session that iterates by itself spends a volet the
//!   human might have wanted to spend on an acceptance instead;
//! - **lifted as a risk**, by a human, with a reason. That is [`accept`].
//!
//! Neither ever touches `VERDICT.json`. The verdict says what the agent
//! found; `hq`'s state says what the human decided; and `hq push` reads the
//! decision there. Letting a lift edit the verdict would turn a red answer
//! into a green one, which is the one thing this whole section exists to
//! prevent.

use crate::mission::dir::Paths;
use crate::mission::flow::{Event, Stage};
use crate::project::Project;
use crate::state::{Accepted, MissionState, Store, lock::SlotLock};

/// What a human is lifting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lift {
    /// One finding, named as the report names it. The verdict stays where it
    /// is: itemising is not concluding.
    Finding(String),
    /// What the report still carries. This is the one that moves the mission
    /// to `Verified`.
    Verdict,
}

#[derive(Debug, thiserror::Error)]
pub enum FindingsError {
    #[error("mission {0} has not started — nothing has reported a finding yet")]
    NotStarted(String),
    #[error(
        "mission {mission} is at {stage:?}, not on a security verdict — there is nothing \
         to lift or to send back yet"
    )]
    NotOnFindings { mission: String, stage: Stage },
    #[error(
        "a risk accepted without a reason is not accepted, it is forgotten — say why \
         with `--because`"
    )]
    NoReason,
    #[error("hq does not know who you are: `hq whoami` says where it looks")]
    NoHuman,
    #[error(transparent)]
    State(#[from] crate::state::StateError),
    #[error(transparent)]
    Lock(#[from] crate::state::LockError),
    #[error(transparent)]
    Slot(#[from] crate::slot::SlotError),
    #[error(transparent)]
    Git(#[from] crate::git::GitError),
    #[error(transparent)]
    Followup(#[from] crate::followup::FollowupError),
}

/// Record a human's acceptance, and — for [`Lift::Verdict`] — conclude.
pub fn accept(
    project: &Project,
    id: &str,
    lift: Lift,
    why: &str,
) -> Result<MissionState, FindingsError> {
    if why.trim().is_empty() {
        return Err(FindingsError::NoReason);
    }
    let who = crate::human::me(&project.hq_home(), Some(&project.root))
        .name
        .ok_or(FindingsError::NoHuman)?;

    let store = Store::open(&project.hq_root)?;
    let mut state = load(&store, id)?;
    let _lock = SlotLock::acquire(
        &project.hq_root.join("locks"),
        &state.slot,
        "mission accept",
    )?;
    on_findings(id, &state)?;

    let slot = crate::slot::find(project, &state.slot)?;
    let head = crate::git::head(&slot.tree)?;
    let paths = Paths::of(&project.hq_root, id);

    match &lift {
        Lift::Finding(finding) => {
            crate::followup::lifted(&paths.followup, &who, finding, why, &head)?;
        }
        Lift::Verdict => {
            crate::followup::lifted_all(&paths.followup, &who, why, &head)?;
        }
    }
    state.accepted.push(Accepted {
        finding: match &lift {
            Lift::Finding(f) => Some(f.clone()),
            Lift::Verdict => None,
        },
        why: why.trim().to_string(),
        who,
        head,
        date: crate::state::now_rfc3339(),
    });
    // The file is written before the state, and the state before the
    // transition: a session that dies in between leaves a record of what the
    // human said, never a mission that moved on a decision nobody can read.
    store.save(&state)?;
    if lift == Lift::Verdict {
        store.apply(&mut state, Event::HumanAccepted)?;
    }
    Ok(state)
}

/// Send the mission back to the coder, bounded by `max_volets`.
pub fn iterate(project: &Project, id: &str) -> Result<MissionState, FindingsError> {
    let store = Store::open(&project.hq_root)?;
    let mut state = load(&store, id)?;
    let _lock = SlotLock::acquire(
        &project.hq_root.join("locks"),
        &state.slot,
        "mission iterate",
    )?;
    on_findings(id, &state)?;
    store.apply(&mut state, Event::Iterate)?;
    Ok(state)
}

fn load(store: &Store, id: &str) -> Result<MissionState, FindingsError> {
    store
        .load(id)
        .map_err(|_| FindingsError::NotStarted(id.to_string()))
}

fn on_findings(id: &str, state: &MissionState) -> Result<(), FindingsError> {
    match state.flow.stage() {
        Stage::Findings { .. } => Ok(()),
        stage => Err(FindingsError::NotOnFindings {
            mission: id.to_string(),
            stage: stage.clone(),
        }),
    }
}
