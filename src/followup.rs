//! `FOLLOWUP_HQ.md` — what the HQ carries to the coder (SPEC 4.5).
//!
//! Two facts about a mission never travel from one agent to another:
//!
//! - **the constat lives in the journal of the role that made it**, and it is
//!   the HQ that carries it to the coder, dated, in this file. An agent never
//!   speaks to another agent — everything goes through the HQ and through
//!   files, so that losing a session loses nothing;
//! - **a security finding is only ever lifted by a human**, through a verb,
//!   and the lift is written here as well as into `nunki`'s state.
//!   `VERDICT.json` stays `FINDINGS`: the verdict says what the agent found,
//!   the state says what the human decided, and confusing the two would let a
//!   red verdict be edited into a green one.
//!
//! This module only appends. The head of the file belongs to the human who
//! framed the mission, and nothing here rewrites it.

use std::path::Path;

use crate::harness::Role;
use crate::mission::Verdict;

#[derive(Debug, thiserror::Error)]
pub enum FollowupError {
    #[error("{0} could not be written: {1}")]
    Io(std::path::PathBuf, std::io::Error),
}

/// Carry a role's verdict to the coder.
///
/// Written whether or not the coder is about to be relaunched: the record is
/// what a later run reads, and a mission that stops at the bound must still
/// say why in the file the human reads.
pub fn carry(
    file: &Path,
    from: Role,
    verdict: Verdict,
    report: &str,
    head: &str,
) -> Result<(), FollowupError> {
    let report = match report.trim() {
        "" => "The verdict carried no report.",
        text => text,
    };
    append(
        file,
        &format!(
            "## {date} — {who} concluded {verdict:?} on {short}\n\n\
             {report}\n",
            date = today(),
            who = crate::role::slug(from),
            short = short(head),
        ),
    )
}

/// Record that a human lifted a finding, with the reason they gave.
///
/// The reason is not optional anywhere it can be helped: a risk accepted
/// without one is not accepted, it is forgotten.
pub fn lifted(
    file: &Path,
    who: &str,
    finding: &str,
    why: &str,
    head: &str,
) -> Result<(), FollowupError> {
    append(
        file,
        &format!(
            "## {date} — {who} lifted a security finding on {short}\n\n\
             **Finding:** {finding}\n\n\
             **Accepted because:** {why}\n\n\
             `VERDICT.json` stays `FINDINGS`. This file and `nunki`'s state are what\n\
             record that a human lifted it, and `nunki push` reads it there.\n",
            date = today(),
            short = short(head),
        ),
    )
}

/// Record that the human lifted what remained of a verdict.
pub fn lifted_all(file: &Path, who: &str, why: &str, head: &str) -> Result<(), FollowupError> {
    append(
        file,
        &format!(
            "## {date} — {who} lifted the security verdict on {short}\n\n\
             **Accepted because:** {why}\n\n\
             Everything the report still carried is lifted from here on. A new\n\
             commit makes this stale: a verdict, and its lift, are worth one\n\
             commit and no other (SPEC 4.5).\n",
            date = today(),
            short = short(head),
        ),
    )
}

/// A mission called off, and why.
pub fn ended(file: &Path, who: &str, why: &str) -> Result<(), FollowupError> {
    append(
        file,
        &format!(
            "## {date} — {who} called this mission off\n\n\
             **Because:** {why}\n\n\
             Nothing here is deleted. `nunki mission archive` moves this folder\n\
             under `archive/`, and what it holds is the record of what was done.\n",
            date = today(),
        ),
    )
}

/// The human took the mission back from a handover, and said what changed.
///
/// It lands here and not only in the state, because this is the file every
/// role reads before anything else: a retry that tells the agent nothing
/// hands it back the work it already failed, with the same tree and the same
/// cause, and spends the budget reaching the same handover. What changed is
/// the whole point of the verb.
pub fn retried(file: &Path, who: &str, why: &str, was: &str) -> Result<(), FollowupError> {
    let head = format!(
        "## {date} \u{2014} {who} took this mission back",
        date = today()
    );
    let body = format!(
        "It had stopped on its own bounds: {was}. They are handed back whole, and this is what changed since:"
    );
    append(
        file,
        &format!(
            "{head}\n\n{body}\n\n{why}\n\nRead it before the journal. Nothing in the tree moved on its own.\n"
        ),
    )
}

/// An instruction left for the next run.
///
/// `say` and not a channel: there is no channel during a run (SPEC 4.3). What
/// is written here is read by the run after this one, by every role, because
/// every role is told to read this file before anything else.
pub fn said(file: &Path, who: &str, what: &str) -> Result<(), FollowupError> {
    append(
        file,
        &format!(
            "## {date} — {who} left an instruction for the next run\n\n{what}\n",
            date = today(),
        ),
    )
}

/// The first twelve characters of a commit, which is how every other message
/// in `nunki` names one.
fn short(head: &str) -> &str {
    &head[..12.min(head.len())]
}

fn today() -> String {
    crate::state::now_rfc3339()
        .split('T')
        .next()
        .unwrap_or_default()
        .to_string()
}

fn append(file: &Path, block: &str) -> Result<(), FollowupError> {
    use std::io::Write;
    let existing = std::fs::read_to_string(file).unwrap_or_default();
    let mut out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
        .map_err(|e| FollowupError::Io(file.to_path_buf(), e))?;
    // One blank line between blocks, and never two: this file is read by an
    // agent that is told to read it first, and a heading glued to the
    // previous paragraph is a heading it may not see.
    let lead = match () {
        _ if existing.is_empty() || existing.ends_with("\n\n") => "",
        _ if existing.ends_with('\n') => "\n",
        _ => "\n\n",
    };
    write!(out, "{lead}{block}").map_err(|e| FollowupError::Io(file.to_path_buf(), e))
}
