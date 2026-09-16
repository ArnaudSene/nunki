//! The ledger of sessions: which home belongs to which repository (SPEC 4.1).
//!
//! A project's home used to be named after its repository's directory, so two
//! repositories called `api` could not be orchestrated at once: the second was
//! refused rather than served, because one name would have meant one home, one
//! HQ and one set of missions. Decided by Arnaud on 2026-09-16: a home is named
//! by an identifier nothing else carries, and `~/.nunki/sessions.json` is the
//! ledger that says, for each identifier, the repository it belongs to.
//!
//! The ledger is an **index, never the authority**. Every home's `nunki.yaml`
//! names its own repository (`root:`), so a ledger that is lost can be rebuilt
//! from the homes, and one that disagrees with a home is caught by
//! [`crate::project::Project::open`] rather than obeyed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The ledger, beside the accounts, under `~/.nunki/`.
pub const FILE: &str = "sessions.json";

/// One line per project: its identifier, and the repository it belongs to.
pub type Sessions = BTreeMap<String, PathBuf>;

#[derive(Debug, thiserror::Error)]
pub enum SessionsError {
    #[error("{0} could not be read: {1}")]
    Unreadable(PathBuf, String),
    #[error("{0} is not valid: {1}")]
    Invalid(PathBuf, String),
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("no session {id} in {file} — `nunki init` opens one")]
    Unknown { id: String, file: PathBuf },
    #[error(
        "session {id} still belongs to the repository at {root}: adopting it here would take \
         its HQ and its missions from there — move that repository, or remove it, first"
    )]
    StillThere { id: String, root: PathBuf },
}

/// Where the ledger lives.
pub fn path(nunki_home: &Path) -> PathBuf {
    nunki_home.join(FILE)
}

/// The ledger as it stands. A file that does not exist is an empty ledger and
/// not an error: the first `nunki init` on a machine writes it.
pub fn load(nunki_home: &Path) -> Result<Sessions, SessionsError> {
    let file = path(nunki_home);
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Sessions::new()),
        Err(e) => return Err(SessionsError::Unreadable(file, e.to_string())),
    };
    serde_json::from_str(&text).map_err(|e| SessionsError::Invalid(file, e.to_string()))
}

/// Write the ledger whole, and atomically: a temporary file beside it, then a
/// rename. Two `nunki init` at once then leave one of the two lines rather
/// than a half-written file neither of them could read afterwards.
fn save(nunki_home: &Path, sessions: &Sessions) -> Result<(), SessionsError> {
    std::fs::create_dir_all(nunki_home)
        .map_err(|e| SessionsError::Io(nunki_home.to_path_buf(), e))?;
    let file = path(nunki_home);
    let text = serde_json::to_string_pretty(sessions)
        .map_err(|e| SessionsError::Invalid(file.clone(), e.to_string()))?;
    let temp = file.with_extension(format!("json.{}", std::process::id()));
    std::fs::write(&temp, format!("{text}\n")).map_err(|e| SessionsError::Io(temp.clone(), e))?;
    std::fs::rename(&temp, &file).map_err(|e| SessionsError::Io(file, e))
}

/// The identifier of the project rooted at `root`, if it has one.
pub fn find(nunki_home: &Path, root: &Path) -> Result<Option<String>, SessionsError> {
    let here = canonical(root);
    Ok(load(nunki_home)?
        .into_iter()
        .find(|(_, at)| canonical(at) == here)
        .map(|(id, _)| id))
}

/// The identifier of the project rooted at `root`, opening a session for it
/// when it has none. Idempotent: the same repository keeps the same
/// identifier, whatever its directory is called today.
pub fn open(nunki_home: &Path, root: &Path) -> Result<String, SessionsError> {
    if let Some(id) = find(nunki_home, root)? {
        return Ok(id);
    }
    let mut sessions = load(nunki_home)?;
    let id = crate::run::session_id();
    sessions.insert(id.clone(), canonical(root));
    save(nunki_home, &sessions)?;
    Ok(id)
}

/// Point an existing session at the repository at `root` — what a human runs
/// after moving or renaming one, so that its HQ and its missions follow.
///
/// It refuses while the recorded path still holds a repository: that one is
/// somebody's live project, and adopting would take its HQ away without a
/// word.
pub fn adopt(nunki_home: &Path, id: &str, root: &Path) -> Result<PathBuf, SessionsError> {
    let mut sessions = load(nunki_home)?;
    let Some(recorded) = sessions.get(id).cloned() else {
        return Err(SessionsError::Unknown {
            id: id.to_string(),
            file: path(nunki_home),
        });
    };
    let here = canonical(root);
    if canonical(&recorded) != here && recorded.join(".git").exists() {
        return Err(SessionsError::StillThere {
            id: id.to_string(),
            root: recorded,
        });
    }
    sessions.insert(id.to_string(), here.clone());
    save(nunki_home, &sessions)?;
    Ok(here)
}

/// A path as the filesystem resolves it, or as given when it does not exist:
/// `/tmp` and `/private/tmp` are one directory on macOS, and a comparison of
/// spellings would call them two.
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
