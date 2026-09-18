//! The secrets a human has ruled on (SPEC 4.4, gate 8).
//!
//! A secret is the one finding of gate 8 that no agent can close: removing it
//! in a later commit leaves it in the branch's history, and `nunki` rewrites
//! none. So the gate stops and hands it over, and what a human decides has to
//! be written somewhere the gate reads next time.
//!
//! **In the project's HQ**, and not in the mission folder: an exception that
//! lives with a mission goes with it into `archive/`, and the same secret
//! stops the next mission on the same line. Not in the repository either —
//! the file is a list of the places a secret has been, which is a map, and a
//! repository is published. The HQ is the human's, it is never mounted whole,
//! and it outlives every mission.
//!
//! Nothing here reads a tool: the file holds the ids gate 8's contract
//! carries, whatever produced them, so a second stack costs no change.

use std::path::{Path, PathBuf};

use crate::project::Project;

/// Where the rulings live, inside the project's HQ.
pub const FILE: &str = "SECRETS.txt";

/// Where the container reads them, read-only.
///
/// Under a top-level directory the image does not create, and not under
/// `/work`: `/work` belongs to the agent (the Dockerfile chowns it), so an
/// agent could write its own exceptions there on a run where the file is
/// absent and nothing is mounted. Measured on 2026-09-18 — as the agent,
/// `mkdir -p /work/advisories` succeeds and `mkdir /nunki` is refused, and a
/// bind mount at `/nunki/secrets.txt` makes Docker create `/nunki` owned by
/// root.
pub const AT: &str = "/nunki/secrets.txt";

/// The header a new file is born with. It is read by whoever opens the file,
/// and `nunki secret accept` is what writes the lines under it.
const HEADER: &str = "\
# Secrets gate 8 found and a human has ruled on (SPEC 4.4).
#
# One ruling per line: the finding's id, then why the risk is accepted.
# Written by `nunki secret accept`, read back by `nunki secret list`, and
# mounted read-only in the container so no agent can add to it.
#
# A secret has no fix — it is revoked, not upgraded — so a ruling here does
# not expire on its own. Revoke the credential and `nunki secret forget` it.
";

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error(
        "the id {0:?} holds a space, and a ruling is one line of id then reason: \
         nothing would tell them apart"
    )]
    SpaceInId(String),
    #[error("the id is empty, and a ruling has to name what it rules on")]
    EmptyId,
    #[error(
        "the reason is empty: a risk accepted without a reason is not accepted, \
         it is forgotten"
    )]
    EmptyReason,
    #[error("the reason spans more than one line, and a ruling is one line")]
    ReasonIsNotALine,
    #[error("{0} could not be read: {1}")]
    Unreadable(PathBuf, #[source] std::io::Error),
    #[error("{0} could not be written: {1}")]
    Unwritable(PathBuf, #[source] std::io::Error),
}

/// One ruling: the finding gate 8 named, and why the risk is accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ruling {
    pub id: String,
    pub because: String,
}

/// Where this project's rulings live.
pub fn file(project: &Project) -> PathBuf {
    project.hq_root.join(FILE)
}

/// Every ruling in the file, in the order it holds them. A file that is not
/// there holds none, which is the honest answer: nothing is accepted.
pub fn read(at: &Path) -> Vec<Ruling> {
    let Ok(text) = std::fs::read_to_string(at) else {
        return Vec::new();
    };
    text.lines().filter_map(parse).collect()
}

/// A line is a ruling when it names something and gives a reason. Comments,
/// blank lines, and a bare id with nothing after it are not rulings — the
/// last one because it would accept a secret silently.
fn parse(line: &str) -> Option<Ruling> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let (id, because) = line.split_once(char::is_whitespace)?;
    let because = because.trim();
    (!because.is_empty()).then(|| Ruling {
        id: id.to_string(),
        because: because.to_string(),
    })
}

/// What `accept` did, so the caller can say it rather than guess.
#[derive(Debug, PartialEq, Eq)]
pub enum Accepted {
    Added,
    /// The id was already ruled on, for this reason, and now it is not.
    Replaced(String),
}

/// Rule on a finding, creating the file if this is the first one.
///
/// An id that already has a ruling is replaced and not doubled: the file is
/// read back by an exact match on the first field, and two lines for one id
/// would make which reason applies a matter of order.
pub fn accept(at: &Path, id: &str, because: &str) -> Result<Accepted, SecretError> {
    let id = id.trim();
    let because = because.trim();
    if id.is_empty() {
        return Err(SecretError::EmptyId);
    }
    if id.chars().any(char::is_whitespace) {
        return Err(SecretError::SpaceInId(id.to_string()));
    }
    if because.is_empty() {
        return Err(SecretError::EmptyReason);
    }
    if because.contains('\n') || because.contains('\r') {
        return Err(SecretError::ReasonIsNotALine);
    }

    let text = match std::fs::read_to_string(at) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => HEADER.to_string(),
        Err(e) => return Err(SecretError::Unreadable(at.to_path_buf(), e)),
    };

    let mut was = None;
    let mut kept: Vec<String> = Vec::new();
    for line in text.lines() {
        match parse(line) {
            Some(r) if r.id == id => was = Some(r.because),
            _ => kept.push(line.to_string()),
        }
    }
    kept.push(format!("{id}  {because}"));
    write(at, &kept)?;
    Ok(match was {
        Some(why) => Accepted::Replaced(why),
        None => Accepted::Added,
    })
}

/// Drop a ruling. `false` when there was none, which is not an error: a
/// credential revoked twice is a human being careful.
pub fn forget(at: &Path, id: &str) -> Result<bool, SecretError> {
    let id = id.trim();
    let text = match std::fs::read_to_string(at) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(SecretError::Unreadable(at.to_path_buf(), e)),
    };
    let mut dropped = false;
    let kept: Vec<String> = text
        .lines()
        .filter(|line| match parse(line) {
            Some(r) if r.id == id => {
                dropped = true;
                false
            }
            _ => true,
        })
        .map(str::to_string)
        .collect();
    if dropped {
        write(at, &kept)?;
    }
    Ok(dropped)
}

fn write(at: &Path, lines: &[String]) -> Result<(), SecretError> {
    if let Some(dir) = at.parent() {
        std::fs::create_dir_all(dir).map_err(|e| SecretError::Unwritable(dir.to_path_buf(), e))?;
    }
    let mut body = lines.join("\n");
    body.push('\n');
    std::fs::write(at, body).map_err(|e| SecretError::Unwritable(at.to_path_buf(), e))
}
