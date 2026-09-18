//! The mechanical security of a project (SPEC 4.4, gate 8): the contract
//! `security.sh` speaks, and nothing about the tools that speak it.
//!
//! A stack's script runs the dependency audit and the secret scan its
//! ecosystem uses, and prints **one JSON object per line**. `nunki` reads
//! seven fields and decides on them alone: it knows no lockfile, no advisory
//! database and no scanner. That boundary is what lets a second stack arrive
//! without the core changing (SPEC, section 1).

use serde::{Deserialize, Serialize};

/// What a finding is about, and the only classes `nunki` reasons on.
///
/// The names are the contract's, not a tool's: Python and Next.js will carry
/// the same distinction under other words, and an ecosystem that invents a
/// class `nunki` does not know is read as [`Kind::Other`] rather than
/// refused — a gate that rejects a line it does not understand would make a
/// new stack's first run a parse error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A known vulnerability, with or without a fix.
    Vulnerability,
    /// An advisory that is not one: abandoned, unsound, withdrawn. Its `fix`
    /// is empty **by nature** and stays that way (SPEC 4.4).
    Unmaintained,
    /// A credential in the tree or in its history.
    Secret,
    /// What a static analysis says.
    Lint,
    /// Something the stack reports and this version has no name for.
    #[serde(other)]
    Other,
}

/// One line of the contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    /// What the ecosystem calls it. Opaque here, and the name an acceptance
    /// is written against.
    pub id: String,
    pub kind: Kind,
    /// Where, for a human: a package and its version, or `file:line`.
    ///
    /// Renamed because the contract's key is `where`, which Rust will not
    /// take as a field name. The wire name is the contract's, always.
    #[serde(rename = "where", default)]
    pub at: String,
    /// The **direct** dependency that brought it in, empty when the finding
    /// is already on one. A transitive dependency is replaceable by nobody
    /// but its parent, which is why this names the parent rather than
    /// propagating anything down (SPEC 4.4).
    #[serde(default)]
    pub via: String,
    /// What fixes it. **Empty means none exists**, and that is the
    /// discriminator every rule below turns on.
    #[serde(default)]
    pub fix: String,
    /// The reason it was accepted, empty when it was not.
    #[serde(default)]
    pub accepted: String,
    /// Whether the mission's base already carried it. What the branch brought
    /// is the mission's; what preceded it is the project's.
    #[serde(default)]
    pub was_at_base: bool,
}

impl Finding {
    /// Does this stop the mission?
    ///
    /// New since the base and not accepted. Everything else is a constat:
    /// what the repository already carried is not this branch's to answer,
    /// and blocking on it would punish the wrong change (SPEC 4.4).
    pub fn blocks(&self) -> bool {
        !self.was_at_base && self.accepted.is_empty()
    }

    /// An acceptance a fix has overtaken.
    ///
    /// The exception was written because nothing could be done; something can
    /// now. This is what replaces an expiry date: the condition is mechanical
    /// rather than guessed, and cargo-deny has no expiry to offer anyway
    /// (measured 2026-09-17).
    pub fn stale(&self) -> bool {
        !self.accepted.is_empty() && !self.fix.is_empty()
    }

    /// One line a human can act on, without `nunki` knowing what produced it.
    ///
    /// `via` is named when there is one, because a transitive finding is not
    /// replaceable where it is: the parent is the only thing this project
    /// chooses (SPEC 4.4).
    pub fn say(&self) -> String {
        let mut said = format!("{} ({:?}) {}", self.id, self.kind, self.at);
        if !self.via.is_empty() {
            said.push_str(&format!(" via {}", self.via));
        }
        if !self.fix.is_empty() {
            said.push_str(&format!(" — fixed in {}", self.fix));
        }
        said
    }

    /// An acceptance no fix can ever overtake: an advisory that is not a
    /// vulnerability, or a secret ruled a false positive. Permanent, and
    /// better said than pretended otherwise (SPEC 4.4).
    pub fn permanent(&self) -> bool {
        !self.accepted.is_empty() && self.fix.is_empty()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SecurityError {
    #[error("{line}: {source}")]
    Unreadable {
        line: String,
        source: serde_json::Error,
    },
}

/// Read what the script printed.
///
/// A line that is not a JSON object is **ignored**, exactly as the mutation
/// campaign ignores one: progress may go to stdout freely, and a script that
/// says what it is doing must not fail a gate for it. A line that *is* an
/// object and cannot be read is an error — a fragment writing a broken
/// contract must be told, not silently dropped.
pub fn read(output: &str) -> Result<Vec<Finding>, SecurityError> {
    let mut findings = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        findings.push(
            serde_json::from_str(line).map_err(|source| SecurityError::Unreadable {
                line: line.to_string(),
                source,
            })?,
        );
    }
    Ok(findings)
}
