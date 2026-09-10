//! The verification gates (SPEC 4.4).
//!
//! Gates **1 to 4** are played at the end of every run, not only at the final
//! verification — decided on 2026-09-09, because a perimeter gate that only
//! falls at the end loses a six-hour mission over a forbidden write in the
//! first lot. Gates **5 and 6** — the deliverable and the battery — are
//! played at the final verification, and need a way into the slot's
//! container. Gate 7, the mutation campaign, is long enough to be watched
//! like a run and is not here yet.
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
    /// 5 — the deliverable this role owes.
    Deliverable,
    /// 6 — the battery, green, on the clean copy of `HEAD`.
    Battery,
    /// 7 — every mutant that survived has an outcome.
    Mutation,
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
            Gate::Deliverable => 5,
            Gate::Battery => 6,
            Gate::Mutation => 7,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Gate::CleanTree => "clean tree",
            Gate::BranchAhead => "branch not protected and ahead of its base",
            Gate::ResumeNamesHead => "the resume block names HEAD",
            Gate::Perimeter => "perimeter",
            Gate::Deliverable => "the deliverable is there",
            Gate::Battery => "the battery is green",
            Gate::Mutation => "every survivor has an outcome",
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
    /// The gate applies and could not be played at all — no profile up, no
    /// engine. Neither green nor red: a check that cannot say "I do not
    /// know" will lie, and the lie here would blame an agent for a machine
    /// (AGENTS.md § 4, learned the hard way on liveness). A mission is not
    /// verified over a gate nobody managed to run.
    Unplayed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub gate: Gate,
    pub decision: Decision,
    /// Something a green gate still owes the reader. Gate 7 uses it to say
    /// how many survivors passed on an outcome no gate can check — left
    /// unsaid, that outcome becomes the escape hatch that empties the gate.
    pub note: Option<String>,
}

impl Outcome {
    fn of(gate: Gate, decision: Decision) -> Self {
        Self {
            gate,
            decision,
            note: None,
        }
    }
}

/// What the gates of one run decided, all four of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub role: Role,
    pub head: String,
    pub outcomes: Vec<Outcome>,
}

