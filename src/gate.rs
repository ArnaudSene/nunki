//! The verification gates (SPEC 4.4), gates 1 to 4.
//!
//! These four are played **at the end of every run**, not only at the final
//! verification — decided on 2026-09-09, because a perimeter gate that only
//! falls at the end loses a six-hour mission over a forbidden write in the
//! first lot. Gates 5 to 7 (deliverable, battery, mutation) belong to the
//! final verification and need `hq exec`; they are not here.
//!
//! Everything in this module is deterministic and reads git. Nothing is
//! asked of an agent: a gate an agent could answer is not a gate.
//!
//! Each role gets the gates that match what it produces. The security agent
//! works in a read-only tree and commits nothing, so three of the four do
//! not apply to it — and "not applicable" is reported as itself, never as a
//! pass.

use std::collections::BTreeSet;
use std::path::Path;

use globset::{Glob, GlobSet, GlobSetBuilder};

use crate::git;
use crate::harness::Role;
use crate::mission::{Header, Integration};
use crate::project::ProtectedPaths;

/// The four gates, in the order SPEC 4.4 lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Gate {
    /// 1 — the working tree holds nothing uncommitted.
    CleanTree,
    /// 2 — the branch is not protected, and it is ahead of its base.
    BranchAhead,
    /// 3 — the resume block at the top of the journal names `HEAD`.
    ResumeNamesHead,
    /// 4 — the perimeter, on the diff **and commit by commit**.
    Perimeter,
}

impl Gate {
    /// The number the specification gives it, for a report a human reads
    /// next to SPEC 4.4.
    pub fn number(self) -> u8 {
        match self {
            Gate::CleanTree => 1,
            Gate::BranchAhead => 2,
            Gate::ResumeNamesHead => 3,
            Gate::Perimeter => 4,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Gate::CleanTree => "clean tree",
            Gate::BranchAhead => "branch not protected and ahead of its base",
            Gate::ResumeNamesHead => "the resume block names HEAD",
            Gate::Perimeter => "perimeter",
        }
    }
}

/// What one gate decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Passed,
    /// Red, and the reason is what a human acts on: which path, which
    /// pattern, which commit.
    Failed(String),
    /// This gate means nothing for this role (SPEC 4.4, the per-role table).
    /// Reported as itself: a skipped gate described as green is how a report
    /// stops being worth reading.
    NotApplicable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub gate: Gate,
    pub decision: Decision,
}

/// What the gates of one run decided, all four of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub role: Role,
    pub head: String,
    pub outcomes: Vec<Outcome>,
}

impl Report {
    /// The first red gate, in specification order. The flow needs one
    /// sentence (`GatesFailed { reason }`); a human needs the whole report,
    /// which is why both exist.
    pub fn failure(&self) -> Option<String> {
        self.outcomes.iter().find_map(|o| match &o.decision {
            Decision::Failed(why) => Some(format!(
                "gate {} ({}): {why}",
                o.gate.number(),
                o.gate.title()
            )),
            _ => None,
        })
    }

