//! `nunki logs <mission>` (SPEC 4.2, verb table) and `nunki mission watch`.
//!
//! What replaces watching a screen. SPEC 4.3 accepts the cost of an agent
//! nobody can look at — "plus d'écran à regarder par curiosité" — on the
//! condition that its output is readable afterwards, and this is that
//! condition met.
//!
//! `nunki` parses no run format of its own. The shape of a run's stream belongs
//! to the harness, so the rendering is asked of the adapter
//! ([`crate::harness::Harness::readable`]) and this module only decides which
//! runs to read, in which order, and how much.

use std::path::{Path, PathBuf};

use crate::harness::{Harness, Line};
use crate::mission::dir::Paths;
use crate::project::Project;
use crate::state::Store;

#[derive(Debug, thiserror::Error)]
pub enum LogsError {
    #[error("no mission {0} at this HQ — `nunki mission list` says which there are")]
    NoMission(String),
    #[error("mission {0} has no run yet: nothing has been written to read")]
    NoRuns(String),
    #[error("{0} could not be read: {1}")]
    Io(PathBuf, std::io::Error),
}

/// One run's output, rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    /// The log file, so a reader who wants the raw stream knows where it is.
    pub log: PathBuf,
    /// The session the file is named after.
    pub session: String,
    pub lines: Vec<Line>,
}

/// Every run of a mission, oldest first, rendered by the harness that wrote
/// them.
///
/// Oldest first because a mission is read forwards: the run that explains the
/// one before it is the one after it.
pub fn of(project: &Project, id: &str, harness: &dyn Harness) -> Result<Vec<Run>, LogsError> {
    let paths = Paths::of(&project.hq_root, id);
    if !paths.dir.is_dir() {
        return Err(LogsError::NoMission(id.to_string()));
    }
    let runs = paths.dir.join("runs");
    let mut files = logs_in(&runs)?;
    if files.is_empty() {
        return Err(LogsError::NoRuns(id.to_string()));
    }
    files.sort();

    let mut out = Vec::new();
    for log in files {
        let text = std::fs::read_to_string(&log).map_err(|e| LogsError::Io(log.clone(), e))?;
        out.push(Run {
            session: log
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            lines: harness.readable(&text),
            log,
        });
    }
    Ok(out)
}

/// The run logs in a directory, by modification time and then by name.
///
/// A missing directory is not an error: a mission framed and never started
/// has none, and that is what [`LogsError::NoRuns`] says, once, in a sentence
/// about the mission rather than about a path.
fn logs_in(runs: &Path) -> Result<Vec<PathBuf>, LogsError> {
    let Ok(entries) = std::fs::read_dir(runs) else {
        return Ok(Vec::new());
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| LogsError::Io(runs.to_path_buf(), e))?;
        let path = entry.path();
        // `.jsonl` and not everything: a run writes its stderr beside its
        // stream, and rendering that as a run would put the wrapper's words
        // in the agent's mouth.
        if path.extension().is_some_and(|e| e == "jsonl") {
            files.push(path);
        }
    }
    Ok(files)
}

/// Which mission a `nunki logs` with no argument would mean: the one this HQ
/// touched last. Saying it out loud is the difference between a convenience
/// and a guess.
pub fn most_recent(project: &Project) -> Result<Option<String>, crate::state::StateError> {
    let store = Store::open(&project.hq_root)?;
    let mut latest: Option<(String, String)> = None;
    for id in store.missions()? {
        let Ok(state) = store.load(&id) else { continue };
        let seen = state.updated_at.clone();
        if latest.as_ref().is_none_or(|(_, when)| *when < seen) {
            latest = Some((id, seen));
        }
    }
    Ok(latest.map(|(id, _)| id))
}