impl Report {
    /// The first gate that stops this report being green, in specification
    /// order — red, or never played at all. The flow needs one sentence
    /// (`GatesFailed { reason }`); a human needs the whole report, which is
    /// why both exist.
    pub fn failure(&self) -> Option<String> {
        self.outcomes.iter().find_map(|o| {
            let (number, title) = (o.gate.number(), o.gate.title());
            match &o.decision {
                Decision::Failed(why) => Some(format!("gate {number} ({title}): {why}")),
                // Not the same sentence, and not the same kind of fact: one
                // is about the work, the other about the machine. But
                // neither is a green report.
                Decision::Unplayed(why) => Some(format!(
                    "gate {number} ({title}) could not be played: {why}"
                )),
                _ => None,
            }
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
    #[error("the mutation campaign could not be read: {0}")]
    Mutants(String),
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
    /// The mission's `PR.md`, on the host — the coder's and the integrator's
    /// deliverable (SPEC 4.4, gate 5).
    pub pr: &'a Path,
    /// The mission's `VERDICT.json`, on the host — where the security
    /// agent's report lives, because it commits nothing.
    pub verdict: &'a Path,
    /// The mission folder itself, where the mutation campaign's file lives
    /// (SPEC 4.4, gate 7: "un fichier du dossier de mission que le HQ lit").
    pub mission_dir: &'a Path,
    pub header: &'a Header,
    pub protected_branches: &'a [String],
    pub protected_paths: &'a ProtectedPaths,
}

/// What gates 5 and 6 need beyond the tree: a way into the slot's container,
/// because a battery is replayed there and nowhere else (SPEC 4.4).
pub struct Verification<'a> {
    pub project: &'a crate::project::Project,
    pub slot: &'a crate::slot::Slot,
    pub engine: std::sync::Arc<dyn crate::engine::Engine>,
    /// Which stack fragment declares the battery.
    pub stack: &'a str,
}

/// Play gates 1 to 4 for one run.
pub fn after_run(subject: &Subject) -> Result<Report, GateError> {
    play(subject, None)
}

/// Play every gate that exists today: 1 to 4, then the deliverable and the
/// battery. Gate 7 is not here yet, and this says so rather than implying a
/// complete verification.
pub fn at_verification(
    subject: &Subject,
    verification: &Verification,
) -> Result<Report, GateError> {
    play(subject, Some(verification))
}

fn play(subject: &Subject, verification: Option<&Verification>) -> Result<Report, GateError> {
    let head = git::head(subject.tree)?;
    let mut outcomes = vec![
        Outcome::of(Gate::CleanTree, clean_tree(subject)?),
        Outcome::of(Gate::BranchAhead, branch_ahead(subject)?),
        Outcome::of(Gate::ResumeNamesHead, resume_names_head(subject, &head)?),
        Outcome::of(Gate::Perimeter, perimeter(subject)?),
    ];
    if let Some(verification) = verification {
        outcomes.push(Outcome::of(Gate::Deliverable, deliverable(subject)?));
        outcomes.push(Outcome::of(Gate::Battery, battery(subject, verification)?));
        outcomes.push(mutation(subject)?);
    }
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

    let Touched { seen, commits } = touched(tree, &base)?;

    match subject.role {
        Role::Integrator => integrator_perimeter(subject, &seen, &commits),
        _ => coder_perimeter(subject, tree, &base, &seen),
    }
}

/// Every path this branch touched, and where it was seen — the diff against
/// the base, and every commit on it.
///
/// Two passes rather than one, because SPEC 4.4 says so and because a
/// forbidden commit that is then reverted leaves a clean tree and an innocent
/// diff. Gate 7 reads the same set: what a mutation campaign runs on is what
/// this branch touched, and computing it twice in two ways is how the two
/// gates would come to disagree.
struct Touched {
    /// Each path, with where it was seen — the diff, or a named commit.
    seen: Vec<(String, String)>,
    /// The commits on the branch, in `rev-list` order.
    commits: Vec<String>,
}

fn touched(tree: &Path, base: &str) -> Result<Touched, GateError> {
    let diff = git::run(tree, &["diff", "--name-only", &format!("{base}...HEAD")])?;
    let commits: Vec<String> = git::run(tree, &["rev-list", &format!("{base}..HEAD")])?
        .lines()
        .map(str::to_string)
        .collect();
    // A merge commit shows nothing under `diff-tree` without `-m`, and that
    // is deliberate: its content is its parents', already judged.
    let mut seen: Vec<(String, String)> = diff
        .lines()
        .filter(|p| !p.is_empty())
        .map(|p| (p.to_string(), "the diff against the base".to_string()))
        .collect();
    for commit in &commits {
        let in_commit = git::run(
            tree,
            &["diff-tree", "--no-commit-id", "--name-only", "-r", commit],
        )?;
        for path in in_commit.lines().filter(|p| !p.is_empty()) {
            seen.push((
                path.to_string(),
                format!("commit {}", &commit[..7.min(commit.len())]),
            ));
        }
    }
    Ok(Touched { seen, commits })
}

/// The paths a mutation campaign runs on: what this branch touched, once
/// each, in a stable order.
pub fn touched_paths(tree: &Path, base: &str) -> Result<Vec<String>, GateError> {
    let mut paths: Vec<String> = touched(tree, base)?
        .seen
        .into_iter()
        .map(|(path, _)| path)
        .collect();
    paths.sort();
    paths.dedup();
    Ok(paths)
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

/// Gate 5: the deliverable this role owes (SPEC 4.4, the per-role table).
///
/// The coder and the integrator owe `PR.md` — the integrator's completed with
/// its own section, and "completed" is decided mechanically, by a heading,
/// because a gate that needs a reader is not a gate. The security agent
/// commits nothing, so what it owes is its report, inside `VERDICT.json`.
fn deliverable(subject: &Subject) -> Result<Decision, GateError> {
    if subject.role == Role::Security {
        let text = read(subject.verdict)?;
        if text.trim().is_empty() {
            return Ok(Decision::Failed(format!(
                "{} is empty: the security agent's deliverable is its report, and it \
                 writes it there",
                subject.verdict.display()
            )));
        }
        let file: crate::mission::VerdictFile = match serde_json::from_str(&text) {
            Ok(f) => f,
            Err(e) => {
                return Ok(Decision::Failed(format!(
                    "{} is not a verdict `hq` can read: {e}",
                    subject.verdict.display()
                )));
            }
        };
        if file.report.trim().is_empty() {
            return Ok(Decision::Failed(format!(
                "{} carries a verdict but no report, and the report is the deliverable",
                subject.verdict.display()
            )));
        }
        return Ok(Decision::Passed);
    }

    let text = read(subject.pr)?;
    if text.trim().is_empty() {
        return Ok(Decision::Failed(format!(
            "{} is empty: a mission's deliverable is the pull request it describes",
            subject.pr.display()
        )));
    }
    if subject.role == Role::Integrator && !has_heading(&text, "integration") {
        return Ok(Decision::Failed(format!(
            "{} has no `Integration` heading: the integrator completes the coder's \
             pull request with its own section, and that heading is how `hq` sees it",
            subject.pr.display()
        )));
    }
    Ok(Decision::Passed)
}

/// A markdown heading whose text begins with `word`, case-insensitively.
fn has_heading(text: &str, word: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim();
        let hashes = line.chars().take_while(|c| *c == '#').count();
        hashes > 0 && line[hashes..].trim().to_lowercase().starts_with(word)
    })
}

/// Where a stack fragment declares what must be silent before anything leaves
/// a slot, and where an integration mission declares its system tests. Paths
/// **inside the tree**, because they are committed and therefore judged as
/// part of what is delivered.
pub const BATTERY: &str = "prepush.sh";
pub const SYSTEM_BATTERY: &str = "system.sh";

/// Gate 6: the battery, green, on the clean copy of `HEAD`.
///
/// Two things SPEC 4.4 is explicit about. A battery that is **absent or not
/// executable** makes the gate fail; it does not make it skip — a proof
/// nobody can run is not a proof that passed. And it runs on the copy of
/// `HEAD` (`hq exec`), never in the tree the agent has been living in.
///
/// The integrator's battery is not the coder's: SPEC 4.4 says its gate 6 is
/// **its system tests, in the system profile**. It is declared separately,
/// and a stack that declares none fails this gate by the same rule.
fn battery(subject: &Subject, verification: &Verification) -> Result<Decision, GateError> {
    if subject.role == Role::Security {
        return Ok(Decision::NotApplicable(
            "the security agent attacks what is built; it owes findings, not a green \
             battery"
                .into(),
        ));
    }
    let script = match subject.role {
        Role::Integrator => SYSTEM_BATTERY,
        _ => BATTERY,
    };
    let at = format!(
        "{}/stacks/{}/{script}",
        crate::project::FRAGMENTS_DIR,
        verification.stack
    );
    // Absent, not executable, or its own status — told apart, because
    // "the gate is red" and "there was nothing to run" send a human to
    // different places.
    let probe = format!(
        "if [ ! -f {at} ]; then exit 66; fi\n\
         if [ ! -x {at} ]; then exit 67; fi\n\
         exec ./{at}\n"
    );
    let out = crate::exec::run(
        verification.project,
        verification.slot,
        verification.engine.clone(),
        &["sh".to_string(), "-c".to_string(), probe],
        crate::exec::On::Proof,
    );
    let out = match out {
        Ok(out) => out,
        // Not red: red would be a verdict on the agent, and this is a
        // verdict on the machine.
        Err(e) => return Ok(Decision::Unplayed(e.to_string())),
    };
    match out.status {
        0 => Ok(Decision::Passed),
        66 => Ok(Decision::Failed(format!(
            "there is no battery at {at} on this commit — a proof nobody can run is \
             not a proof that passed (SPEC 4.4)"
        ))),
        67 => Ok(Decision::Failed(format!(
            "{at} is not executable on this commit, so nothing ran"
        ))),
        status => Ok(Decision::Failed(format!(
            "the battery came back {status}:\n{}",
            tail(&out.stderr, &out.stdout)
        ))),
    }
}

/// The last lines a human needs, from whichever stream said something.
fn tail(stderr: &str, stdout: &str) -> String {
    let text = if stderr.trim().is_empty() {
        stdout
    } else {
        stderr
    };
    let lines: Vec<&str> = text.lines().collect();
    let from = lines.len().saturating_sub(20);
    lines[from..].join("\n")
}

fn read(path: &Path) -> Result<String, GateError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        // A file the agent never wrote reads as an empty deliverable, which
        // is what it is.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(GateError::Unreadable(path.to_path_buf(), e)),
    }
}

