//! `nunki exec <slot> <cmd>` (SPEC 4.2): run a command in a slot's container.
//!
//! This is how the HQ **replays a proof** without having the stack on the
//! host — and, later, how gates 6 and 7 run at all (SPEC 4.4).
//!
//! It runs by default on a **clean git copy of `HEAD`**, never on the tree
//! the agent has been living in. The reason is written in SPEC 4.2 and it is
//! not hypothetical: a battery replayed in a tree where an agent left a
//! `Makefile`, a `pytest.ini` or a cargo alias proves what that tree does,
//! not what the commit does. The copy is a fresh checkout of a commit, which
//! is the whole answer: nothing uncommitted can reach it.
//!
//! The copy keeps **its own build cache**, warmed once per slot and kept
//! (SPEC 4.4, gate 7). It lives in a named volume, so it survives the
//! container, and the refresh deliberately does not use `git clean -x`:
//! that would delete `target/` and every campaign would recompile from cold.

use std::path::PathBuf;
use std::sync::Arc;

use crate::compose::AGENT_SERVICE;
use crate::engine::{Engine, EngineError, ExecOutput};
use crate::git;
use crate::project::Project;
use crate::slot::Slot;

/// Where the clean copy of `HEAD` is mounted, in every profile.
pub const PROOF_AT: &str = "/work/proof";

/// The named volume that holds it, per slot: the copy and its build cache
/// outlive the container, which is what "warmed once per slot" means.
pub fn proof_volume(slot: &str) -> String {
    format!("nunki-{slot}-proof")
}

/// Which tree a command runs against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum On {
    /// The clean copy of `HEAD` — the default, and the only one a proof
    /// should ever be replayed on.
    Proof,
    /// The working tree the agent inhabits. Never during a run (SPEC 4.2).
    Tree,
}

#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    #[error(
        "slot {0:?} has no profile up — `nunki mission start` lifts one, or \
         `nunki slot rebuild` if its images are stale"
    )]
    NoProfile(String),
    #[error("the copy of HEAD could not be refreshed:\n{0}")]
    Refresh(String),
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error(transparent)]
    Git(#[from] git::GitError),
    #[error(transparent)]
    Compose(#[from] crate::compose::ComposeError),
}

/// Run `argv` in the slot's agent container and give back what it said.
pub fn run(
    project: &Project,
    slot: &Slot,
    engine: Arc<dyn Engine>,
    argv: &[String],
    on: On,
) -> Result<ExecOutput, ExecError> {
    let file = crate::run::profile_path(project, &slot.name);
    if !file.is_file() {
        return Err(ExecError::NoProfile(slot.name.clone()));
    }
    let compose_project = crate::compose::project_name(&project.session(), &slot.name)?;

    let at = match on {
        On::Proof => {
            let head = git::head(&slot.tree)?;
            refresh_at(&engine, &file, &compose_project, &head)?;
            PROOF_AT.to_string()
        }
        On::Tree => crate::run::TREE_AT.to_string(),
    };
    // The engine's `exec` has no working directory of its own, and adding
    // one to the trait for this would put a shell's job in the boundary.
    // Arguments travel as arguments — `"$@"` — so nothing is quoted into a
    // script.
    let mut wrapped = vec![
        "sh".to_string(),
        "-c".to_string(),
        "cd \"$0\" && exec \"$@\"".to_string(),
        at,
    ];
    wrapped.extend(argv.iter().cloned());
    Ok(engine.exec(&file, &compose_project, AGENT_SERVICE, &wrapped)?)
}

/// Bring the slot's copy of `HEAD` up to date, creating it the first time.
/// Public because a caller that launches something **detached** in the copy —
/// a mutation campaign (SPEC 4.4, gate 7) — has to refresh it first and then
/// spawn, rather than go through [`run`], which waits.
///
/// Bring the copy to `head`, creating it the first time.
///
/// `git fetch <path> HEAD` rather than a named branch: what has to be
/// replayed is the commit the slot is on, whatever branch holds it, and a
/// detached checkout of `FETCH_HEAD` says exactly that.
///
/// `git clean` without `-x`: it removes what a previous `nunki exec` left
/// untracked in the copy, and keeps everything the project ignores — which
/// is where the build cache lives. With `-x` the cache would go and gate 7
/// would recompile from cold every campaign, which SPEC 4.4 explicitly
/// refuses.
pub fn refresh(project: &Project, slot: &Slot, engine: Arc<dyn Engine>) -> Result<(), ExecError> {
    let file = crate::run::profile_path(project, &slot.name);
    if !file.is_file() {
        return Err(ExecError::NoProfile(slot.name.clone()));
    }
    let compose_project = crate::compose::project_name(&project.session(), &slot.name)?;
    let head = git::head(&slot.tree)?;
    refresh_at(&engine, &file, &compose_project, &head)
}

fn refresh_at(
    engine: &Arc<dyn Engine>,
    file: &std::path::Path,
    compose_project: &str,
    head: &str,
) -> Result<(), ExecError> {
    let script = format!(
        "set -eu\n\
         if [ ! -d {PROOF_AT}/.git ]; then\n\
         \x20 git clone --quiet --no-hardlinks {tree} {PROOF_AT}\n\
         fi\n\
         cd {PROOF_AT}\n\
         git fetch --quiet {tree} HEAD\n\
         git checkout --quiet --detach FETCH_HEAD\n\
         git reset --quiet --hard FETCH_HEAD\n\
         git clean -qdff\n\
         test \"$(git rev-parse HEAD)\" = \"{head}\"\n",
        tree = crate::run::TREE_AT,
    );
    let out = engine.exec(
        file,
        compose_project,
        AGENT_SERVICE,
        &["sh".to_string(), "-c".to_string(), script],
    )?;
    if out.ok() {
        Ok(())
    } else {
        Err(ExecError::Refresh(
            [out.stdout.trim(), out.stderr.trim()]
                .iter()
                .filter(|s| !s.is_empty())
                .cloned()
                .collect::<Vec<_>>()
                .join("\n"),
        ))
    }
}

/// The volume a slot's profile must carry for `nunki exec` to have somewhere to
/// put the copy, as the plan wants it.
pub fn volume(slot: &str) -> crate::compose::NamedVolume {
    crate::compose::NamedVolume {
        name: proof_volume(slot),
        at: PathBuf::from(PROOF_AT),
    }
}
