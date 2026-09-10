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
//!   unfreezes it exactly there — and `resume` unfreezes only what is
//!   actually frozen, because the engine refuses to unpause a container that
//!   is merely running | nothing. A model call in flight may time out during
//!   a long freeze, and the harness replays it |
//! | `kill` | the emergency brake: the container is killed, nothing is
//!   waited for | nothing, it no longer exists |
//!
//! `stop` is the fourth, and it is two things at once (SPEC 4.5). Always, it
//! **holds the mission**: `hq` launches no further run for it, which is a
//! write to the state file and not a signal — the old `STOP` file, which was
//! a marker and not a gesture. With `--now` it also ends the turn in
//! progress, and that half is the harness's, because interrupting a turn
//! cleanly is a signal to a process. Without it, the run in progress finishes
//! its lot and nothing follows it. `resume` lifts the hold.
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
    Harness(#[from] crate::harness::HarnessError),
    #[error(transparent)]
    State(#[from] crate::state::StateError),
    #[error(transparent)]
    Compose(#[from] crate::compose::ComposeError),
    #[error(transparent)]
    Followup(#[from] crate::followup::FollowupError),
}

/// Freeze the agent's container where it is.
///
/// The firewall goes with it. A frozen agent behind a live sidecar is a pair
/// where one half can still answer and the other cannot — and the sidecar
/// exists to fence the agent, so nothing is gained by leaving it running.
///
/// There is no `unfreeze` beside it: `resume` unfreezes, because unfreezing
/// and lifting a hold are one gesture to the human, and because only one
/// place should decide that a container is frozen before asking the engine
/// to unfreeze it.
pub fn pause(
    project: &Project,
    id: &str,
    engine: Arc<dyn Engine>,
) -> Result<MissionState, GestureError> {
    let (state, file, compose_project) = target(project, id)?;
    engine.pause(&file, &compose_project, &crate::run::SERVICES)?;
    Ok(state)
}

/// Hold the mission, and with `now` end the turn in progress too.
///
/// The hold is the whole point and it is written to the state file: `hq`
/// launches no further run until `resume` lifts it. It is recorded even when
/// no run is going — a human holding a mission between two runs is exactly
/// the case a signal cannot express, and the case SPEC's old `STOP` file
/// existed for.
///
/// `now` is the other half: SIGINT, not SIGTERM, so the agent ends its turn
/// and writes its resume block (SPEC 4.3). It is only ever the second
/// clause — a `--now` typed a second after the run ended still holds the
/// mission, and the record says the interruption did not happen rather than
/// refusing the hold along with it.
pub fn stop(
    project: &Project,
    id: &str,
    harness: &dyn crate::harness::Harness,
    now: bool,
) -> Result<crate::state::Stopped, GestureError> {
    let store = Store::open(&project.hq_root)?;
    let mut state = started(project, id)?;

    let interrupted = match (now, state.run.clone()) {
        (true, Some(handle)) => {
            harness.stop(&handle)?;
            true
        }
        _ => false,
    };

    let who = crate::human::me(&project.hq_home(), Some(&project.root)).addressed();
    state.hold(&who, interrupted);
    store.save(&state)?;
    // `expect`: `hold` has just set it.
    Ok(state.stopped.expect("hold sets the hold"))
}

/// Lift the hold, and unfreeze the container if one is frozen.
///
/// One verb for both because they are one thing to the human who types it:
/// the mission was held, and it is held no longer. SPEC 4.5 names `resume`
/// as `pause`'s antonym and calls a stopped mission "reprenable" without
/// saying by which verb; the pairing was derived from that, then confirmed
/// by Arnaud on 2026-09-10.
///
/// It does not require a run: a mission held between two runs has none, and
/// that is precisely a mission worth resuming.
pub fn resume(
    project: &Project,
    id: &str,
    engine: Arc<dyn Engine>,
) -> Result<Option<crate::state::Stopped>, GestureError> {
    let store = Store::open(&project.hq_root)?;
    let mut state = started(project, id)?;
    let lifted = state.release();
    store.save(&state)?;

    // Only a frozen container is unfrozen. Measured on Docker 28 the day
    // this was written: `unpause` on a container that is merely running is
    // refused ("is not paused", exit 1) — and after a bare `stop` the run
    // keeps going, so that is the *common* path here, not the odd one. The
    // engine already has the word for the distinction (`Liveness::Paused`),
    // which is what it was given one for.
    if let Some(handle) = &state.run
        && engine.liveness(&handle.container)? == crate::engine::Liveness::Paused
    {
        let file = crate::run::profile_path(project, &state.slot);
        let compose_project = crate::compose::project_name(&state.slot)?;
        engine.unpause(&file, &compose_project, &crate::run::SERVICES)?;
    }
    Ok(lifted)
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

/// A mission that has started, or the one sentence that says it has not.
/// Public because a caller often needs the state before it can build the
/// harness the gesture acts through, and two places saying "has not started"
/// in two ways is two answers to one question.
pub fn started(project: &Project, id: &str) -> Result<MissionState, GestureError> {
    Store::open(&project.hq_root)?
        .load(id)
        .map_err(|_| GestureError::NotStarted(id.to_string()))
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