/// Gate 7: every mutant that survived has received an outcome (SPEC 4.4).
///
/// Deterministic, and it reads a file: the campaign itself is long, runs in
/// the slot's container and is watched like a run (see [`crate::mutants`]).
/// **No threshold** — the gate is green when every survivor has one of three
/// outcomes, never when a score clears a bar.
///
/// Three states it must keep apart:
///
/// - no campaign, or one that ran on other content: **unplayed**. Nobody has
///   asked the question yet, and answering "green" would be a lie about work
///   that was never done.
/// - survivors nobody has triaged: **red**, and the message names them. SPEC
///   makes the return of survivors to the coder "un run de plus sur le lot,
///   pas un volet", which is what a failed gate produces.
/// - every survivor answered: green, and the note says how many rode on
///   `equivalent`, the one outcome no gate can check.
fn mutation(subject: &Subject) -> Result<Outcome, GateError> {
    let gate = Gate::Mutation;
    if subject.role != Role::Coder {
        // SPEC 4.4's per-role table: "non (des tests système et de la
        // configuration ne se mutent pas)".
        return Ok(Outcome::of(
            gate,
            Decision::NotApplicable(
                "system tests and configuration are not mutated (SPEC 4.4)".into(),
            ),
        ));
    }
    let base = base_ref(subject.tree, &subject.header.base)?;
    let paths = touched_paths(subject.tree, &base)?;
    let want = crate::mutants::fingerprint(subject.tree, &paths)
        .map_err(|e| GateError::Mutants(e.to_string()))?;

    let campaign =
        crate::mutants::read(subject.mission_dir).map_err(|e| GateError::Mutants(e.to_string()))?;
    let Some(campaign) = campaign else {
        return Ok(Outcome::of(
            gate,
            Decision::Unplayed(
                "no mutation campaign has run on this mission — `hq mission mutants` \
                 starts one"
                    .into(),
            ),
        ));
    };
    if campaign.fingerprint != want {
        return Ok(Outcome::of(
            gate,
            Decision::Unplayed(format!(
                "the campaign in {} ran on other content ({} against {}), so it says \
                 nothing about the code as it stands — `hq mission mutants` runs it again",
                crate::mutants::FILE,
                &campaign.fingerprint[..7.min(campaign.fingerprint.len())],
                &want[..7.min(want.len())]
            )),
        ));
    }

    // Two sources, and which one a line came from is decided by the mount,
    // not by the line: the coder can only write its own file, and the
    // outcome no machine can check is not in it (SPEC 4.1, 4.4).
    let coders = crate::mutants::read_triage(subject.mission_dir)
        .map_err(|e| GateError::Mutants(e.to_string()))?;
    for (id, outcome) in &coders {
        if !outcome.is_the_coders_to_give() {
            return Ok(Outcome::of(
                gate,
                Decision::Failed(format!(
                    "{} answers {id} with `{}`, and that outcome is not the coder's to \
                     give: nothing can check it, so it is the HQ's — write it in {}",
                    crate::mutants::TRIAGE_FILE,
                    outcome.kind(),
                    crate::mutants::FILE
                )),
            ));
        }
    }
    let answer = |s: &crate::mutants::Survivor| -> Option<crate::mutants::Triage> {
        coders.get(&s.id).cloned().or_else(|| s.outcome.clone())
    };

    let untriaged: Vec<String> = campaign
        .survivors
        .iter()
        .filter(|s| answer(s).is_none())
        .map(|s| format!("{}:{} {}", s.file, s.line, s.id))
        .collect();
    if !untriaged.is_empty() {
        return Ok(Outcome::of(
            gate,
            Decision::Failed(format!(
                "{} survivor(s) have no outcome, and there is no threshold to hide \
                 behind — each needs a named test, a sentence saying it is equivalent, \
                 or a bug frozen in a test: {}",
                untriaged.len(),
                head_of(&untriaged, 10)
            )),
        ));
    }

    // A named test has to exist. "A test covers this" is not an outcome; a
    // test called `x` is, and whether `x` is there is a fact.
    for survivor in &campaign.survivors {
        let Some(outcome) = answer(survivor) else {
            continue;
        };
        if let Some(test) = outcome.test()
            && git::run(subject.tree, &["grep", "--quiet", "-F", "--", test]).is_err()
        {
            return Ok(Outcome::of(
                gate,
                Decision::Failed(format!(
                    "{}:{} names the test {test:?}, and nothing in the tree is called \
                     that",
                    survivor.file, survivor.line
                )),
            ));
        }
    }

    let equivalent = campaign
        .survivors
        .iter()
        .filter(|s| matches!(answer(s), Some(crate::mutants::Triage::Equivalent { .. })))
        .count();
    let mut outcome = Outcome::of(gate, Decision::Passed);
    if equivalent > 0 {
        outcome.note = Some(format!(
            "{equivalent} of {} rode on `equivalent`, which no machine can check — they \
             come from the HQ's own hand, and they are counted here so nobody has to \
             go looking",
            campaign.survivors.len()
        ));
    }
    Ok(outcome)
}

fn head_of(items: &[String], n: usize) -> String {
    let shown = items.iter().take(n).cloned().collect::<Vec<_>>().join("; ");
    if items.len() > n {
        format!("{shown}; …")
    } else {
        shown
    }
}
