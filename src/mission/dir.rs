//! The mission folder, at the HQ (SPEC 4.1).
//!
//! It lives under `~/.hq/<project>/missions/<id>/`, outside the git tree,
//! and is mounted into the container. That is what lets the HQ and the agent
//! talk through files without a channel, while the tree stays clean and the
//! folder survives `hq slot rm`.
//!
//! Five files, and the split between them is a restriction, not a
//! convention. `MISSION.md` and `FOLLOWUP_HQ.md` belong to the human and the
//! HQ, and are mounted read-only: an agent that could rewrite its own header
//! could grant itself a domain and a credentials file for the next run.
//! `JOURNAL.md`, `PR.md` and `VERDICT.json` are the agent's three.

use std::path::{Path, PathBuf};

use crate::mission::Header;

/// The files of one mission. `MUTANTS.json` is `hq`'s and is written only
/// when a campaign ends, so it is not created here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub dir: PathBuf,
    pub mission: PathBuf,
    pub followup: PathBuf,
    pub journal: PathBuf,
    pub pr: PathBuf,
    pub verdict: PathBuf,
    /// The coder's answers to a mutation campaign's survivors (SPEC 4.4,
    /// gate 7). Separate from `MUTANTS.json`, which is the HQ's.
    pub triage: PathBuf,
    /// The campaign itself, written by `hq` and read-only for the agent.
    pub mutants: PathBuf,
}

impl Paths {
    pub fn of(hq_root: &Path, id: &str) -> Self {
        let dir = hq_root.join("missions").join(id);
        Self {
            mission: dir.join("MISSION.md"),
            followup: dir.join("FOLLOWUP_HQ.md"),
            journal: dir.join("JOURNAL.md"),
            pr: dir.join("PR.md"),
            verdict: dir.join("VERDICT.json"),
            triage: dir.join(crate::mutants::TRIAGE_FILE),
            mutants: dir.join(crate::mutants::FILE),
            dir,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MissionDirError {
    #[error("mission {0:?} already exists at {1}")]
    Exists(String, PathBuf),
    #[error("no mission {0:?} at {1}")]
    Unknown(String, PathBuf),
    #[error("a mission id may hold letters, digits, - and _ only, and {0:?} does not")]
    BadId(String),
    #[error("{0} has no header: a mission starts with a `---` block")]
    NoHeader(PathBuf),
    #[error("the header of {path} is not valid: {source}")]
    BadHeader {
        path: PathBuf,
        source: serde_yaml_ng::Error,
    },
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
}

/// Write a new mission folder. Refuses to touch one that exists: a mission is
/// reframed by a verb, never by writing over it (SPEC 4.1).
pub fn create(
    hq_root: &Path,
    id: &str,
    header: &Header,
    prose: &str,
) -> Result<Paths, MissionDirError> {
    check_id(id)?;
    let paths = Paths::of(hq_root, id);
    if paths.dir.exists() {
        return Err(MissionDirError::Exists(id.to_string(), paths.dir));
    }
    std::fs::create_dir_all(&paths.dir).map_err(|e| MissionDirError::Io(paths.dir.clone(), e))?;

    write(&paths.mission, &render(header, prose))?;
    write(&paths.followup, &followup(header))?;
    write(&paths.journal, &journal(id))?;
    write(&paths.pr, "")?;
    // Not `{}`: an empty verdict is the absence of one, and `hq` refuses a
    // verdict whose `head` is not the branch's (SPEC 4.1).
    write(&paths.verdict, "")?;
    // Created empty, and that is not cosmetic: a bind mount whose source
    // does not exist makes the engine create a **directory** there, and the
    // agent would find a directory where its file should be.
    write(&paths.triage, "")?;
    Ok(paths)
}

/// The header of a mission, read back from its folder. `hq` reads this once,
/// when the human validates the framing, and freezes it in its state; during
/// the mission it never reads it again.
pub fn read_header(hq_root: &Path, id: &str) -> Result<Header, MissionDirError> {
    let paths = Paths::of(hq_root, id);
    if !paths.mission.is_file() {
        return Err(MissionDirError::Unknown(id.to_string(), paths.dir));
    }
    let text = std::fs::read_to_string(&paths.mission)
        .map_err(|e| MissionDirError::Io(paths.mission.clone(), e))?;
    let yaml =
        front_matter(&text).ok_or_else(|| MissionDirError::NoHeader(paths.mission.clone()))?;
    serde_yaml_ng::from_str(yaml).map_err(|source| MissionDirError::BadHeader {
        path: paths.mission.clone(),
        source,
    })
}

/// The missions a project's HQ holds, in name order.
pub fn list(hq_root: &Path) -> Vec<String> {
    let mut ids: Vec<String> = std::fs::read_dir(hq_root.join("missions"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("MISSION.md").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    ids.sort();
    ids
}

/// The YAML between the first two `---` lines.
fn front_matter(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    Some(&rest[..end])
}

fn render(header: &Header, prose: &str) -> String {
    let yaml = serde_yaml_ng::to_string(header).expect("a header serialises");
    format!("---\n{yaml}---\n\n# Mission\n\n{}\n", prose.trim_end())
}

fn write(path: &Path, body: &str) -> Result<(), MissionDirError> {
    std::fs::write(path, body).map_err(|e| MissionDirError::Io(path.to_path_buf(), e))
}

fn journal(id: &str) -> String {
    format!(
        "# Journal — {id}\n\n\
         ## ÉTAT DE REPRISE\n\n\
         Nothing has run yet.\n\n\
         > Rewrite the block above at every checkpoint and before stopping. A\n\
         > run that ends without it has not honoured the run contract. The\n\
         > coder ends it with `Lot: <lot> — done`, or `Lot: <lot> — failed: <why>`.\n"
    )
}

/// Addressed to somebody by name when the mission says who: an arbitration
/// is for a person, and "waiting on the human" stops being enough as soon as
/// there are two.
fn followup(header: &Header) -> String {
    let who = header
        .arbiter
        .clone()
        .unwrap_or_else(|| "the human".to_string());
    // A raw string, and not a `\`-continued one: `cargo fmt` joins a
    // continued literal onto one line and keeps its indentation as real
    // spaces, which is how this file came to be written with nine-space
    // indents — Markdown renders those as a code block. Measured on this very
    // function, 2026-09-10.
    const TEMPLATE: &str = r#"# Follow-up — for {who}

For {who} and the HQ. The agent reads this file and never writes it.

Anything this mission cannot decide on its own is written here, dated and
addressed to {who}. `hq` appends to it too: a verdict another role concluded
on, and a finding {who} has lifted (SPEC 4.5).
"#;
    TEMPLATE.replace("{who}", &who)
}

fn check_id(id: &str) -> Result<(), MissionDirError> {
    let ok = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && id.chars().next().is_some_and(|c| c.is_ascii_alphanumeric());
    if ok {
        Ok(())
    } else {
        Err(MissionDirError::BadId(id.to_string()))
    }
}
