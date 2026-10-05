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
             commit and no other (SPEC 4.5) — unless the security rounds are\n\
             spent, when no verdict can follow and this lift stands for the\n\
             commits after it, which `nunki push` names as not attacked.\n",
            date = today(),
            short = short(head),
        ),
    )
}

/// Record that `nunki` lifted a security verdict itself, every finding in it
/// being `LOW` or `INFO`: the findings, one per line, with the agent's
/// reason for each (SPEC 4.5).
///
/// Said as `nunki`'s, never as a human's: the HQ reads here which lifts it
/// did not make, and can still send the mission back with `iterate`.
pub fn lifted_by_nunki(file: &Path, findings: &[String], head: &str) -> Result<(), FollowupError> {
    let listed = findings
        .iter()
        .map(|line| format!("- {}", crate::text::one_line(line)))
        .collect::<Vec<_>>()
        .join("\n");
    append(
        file,
        &format!(
            "## {date} — nunki lifted the security verdict on {short} (LOW/INFO)\n\n\
             Every finding the security agent ranked is LOW or INFO, so nunki\n\
             accepted them itself, with the agent's own reasons:\n\n\
             {listed}\n\n\
             `VERDICT.json` stays `FINDINGS`. This lift obeys the rules of a\n\
             human's: it is worth this commit, and a later `FINDINGS` is not\n\
             covered by it. `nunki mission iterate` still sends the mission back.\n",
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

/// What a retry took the mission back from, for the record it leaves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Retaken<'a> {
    /// A bound ran out — `was` says which, in one clause — and the retry
    /// hands it back whole.
    Bound { was: &'a str },
    /// The coder awaited the HQ's ruling on `survivors` at `asked` of `of`
    /// attempts. A ruling is not a bound, so none is handed back: the lot
    /// resumes at the next attempt.
    Ruling {
        lot: &'a str,
        survivors: &'a [String],
        asked: u32,
        of: u32,
    },
    /// The same, asked on the last attempt: there is no next one, so the
    /// retry hands the lot over as out of attempts instead.
    RulingOnTheLastAttempt {
        lot: &'a str,
        survivors: &'a [String],
        asked: u32,
    },
}

/// The human took the mission back from a handover, and said what changed.
///
/// It lands here and not only in the state, because this is the file every
/// role reads before anything else: a retry that tells the agent nothing
/// hands it back the work it already failed, with the same tree and the same
/// cause, and spends the budget reaching the same handover. What changed is
/// the whole point of the verb.
///
/// What it says about the bounds depends on what stopped the mission, and is
/// read by the next run as a statement of fact: only a bound is handed back
/// whole, never a ruling.
pub fn retried(file: &Path, who: &str, why: &str, taken: &Retaken) -> Result<(), FollowupError> {
    let head = format!(
        "## {date} \u{2014} {who} took this mission back",
        date = today()
    );
    let quoted = |ids: &[String]| {
        ids.iter()
            .map(|id| format!("`{id}`"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let body = match taken {
        Retaken::Bound { was } => format!(
            "It had stopped on its own bounds: {was}. They are handed back whole, and this is what changed since:"
        ),
        Retaken::Ruling {
            lot,
            survivors,
            asked,
            of,
        } => format!(
            "It had stopped on {lot}, attempt {asked}, awaiting the HQ's ruling on {}. A ruling \
             is not a bound, and none is handed back: this is attempt {} of {of}. What the HQ \
             ruled, or what changed since:",
            quoted(survivors),
            asked + 1
        ),
        Retaken::RulingOnTheLastAttempt {
            lot,
            survivors,
            asked,
        } => format!(
            "It had stopped on {lot}, attempt {asked}, awaiting the HQ's ruling on {}. A ruling \
             is not a bound, and none is handed back — and that was the last attempt, so {lot} \
             is handed over as out of attempts rather than run past the bound. A further \
             `nunki mission retry` hands the attempts back whole. What the HQ ruled, or what \
             changed since:",
            quoted(survivors)
        ),
    };
    append(
        file,
        &format!(
            "{head}\n\n{body}\n\n{why}\n\nRead it before the journal. Nothing in the tree moved on its own.\n"
        ),
    )
}

/// The HQ read the verified branch and sends it back to the coder.
///
/// Here, because this is what the coder reads first: a volet whose cause is
/// only in `nunki`'s state reaches the coder as a sentence in its prompt, and
/// the record of what the human refused belongs with the others.
pub fn reviewed(file: &Path, who: &str, why: &str) -> Result<(), FollowupError> {
    append(
        file,
        &format!(
            "## {date} — {who} read the verified branch and sends it back\n\n\
             **What to change:** {why}\n\n\
             This is a volet: change what is asked, prove it as for any lot, and\n\
             the verification runs again from the gates.\n",
            date = today(),
        ),
    )
}

/// The mission was verified without the security agent, after a `CLEAR` or
/// a lifted `FINDINGS`, because the rounds its rigor allows were spent
/// (SPEC 4.5): `rounds` played of `max`, and `not_attacked` the commits
/// after the last round, as [`crate::push::not_attacked`] names them.
///
/// Written here, beside the verdicts the rounds concluded, so that whoever
/// validates the push reads that the last commit was not seen by the agent,
/// and why — a verified mission otherwise reads as if every stage had run.
pub fn security_capped(
    file: &Path,
    rounds: u32,
    max: u32,
    not_attacked: &[String],
) -> Result<(), FollowupError> {
    append(
        file,
        &format!(
            "## {date} — nunki did not call the security agent again\n\n\
             The security round cap was reached: {rounds} / {max}. The gates are\n\
             green on the code as it stands, and the mission is verified without\n\
             another round: what the last commits changed has not been read by\n\
             the security agent.\n{commits}",
            date = today(),
            commits = unattacked(not_attacked),
        ),
    )
}

/// The security agent was not called again because its rounds were spent,
/// and the last verdict it concluded was `FINDINGS`: the mission is back on
/// that report rather than verified (SPEC 4.5). What the coder changed after
/// it has not been attacked, and whoever accepts or iterates must know it:
/// `not_attacked` names those commits.
pub fn security_capped_on_findings(
    file: &Path,
    rounds: u32,
    max: u32,
    not_attacked: &[String],
) -> Result<(), FollowupError> {
    append(
        file,
        &format!(
            "## {date} — nunki did not call the security agent again\n\n\
             The security round cap was reached: {rounds} / {max}. The last round\n\
             concluded FINDINGS, and the fix made after it has not been attacked\n\
             again: the gates are green on it, and that is all that was played.\n\
             The mission is back on those findings — `nunki mission accept`\n\
             lifts them, `nunki mission iterate` sends one more volet.\n{commits}",
            date = today(),
            commits = unattacked(not_attacked),
        ),
    )
}

/// The commits after the last round, as a list a human can check against
/// `git log`; nothing when there are none to name.
fn unattacked(commits: &[String]) -> String {
    if commits.is_empty() {
        return String::new();
    }
    let mut out = String::from("\nNot attacked by the security agent:\n\n");
    for commit in commits {
        out.push_str(&format!("- {commit}\n"));
    }
    out
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
    // Reports, causes and commit subjects are written here by agents and
    // authors, and this file is read by a human, in a terminal as often as
    // not: every block is made printable on its way in.
    let block = crate::text::printable(block);
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