    pub fn passed(&self) -> bool {
        self.failure().is_none()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GateError {
    #[error(transparent)]
    Git(#[from] git::GitError),
    #[error("{0} could not be read: {1}")]
    Unreadable(std::path::PathBuf, std::io::Error),
    #[error("{pattern:?} is not a usable path pattern: {source}")]
    BadPattern {
        pattern: String,
        source: globset::Error,
    },
}

/// Everything the four gates read, gathered by the caller so this module
/// opens nothing it was not given.
pub struct Subject<'a> {
    pub role: Role,
    /// The slot's working tree.
    pub tree: &'a Path,
    /// The mission's journal, on the host.
    pub journal: &'a Path,
    pub header: &'a Header,
    pub protected_branches: &'a [String],
    pub protected_paths: &'a ProtectedPaths,
}

/// Play gates 1 to 4 for one run.
pub fn run(subject: &Subject) -> Result<Report, GateError> {
    let head = git::head(subject.tree)?;
    let outcomes = vec![
        Outcome {
            gate: Gate::CleanTree,
            decision: clean_tree(subject)?,
        },
        Outcome {
            gate: Gate::BranchAhead,
            decision: branch_ahead(subject)?,
        },
        Outcome {
            gate: Gate::ResumeNamesHead,
            decision: resume_names_head(subject, &head)?,
        },
        Outcome {
            gate: Gate::Perimeter,
            decision: perimeter(subject)?,
        },
    ];
    Ok(Report {
        role: subject.role,
        head,
        outcomes,
    })
}

/// The security agent's tree is mounted read-only, so it has nothing to
/// commit and nothing to leave behind (SPEC 4.4, the per-role table).
const READ_ONLY_TREE: &str = "the security agent works in a read-only tree and commits nothing";

fn clean_tree(subject: &Subject) -> Result<Decision, GateError> {
    if subject.role == Role::Security {
        return Ok(Decision::NotApplicable(READ_ONLY_TREE.into()));
    }
    let dirty = git::run(subject.tree, &["status", "--porcelain"])?;
    if dirty.is_empty() {
        return Ok(Decision::Passed);
    }
    let listed: Vec<&str> = dirty.lines().take(10).collect();
    Ok(Decision::Failed(format!(
        "the tree holds {} uncommitted change(s): {}",
        dirty.lines().count(),
        listed.join("; ")
    )))
}

fn branch_ahead(subject: &Subject) -> Result<Decision, GateError> {
    if subject.role == Role::Security {
        return Ok(Decision::NotApplicable(READ_ONLY_TREE.into()));
    }
    let branch = git::current_branch(subject.tree)?;
    if subject.protected_branches.contains(&branch) {
        return Ok(Decision::Failed(format!(
            "the slot is on {branch:?}, which the project protects"
        )));
    }
    let base = base_ref(subject.tree, &subject.header.base)?;
    let ahead = git::run(
        subject.tree,
        &["rev-list", "--count", &format!("{base}..HEAD")],
    )?;
    if ahead.trim() == "0" {
        return Ok(Decision::Failed(format!(
            "{branch:?} has no commit that {base:?} does not already have"
        )));
    }
    Ok(Decision::Passed)
}

/// The base as this clone knows it: a fresh clone has the remote's branches
/// and only one of its own, so `dev` may only exist as `origin/dev`.
fn base_ref(tree: &Path, base: &str) -> Result<String, GateError> {
    if git::run(tree, &["rev-parse", "--verify", "--quiet", base]).is_ok() {
        Ok(base.to_string())
    } else {
        Ok(format!("origin/{base}"))
    }
}

/// Gate 3 reads **the block**, not the file.
///
/// A journal that names an earlier commit somewhere in its history is
/// exactly the stale-resume case this gate exists to catch, so searching the
/// whole file for the hash would delete the gate. What counts is the block
/// at the top: the first heading whose text is `ÉTAT DE REPRISE`, up to the
/// next heading at the same level or higher.
fn resume_names_head(subject: &Subject, head: &str) -> Result<Decision, GateError> {
    let text = std::fs::read_to_string(subject.journal)
        .map_err(|e| GateError::Unreadable(subject.journal.to_path_buf(), e))?;
    let Some(block) = resume_block(&text) else {
        return Ok(Decision::Failed(format!(
            "{} has no `ÉTAT DE REPRISE` block; a run that ends without one has not \
             honoured the run contract",
            subject.journal.display()
        )));
    };
    if names(block, head) {
        Ok(Decision::Passed)
    } else {
        Ok(Decision::Failed(format!(
            "the resume block does not name HEAD ({}); it is the block at the top of the \
             journal that must, not the file somewhere",
            &head[..head.len().min(12)]
        )))
    }
}

/// The `ÉTAT DE REPRISE` section of a journal, heading included.
pub fn resume_block(text: &str) -> Option<&str> {
    let mut start = None;
    let mut level = 0usize;
    let mut offset = 0usize;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        let hashes = trimmed.chars().take_while(|c| *c == '#').count();
        if start.is_none() {
            if hashes > 0
                && trimmed[hashes..]
                    .trim()
                    .eq_ignore_ascii_case("ÉTAT DE REPRISE")
            {
                start = Some(offset);
                level = hashes;
            }
        } else if hashes > 0 && hashes <= level {
            return Some(&text[start?..offset]);
        }
        offset += line.len();
    }
    start.map(|s| &text[s..])
}

/// Whether `block` names `head` — in full or abbreviated, as a human or an
/// agent writes it. Seven hex characters is git's own floor for a short id.
fn names(block: &str, head: &str) -> bool {
    let mut run = String::new();
    for c in block.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_hexdigit() {
            run.push(c.to_ascii_lowercase());
        } else {
            if run.len() >= 7 && head.starts_with(&run) {
                return true;
            }
            run.clear();
        }
    }
    false
}

