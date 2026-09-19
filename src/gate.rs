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
    /// 8 — the mechanical security: the dependency audit, the secret scan and
    /// the static analysis the stack declares (SPEC 4.4).
    MechanicalSecurity,
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
            Gate::MechanicalSecurity => 8,
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
            Gate::MechanicalSecurity => "mechanical security",
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
    /// Set when this gate could not be played **because a mutation campaign
    /// is in flight**. Unplayed like any other, to a human; to the flow, the
    /// one obstacle `nunki` clears itself rather than a wall it must stop at.
    ///
    /// A flag and not a sentence read back: [`Report::campaign_owed`] says
    /// why — a flow that told the two apart by matching on a string would
    /// start doing something else the day the string was reworded.
    pub waits_on_campaign: bool,
}

impl Outcome {
    fn of(gate: Gate, decision: Decision) -> Self {
        Self {
            gate,
            decision,
            note: None,
            waits_on_campaign: false,
        }
    }

    /// A gate standing down while the campaign rewrites the copy of `HEAD`.
    fn waiting(gate: Gate, why: &str) -> Self {
        Self {
            gate,
            decision: Decision::Unplayed(why.to_string()),
            note: None,
            waits_on_campaign: true,
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

    /// The first gate that is **red**: a verdict on the work, and the
    /// agent's to fix.
    ///
    /// Told apart from [`Self::unplayed`] because the two ask different
    /// things of the flow, even though neither is green. A red gate earns
    /// the agent a run. A gate nobody managed to play earns it nothing: it
    /// is the machine's or the HQ's, and sending an agent back at it asks
    /// for a repair it cannot reach. Measured on 2026-09-13, where a
    /// mutation campaign that had not been run cost three coder runs — each
    /// one reading the same instruction, saying it could not act on it, and
    /// stopping — before a human held the mission.
    /// The first red gate. Private: [`Self::verdict`] is what the flow asks,
    /// and a second reading of the same outcomes is what this module spent a
    /// day removing.
    fn failed(&self) -> Option<String> {
        self.outcomes.iter().find_map(|o| match &o.decision {
            Decision::Failed(why) => Some(format!(
                "gate {} ({}): {why}",
                o.gate.number(),
                o.gate.title()
            )),
            _ => None,
        })
    }

    /// What this report means for the flow, decided **once**.
    ///
    /// Four answers and one reading of them. They used to be three
    /// projections — `failed`, `unplayed`, `campaign_owed` — recombined by
    /// hand at five sites in `verify`, and the recombination is where the
    /// defects were: a new reason for a gate to be unplayed changed what the
    /// old rule meant at all five, silently.
    ///
    /// The order is the doctrine. Red first: it is the agent's to fix, and it
    /// outranks a gate nobody could play, because then there is something to
    /// send an agent back for. Then the campaign, which is the one obstacle
    /// `nunki` clears itself. Then the wall, which waits on a human.
    pub fn verdict(&self) -> Verdict {
        if let Some(reason) = self.failed() {
            return Verdict::Red(reason);
        }
        // Every gate nobody could play, and whether the campaign is what it
        // waits on. Gate 7 unplayed **is** a campaign owed — no campaign has
        // run, or the one on file ran on other content, and running one
        // answers both. A gate that stood down *while* a campaign runs is the
        // same obstacle seen from the other side.
        let mut owed: Option<String> = None;
        let mut wall: Option<&Outcome> = None;
        for outcome in &self.outcomes {
            let Decision::Unplayed(why) = &outcome.decision else {
                continue;
            };
            if outcome.gate == Gate::Mutation {
                // Gate 7's own sentence first: it is the one that names the
                // verb, where a gate standing down only says to look again.
                owed = Some(why.clone());
            } else if outcome.waits_on_campaign {
                owed.get_or_insert_with(|| why.clone());
            } else {
                wall.get_or_insert(outcome);
            }
        }
        // A wall outranks the campaign, and this is the older rule kept: a
        // profile that will not come up leaves gate 6 unplayed too, and
        // spending an hour of mutation in front of a wall nothing will move
        // is an hour spent for nothing.
        if let Some(outcome) = wall {
            let Decision::Unplayed(why) = &outcome.decision else {
                unreachable!("only unplayed outcomes reach here")
            };
            return Verdict::Wall(format!(
                "gate {} ({}) could not be played: {why}",
                outcome.gate.number(),
                outcome.gate.title()
            ));
        }
        match owed {
            Some(why) => Verdict::CampaignOwed(why),
            None => Verdict::Green,
        }
    }

    pub fn passed(&self) -> bool {
        self.failure().is_none()
    }
}

/// What a report means for the flow (SPEC 4.4).
///
/// One value and not three questions, because the three had to be asked in
/// the right order and every caller asked them itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Every gate that applies was played and none is red.
    Green,
    /// A gate is red, and it is the agent's to fix.
    Red(String),
    /// A gate could not be played, and no run would change that: the profile
    /// that is not up, the database nobody filled. The flow waits on a human,
    /// and the sentence names the verb that unblocks it.
    Wall(String),
    /// The only thing in the way is a mutation campaign — one that has not
    /// run, one that ran on other content, or one running right now that the
    /// gates are standing down for. `nunki` clears this itself.
    CampaignOwed(String),
}

#[derive(Debug, thiserror::Error)]
pub enum GateError {
    #[error(transparent)]
    Git(#[from] git::GitError),
    #[error("{0} could not be read: {1}")]
    Unreadable(std::path::PathBuf, std::io::Error),
    #[error("the mutation campaign could not be read: {0}")]
    Mutants(String),
    #[error("the mechanical security report could not be read: {0}")]
    Security(String),
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
    /// The commit the coder's gates were green on, once they were — where the
    /// integrator's work begins. `None` until then.
    pub coder_head: Option<&'a str>,
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

/// Play the gates a run owes at its end: 1 to 4, and 8.
///
/// Gate 8 is here for the reason gates 1 to 4 are, and because it is cheap:
/// an advisory introduced at the first lot must not be found at the fifth
/// (SPEC 4.4).
pub fn after_run(subject: &Subject, verification: &Verification) -> Result<Report, GateError> {
    play(subject, verification, Phase::AfterRun)
}

/// Play every gate that exists today: 1 to 4, then the deliverable and the
/// battery. Gate 7 is not here yet, and this says so rather than implying a
/// complete verification.
pub fn at_verification(
    subject: &Subject,
    verification: &Verification,
) -> Result<Report, GateError> {
    play(subject, verification, Phase::Final)
}

/// Which gates are owed: SPEC 4.4 plays 1 to 4 and 8 at the end of every run,
/// and 5 to 7 once the last lot is done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    AfterRun,
    Final,
}

fn play(subject: &Subject, verification: &Verification, phase: Phase) -> Result<Report, GateError> {
    let head = git::head(subject.tree)?;
    // Asked once, here, and not inside the two gates that would have to
    // answer it: the gates stand down together or not at all, and the flow
    // needs to know **that** is why they did.
    let campaign = campaign_in_flight(verification);
    let mut outcomes = vec![
        Outcome::of(Gate::CleanTree, clean_tree(subject)?),
        Outcome::of(Gate::BranchAhead, branch_ahead(subject)?),
        Outcome::of(Gate::ResumeNamesHead, resume_names_head(subject, &head)?),
        Outcome::of(Gate::Perimeter, perimeter(subject)?),
    ];
    if phase == Phase::Final {
        outcomes.push(Outcome::of(Gate::Deliverable, deliverable(subject)?));
        outcomes.push(match &campaign {
            Some(why) => Outcome::waiting(Gate::Battery, why),
            None => Outcome::of(Gate::Battery, battery(subject, verification)?),
        });
        outcomes.push(mutation(subject)?);
    }
    outcomes.push(match &campaign {
        Some(why) => Outcome::waiting(Gate::MechanicalSecurity, why),
        None => Outcome::of(
            Gate::MechanicalSecurity,
            mechanical_security(subject, verification)?,
        ),
    });
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

/// The base as this clone knows it, and **the remote-tracking one first**.
///
/// A slot is a clone whose `origin` is the project on this machine, and
/// `run::branch` starts every mission branch from `origin/<base>` after
/// fetching it. The clone's own `<base>` is written once, when the slot is
/// made, and nothing moves it again — so from the second mission onwards the
/// two disagree, and the gates were reading the one the branch did not come
/// from.
///
/// Measured on 2026-09-18, on `notes-api`'s slot, with a branch that had
/// touched nothing: `dev...HEAD` named eight files — everything the previous
/// mission had merged — while `origin/dev...HEAD` named none. Gate 4 would
/// have failed a coder for a perimeter it had not left, on its first lot,
/// and gate 8's fork point was one merge too early.
///
/// The local branch stays as the fallback: a clone whose origin does not
/// carry the base still has to be judged against something, and that is the
/// only candidate left. It is the same order `run::branch` uses.
fn base_ref(tree: &Path, base: &str) -> Result<Rev, GateError> {
    let remote = format!("origin/{base}");
    if git::run(
        tree,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/remotes/{remote}"),
        ],
    )
    .is_ok()
    {
        return Ok(Rev(remote));
    }
    Ok(Rev(base.to_string()))
}

/// A revision `git` resolves exactly as given: a commit, or a ref that
/// already names one.
///
/// **Never a mission's base by its header name.** In a slot that name has two
/// readings, `dev` and `origin/dev`, and they diverge from the second mission
/// onwards; [`touched_since_base`] is the one thing that picks between them.
///
/// A type and not a convention, because the convention is what failed twice.
/// `touched_paths` and `touched_since_base` had the same signature — `(&Path,
/// &str)` — and differed only by a doc comment telling the caller which to
/// use. One caller read the name where the other read the ref, gate 7's
/// fingerprint never matched the campaign's, and `verify` went round it
/// fifty-seven times (`notes-4`, 2026-09-18). A third caller would have had
/// the same chance to get it wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rev(String);

impl Rev {
    /// A commit, as git printed it. The one caller: `nunki push`, asking what
    /// the integrator added after the coder's gates were green — a commit,
    /// never a base.
    pub fn commit(sha: impl Into<String>) -> Self {
        Self(sha.into())
    }
}

impl std::fmt::Display for Rev {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
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
    if subject.role == Role::Integrator {
        // The integrator works on the coder's branch (SPEC 2), so the coder's
        // commits come first on it and none of them is wiring. Both passes —
        // the diff and every commit — start where the coder's gates were
        // green, the same line `nunki push` draws.
        let Some(from) = subject.coder_head else {
            return Ok(Decision::Unplayed(
                "no commit is recorded for the coder's green gates, so nothing tells the \
                 integrator's commits from the coder's"
                    .into(),
            ));
        };
        let Touched { seen, commits } = touched(tree, &Rev::commit(from))?;
        return integrator_perimeter(subject, &seen, &commits);
    }

