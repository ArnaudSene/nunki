//! The human's gestures on a run (SPEC 4.3, "les trois gestes de l'humain sur
//! un agent").
//!
//! They apply to the **container**, never to a hook the agent could fail to
//! see, and they **always pass** — slot lock or no slot lock, from any
//! terminal. That is deliberate: if the HQ session is dead, the human types
//! them themselves, and a verb that waited on a lock held by a dead session
//! would be a brake that does not work when the brake is needed.
//!
//! | verb | what it does | what the agent sees |
//! |---|---|---|
//! | `pause` / `resume` | freezes the container where it is; `resume`
//!   unfreezes it exactly there | nothing. A model call in flight may time
//!   out during a long freeze, and the harness replays it |
//! | `kill` | the emergency brake: the container is killed, nothing is
//!   waited for | nothing, it no longer exists |
//!
//! `stop` is the fourth and lives with the harness, because ending a turn
//! cleanly is a signal to the process and not a gesture on the container.
//!
//! `say` is here for the opposite reason: **there is no channel during a
//! run**. An instruction is left in `FOLLOWUP_HQ.md` and read by the next
//! run; if it is urgent, `hq mission stop --now` ends the current run
//! properly and the relaunch carries it.

use std::sync::Arc;

use crate::engine::Engine;
use crate::project::Project;
use crate::state::{MissionState, Store};

#[derive(Debug, thiserror::Error)]
pub enum GestureError {
    #[error("mission {0} has not started — there is no run to act on")]
    NotStarted(String),
    #[error("mission {0} has no run in progress")]
    NoRun(String),
    #[error("say what: an empty instruction is not one")]
    NothingSaid,
    #[error(transparent)]
    Engine(#[from] crate::engine::EngineError),
    #[error(transparent)]
    State(#[from] crate::state::StateError),
    #[error(transparent)]
    Compose(#[from] crate::compose::ComposeError),
    #[error(transparent)]
    Followup(#[from] crate::followup::FollowupError),
}

/// Which way the freeze goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freeze {
    On,
    Off,
}

/// Freeze the agent's container, or unfreeze it exactly where it was.
///
/// The firewall goes with it. A frozen agent behind a live sidecar is a pair
/// where one half can still answer and the other cannot — and the sidecar
/// exists to fence the agent, so nothing is gained by leaving it running.
pub fn freeze(
    project: &Project,
    id: &str,
    engine: Arc<dyn Engine>,
    which: Freeze,
) -> Result<MissionState, GestureError> {
    let (state, file, compose_project) = target(project, id)?;
    match which {
        Freeze::On => engine.pause(&file, &compose_project, &crate::run::SERVICES)?,
        Freeze::Off => engine.unpause(&file, &compose_project, &crate::run::SERVICES)?,
    }
    Ok(state)
}

/// The emergency brake.
///
/// After a kill the lot in progress is a failed attempt, and the relaunch
/// starts from the last resume block the agent wrote (SPEC 4.3). Nothing is
/// recorded here: what a killed run costs is the flow's to decide, on the
/// `Stalled` event a watching `hq` raises, and writing it from a verb that
/// must always pass would be two places deciding one thing.
pub fn kill(
    project: &Project,
    id: &str,
    engine: Arc<dyn Engine>,
) -> Result<MissionState, GestureError> {
    let (state, file, compose_project) = target(project, id)?;
    engine.kill(&file, &compose_project, &crate::run::SERVICES)?;
    Ok(state)
}

/// Leave an instruction for the **next** run.
///
/// Not for this one: there is no channel during a run, and pretending
/// otherwise would be a message the agent never reads. It lands in
/// `FOLLOWUP_HQ.md`, which every role is told to read before anything else.
pub fn say(project: &Project, id: &str, what: &str) -> Result<(), GestureError> {
    if what.trim().is_empty() {
        return Err(GestureError::NothingSaid);
    }
    let store = Store::open(&project.hq_root)?;
    // Not required to have started: an instruction left before the first run
    // is read by the first run, which is exactly what it is for.
    let _ = store.load(id);
    let paths = crate::mission::dir::Paths::of(&project.hq_root, id);
    let who = crate::human::me(&project.hq_home(), Some(&project.root)).addressed();
    crate::followup::said(&paths.followup, &who, what.trim())?;
    Ok(())
}

/// The mission's run, its profile, and its Compose project — or the reason
/// there is nothing to act on.
fn target(
    project: &Project,
    id: &str,
) -> Result<(MissionState, std::path::PathBuf, String), GestureError> {
    let store = Store::open(&project.hq_root)?;
    let state = store
        .load(id)
        .map_err(|_| GestureError::NotStarted(id.to_string()))?;
    if state.run.is_none() {
        return Err(GestureError::NoRun(id.to_string()));
    }
    let file = crate::run::profile_path(project, &state.slot);
    let compose_project = crate::compose::project_name(&state.slot)?;
    Ok((state, file, compose_project))
}