/// Gate 4, and it is two passes rather than one.
///
/// SPEC 4.4: **on the `base..HEAD` diff and commit by commit**, because a
/// forbidden commit that is then reverted leaves a clean tree and a dirty
/// history. The diff pass alone would let it through; the per-commit pass is
/// the one that catches it.
fn perimeter(subject: &Subject) -> Result<Decision, GateError> {
    if subject.role == Role::Security {
        return Ok(Decision::NotApplicable(READ_ONLY_TREE.into()));
    }
    let tree = subject.tree;
    let base = base_ref(tree, &subject.header.base)?;

    let diff = git::run(tree, &["diff", "--name-only", &format!("{base}...HEAD")])?;
    let commits: Vec<String> = git::run(tree, &["rev-list", &format!("{base}..HEAD")])?
        .lines()
        .map(str::to_string)
        .collect();

    // Where each path was seen, so the refusal can name it. A merge commit
    // shows nothing under `diff-tree` without `-m`, and that is deliberate:
    // its content is its parents', already judged.
    let mut seen: Vec<(String, String)> = diff
        .lines()
        .filter(|p| !p.is_empty())
        .map(|p| (p.to_string(), "the diff against the base".to_string()))
        .collect();
    for commit in &commits {
        let touched = git::run(
            tree,
            &["diff-tree", "--no-commit-id", "--name-only", "-r", commit],
        )?;
        for path in touched.lines().filter(|p| !p.is_empty()) {
            seen.push((
                path.to_string(),
                format!("commit {}", &commit[..7.min(commit.len())]),
            ));
        }
    }

    match subject.role {
        Role::Integrator => integrator_perimeter(subject, &seen, &commits),
        _ => coder_perimeter(subject, tree, &base, &seen),
    }
}

/// The coder's gate 4 is a **denylist**: the project's protected paths.
fn coder_perimeter(
    subject: &Subject,
    tree: &Path,
    base: &str,
    seen: &[(String, String)],
) -> Result<Decision, GateError> {
    let refuse = compile(&subject.protected_paths.refuse)?;
    let if_exists = compile(&subject.protected_paths.refuse_if_exists)?;
    for (path, where_) in seen {
        if refuse.is_match(path) {
            return Ok(Decision::Failed(format!(
                "{path} is a protected path of this project, and {where_} touches it"
            )));
        }
        // "Refused where the file already exists **on the base**": the
        // question is about the base, not about the slot's tree, where the
        // agent may have just created or deleted it.
        if if_exists.is_match(path)
            && git::run(tree, &["cat-file", "-e", &format!("{base}:{path}")]).is_ok()
        {
            return Ok(Decision::Failed(format!(
                "{path} already exists on {base} and is protected there, and {where_} touches it"
            )));
        }
    }
    Ok(Decision::Passed)
}

/// The integrator's gate 4 is an **allowlist**, and the inverse of the
/// coder's: SPEC 4.4 defines "wiring" mechanically as the path list the
/// mission declares, and every commit outside it is out of perimeter.
fn integrator_perimeter(
    subject: &Subject,
    seen: &[(String, String)],
    commits: &[String],
) -> Result<Decision, GateError> {
    let wiring = match &subject.header.integration {
        Integration::Services { wiring, .. } => wiring.clone(),
        // Nothing to integrate, so nothing declares a wiring list. An
        // integrator run on such a mission is a framing mistake, and saying
        // which one is more use than a perimeter verdict.
        Integration::None { reason } => {
            return Ok(Decision::Failed(format!(
                "this mission declares no integration ({reason}), so no wiring list exists \
                 to judge an integrator's commits against"
            )));
        }
    };
    if wiring.is_empty() {
        if commits.is_empty() {
            return Ok(Decision::Passed);
        }
        return Ok(Decision::Failed(format!(
            "the mission declares no wiring, so every commit is out of perimeter — \
             {} commit(s) are on this branch; declare `--wiring` at framing",
            commits.len()
        )));
    }
    let allowed = compile(&wiring)?;
    for (path, where_) in seen {
        if !allowed.is_match(path) {
            return Ok(Decision::Failed(format!(
                "{path} is not in this mission's wiring list, and {where_} touches it"
            )));
        }
    }
    Ok(Decision::Passed)
}

/// Path patterns, compiled once.
///
/// `globset` rather than a matcher of our own: this is the gate that keeps an
/// agent out of `.github/workflows/**` and `AGENTS.md` while those files are
/// gating it, and hand-rolled glob semantics fail in the quiet direction —
/// `**` across zero segments, `*` crossing a `/`, a leading `**/`. A
/// dependency is a decision here rather than a reflex (AGENTS.md § 7), and
/// this one is from the ripgrep tree, permissively licensed and read by half
/// the ecosystem.
fn compile(patterns: &[String]) -> Result<GlobSet, GateError> {
    let mut builder = GlobSetBuilder::new();
    let mut added: BTreeSet<&str> = BTreeSet::new();
    for pattern in patterns {
        if !added.insert(pattern.as_str()) {
            continue;
        }
        let glob = Glob::new(pattern).map_err(|source| GateError::BadPattern {
            pattern: pattern.clone(),
            source,
        })?;
        builder.add(glob);
        // `a/**` does not match `a` itself, and a project writing
        // `src/vendor/**` means the directory and everything under it.
        if let Some(prefix) = pattern.strip_suffix("/**") {
            let glob = Glob::new(prefix).map_err(|source| GateError::BadPattern {
                pattern: pattern.clone(),
                source,
            })?;
            builder.add(glob);
        }
    }
    builder.build().map_err(|source| GateError::BadPattern {
        pattern: patterns.join(", "),
        source,
    })
}
