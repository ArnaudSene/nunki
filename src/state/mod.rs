//! The persisted state of `hq` (SPEC 4.2, "L'état de `hq` est persisté, et
//! verrouillé").
//!
//! `hq verify` lasts hours and must survive the death of the HQ session, a
//! sleeping machine, a closed terminal. So every transition is written to
//! disk under `<hq-root>/state/`, one file per mission, atomically; and a
//! lock per slot keeps two `hq` from driving the same slot at once. A
//! restarted `hq` reads the state back and, before deciding anything,
//! re-derives whether the run it recorded is still alive — that part is the
//! engine's, not this module's: here we only keep and hand back the facts.

pub mod lock;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::harness::RunHandle;
use crate::mission::flow::{Event, Flow, FlowError};

pub use lock::{LockError, SlotLock};

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("io on {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("state file {path} is not valid: {source}")]
    Corrupt {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("no state for mission {0}")]
    Missing(String),
    #[error(transparent)]
    Flow(#[from] FlowError),
}

/// Everything `hq` knows about a mission that is not in its files: the flow
/// (which carries the frozen header), the slot, and the run in progress if
/// any, as a persistable handle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissionState {
    pub id: String,
    pub slot: String,
    pub flow: Flow,
    /// The run launched for the current stage, if one is (or was) running.
    /// Kept so a restarted `hq` can ask the engine whether it is still alive.
    pub run: Option<RunHandle>,
    /// The application `hq` started for the current stage, if the mission
    /// has one (SPEC 4.2, "les services et le lancement de l'application").
    /// Its own field and not the run's: "the agent is up" and "the
    /// deliverable is up" are two facts, and one handle would report them as
    /// one.
    #[serde(default)]
    pub app: Option<RunHandle>,
    /// RFC 3339 time of the last write; informational.
    pub updated_at: String,
}

/// The state directory of one project's HQ.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    /// Open (creating if needed) the state directory under an HQ root —
    /// `~/.hq/<project>/` in production, a temp dir in tests.
    pub fn open(hq_root: &Path) -> Result<Self, StateError> {
        let root = hq_root.join("state");
        for dir in [root.join("missions"), root.join("locks")] {
            fs::create_dir_all(&dir).map_err(|source| StateError::Io {
                path: dir.clone(),
                source,
            })?;
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn mission_path(&self, id: &str) -> PathBuf {
        self.root.join("missions").join(format!("{id}.json"))
    }

    /// Write a mission's state atomically: to a temp file in the same
    /// directory, then rename. A crash mid-write leaves the previous file
    /// intact, never a truncated one.
    pub fn save(&self, state: &MissionState) -> Result<(), StateError> {
        let path = self.mission_path(&state.id);
        let tmp = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(state).expect("MissionState serializes");
        fs::write(&tmp, bytes).map_err(|source| StateError::Io {
            path: tmp.clone(),
            source,
        })?;
        fs::rename(&tmp, &path).map_err(|source| StateError::Io {
            path: path.clone(),
            source,
        })
    }

    pub fn load(&self, id: &str) -> Result<MissionState, StateError> {
        let path = self.mission_path(id);
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(StateError::Missing(id.to_string()));
            }
            Err(source) => return Err(StateError::Io { path, source }),
        };
        serde_json::from_slice(&bytes).map_err(|source| StateError::Corrupt { path, source })
    }

    /// Identifiers of every mission with a state file.
    pub fn missions(&self) -> Result<Vec<String>, StateError> {
        let dir = self.root.join("missions");
        let entries = fs::read_dir(&dir).map_err(|source| StateError::Io {
            path: dir.clone(),
            source,
        })?;
        let mut ids = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| StateError::Io {
                path: dir.clone(),
                source,
            })?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(id) = name.strip_suffix(".json") {
                ids.push(id.to_string());
            }
        }
        ids.sort();
        Ok(ids)
    }

    /// Advance a mission's flow by one event and persist the result before
    /// returning it. The flow is only changed if the event is valid; a
    /// rejected event writes nothing.
    pub fn apply(&self, state: &mut MissionState, event: Event) -> Result<(), StateError> {
        state.flow.advance(event)?;
        state.updated_at = now_rfc3339();
        self.save(state)
    }

    /// Record the run launched for the current stage (or clear it), and
    /// persist.
    pub fn set_run(
        &self,
        state: &mut MissionState,
        run: Option<RunHandle>,
    ) -> Result<(), StateError> {
        state.run = run;
        state.updated_at = now_rfc3339();
        self.save(state)
    }
}

/// Seconds since the epoch, formatted as RFC 3339 in UTC, without pulling
/// a date crate for one informational field.
pub(crate) fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Civil-from-days, Howard Hinnant's algorithm.
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        (rem % 3_600) / 60,
        rem % 60
    )
}
