//! Re-framing a mission, and closing one (SPEC 4.1 rule 3, 4.2 verb table).
//!
//! Two verbs at opposite ends of a mission's life, together because they are
//! the two moments `hq` touches its framing rather than its work.
//!
//! **`reframe`** exists because `hq` never re-reads the header during a
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
         agent that is inside it — `hq mission stop {mission}` ends its turn first"
    )]
    RunInProgress { mission: String, slot: String },
    #[error(
        "mission {mission} is at {stage:?}. A mission is archived once it is finished — \
         verified and pushed, or handed back to you and left there"
    )]
    NotFinished { mission: String, stage: Stage },
    #[error("{0} already exists: this mission has been archived once")]
    AlreadyArchived(PathBuf),
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
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
/// verdicts are the record of what was done, and `hq` deletes nothing it did
/// not create (SPEC 3.3). The slot is left alone: `hq slot reset` and
/// `hq slot rm` are the verbs for a slot, and archiving a mission is not one
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