    let base = base_ref(tree, &subject.header.base)?;
    let Touched { seen, .. } = touched(tree, &base)?;
    coder_perimeter(subject, tree, &base, &seen)
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

fn touched(tree: &Path, base: &Rev) -> Result<Touched, GateError> {
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

/// What this branch brought, by the **name** its mission header gives the
/// base — the one function anything outside this module should ask.
///
/// A name has two readings in a slot, `dev` and `origin/dev`, and they differ
/// from the second mission onwards ([`base_ref`] says why). Gate 4 and gate 7
/// resolve it; the campaign's launcher passed the raw name, so the two
/// computed different sets of the same thing.
///
/// Measured on `notes-4`, 2026-09-18: the launcher's set held eight paths,
/// gate 7's held four, and their fingerprints could therefore never agree.
/// Gate 7 asked for a campaign, the campaign ran, gate 7 said it had run on
/// other content, and round again — fifty-seven turns of it. The comment on
/// [`touched`] already names this hazard for gates 4 and 7: "computing it
/// twice in two ways is how the two gates would come to disagree". It was a
/// third caller that did it.
///
/// [`touched_paths`] stays, for the one caller that has a [`Rev`] rather than
/// a name: `nunki push`, which asks what the integrator added after the
/// coder's gates were green — a commit, and now a commit in the type too.
pub fn touched_since_base(tree: &Path, base: &str) -> Result<Vec<String>, GateError> {
    let base = base_ref(tree, base)?;
    touched_paths(tree, &base)
}

/// The paths a mutation campaign runs on: what this branch touched, once
/// each, in a stable order.
///
/// Takes a [`Rev`], which a mission's base by name is not — see
/// [`touched_since_base`] for that. The two used to share a signature and be
/// told apart by this sentence alone.
pub fn touched_paths(tree: &Path, base: &Rev) -> Result<Vec<String>, GateError> {
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
    base: &Rev,
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
///
/// Public because `nunki push` judges the integrator's commits by the very
/// allowlist gate 4 judged them by, and two compilations of one list are two
/// lists waiting to disagree.
pub fn compile(patterns: &[String]) -> Result<GlobSet, GateError> {
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
                    "{} is not a verdict `nunki` can read: {e}",
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
             pull request with its own section, and that heading is how `nunki` sees it",
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
/// a slot, and where an integration mission declares its system tests. They
/// live in the project's home and reach the container read-only, at
/// [`crate::run::STACK_AT`]: what judges an agent is not the agent's to write.
pub const BATTERY: &str = "prepush.sh";
pub const SYSTEM_BATTERY: &str = "system.sh";

/// Where a stack fragment declares its mechanical security (SPEC 4.4, gate
/// 8): the dependency audit, the secret scan and the static analysis, each
/// reported through the contract `nunki` reads and neither runs nor
/// understands.
pub const SECURITY: &str = "security.sh";

/// Gate 8: the mechanical security the stack declares (SPEC 4.4).
///
/// `nunki` runs `security.sh` and reads the contract it prints. It knows no
/// lockfile, no advisory database and no scanner: the three rules below turn
/// on the contract's own fields, and none of them names a tool.
///
/// Not the security agent's. It attacks what is built and owes findings, not
/// a tool's report — the per-role table says so, and so does the battery two
/// gates above.
fn mechanical_security(
    subject: &Subject,
    verification: &Verification,
) -> Result<Decision, GateError> {
    if subject.role == Role::Security {
        return Ok(Decision::NotApplicable(
            "the security agent attacks what is built; it owes findings, not a tool's \
             report"
                .into(),
        ));
    }
    // The base as a commit, and never as a name. The script runs in the clean
    // copy of `HEAD`, which is a detached clone of the slot: it holds no local
    // branch, and the refresh only ever fetches `HEAD`, so no remote-tracking
    // ref follows the base either. A name resolves to nothing there and the
    // script counts every finding as new — so the gate blocks on what the
    // branch did not bring, which is the one thing SPEC 4.4 asks it not to do.
    //
    // Measured on 2026-09-18, the first time gate 8 ran against a real
    // container: `git rev-parse --verify dev` in the copy answered "Needed a
    // single revision", and notes-api's single finding — a test credential its
    // base already carried — came back `was_at_base: false` and red. Passing
    // the commit turned it green in the same container.
    //
    // The fork point rather than the base's tip: gate 2 has already said the
    // branch is ahead of its base, so the fork point is an ancestor of `HEAD`
    // and is in the copy by construction. The tip is not — a slot's base can
    // move after the copy was cloned, and then the commit would not be there.
    //
    // Through `base_ref`, because a slot is a clone: its base may exist only
    // as `origin/<base>`, and that is gate 2's reading of the same name. A base
    // with no commit in common stops the report here rather than deciding, the
    // way gates 2 and 4 already do — they read the same two names and would
    // have refused first.
    let named = base_ref(subject.tree, &subject.header.base)?;
    let base = crate::git::run(subject.tree, &["merge-base", "HEAD", &named.to_string()])?
        .trim()
        .to_string();
    let at = format!("{}/{SECURITY}", crate::run::STACK_AT);
    // Absent, not executable, no database, a base it cannot read, or its own
    // status — told apart, because "the gate is red", "there was nothing to
    // run" and "it could not look" send a human to three different places.
    let probe = format!(
        "if [ ! -f {at} ]; then exit 66; fi\n\
         if [ ! -x {at} ]; then exit 67; fi\n\
         exec {at} {base} {db} {mission} {secrets}\n",
        db = crate::run::ADVISORIES_AT,
        mission = crate::run::MISSION_AT,
        secrets = crate::secrets::AT,
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
        Err(e) => return Ok(Decision::Unplayed(e.to_string())),
    };
    match out.status {
        66 => {
            return Ok(Decision::Failed(format!(
                "there is no security script at {at}: the stack in the project's home \
                 ships no {SECURITY} — a proof nobody can run is not a proof that \
                 passed (SPEC 4.4). `nunki init --stack <name>` writes it"
            )));
        }
        67 => {
            return Ok(Decision::Failed(format!(
                "{at} is not executable, so nothing ran"
            )));
        }
        // The script's own word for "I have no advisory database". Unplayed
        // and not red: a verdict on the machine rather than on the agent, and
        // a security gate that could not consult its database must not report
        // green (SPEC 4.4).
        69 => {
            return Ok(Decision::Unplayed(format!(
                "no advisory database at {}: the stack declares where it lives on the \
                 host, and the host fills it — `cargo deny check advisories` once is \
                 enough",
                crate::run::ADVISORIES_AT
            )));
        }
        // The script's word for "the base is not here". `nunki` resolves it to
        // a commit above and should never hand over one the copy lacks, so
        // this is the second lock: unplayed rather than a comparison against
        // nothing, which would report every finding as brought by the branch.
        70 => {
            return Ok(Decision::Unplayed(format!(
                "{base} is not in the clean copy of HEAD, so there is nothing to \
                 compare this branch against"
            )));
        }
        _ => {}
    }
    let findings =
        crate::security::read(&out.stdout).map_err(|e| GateError::Security(e.to_string()))?;

    // What the branch brought and nobody accepted.
    let blocking: Vec<String> = findings
        .iter()
        .filter(|f| f.blocks())
        .map(|f| f.say())
        .collect();
    if !blocking.is_empty() {
        // A secret is the one finding no agent can close: removing it in a
        // later commit leaves it in the branch's history. So the line says
        // what the human's next gesture is, and the id it names is the
        // argument of that gesture — `say()` puts it first for this.
        let secret = findings
            .iter()
            .any(|f| f.blocks() && f.kind == crate::security::Kind::Secret);
        let next = if secret {
            ". A secret is yours to rule on: revoke it, then `nunki secret accept <id> \
             --because <why>` so the gate stops reporting it"
        } else {
            ""
        };
        return Ok(Decision::Failed(format!(
            "{} finding(s) this branch brought, and nobody has ruled on: {}{next}",
            blocking.len(),
            head_of(&blocking, 10)
        )));
    }
    // An acceptance a fix has overtaken. Red, because it is work an agent can
    // do: the exception was written when nothing could be, and something can.
    let stale: Vec<String> = findings
        .iter()
        .filter(|f| f.stale())
        .map(|f| f.say())
        .collect();
    if !stale.is_empty() {
        return Ok(Decision::Failed(format!(
            "{} exception(s) a fix has overtaken — apply it and drop the exception: {}",
            stale.len(),
            head_of(&stale, 10)
        )));
    }
    Ok(Decision::Passed)
}

/// Gate 6: the battery, green, on the clean copy of `HEAD`.
///
/// Two things SPEC 4.4 is explicit about. A battery that is **absent or not
/// executable** makes the gate fail; it does not make it skip — a proof
/// nobody can run is not a proof that passed. And it runs on the copy of
/// `HEAD` (`nunki exec`), never in the tree the agent has been living in.
///
/// The integrator's battery is not the coder's: SPEC 4.4 says its gate 6 is
/// **its system tests, in the system profile**. It is declared separately,
/// and a stack that declares none fails this gate by the same rule.
/// Why nothing may run in the clean copy of `HEAD` right now, if a campaign
/// is in flight.
///
/// Gate 7's campaign runs `cargo mutants --in-place` **in that copy**, and
/// every other gate that runs there goes through `exec::run(On::Proof)`,
/// which refreshes it first — `git reset --hard`, `git clean`. So the two
/// wreck each other, both ways at once: the gate compiles and tests a
/// mutant, and the reset pulls the tree out from under the campaign.
///
/// Measured on `notes-4`, 2026-09-18, the first three-agent mission run end
/// to end. The battery came back 101 on `warning: unused variable: value` at
/// `src/api.rs:160` — a function whose body cargo-mutants had replaced, and
/// which uses its argument in the coder's own code. The flow read gate 6 red,
/// opened a volet, and did it again until the volets were spent: six runs,
/// 59M tokens, and the integrator and the security agent never ran. No agent
/// had done anything wrong.
///
/// Unplayed and not red, which is the rule the missing advisory database
/// already follows: a verdict on the machine rather than on the agent. A gate
/// nobody could play stops the verification instead of opening a volet, so
/// the mission waits for the campaign rather than paying for it.
///
/// The file's presence is the answer, not the process's liveness: `verify`
/// clears it on the turn that reads the campaign back, so a file left by a
/// crash costs one unplayed turn and no more. Erring towards "I could not
/// look" is the direction this project errs in.
///
/// Read by slot and not by mission, because that is what the copy belongs to.
fn campaign_in_flight(verification: &Verification) -> Option<String> {
    let running =
        crate::mutants::read_running(&verification.project.hq_root, &verification.slot.name)
            .ok()??;
    Some(format!(
        "a mutation campaign has been rewriting the clean copy of HEAD since {} — it \
         runs in place there, so this would judge a mutant and reset the tree under \
         the campaign. `nunki verify` plays it again once the campaign is read back",
        running.started_at
    ))
}

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
    let at = format!("{}/{script}", crate::run::STACK_AT);
    // Absent, not executable, or its own status — told apart, because
    // "the gate is red" and "there was nothing to run" send a human to
    // different places.
    let probe = format!(
        "if [ ! -f {at} ]; then exit 66; fi\n\
         if [ ! -x {at} ]; then exit 67; fi\n\
         exec {at}\n"
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
            "there is no battery at {at}: the stack in the project's home ships no \
             {script} — a proof nobody can run is not a proof that passed (SPEC 4.4)"
        ))),
        67 => Ok(Decision::Failed(format!(
            "{at} is not executable, so nothing ran"
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
    let paths = touched_since_base(subject.tree, &subject.header.base)?;
    let want = crate::mutants::fingerprint(subject.tree, &paths)
        .map_err(|e| GateError::Mutants(e.to_string()))?;

    let campaign =
        crate::mutants::read(subject.mission_dir).map_err(|e| GateError::Mutants(e.to_string()))?;
    let Some(campaign) = campaign else {
        return Ok(Outcome::of(
            gate,
            Decision::Unplayed(
                "no mutation campaign has run on this mission — `nunki mission mutants` \
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
                 nothing about the code as it stands — `nunki mission mutants` runs it again",
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
