//! The mutation campaign and its file (SPEC 4.4, gate 7).
//!
//! Gate 7 is deterministic and reads a file — SPEC says so in as many words:
//! *"Son résultat est un fichier du dossier de mission que le HQ lit."* The
//! campaign that produces that file is a separate, long thing: it runs in the
//! slot's container, on the clean copy of `HEAD`, launched detached and
//! watched like a run. This module owns both halves; the gate itself lives in
//! [`crate::gate`].
//!
//! **No threshold at `critical`**, the default rigor. The gate is green when
//! every survivor has received one of three outcomes, not when a score clears
//! a bar. A threshold and a triage pull in opposite directions, and Google,
//! whose practice this borrows, keeps neither score nor bar. A `standard`
//! mission trades the triage for the project's `mutation_threshold`, and that
//! is why a campaign also records how many mutants it [`Campaign::tried`].
//!
//! **A campaign replays only when the touched files have changed**, and
//! "changed" means their content: the fingerprint is over git blob ids, not
//! over `HEAD`. A rebase moves `HEAD` while every touched file is byte for
//! byte the same, and re-running the campaign then spends an hour SPEC § 7 is
//! counting.
//!
//! **And at `standard`, a replay covers what changed since the last one.**
//! A mission's campaigns form a [`Chain`]: the first is full, and a later one
//! is partial — the touched files whose content changed since the previous
//! campaign's `HEAD` — whenever [`scope`] finds every condition for it met.
//! The chain keeps each file's latest counts, and gate 7 judges it once, as
//! one campaign at `HEAD` ([`owed`]).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::git;

/// The campaign's result, in the mission folder. **The HQ's file**, mounted
/// read-only in the container: it carries the survivors, and the one outcome
/// no machine can check — `equivalent`.
pub const FILE: &str = "MUTANTS.json";

/// The coder's answers, beside it. **The agent's file**, and the only one of
/// the two it can write.
///
/// Two files rather than one field, because what decides who wrote a line is
/// the **mount**, not the content: `nunki` cannot read a file and tell whose
/// hand a line came from (SPEC 4.1). The agent may write
/// the two outcomes that rest on a committed test, and physically cannot
/// write the third: it may only **propose** it
/// ([`Triage::EquivalentProposed`]), and the HQ rules.
pub const TRIAGE_FILE: &str = "MUTANTS.triage.json";

/// Where the campaigns in flight are filed, under the HQ.
///
/// Its own record on purpose: the mission state holds one run handle and that
/// one belongs to the agent — a campaign filed there would show up in `nunki
/// mission status` as an agent's run.
///
/// And **keyed by slot, not by mission**, because the thing it speaks for is
/// the slot's: `cargo mutants` rewrites the clean copy of `HEAD` in place, and
/// everything that reaches that copy — `exec::run(On::Proof)`, `nunki exec`,
/// `nunki mission gates`, the battery — knows which slot it is working in and
/// not which mission asked. Filed per mission, the record could not be found
/// by the code that has to respect it, and four callers walked over a running
/// campaign because of it (SPEC 4.4).
pub const CAMPAIGNS_DIR: &str = "campaigns";

/// What the stack fragment declares. It prints the survivors on stdout, one
/// JSON object per line, and `nunki` never parses a mutation tool itself: which
/// tool, and how it is invoked in place on the copy, is the stack's business
/// (SPEC 4.4: "une commande déterministe déclarée par le fragment de stack").
pub const SCRIPT: &str = "mutation.sh";

/// The variable that hands a campaign the commit its branch forked from.
///
/// Gate 7 answers for what the branch **changed**, not for every line of the
/// files it touched (SPEC 4.4): a script that can restrict itself to the
/// changed lines diffs against this commit. An environment variable rather
/// than an argument, so a project's own `mutation.sh`, which reads
/// `<campaign-id> <path>...`, keeps working unchanged.
pub const BASE_ENV: &str = "NUNKI_BASE";

/// The variable that hands a campaign how many mutants it may run at once:
/// the project's `mutation_jobs` (`nunki.yaml`). An environment variable for
/// the reason [`BASE_ENV`] is one: a project's own `mutation.sh` keeps
/// working unchanged, and one that ignores it runs as it always has.
pub const JOBS_ENV: &str = "NUNKI_MUTATION_JOBS";

/// A mutant the campaign could not kill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Survivor {
    /// What the tool calls it — enough to find it again.
    pub id: String,
    pub file: String,
    pub line: u32,
    /// The last line of the code the mutation replaces, from the tool's own
    /// listing: a function body replaced whole spans every line of it.
    /// Absent for a tool that gives no span, which is then the single
    /// [`Self::line`]; a value before `line` — what a stack's script prints
    /// when the tool's listing did not name the mutant once — is a span
    /// nobody knows ([`Survivor::span`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    /// What the mutation did, in the tool's words.
    #[serde(default)]
    pub description: String,
    /// The coder's answer, absent until one is written.
    #[serde(default)]
    pub outcome: Option<Triage>,
    /// The HQ's refusal of an equivalence the coder proposed, when it gave
    /// one ([`refuse`]). A proposal on a survivor carrying one is no outcome:
    /// the HQ has already said no, and the survivor is open again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused: Option<Refusal>,
    /// The commit of the earlier campaign of the chain that found it, when a
    /// partial campaign kept it because its file did not change since
    /// ([`Chain`]); absent for a survivor this campaign found itself. Said,
    /// never judged on: gate 7 judges the chain once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub found_on: Option<String>,
}

impl Survivor {
    /// The first and last line of the code the mutation replaces: the one
    /// line when the tool gives no span, and `None` when the span is not
    /// known — an end before the start, which no ruling may be tied to.
    pub fn span(&self) -> Option<(u32, u32)> {
        match self.end_line {
            None => Some((self.line, self.line)),
            Some(end) if end >= self.line => Some((self.line, end)),
            Some(_) => None,
        }
    }
}

/// The HQ said no to an equivalence the coder proposed, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refusal {
    /// The coder's sentence, as it stood when it was refused.
    pub proposed: String,
    /// The HQ's reason, which the next coder run reads in `FOLLOWUP_HQ.md`.
    pub because: String,
}

/// The three outcomes SPEC 4.4 allows, and there is no fourth — and the
/// coder's proposal of the third, which is not that outcome until the HQ
/// ratifies it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Triage {
    /// Killed by a test that is named. The name has to exist in the tree —
    /// "a test covers this" is not an outcome, a test called `x` is.
    Killed { test: String },
    /// Shown equivalent, in one sentence. The one outcome no machine can
    /// check — which is why it is not the agent's to give: it lives in the
    /// HQ's file, and one found in the agent's makes the gate red. Letting
    /// the graded fill in the only box nobody can check is a gate that
    /// empties itself (SPEC 4.4).
    Equivalent {
        why: String,
        /// The commit of the campaign this ruling was first given on, when it
        /// was carried here from there rather than given on this one. Absent
        /// on a ruling the HQ gave on this very campaign.
        ///
        /// Said, because a carried ruling is a judgement about code that has
        /// since changed around it: the mutation is the same, its neighbours
        /// may not be. Gate 7 counts them apart, and the HQ can lift one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        carried_from: Option<String>,
    },
    /// Recognised as a bug and frozen in a named test.
    Bug { test: String },
    /// The coder believes the survivor equivalent, and says why in one
    /// sentence. **Never a ruling**: gate 7 counts it as an outcome so the
    /// mission goes on, `nunki push` refuses while one is neither ratified
    /// nor refused, and it is never carried to the next campaign nor written
    /// in [`FILE`] as `equivalent` — only [`ratify`], the HQ's verb, turns it
    /// into one. A blank `why` is no outcome at all ([`answer`]).
    EquivalentProposed { why: String },
    /// An equivalence `nunki` proposes, because it matched the survivor to a
    /// ruling the HQ gave elsewhere — on another mission, through the
    /// project's registry ([`crate::equivalences`]), or on another survivor
    /// of this mission, by file and description alone ([`carry`]). **Never a
    /// ruling** (HQ review 2): which mutant a ruling was about is a heuristic
    /// across campaigns, and three rounds of review each found a narrower way
    /// for one to land on a mutant the HQ never ruled on. So a match only
    /// proposes, exactly as the coder does: gate 7 counts it as a proposal,
    /// the mission goes on, and `nunki push` refuses until the HQ ratifies
    /// or refuses it. A wrong match costs one refusal, never a wrong ruling.
    ///
    /// Written in [`FILE`] by `nunki` alone, and never the coder's to give.
    ProposedByNunki {
        /// The HQ's own sentence, from the ruling matched.
        why: String,
        /// Where the ruling it matched was given.
        from: ProposedFrom,
    },
    /// What an older `nunki` wrote for a ruling it applied from the registry.
    /// Only ever read, and read as what it is now: a proposal from the
    /// registry ([`read`] turns it into [`Triage::ProposedByNunki`]).
    EquivalentRegistered {
        why: String,
        mission: String,
        commit: String,
    },
}

/// Where the ruling behind a [`Triage::ProposedByNunki`] was given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum ProposedFrom {
    /// The project's registry of equivalences: a ruling given on another
    /// mission, on code that reads the same.
    Registry {
        mission: String,
        commit: String,
        /// When it was given; empty for an entry an older file carried.
        #[serde(default)]
        date: String,
    },
    /// A ruling given on this mission, on a survivor of the previous
    /// campaign with the same file and description but another id — the
    /// looser tier of [`carry`].
    Carried {
        /// The commit of the campaign the ruling was given on.
        commit: String,
    },
}

impl ProposedFrom {
    /// How a listing names this source.
    pub fn said(&self) -> String {
        match self {
            ProposedFrom::Registry {
                mission, commit, ..
            } => format!(
                "from the registry: ruled on mission {} at {}",
                crate::text::one_line(mission),
                crate::text::one_line(commit.get(..12).unwrap_or(commit))
            ),
            ProposedFrom::Carried { commit } => format!(
                "carried by file and mutation: ruled at {} on a survivor of another id",
                crate::text::one_line(commit.get(..12).unwrap_or(commit))
            ),
        }
    }
}

impl Triage {
    /// Whether the coder may write this outcome itself. The two that rest on
    /// a committed test, yes, and the proposal of the third; the judgement
    /// itself, no.
    pub fn is_the_coders_to_give(&self) -> bool {
        match self {
            Triage::Killed { .. } | Triage::Bug { .. } | Triage::EquivalentProposed { .. } => true,
            Triage::Equivalent { .. }
            | Triage::ProposedByNunki { .. }
            | Triage::EquivalentRegistered { .. } => false,
        }
    }

    /// The name this outcome goes by in a refusal.
    pub fn kind(&self) -> &'static str {
        match self {
            Triage::Killed { .. } => "killed",
            Triage::Equivalent { .. } => "equivalent",
            Triage::Bug { .. } => "bug",
            Triage::EquivalentProposed { .. } => "equivalent_proposed",
            Triage::ProposedByNunki { .. } => "proposed_by_nunki",
            Triage::EquivalentRegistered { .. } => "equivalent_registered",
        }
    }

    /// Whether this is the HQ's `equivalent` ruling: what `--refuse` will not
    /// undo and `--lift` takes back. A proposal — the coder's or `nunki`'s —
    /// never is.
    pub fn is_a_ruling(&self) -> bool {
        matches!(self, Triage::Equivalent { .. })
    }

    /// Whether this is a proposal awaiting the HQ, and from whom: `None`
    /// inside for the coder's own.
    pub fn proposal(&self) -> Option<(&str, Option<&ProposedFrom>)> {
        match self {
            Triage::EquivalentProposed { why } => Some((why, None)),
            Triage::ProposedByNunki { why, from } => Some((why, Some(from))),
            _ => None,
        }
    }

    /// The test this outcome rests on, when it rests on one.
    pub fn test(&self) -> Option<&str> {
        match self {
            Triage::Killed { test } | Triage::Bug { test } => Some(test),
            Triage::Equivalent { .. }
            | Triage::EquivalentProposed { .. }
            | Triage::ProposedByNunki { .. }
            | Triage::EquivalentRegistered { .. } => None,
        }
    }
}

/// `MUTANTS.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Campaign {
    /// The content fingerprint of the touched files this campaign ran on.
    /// A campaign whose fingerprint is not the current one has been overtaken
    /// and says nothing about the code as it stands.
    pub fingerprint: String,
    /// The commit it ran on, for a human reading the file later.
    pub head: String,
    pub date: String,
    #[serde(default)]
    pub survivors: Vec<Survivor>,
    /// How many mutants the campaign tried — every mutant it ran the tests
    /// against, survivors included, unviable ones not — as the stack's script
    /// said on its terminal line. What a `standard` mission's gate 7 divides
    /// by (SPEC 4.4). Absent from a campaign written by an older script or
    /// before the count existed, and then gate 7 judges it as `critical`
    /// does; never worked out from the survivors, which would make every
    /// campaign look perfect.
    ///
    /// For a partial campaign, the sum of [`Campaign::files`]: what one full
    /// campaign at this `HEAD` would have tried, each file counted as the
    /// campaign that last measured it measured it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tried: Option<u32>,
    /// Each touched file's counts, as the latest campaign of the chain that
    /// measured it gave them ([`by_file`]). `None` when the stack's script
    /// gives no per-file counts, or gave ones that do not add up — and then
    /// the next campaign is full: a partial one is rebuilt from these.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<BTreeMap<String, Measured>>,
    /// Where this campaign stands in the mission's chain of campaigns: full
    /// or partial, what it ran with, and the earlier campaigns it continues.
    /// Absent from a file written before campaigns formed a chain, which
    /// then reads as a full campaign with nothing before it.
    #[serde(default)]
    pub chain: Chain,
}

/// One file's counts, and the campaign that measured them (SPEC 4.4, the
/// chain of campaigns).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Measured {
    /// The mutants tried in this file, unviable ones left out.
    pub tried: u32,
    /// The testable mutants found in it, when the script said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub found: Option<u32>,
    /// The `HEAD` of the campaign that measured it.
    pub on: String,
}

/// A mission's campaigns form a chain (SPEC 4.4, gate 7). The first is
/// **full**: every touched file, mutated where the branch changed it since
/// its fork point. At `standard`, a later one may be **partial** — the same
/// campaign, restricted to the touched files whose content changed since the
/// previous campaign's `HEAD` — when that campaign passed gate 7, ran with
/// the same script and tool, and counted each file ([`scope`]).
///
/// The granularity is the file, and no diff hunk is ever read: a file
/// unchanged since keeps its counts and its survivors as they were; a file
/// changed, removed or renamed is measured again whole, or is gone. Gate 7
/// judges **once**, on that reconstruction ([`Campaign::files`]): what one
/// full campaign at `HEAD` would give, under the README's assumption that a
/// mutant killed in an unchanged file is still killed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chain {
    #[serde(default)]
    pub scope: Scope,
    /// A digest of what the campaign ran with — each stack's `mutation.sh`
    /// and its tool's version ([`tooling`]) — or `None` when that could not
    /// be read. A later campaign continues this one only when both are known
    /// and equal: the fingerprint covers the touched files, and says nothing
    /// of a script or a tool that changed under them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tooling: Option<String>,
    /// The earlier campaigns of the chain, oldest first, each as it was
    /// recorded. Their survivors still standing are listed among
    /// [`Campaign::survivors`], with [`Survivor::found_on`] naming the one
    /// that found them. Empty for a full campaign.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub earlier: Vec<Link>,
    /// What a partial campaign itself tried, as its terminal line said —
    /// [`Campaign::tried`] being the chain's reconstruction. Absent for a
    /// full campaign, whose own count is the whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ran: Option<u32>,
}

/// Which files a campaign covered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Scope {
    /// Every file the branch touched, and why the campaign was not partial
    /// — empty in a file written before the chain existed.
    Full {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        why: String,
    },
    /// The touched files whose content changed since `since`, the previous
    /// campaign's `HEAD`.
    Partial { since: String },
}

impl Default for Scope {
    fn default() -> Self {
        Scope::Full { why: String::new() }
    }
}

impl Scope {
    /// How a listing names it: `full`, or `partial since <commit>`.
    pub fn said(&self) -> String {
        match self {
            Scope::Full { .. } => "full".to_string(),
            Scope::Partial { since } => {
                format!("partial since {}", crate::text::one_line(short(since)))
            }
        }
    }

    /// The commit a partial campaign continues from, when it is one.
    pub fn since(&self) -> Option<&str> {
        match self {
            Scope::Full { .. } => None,
            Scope::Partial { since } => Some(since),
        }
    }
}

/// An earlier campaign of the chain, as it was recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub head: String,
    pub date: String,
    #[serde(default)]
    pub scope: Scope,
    /// What it itself tried, as its terminal line said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tried: Option<u32>,
}

/// The first twelve characters of a commit, as `nunki` names one.
fn short(commit: &str) -> &str {
    commit.get(..12).unwrap_or(commit)
}

/// `MUTANTS.run.json`: a campaign in flight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Running {
    pub fingerprint: String,
    pub head: String,
    pub started_at: String,
    pub container: String,
    pub pid: Option<u32>,
    /// Where the campaign's stdout lands, on the host.
    pub log: PathBuf,
    /// How long it is given before `nunki` calls it hung (SPEC 4.4, "un délai
    /// paramétré").
    pub deadline_minutes: u32,
    /// Full or partial, and what it runs with: what the campaign is recorded
    /// with once it ends. The earlier links are taken from the campaign on
    /// file then, never kept here. Absent from a record an older `nunki`
    /// wrote, which reads as a full campaign.
    #[serde(default)]
    pub chain: Chain,
}

#[derive(Debug, thiserror::Error)]
pub enum MutantsError {
    #[error(transparent)]
    Git(#[from] git::GitError),
    #[error("{0} is not a campaign `nunki` can read: {1}")]
    Unreadable(PathBuf, String),
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error(transparent)]
    Exec(#[from] crate::exec::ExecError),
    #[error("the campaign could not be launched: {0}")]
    Launch(String),
    #[error("{0}")]
    NoProposal(String),
    #[error(transparent)]
    Followup(#[from] crate::followup::FollowupError),
    #[error("the chain of campaigns is broken: {0}")]
    Broken(String),
}

/// Where a mission's campaign result lives.
pub fn paths(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

/// Where the record of the campaign rewriting **this slot's** clean copy
/// lives, whether or not one is in flight.
pub fn running_path(hq_root: &Path, slot: &str) -> PathBuf {
    hq_root.join(CAMPAIGNS_DIR).join(format!("{slot}.json"))
}

/// The fingerprint of what a campaign would run on: the **content** of the
/// touched files, as git names it.
///
/// `git rev-parse HEAD:<path>` gives a blob id, so two commits holding the
/// same bytes fingerprint the same — which is the point. A path that no
/// longer exists at `HEAD` (touched and then deleted) contributes its absence
/// rather than an error. The digest is `git hash-object`, so it needs no
/// crate and a human can reproduce it by hand.
pub fn fingerprint(tree: &Path, touched: &[String]) -> Result<String, MutantsError> {
    let mut lines: Vec<String> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for path in touched {
        if !seen.insert(path.clone()) {
            continue;
        }
        let blob = git::run(tree, &["rev-parse", &format!("HEAD:{path}")])
            .unwrap_or_else(|_| "absent".to_string());
        lines.push(format!("{blob} {path}"));
    }
    lines.sort();
    let manifest = lines.join("\n");
    Ok(hash_object(tree, &manifest)?)
}

/// `git hash-object --stdin` — git is already here, and this keeps the digest
/// something a human can check with one command.
fn hash_object(tree: &Path, text: &str) -> Result<String, git::GitError> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("git")
        .arg("-C")
        .arg(tree)
        .args(["hash-object", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| git::GitError::Missing(e.to_string()))?;
    child
        .stdin
        .take()
        .expect("stdin was piped")
        .write_all(text.as_bytes())
        .map_err(|e| git::GitError::Missing(e.to_string()))?;
    let out = child
        .wait_with_output()
        .map_err(|e| git::GitError::Missing(e.to_string()))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(git::GitError::Failed {
            verb: "hash-object".to_string(),
            at: tree.display().to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        })
    }
}

/// Read the campaign a mission holds, if it holds one.
pub fn read(dir: &Path) -> Result<Option<Campaign>, MutantsError> {
    let file = paths(dir);
    let text = match std::fs::read_to_string(&file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(MutantsError::Io(file, e)),
    };
    if text.trim().is_empty() {
        return Ok(None);
    }
    let mut campaign: Campaign =
        serde_json::from_str(&text).map_err(|e| MutantsError::Unreadable(file, e.to_string()))?;
    for survivor in &mut campaign.survivors {
        survivor.outcome = survivor.outcome.take().map(as_now);
    }
    Ok(Some(campaign))
}

/// An outcome an older file holds, as it reads now: a ruling an older
/// `nunki` applied from the registry is a proposal from the registry (HQ
/// review 2). Everything else as it is.
fn as_now(outcome: Triage) -> Triage {
    match outcome {
        Triage::EquivalentRegistered {
            why,
            mission,
            commit,
        } => Triage::ProposedByNunki {
            why,
            from: ProposedFrom::Registry {
                mission,
                commit,
                date: String::new(),
            },
        },
        other => other,
    }
}

/// The coder's answers, by survivor id. Absent until it writes one, and an
/// unreadable one is an error rather than an empty triage: a file the coder
/// wrote and `nunki` cannot parse must be said, not silently ignored.
///
/// **Only the outcomes the coder may give are ever read from it** —
/// `killed`, `bug`, `equivalent_proposed` (HQ review 3, the class rule) —
/// and [`answer`] is where: every reader of an outcome goes through it, and
/// it reads nothing else from this map. Anything else found there —
/// `equivalent`, `equivalent_registered`, `proposed_by_nunki` — is no
/// outcome and never shadows what [`FILE`] holds; an entry that does not
/// parse, a kind nobody knows among them, is left out here and refuses
/// nothing but itself. [`foreign`] names them all, for gate 7 and `nunki
/// push` to say.
pub fn read_triage(dir: &Path) -> Result<BTreeMap<String, Triage>, MutantsError> {
    triage_entries(dir).map(|(read, _)| read)
}

/// An entry of the coder's file that `nunki` does not read: an outcome
/// that is not the coder's to give, or one it cannot make out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Foreign {
    pub id: String,
    /// The kind it claims, as written.
    pub kind: String,
    /// Why it is not read: not the coder's to give, or not readable.
    pub why: String,
}

/// The entries of the coder's file `nunki` does not read ([`read_triage`]).
pub fn foreign(dir: &Path) -> Result<Vec<Foreign>, MutantsError> {
    let (read, unreadable) = triage_entries(dir)?;
    let mut foreign: Vec<Foreign> = read
        .into_iter()
        .filter(|(_, outcome)| !outcome.is_the_coders_to_give())
        .map(|(id, outcome)| Foreign {
            id,
            kind: outcome.kind().to_string(),
            why: "is not the coder's to give".to_string(),
        })
        .collect();
    foreign.extend(unreadable);
    Ok(foreign)
}

/// What gate 7 and `nunki push` say of [`foreign`] entries — nothing when
/// there are none.
pub fn foreign_said(foreign: &[Foreign]) -> Option<String> {
    (!foreign.is_empty()).then(|| {
        let listed = foreign
            .iter()
            .map(|f| {
                format!(
                    "{} (`{}`, which {})",
                    crate::text::one_line(&f.id),
                    crate::text::one_line(&f.kind),
                    crate::text::one_line(&f.why)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "{TRIAGE_FILE} answers {listed}, so each is refused and read as no outcome: \
             rulings and `nunki`'s own proposals live in {FILE}, and the coder proposes \
             with `equivalent_proposed` and its reason"
        )
    })
}

/// The coder's file, split into the entries that parse as an outcome and
/// those that do not. A file that is not a JSON object of entries is an
/// error — said, never read as empty; an entry within it that does not
/// parse is only foreign.
fn triage_entries(dir: &Path) -> Result<(BTreeMap<String, Triage>, Vec<Foreign>), MutantsError> {
    let mut read = BTreeMap::new();
    let mut foreign = Vec::new();
    for (id, value) in triage_raw(dir)? {
        match serde_json::from_value::<Triage>(value.clone()) {
            Ok(outcome) => {
                read.insert(id, outcome);
            }
            Err(e) => foreign.push(Foreign {
                id,
                kind: value
                    .get("kind")
                    .and_then(|k| k.as_str())
                    .map_or_else(|| "unreadable".to_string(), str::to_string),
                why: format!("cannot be read: {e}"),
            }),
        }
    }
    Ok((read, foreign))
}

/// The coder's file as the coder wrote it, entry by entry: empty when it is
/// absent or blank, an error naming it when it cannot be read or is not a
/// JSON object of entries — never read as empty. The one reader of the
/// file, for [`triage_entries`] and [`remove_triage_entry`] alike.
fn triage_raw(dir: &Path) -> Result<BTreeMap<String, serde_json::Value>, MutantsError> {
    let file = dir.join(TRIAGE_FILE);
    let text = match std::fs::read_to_string(&file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(MutantsError::Io(file, e)),
    };
    if text.trim().is_empty() {
        return Ok(BTreeMap::new());
    }
    serde_json::from_str(&text).map_err(|e| MutantsError::Unreadable(file, e.to_string()))
}

/// Take survivor `id`'s entry out of the coder's file, leaving every other
/// entry as the coder wrote it — those `nunki` does not read included.
fn remove_triage_entry(dir: &Path, id: &str) -> Result<(), MutantsError> {
    let mut entries = triage_raw(dir)?;
    if entries.remove(id).is_none() {
        return Ok(());
    }
    let file = dir.join(TRIAGE_FILE);
    let body = serde_json::to_string_pretty(&entries).expect("a triage serialises");
    std::fs::write(&file, format!("{body}\n")).map_err(|e| MutantsError::Io(file, e))
}

/// The ids of the survivors still waiting for an outcome: in the mission's
/// campaign, answered neither there (the HQ's file) nor in the coder's
/// triage. The only survivors a coder may hand a lot over for, awaiting a
/// ruling ([`crate::mission::journal::awaitable`]). No campaign, none.
///
/// A survivor with a valid proposal is not open: it cannot be awaited and
/// proposed at once. One whose proposal gives no reason, or that the HQ
/// already refused, is.
pub fn open(dir: &Path) -> Result<Vec<String>, MutantsError> {
    let Some(campaign) = read(dir)? else {
        return Ok(Vec::new());
    };
    let coders = read_triage(dir)?;
    Ok(campaign
        .survivors
        .iter()
        .filter(|s| answer(s, &coders).is_none())
        .map(|s| s.id.clone())
        .collect())
}

/// The outcome a survivor holds, from the two files together.
///
/// The coder's two outcomes that rest on a test come first, then what the
/// HQ's file holds — its own ruling, or `nunki`'s proposal — then the coder's
/// proposal — so a ratified proposal answers as the HQ's `equivalent`,
/// whatever the coder's file still says. A proposal is
/// an outcome only when it says why ([`crate::text::blank`] says it does
/// not) and the HQ has not refused it on this survivor. Anything else in the
/// coder's map — an `equivalent` of its own, `nunki`'s proposal — is no
/// outcome, and the HQ's file answers ([`read_triage`]).
pub fn answer(survivor: &Survivor, coders: &BTreeMap<String, Triage>) -> Option<Triage> {
    // Only what the coder may give, whoever built the map: anything else
    // would shadow the HQ's file (HQ review 3).
    match coders.get(&survivor.id).filter(|outcome| {
        outcome.is_the_coders_to_give() && outcome.test().is_none_or(is_test_name)
    }) {
        Some(Triage::EquivalentProposed { why }) => survivor.outcome.clone().or_else(|| {
            (!crate::text::blank(why) && survivor.refused.is_none())
                .then(|| Triage::EquivalentProposed { why: why.clone() })
        }),
        Some(other) => Some(other.clone()),
        None => survivor.outcome.clone(),
    }
}

/// Whether `name` can be what an outcome names as its test: an identifier
/// — letters, digits and underscores — of three characters at least (HQ
/// review 4). A blank name, or one of a letter or two, is matched by
/// nearly any line of a tree, and would answer a survivor with nothing; a
/// name with spaces or punctuation is not one a test is called by. Gate 7
/// and `nunki push` then look for it as a whole word
/// ([`crate::gate::named_test_missing`]).
pub fn is_test_name(name: &str) -> bool {
    name.chars().count() >= 3 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Why a survivor the coder answered still has no outcome, when that is the
/// case: a proposal with no reason, or one the HQ refused. Said beside the
/// survivor in gate 7's message, so the coder is not left wondering why a
/// line it wrote was not read.
pub fn not_an_outcome(survivor: &Survivor, coders: &BTreeMap<String, Triage>) -> Option<String> {
    if answer(survivor, coders).is_some() {
        return None;
    }
    match coders.get(&survivor.id) {
        Some(outcome @ (Triage::Killed { test } | Triage::Bug { test })) if !is_test_name(test) => {
            Some(format!(
                "its `{}` names {test:?}, which is not a test name: letters, digits and \
                 underscores, three at least",
                outcome.kind()
            ))
        }
        Some(Triage::EquivalentProposed { why }) if crate::text::blank(why) => {
            Some("its proposal gives no reason, and a proposal is one sentence".to_string())
        }
        Some(Triage::EquivalentProposed { .. }) => survivor.refused.as_ref().map(|r| {
            format!(
                "the HQ refused its proposal: {}",
                crate::text::one_line(&r.because)
            )
        }),
        _ => None,
    }
}

/// An equivalence proposed — by the coder, or by `nunki` from a ruling it
/// matched — that the HQ has neither ratified nor refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    pub id: String,
    pub file: String,
    pub line: u32,
    /// The proposal's sentence: the coder's, or the HQ's own from the ruling
    /// matched.
    pub why: String,
    /// Where it comes from: `None` for the coder.
    pub from: Option<ProposedFrom>,
}

impl Proposal {
    /// Who proposed it, as `mission status`, `mission wait`, `nunki push`
    /// and the follow-up name it.
    pub fn source(&self) -> String {
        match &self.from {
            None => "the coder's".to_string(),
            Some(from) => format!("nunki's, {}", from.said()),
        }
    }
}

/// How many of `proposals` come from each source, as one phrase: "2 from
/// the coder, 1 from the registry".
pub fn by_source(proposals: &[Proposal]) -> String {
    let count = |which: fn(&Option<ProposedFrom>) -> bool| {
        proposals.iter().filter(|p| which(&p.from)).count()
    };
    [
        (count(|f| f.is_none()), "from the coder"),
        (
            count(|f| matches!(f, Some(ProposedFrom::Registry { .. }))),
            "from the registry",
        ),
        (
            count(|f| matches!(f, Some(ProposedFrom::Carried { .. }))),
            "carried by file and mutation",
        ),
    ]
    .into_iter()
    .filter(|(n, _)| *n > 0)
    .map(|(n, what)| format!("{n} {what}"))
    .collect::<Vec<_>>()
    .join(", ")
}

/// Whether `killed` of `tried` reaches `threshold` percent, compared in
/// whole numbers so that 79.5% is never rounded up to a pass. Gate 7 at
/// `standard` and [`owed`] both ask it here, so they cannot disagree.
pub fn share_reached(killed: u64, tried: u64, threshold: u64) -> bool {
    killed * 100 >= threshold * tried
}

/// What gate 7's rule still owes on `campaign`, read with the coder's
/// answers, as a sentence naming the survivors left open — `None` when
/// nothing is owed.
///
/// The rule as the gate plays it (SPEC 4.4): nothing at `prototype`; at
/// `standard`, the share of tried mutants killed reaches `threshold` (a
/// campaign that gives no usable count is judged as `critical`); at
/// `critical`, every survivor has an outcome. A proposal still pending is an
/// outcome here, as at the gate — `nunki push` refuses it on its own.
///
/// `nunki push` asks it again on the campaign as it stands, because the HQ
/// can reopen a survivor after the gates were green: a refused proposal
/// leaves one without an outcome, and the gates are not replayed before the
/// push (security round 1, MEDIUM).
///
/// **Once**, on the campaign on file. For a partial campaign that is the
/// chain's reconstruction ([`Chain`]): its `tried` the sum of each touched
/// file's latest count, its survivors those of the files unchanged since the
/// previous campaign and the new campaign's — what one full campaign at
/// `HEAD` would be judged on. Never each campaign on its own: a volet that
/// re-mutates well-killed lines would otherwise pad its own share.
pub fn owed(
    campaign: &Campaign,
    coders: &BTreeMap<String, Triage>,
    rigor: crate::mission::Rigor,
    threshold: u32,
) -> Option<String> {
    use crate::mission::Rigor;
    if rigor == Rigor::Prototype {
        return None;
    }
    let open: Vec<String> = campaign
        .survivors
        .iter()
        .filter(|s| answer(s, coders).is_none())
        .map(|s| {
            format!(
                "{}:{} {}",
                crate::text::one_line(&s.file),
                s.line,
                crate::text::one_line(&s.id)
            )
        })
        .collect();
    let listed = open.join("; ");
    match campaign.tried {
        Some(tried) if rigor == Rigor::Standard && tried as usize >= campaign.survivors.len() => {
            let (tried, killed) = (u64::from(tried), u64::from(tried) - open.len() as u64);
            if share_reached(killed, tried, u64::from(threshold)) {
                return None;
            }
            Some(format!(
                "{killed} of {tried} tried mutant(s) killed, below the threshold of \
                 {threshold}%, with {} survivor(s) left without an outcome: {listed}",
                open.len()
            ))
        }
        _ if open.is_empty() => None,
        _ => Some(format!(
            "{} survivor(s) have no outcome: {listed}",
            open.len()
        )),
    }
}

/// The campaigns of the chain in one line each, oldest first — full or
/// partial and from which commit, and what it itself tried — and, for a
/// chain, one line more for what gate 7 judges: the reconstruction, tried,
/// killed and left. What `mission status`, the monitor's log and the
/// follow-up say of a mission's campaigns. Killed is counted as gate 7
/// counts it: tried, less the survivors left without an outcome.
pub fn chain_said(campaign: &Campaign, coders: &BTreeMap<String, Triage>) -> Vec<String> {
    let listed = campaign.survivors.len();
    let open = campaign
        .survivors
        .iter()
        .filter(|s| answer(s, coders).is_none())
        .count();
    let counts = match campaign.tried {
        Some(tried) => format!(
            "tried {tried}, killed {}",
            (tried as usize).saturating_sub(open)
        ),
        None => "tried: not said".to_string(),
    };
    let named = |scope: &Scope, head: &str| {
        let why = match scope {
            Scope::Full { why } if !crate::text::blank(why) => {
                format!(" ({})", crate::text::one_line(why))
            }
            _ => String::new(),
        };
        format!(
            "{} at {}{why}",
            scope.said(),
            crate::text::one_line(short(head))
        )
    };
    let own = |tried: Option<u32>| match tried {
        Some(tried) => format!("tried {tried}"),
        None => "tried: not said".to_string(),
    };
    let earlier = &campaign.chain.earlier;
    if earlier.is_empty() {
        return vec![format!(
            "{} — {counts}, {listed} survivor(s), {open} without an outcome",
            named(&campaign.chain.scope, &campaign.head)
        )];
    }
    let mut lines: Vec<String> = earlier
        .iter()
        .map(|link| format!("{} — {}", named(&link.scope, &link.head), own(link.tried)))
        .collect();
    lines.push(format!(
        "{} — {}",
        named(&campaign.chain.scope, &campaign.head),
        own(campaign.chain.ran)
    ));
    let files = match &campaign.files {
        Some(files) => format!(" over {} file(s)", files.len()),
        None => String::new(),
    };
    lines.push(format!(
        "judged as one campaign at {}{files} — {counts}, {listed} survivor(s), {open} \
         without an outcome",
        crate::text::one_line(short(&campaign.head))
    ));
    lines
}

/// [`chain_said`] on the mission folder's own files; nothing when no
/// campaign is on file.
pub fn chain_on_file(dir: &Path) -> Result<Vec<String>, MutantsError> {
    let Some(campaign) = read(dir)? else {
        return Ok(Vec::new());
    };
    Ok(chain_said(&campaign, &read_triage(dir)?))
}

/// [`owed`], on the mission folder's own two files. No campaign, nothing
/// owed here: whether one is due is the caller's question — `nunki push`
/// refuses a mission that owes one and has none before it asks this.
pub fn owed_on_file(
    dir: &Path,
    rigor: crate::mission::Rigor,
    threshold: u32,
) -> Result<Option<String>, MutantsError> {
    let Some(campaign) = read(dir)? else {
        return Ok(None);
    };
    Ok(owed(&campaign, &read_triage(dir)?, rigor, threshold))
}

/// How `mission status` and `mission wait` say that `proposals` await the
/// HQ, by source — nothing when none does.
pub fn proposals_await(proposals: &[Proposal]) -> Option<String> {
    (!proposals.is_empty()).then(|| {
        format!(
            "{} equivalence proposal(s) await the HQ's ruling ({}), and `nunki push` \
             refuses until each is ratified or refused",
            proposals.len(),
            by_source(proposals)
        )
    })
}

/// The proposals of the current campaign that await the HQ — exactly those
/// [`answer`] reads as a proposal. What `nunki push` refuses on while any is
/// left, and what `mission status`, `mission wait` and the follow-up count.
/// No campaign, none.
pub fn awaiting_ruling(dir: &Path) -> Result<Vec<Proposal>, MutantsError> {
    let Some(campaign) = read(dir)? else {
        return Ok(Vec::new());
    };
    let coders = read_triage(dir)?;
    Ok(campaign
        .survivors
        .iter()
        .filter_map(|s| {
            let answered = answer(s, &coders)?;
            let (why, from) = answered.proposal()?;
            Some(Proposal {
                id: s.id.clone(),
                file: s.file.clone(),
                line: s.line,
                why: why.to_string(),
                from: from.cloned(),
            })
        })
        .collect())
}

pub fn write_triage(dir: &Path, triage: &BTreeMap<String, Triage>) -> Result<(), MutantsError> {
    let file = dir.join(TRIAGE_FILE);
    let body = serde_json::to_string_pretty(triage).expect("a triage serialises");
    std::fs::write(&file, format!("{body}\n")).map_err(|e| MutantsError::Io(file, e))
}

pub fn read_running(hq_root: &Path, slot: &str) -> Result<Option<Running>, MutantsError> {
    let file = running_path(hq_root, slot);
    let text = match std::fs::read_to_string(&file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(MutantsError::Io(file, e)),
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| MutantsError::Unreadable(file, e.to_string()))
}

pub fn write(dir: &Path, campaign: &Campaign) -> Result<(), MutantsError> {
    let file = paths(dir);
    let body = serde_json::to_string_pretty(campaign).expect("a campaign serialises");
    std::fs::write(&file, format!("{body}\n")).map_err(|e| MutantsError::Io(file, e))
}

pub fn write_running(hq_root: &Path, slot: &str, running: &Running) -> Result<(), MutantsError> {
    let file = running_path(hq_root, slot);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| MutantsError::Io(dir.to_path_buf(), e))?;
    }
    let body = serde_json::to_string_pretty(running).expect("a record serialises");
    std::fs::write(&file, format!("{body}\n")).map_err(|e| MutantsError::Io(file, e))
}

pub fn forget_running(hq_root: &Path, slot: &str) -> Result<(), MutantsError> {
    let file = running_path(hq_root, slot);
    match std::fs::remove_file(&file) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(MutantsError::Io(file, e)),
    }
}

/// The survivors a campaign printed: one JSON object per line, and anything
/// that is not one is ignored — a mutation tool writes progress on the same
/// stream, and a campaign that produced a hundred good lines and one banner
/// is not a campaign that failed.
/// The line a campaign prints, last of all, to say it got to the end.
///
/// Without it `nunki` cannot tell "no survivor" from "no answer": a campaign
/// killed halfway, one whose container went away and one that never compiled
/// all leave a log that parses to zero survivors, and gate 7 would go green
/// on a measurement nobody made. SPEC 4.4 forbids exactly that — "une porte 7
/// qui ne peut pas dire « je n'ai pas pu mesurer » ment".
///
/// A line the script prints rather than an exit status, because the status
/// does not survive: the spawner `exec`s the command so that the pid it
/// published is the campaign's own, and a shell that has been replaced cannot
/// write `$?`. Dropping the `exec` would take the identity check for every
/// harness run with it (`engine/spawn.rs`).
#[derive(serde::Deserialize)]
struct Terminal {
    campaign: String,
    /// How many mutants the campaign tried, unviable ones left out. Optional:
    /// a script written before the count still says it finished, and is
    /// read exactly as it was.
    #[serde(default)]
    tried: Option<u32>,
    /// How many testable mutants the tool found — unviable ones left out, as
    /// they are from `tried`. A campaign that found some and tried none did
    /// not measure: a failing baseline leaves exactly that, with every
    /// mutant found and none tested ([`unmeasured`]).
    #[serde(default)]
    found: Option<u32>,
    /// Whether the script counted each file, on its own lines before this
    /// one ([`by_file`]). Absent from a script that does not: no per-file
    /// count is then read, even from lines that look like one.
    #[serde(default)]
    by_file: bool,
}

/// One per-file line of a campaign's log: `{"measured":"<path>","tried":3,
/// "found":4}`, printed before the terminal line.
#[derive(serde::Deserialize)]
struct FileLine {
    measured: String,
    tried: u32,
    #[serde(default)]
    found: Option<u32>,
}

/// Each file's counts, from a log whose terminal line says the script
/// counted each file — `None` otherwise, and `None` too when the counts
/// cannot be trusted: a file named twice, a `found` given for some files and
/// not others, or counts that do not add up to the terminal line's totals.
/// `on` is the commit the campaign ran on.
///
/// What a later partial campaign is rebuilt from ([`Campaign::files`]), so
/// it fails closed: counts that are not all there make the next campaign
/// full rather than a chain built on a guess.
pub fn by_file(text: &str, on: &str) -> Option<BTreeMap<String, Measured>> {
    let line = terminal(text)?;
    if !line.by_file {
        return None;
    }
    let mut files = BTreeMap::new();
    for counted in text
        .lines()
        .filter_map(|l| serde_json::from_str::<FileLine>(l.trim()).ok())
    {
        let measured = Measured {
            tried: counted.tried,
            found: counted.found,
            on: on.to_string(),
        };
        if files.insert(counted.measured, measured).is_some() {
            return None;
        }
    }
    let tried: u64 = files.values().map(|m: &Measured| u64::from(m.tried)).sum();
    if line.tried.map(u64::from) != Some(tried) {
        return None;
    }
    let found: Option<u64> = files
        .values()
        .map(|m: &Measured| m.found.map(u64::from))
        .sum();
    if found.is_none() && files.values().any(|m| m.found.is_some()) {
        return None;
    }
    match (line.found, found) {
        (Some(total), Some(sum)) if u64::from(total) == sum => {}
        (Some(_), Some(_)) => return None,
        // A `found` for every file and none in total, or the reverse, is a
        // script that does not say the same thing twice.
        (None, Some(_)) if !files.is_empty() => return None,
        (Some(total), None) if total > 0 || !files.is_empty() => return None,
        _ => {}
    }
    Some(files)
}

/// What a set of per-file counts adds up to.
pub fn tried_in(files: &BTreeMap<String, Measured>) -> Option<u32> {
    u32::try_from(files.values().map(|m| u64::from(m.tried)).sum::<u64>()).ok()
}

/// Every line of the log that says the campaign is done.
fn done_lines(text: &str) -> impl Iterator<Item = Terminal> + '_ {
    text.lines()
        .filter_map(|line| serde_json::from_str::<Terminal>(line.trim()).ok())
        .filter(|t| t.campaign == "done")
}

/// The terminal line, if the campaign printed exactly one.
///
/// A script says it once, last. A second done line is something else
/// writing to the campaign's output — a test reaching the script's stdout,
/// say — and nothing tells which of the two is the script's, so neither is
/// taken: the first would let a forged line sit before the real one, the
/// last would let it sit after. [`several_campaigns`] reads each stack's
/// output by the same rule.
fn terminal(text: &str) -> Option<Terminal> {
    let mut done = done_lines(text);
    let only = done.next()?;
    done.next().is_none().then_some(only)
}

/// Whether the campaign said it finished, exactly once.
///
/// The whole log, not its last line: a campaign is read back through a file
/// the engine is still writing, and asking for the last line would turn a
/// half-flushed newline into "it did not finish". The line is printed last,
/// so a truncated log has lost it either way.
pub fn completed(text: &str) -> bool {
    terminal(text).is_some()
}

/// Why a log that says the campaign finished more than once is not read,
/// if it does: such a log is not a finished campaign ([`completed`]), and
/// this is the sentence that says so rather than "it stopped".
pub fn repeated(text: &str) -> Option<String> {
    let count = done_lines(text).count();
    (count > 1).then(|| {
        format!(
            "the campaign said it had finished {count} times, and a campaign says it once, \
             last — something else wrote to its output, so nothing it printed is read"
        )
    })
}

/// How many mutants the campaign says it tried, from its terminal line.
///
/// `None` when the line carries no count — an older `mutation.sh` — or when
/// there is no terminal line at all. Only the script's own word is taken:
/// counting the survivors would make a campaign that tried a hundred and
/// lost three look like one that tried three and lost them all.
pub fn tried(text: &str) -> Option<u32> {
    terminal(text).and_then(|t| t.tried)
}

/// How many testable mutants the campaign says it found, from its terminal
/// line; `None` when the line does not say.
pub fn found(text: &str) -> Option<u32> {
    terminal(text).and_then(|t| t.found)
}

/// Why a campaign that said it finished still measured nothing, if it did:
/// it found mutants and tried none.
///
/// The terminal line says the script got to its end, not that the tool
/// tested anything. A baseline that fails before the first mutant —
/// cargo-mutants exits 4, measured on 27.1.0, with 12 mutants found and
/// none tested — would otherwise be recorded as a campaign with no
/// survivor, and gate 7 green at every rigor on a measurement nobody made.
/// The stack's script refuses such a run itself; this is the same rule kept
/// on `nunki`'s side, for a script that does not.
///
/// Both counts are needed: a `found` without a `tried` is a line that gives
/// no count — which gate 7 judges as `critical` does — and not a campaign
/// that said it tried nothing.
pub fn unmeasured(text: &str) -> Option<String> {
    let line = terminal(text)?;
    match line.found {
        Some(found) if found > 0 && line.tried == Some(0) => Some(format!(
            "the campaign found {found} mutant(s) and tried none, so it measured nothing — \
             a baseline whose tests fail before any mutant is tried leaves exactly this"
        )),
        _ => None,
    }
}

/// Settle a campaign that said it finished and measured nothing: its reason
/// is appended to the campaign's stderr, beside the log, where gate 7 reads
/// the cause of a campaign that could not run — so the gate is red and says
/// why, rather than asking for the same campaign again. Returns the reason,
/// or `None` for a campaign that did measure.
pub fn settle_unmeasured(log: &Path, text: &str) -> Result<Option<String>, MutantsError> {
    match unmeasured(text) {
        Some(why) => settle(log, &why).map(Some),
        None => Ok(None),
    }
}

/// Settle a campaign whose log says it finished more than once
/// ([`repeated`]) the way [`settle_unmeasured`] settles one that measured
/// nothing: its reason beside the log, where gate 7 reads it. Returns the
/// reason, or `None` for a log that says it at most once.
pub fn settle_repeated(log: &Path, text: &str) -> Result<Option<String>, MutantsError> {
    match repeated(text) {
        Some(why) => settle(log, &why).map(Some),
        None => Ok(None),
    }
}

/// Append `why` to the campaign's stderr, and say where it went.
fn settle(log: &Path, why: &str) -> Result<String, MutantsError> {
    use std::io::Write;
    let stderr = log.with_extension("err");
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&stderr)
        .and_then(|mut file| writeln!(file, "nunki: {why}"))
        .map_err(|e| MutantsError::Io(stderr.clone(), e))?;
    Ok(format!("{why}. What it said is in {}", stderr.display()))
}

/// The survivors a campaign's log names, **with no outcome** and no refusal.
///
/// An outcome is never read from the log. The log is what the stack's script
/// printed inside the agent's container, and an `equivalent` in it would land
/// in the HQ's file with no human having given it — the one outcome no
/// machine may give (SPEC 4.4). The rulings a new campaign carries come from
/// the HQ's previous file, in [`record_finished`], and from nowhere else.
pub fn parse(text: &str) -> Vec<Survivor> {
    text.lines()
        .filter_map(|line| serde_json::from_str::<Survivor>(line.trim()).ok())
        .map(|survivor| Survivor {
            outcome: None,
            refused: None,
            // Which campaign of the chain found it is `nunki`'s to record:
            // a log claiming an earlier one would move a survivor out of the
            // count it belongs to.
            found_on: None,
            ..survivor
        })
        .collect()
}

/// Write the campaign a finished log describes, carrying over the HQ's
/// rulings from the campaign it replaces.
///
/// A new campaign runs whenever a touched file changes, and a commit that
/// only adds a line above a ruled survivor would otherwise lose the ruling
/// with the file it lived in, sending the mission back to the HQ to be told
/// the same thing again.
///
/// Returns how many survivors the new campaign holds. The project's
/// registry is not asked: [`record_finished_with_registry`] is what a
/// campaign read back in its slot records through.
pub fn record_finished(
    dir: &Path,
    fingerprint: &str,
    head: &str,
    text: &str,
) -> Result<usize, MutantsError> {
    record(
        dir,
        None,
        fingerprint,
        head,
        text,
        &Chain::default(),
        |_| true,
        |_| {},
    )
}

/// What [`record_finished_with_registry`] did with the project's registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    /// How many survivors the new campaign holds.
    pub survivors: usize,
    /// How many of them received a proposal from the registry.
    pub from_registry: usize,
    /// Why the registry could not be read, when it could not: nothing was
    /// proposed from it, and `FOLLOWUP_HQ.md` says so.
    pub registry_unread: Option<String>,
}

/// [`record_finished`], and then the project's registry of equivalences
/// ([`crate::equivalences`]) matched against every survivor still without
/// an outcome — a match **proposes**, never rules (HQ review 2) — reading
/// each survivor's code at `head` in the repository at `tree`: the slot,
/// which holds the campaign's commit.
///
/// After [`carry`], so that a ruling given on this mission keeps its own
/// origin. **Fails closed**: a registry that cannot be read applies nothing,
/// and the reason is appended to the mission's `FOLLOWUP_HQ.md`, the file the
/// HQ reads, rather than lost in a log. The registry is read under its lock
/// ([`crate::equivalences::read_locked`]).
///
/// With the source at hand, the class rule filters [`carry`]'s proposals
/// too: its looser tier — the same file and description under another id —
/// proposes nothing unless the code its mutation replaces occurs once in the
/// file ([`crate::equivalences::identify`]). A refusal is carried whatever
/// that says.
///
/// `chain` is what the campaign was launched as ([`Running::chain`]): a
/// partial one continues the campaign on file ([`continued`]), file by
/// file, its kept survivors carried by the same rules as any other.
#[allow(clippy::too_many_arguments)]
pub fn record_finished_with_registry(
    dir: &Path,
    hq_root: &Path,
    tree: &Path,
    fingerprint: &str,
    head: &str,
    text: &str,
    chain: &Chain,
) -> Result<Recorded, MutantsError> {
    let registry = crate::equivalences::read_locked(hq_root);
    let identified = |s: &Survivor| crate::equivalences::identify(tree, head, s).ok();
    let mut from_registry = 0;
    let survivors = record(
        dir,
        Some(tree),
        fingerprint,
        head,
        text,
        chain,
        |s| identified(s).is_some(),
        |survivors| {
            if let Ok(registry) = &registry {
                from_registry = crate::equivalences::apply(registry, survivors, identified);
            }
        },
    )?;
    let registry_unread = match registry {
        Ok(_) => None,
        Err(why) => {
            crate::followup::registry_unread(&dir.join(crate::mission::dir::FOLLOWUP_FILE), &why)?;
            Some(why)
        }
    };
    Ok(Recorded {
        survivors,
        from_registry,
        registry_unread,
    })
}

/// Parse, continue the chain when `chain` is partial, carry ([`carry`] with
/// `identified`), let `then` add what it has, write.
///
/// A partial campaign continues the campaign on file, and only that one: one
/// whose `HEAD` is not the commit it ran since is a chain broken under it,
/// and nothing is written — a partial campaign alone would answer for the
/// files changed since, and say nothing of the rest.
///
/// Continuing it is a matter of files, never of lines ([`Chain`]). The files
/// whose content changed between the two commits — `git diff --name-only
/// --no-renames`, no hunk read — are the new campaign's: their counts and
/// their survivors come from it alone, and a file removed or renamed is
/// gone. Every other file keeps its counts and its survivors as the campaign
/// on file had them, unchanged since: the same file, the same lines. Kept
/// survivors are given back nothing the HQ said on them here; that comes
/// through [`carry`], as it does from one campaign to the next.
///
/// **Fails closed.** When the new campaign's own per-file counts are
/// missing or do not add up, or it counted or named a survivor in a file
/// that did not change, the reconstruction cannot be trusted: the campaign
/// is recorded with every survivor listed and **no** count, so gate 7
/// judges it as `critical` does, and the next campaign is full.
#[allow(clippy::too_many_arguments)]
fn record(
    dir: &Path,
    tree: Option<&Path>,
    fingerprint: &str,
    head: &str,
    text: &str,
    chain: &Chain,
    identified: impl Fn(&Survivor) -> bool,
    then: impl FnOnce(&mut [Survivor]),
) -> Result<usize, MutantsError> {
    let mut survivors = parse(text);
    let previous = read(dir)?;
    let mut chain = chain.clone();
    chain.earlier = Vec::new();
    chain.ran = None;
    let mut files = by_file(text, head);
    let mut counted = tried(text);
    if let Some(since) = chain.scope.since() {
        let previous = previous
            .as_ref()
            .filter(|p| p.head == since)
            .ok_or_else(|| MutantsError::Broken(broken(since, previous.as_ref())))?;
        let tree = tree.ok_or_else(|| {
            MutantsError::Broken("a partial campaign is recorded in its slot only".to_string())
        })?;
        let changed = changed_between(tree, since, head)?;
        let (earlier, kept, before) = continued(previous, &changed);
        chain.earlier = earlier;
        chain.ran = counted;
        let within = |file: &str| changed.contains(file);
        let trusted = survivors.iter().all(|s| within(&s.file))
            && files
                .as_ref()
                .is_some_and(|f| f.keys().all(|file| within(file)));
        (files, counted) = match (trusted, before, files) {
            (true, Some(mut before), Some(new)) => {
                before.extend(new);
                let total = tried_in(&before);
                (total.map(|_| before), total)
            }
            _ => (None, None),
        };
        survivors.extend(kept);
    }
    if let Some(previous) = &previous {
        carry(previous, &mut survivors, identified);
    }
    then(&mut survivors);
    let count = survivors.len();
    write(
        dir,
        &Campaign {
            fingerprint: fingerprint.to_string(),
            head: head.to_string(),
            date: crate::state::now_rfc3339(),
            survivors,
            tried: counted,
            files,
            chain,
        },
    )?;
    Ok(count)
}

/// Why a partial campaign since `since` does not continue `previous`, the
/// campaign on file.
fn broken(since: &str, previous: Option<&Campaign>) -> String {
    format!(
        "the campaign ran since {}, and {} — a partial campaign answers only for what \
         changed since the one it continues, so it is not recorded alone; `nunki mission \
         mutants --again` runs a full one",
        short(since),
        match previous {
            Some(p) => format!("the campaign on file is at {}", short(&p.head)),
            None => "no campaign is on file".to_string(),
        }
    )
}

/// What a partial campaign takes from the campaign it continues, given the
/// files `changed` since it: the chain so far with that campaign appended;
/// the survivors of the files that did not change, as they stood, each
/// naming the campaign that found it and with nothing the HQ said on it;
/// and those files' counts — `None` when the campaign on file kept none, in
/// which case nothing can be rebuilt.
///
/// A survivor of a changed file is dropped: that file is the new campaign's,
/// which measures it again whole. A survivor of a file removed or renamed is
/// dropped with it.
pub fn continued(
    previous: &Campaign,
    changed: &BTreeSet<String>,
) -> (Vec<Link>, Vec<Survivor>, Option<BTreeMap<String, Measured>>) {
    let mut earlier = previous.chain.earlier.clone();
    earlier.push(Link {
        head: previous.head.clone(),
        date: previous.date.clone(),
        scope: previous.chain.scope.clone(),
        tried: previous.chain.ran.or(previous.tried),
    });
    let kept = previous
        .survivors
        .iter()
        .filter(|s| !changed.contains(&s.file))
        .map(|s| Survivor {
            outcome: None,
            refused: None,
            found_on: s.found_on.clone().or_else(|| Some(previous.head.clone())),
            ..s.clone()
        })
        .collect();
    let before = previous.files.as_ref().map(|files| {
        files
            .iter()
            .filter(|(file, _)| !changed.contains(*file))
            .map(|(file, m)| (file.clone(), m.clone()))
            .collect()
    });
    (earlier, kept, before)
}

/// Every path whose content differs between `from` and `to`: added, removed
/// or modified, a file renamed counting as its old path removed and its new
/// one added (`--no-renames`). Compared by git as blobs; no hunk is read.
pub fn changed_between(
    tree: &Path,
    from: &str,
    to: &str,
) -> Result<BTreeSet<String>, MutantsError> {
    let names = git::run(
        tree,
        &["diff", "--name-only", "--no-renames", "-z", from, to],
    )?;
    Ok(names
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect())
}

/// Give each new survivor what the HQ said on its twin in the previous
/// campaign: its ruling, as a **proposal**; or its refusal.
///
/// **Carry never rules** (HQ review 4). Which mutant a ruling was about is
/// a heuristic across campaigns on both tiers — an id is a position, and
/// other code, even identical code, can land on it. So between campaigns a
/// ruling is only ever carried as a [`Triage::ProposedByNunki`] proposal
/// marked as carried, with its sentence and the commit it was given on, for
/// the HQ to ratify (`--ratify --all` takes them all). The only
/// equivalences in a campaign's [`FILE`] are the ones the HQ typed on that
/// campaign.
///
/// **The line is never part of the match**: it is exactly what a commit
/// above the mutant moves. The mutation itself — its file and what it
/// changed, the `description` — is what the ruling was about. Two tiers,
/// each over **every** survivor of the previous campaign, whatever it held:
///
/// - **the same id, file and description**: the same mutant, by the tool's
///   own name for it. Tried first, and when it finds one, it decides: the
///   twin's ruling is proposed, a proposal this tier made on it is carried
///   as that proposal, its refusal as a refusal — or nothing, for a twin
///   that held none of them. A proposal from the registry is not carried:
///   the registry is asked again;
/// - otherwise **the same file and description**, when that pair names
///   exactly one survivor of the whole previous campaign and exactly one
///   now, and `identified` says the survivor is told apart by its source —
///   the code its mutation replaces occurs once in its file: the same.
///
/// **A refusal is always carried** (HQ review 2, C): when no survivor of
/// the same id stood before, a refusal the HQ gave on any survivor of the
/// same file and description reaches the new one, whatever the uniqueness
/// of either. Carrying one too many reopens a survivor the coder must then
/// kill, freeze or have ruled; carrying one too few lets a proposal the HQ
/// said no to count as an outcome again. A refused survivor is given no
/// proposal.
///
/// The coder's own proposals are not carried here: they live in its own
/// file, by id. A changed description — including a status that moved from
/// `survived` to `no tests` — is a different mutant, and nothing is carried.
/// No id is parsed: which tool named it is the stack's business, not
/// `nunki`'s.
pub fn carry(
    previous: &Campaign,
    survivors: &mut [Survivor],
    identified: impl Fn(&Survivor) -> bool,
) {
    let from = |old: &Survivor| match &old.outcome {
        Some(Triage::Equivalent {
            carried_from: Some(first),
            ..
        }) => first.clone(),
        _ => previous.head.clone(),
    };
    // What a twin's outcome becomes in the new campaign: a ruling, a
    // proposal of it; a proposal this function made, itself.
    let proposed = |old: &Survivor| match &old.outcome {
        Some(Triage::Equivalent { why, .. }) => Some(Triage::ProposedByNunki {
            why: why.clone(),
            from: ProposedFrom::Carried { commit: from(old) },
        }),
        Some(made) if carried(made) => Some(made.clone()),
        _ => None,
    };
    let mut now: BTreeMap<(String, String), usize> = BTreeMap::new();
    for s in survivors.iter() {
        *now.entry(pair(s)).or_default() += 1;
    }
    for survivor in survivors.iter_mut() {
        if let Some(old) = previous.survivors.iter().find(|old| {
            old.id == survivor.id
                && old.file == survivor.file
                && old.description == survivor.description
        }) {
            survivor.outcome = proposed(old);
            survivor.refused = old.refused.clone();
            continue;
        }
        let same: Vec<&Survivor> = previous
            .survivors
            .iter()
            .filter(|old| pair(old) == pair(survivor))
            .collect();
        if let Some(refused) = same.iter().find_map(|old| old.refused.clone()) {
            survivor.refused = Some(refused);
            continue;
        }
        let [old] = same.as_slice() else {
            continue;
        };
        if now.get(&pair(survivor)) != Some(&1) || !identified(survivor) {
            continue;
        }
        survivor.outcome = proposed(old);
    }
}

/// Whether `outcome` is a proposal [`carry`] made, which it carries again
/// while the HQ has not ruled on it. One from the registry is not: the
/// registry is asked again by the next campaign, against the code as it
/// stands then, and an entry lifted since proposes nothing more.
fn carried(outcome: &Triage) -> bool {
    matches!(
        outcome,
        Triage::ProposedByNunki {
            from: ProposedFrom::Carried { .. },
            ..
        }
    )
}

/// What [`carry`] matches a mutation on when the id has changed: its file
/// and what it changed, never its line.
fn pair(s: &Survivor) -> (String, String) {
    (s.file.clone(), s.description.clone())
}

/// Record the HQ's own ruling on a survivor: this mutant changes nothing
/// observable, and here is why in one sentence.
///
/// A verb rather than an invitation to hand-edit JSON, because this is the
/// one outcome no machine can check and the person giving it should have to
/// say so on purpose. It writes into [`FILE`], which the agent cannot.
pub fn rule_equivalent(dir: &Path, id: &str, why: &str) -> Result<(), MutantsError> {
    let mut campaign = read(dir)?.ok_or_else(|| {
        MutantsError::Unreadable(
            dir.join(FILE),
            "there is no campaign to rule on — `nunki mission mutants` runs one".to_string(),
        )
    })?;
    // Every survivor the id names: a ruling given on one id and read on all
    // of them (`answer` goes by id) must be written on all of them.
    for found in called(&mut campaign, dir, id)? {
        found.outcome = Some(Triage::Equivalent {
            why: why.to_string(),
            carried_from: None,
        });
        // A ruling supersedes a refusal the HQ gave earlier on the same
        // survivor.
        found.refused = None;
    }
    write(dir, &campaign)
}

/// Every survivor of `campaign` called `id`, or an error naming the id when
/// none is.
///
/// Every one, not the first: an id is what the coder's file answers by
/// ([`answer`]), so one line there answers every survivor carrying it, and a
/// ruling or a refusal written on only the first would leave its twin
/// answered by a proposal nobody ruled on — and `nunki push` would pass it.
fn called<'a>(
    campaign: &'a mut Campaign,
    dir: &Path,
    id: &str,
) -> Result<Vec<&'a mut Survivor>, MutantsError> {
    let found: Vec<&mut Survivor> = campaign
        .survivors
        .iter_mut()
        .filter(|s| s.id == id)
        .collect();
    if found.is_empty() {
        return Err(MutantsError::Unreadable(
            dir.join(FILE),
            format!("no survivor is called {id:?} in this campaign"),
        ));
    }
    Ok(found)
}

/// The sentence of the proposal pending on survivor `id` — the coder's, or
/// `nunki`'s from a ruling it matched — or an error that says why there is
/// none to rule on.
fn proposal_on(dir: &Path, id: &str) -> Result<String, MutantsError> {
    let coders = read_triage(dir)?;
    let pending = read(dir)?
        .into_iter()
        .flat_map(|c| c.survivors)
        .filter(|s| s.id == id)
        .find_map(|s| {
            answer(&s, &coders).and_then(|a| a.proposal().map(|(why, _)| why.to_string()))
        });
    if let Some(why) = pending {
        return Ok(why);
    }
    match coders.get(id) {
        // Written again after the HQ refused it: no outcome for gate 7, but
        // the HQ may still change its mind and rule on it.
        Some(Triage::EquivalentProposed { why }) if !crate::text::blank(why) => Ok(why.clone()),
        // What is left of a proposal here is a blank one.
        Some(Triage::EquivalentProposed { .. }) => Err(MutantsError::NoProposal(format!(
            "the coder's proposal on {id:?} gives no reason, so it is no proposal — \
                 `--equivalent {id} --because <why>` rules on the survivor yourself"
        ))),
        _ => Err(MutantsError::NoProposal(format!(
            "{TRIAGE_FILE} proposes no equivalence on {id:?}, and `nunki` proposes none"
        ))),
    }
}

/// The HQ ratifies the proposal pending on survivor `id` — the coder's, or
/// `nunki`'s: exactly the `equivalent` [`rule_equivalent`] writes, with the
/// proposal's sentence as its reason unless `because` replaces it. The proposal is taken out of the
/// coder's file, since it is now a ruling — and [`carry`] proposes it again
/// to the next campaign, for the HQ to ratify there.
///
/// Returns the reason the ruling was written with.
pub fn ratify(dir: &Path, id: &str, because: Option<&str>) -> Result<String, MutantsError> {
    let proposed = proposal_on(dir, id)?;
    let why = match because {
        Some(because) if crate::text::blank(because) => {
            return Err(MutantsError::NoProposal(
                "--because is blank: a ruling nobody can check must say what it rests on"
                    .to_string(),
            ));
        }
        Some(because) => because.to_string(),
        None => proposed,
    };
    rule_equivalent(dir, id, &why)?;
    remove_triage_entry(dir, id)?;
    Ok(why)
}

/// The HQ refuses the proposal pending on survivor `id` — the coder's, or
/// `nunki`'s — because of `because`: the proposal is taken out of whichever
/// file held it, the refusal is recorded on the survivor in [`FILE`], and the
/// survivor is open again — a proposal written again on it is no outcome
/// ([`answer`]), and [`carry`] takes the refusal to every later campaign.
///
/// Returns the refusal, for the follow-up the next coder run reads.
pub fn refuse(dir: &Path, id: &str, because: &str) -> Result<Refusal, MutantsError> {
    if crate::text::blank(because) {
        return Err(MutantsError::NoProposal(
            "--refuse needs --because: the next coder run reads why, and a refusal that \
             says nothing teaches it nothing"
                .to_string(),
        ));
    }
    let mut campaign = read(dir)?.ok_or_else(|| {
        MutantsError::Unreadable(
            dir.join(FILE),
            "there is no campaign to rule on — `nunki mission mutants` runs one".to_string(),
        )
    })?;
    let found = called(&mut campaign, dir, id)?;
    // The HQ already ruled on it: its ruling answers the survivor whatever
    // the coder's file says, so a refusal would change nothing and send a
    // verified mission back for a survivor that is answered. Taking a ruling
    // back is `--lift`'s (HQ review of the pull request, item 5).
    if found
        .iter()
        .any(|s| s.outcome.as_ref().is_some_and(Triage::is_a_ruling))
    {
        return Err(MutantsError::NoProposal(format!(
            "{id:?} holds the HQ's own `equivalent` ruling, and a refusal cannot undo a \
             ruling — `nunki mission mutants <id> --lift {id}` takes it back first"
        )));
    }
    let proposed = proposal_on(dir, id)?;
    let refusal = Refusal {
        proposed,
        because: because.to_string(),
    };
    for found in found {
        found.refused = Some(refusal.clone());
        if matches!(found.outcome, Some(Triage::ProposedByNunki { .. })) {
            found.outcome = None;
        }
    }
    write(dir, &campaign)?;
    remove_triage_entry(dir, id)?;
    Ok(refusal)
}

/// [`refuse`], and the refusal written in `FOLLOWUP_HQ.md` as `who`'s — the
/// file the next coder run reads first, so the survivor it finds open again
/// comes with the reason it is.
pub fn refuse_and_say(
    paths: &crate::mission::dir::Paths,
    who: &str,
    id: &str,
    because: &str,
) -> Result<(), MutantsError> {
    let refusal = refuse(&paths.dir, id, because)?;
    crate::followup::proposal_refused(&paths.followup, who, id, &refusal)?;
    Ok(())
}

/// Lift the HQ's `equivalent` ruling from a survivor, so that it needs an
/// outcome again.
///
/// The way back from a ruling that was wrong, or that a changed neighbour
/// made wrong. Rulings are carried from one campaign to the next
/// ([`carry`]), so without this a mistaken one would outlive every replay.
/// Only an `equivalent` is lifted — and with it a proposal `nunki` made from
/// a ruling it matched, whose ruling is what is being taken back: the
/// coder's outcomes live in its own file, and this one never held them.
/// Taking it out of the registry too is the caller's
/// ([`crate::equivalences::remove`]), from the campaign as it stood before
/// the lift.
pub fn lift_equivalent(dir: &Path, id: &str) -> Result<(), MutantsError> {
    let mut campaign = read(dir)?.ok_or_else(|| {
        MutantsError::Unreadable(
            dir.join(FILE),
            "there is no campaign to lift a ruling from".to_string(),
        )
    })?;
    let found = called(&mut campaign, dir, id)?;
    if !found.iter().any(|s| liftable(s.outcome.as_ref())) {
        return Err(MutantsError::Unreadable(
            dir.join(FILE),
            format!("{id:?} holds no `equivalent` ruling to lift"),
        ));
    }
    for survivor in found {
        if liftable(survivor.outcome.as_ref()) {
            survivor.outcome = None;
        }
    }
    write(dir, &campaign)
}

/// What `--lift` takes off a survivor: the HQ's ruling, or `nunki`'s
/// proposal of one.
pub fn liftable(outcome: Option<&Triage>) -> bool {
    matches!(
        outcome,
        Some(Triage::Equivalent { .. } | Triage::ProposedByNunki { .. })
    )
}

/// Whether a campaign already on file is enough.
///
/// A campaign is expensive — SPEC § 7 counts the hour — so the default is
/// that one on the same content is not run again. But "the same content" is
/// the fingerprint over the touched files, and a campaign's answer also
/// depends on what it ran *with*: the stack's `mutation.sh`, the tool's
/// version, an exclusion added since. None of those move the fingerprint.
///
/// Without it the only way past is to delete `MUTANTS.json` by hand, which
/// can take `MUTANTS.triage.json` with it — a file the engine then replaces
/// with a directory, which brings the mission down. A verb is cheaper than
/// the workaround it replaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Replay {
    /// Only when the touched files have changed. The default (SPEC 4.4).
    WhenChanged,
    /// Now, whatever is on file. Never touches a campaign in flight: one
    /// already running is reported as running, and asking again does not
    /// start a second.
    Now,
}

/// Whether the next campaign may be partial, and when not, why (SPEC 4.4,
/// gate 7: the chain of a mission's campaigns).
///
/// Partial — the touched files whose content changed since the previous
/// campaign's `HEAD`, each measured as a full campaign would measure it —
/// only when **every** condition holds, asked in this order; the first that
/// does not makes the campaign full, and its reason is recorded:
///
/// - the mission's rigor is `standard`: `critical` runs one full campaign
///   over everything the branch changed, every time;
/// - the human did not ask `--again`, which is a full campaign by
///   definition: it is asked for what the chain cannot see;
/// - a previous campaign of this mission is on file — only a completed one
///   ever is;
/// - gate 7 passed on it: `owed` says what it still owes, as the gate
///   judges the whole chain it closes ([`owed`]);
/// - its `HEAD` is an ancestor of the current one (`is_ancestor`): a history
///   rewritten under it leaves no diff to continue from;
/// - it counted each file ([`Campaign::files`]), and those counts add up to
///   its total: a partial campaign is rebuilt from them, and a script that
///   gives none — the Python and Next.js ones, a project's own — or gave
///   ones that do not add up leaves nothing to rebuild from;
/// - the files that changed since it could be listed (`compare`, which says
///   why when they could not): what is not compared is measured again;
/// - it ran with the same `mutation.sh` and tool as this one will: both
///   digests known ([`tooling`]) and equal.
///
/// **Fails closed**: a condition that cannot be checked is one that does not
/// hold. A partial campaign assumes that a mutant killed earlier in a file
/// nobody changed is still killed, and the README says so; `--again` and
/// `critical` do not assume it.
pub fn scope(
    rigor: crate::mission::Rigor,
    replay: Replay,
    previous: Option<&Campaign>,
    tooling: Option<&str>,
    owed: impl Fn(&Campaign) -> Option<String>,
    is_ancestor: impl Fn(&str) -> bool,
    compare: impl Fn(&Campaign) -> Option<String>,
) -> Scope {
    let full = |why: String| Scope::Full { why };
    if rigor != crate::mission::Rigor::Standard {
        return full(format!(
            "a `{rigor}` mission's campaigns are all full: only `standard` chains them"
        ));
    }
    if replay == Replay::Now {
        return full("asked `--again`, which is a full campaign".to_string());
    }
    let Some(previous) = previous else {
        return full("the first campaign of this mission".to_string());
    };
    let at = crate::text::one_line(short(&previous.head)).to_string();
    if let Some(owed) = owed(previous) {
        return full(format!(
            "gate 7 did not pass on the previous campaign, at {at}: {}",
            crate::text::one_line(&owed)
        ));
    }
    if !is_ancestor(&previous.head) {
        return full(format!(
            "the previous campaign's commit {at} is not an ancestor of HEAD"
        ));
    }
    let Some(files) = &previous.files else {
        return full(format!(
            "the previous campaign, at {at}, did not count each file — its stack's \
             mutation.sh gives no per-file counts, or they did not add up — so there is \
             nothing to rebuild a partial campaign from"
        ));
    };
    if tried_in(files) != previous.tried {
        return full(format!(
            "the previous campaign's per-file counts, at {at}, do not add up to its total"
        ));
    }
    if let Some(why) = compare(previous) {
        return full(format!(
            "the files changed since the previous campaign, at {at}, could not be listed: {}",
            crate::text::one_line(&why)
        ));
    }
    match (tooling, previous.chain.tooling.as_deref()) {
        (None, _) => full(
            "what this campaign runs with could not be read: a stack's mutation.sh, or its \
             tool's version"
                .to_string(),
        ),
        (Some(_), None) => full(format!(
            "the previous campaign, at {at}, recorded nothing of what it ran with"
        )),
        (Some(now), Some(then)) if now != then => full(format!(
            "a stack's mutation.sh or its tool's version changed since the previous \
             campaign, at {at}"
        )),
        (Some(_), Some(_)) => Scope::Partial {
            since: previous.head.clone(),
        },
    }
}

/// The line of a stack's `mutation.sh` that says how to read its tool's
/// version: `# nunki-tool-version: <command>`, run in the clean copy.
///
/// The stack's to say, since `nunki` knows no mutation tool (SPEC 4.4): a
/// script that says nothing — a project's own, written before this — runs
/// full campaigns only, which is today's behaviour.
pub const TOOL_VERSION_MARK: &str = "# nunki-tool-version:";

/// The command a `mutation.sh` names after [`TOOL_VERSION_MARK`], if it
/// names one.
pub fn tool_version_command(script: &str) -> Option<&str> {
    script
        .lines()
        .find_map(|line| line.trim().strip_prefix(TOOL_VERSION_MARK))
        .map(str::trim)
        .filter(|command| !command.is_empty())
}

/// What a campaign runs with, stack by stack — its name, its `mutation.sh`
/// as the container reads it, and what its tool said of its version — as
/// the text [`tooling`] digests. `None` when a stack's version is unknown:
/// what cannot be compared does not match.
pub fn tooling_manifest(stacks: &[(String, String, Option<String>)]) -> Option<String> {
    let mut manifest = String::new();
    for (stack, script, version) in stacks {
        let version = version
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())?;
        manifest.push_str(&format!(
            "stack {stack}\nversion {version}\nscript {}\n{script}\n",
            script.len()
        ));
    }
    Some(manifest)
}

/// The digest of what a campaign about to run will run with: each judged
/// stack's `mutation.sh` and its tool's version, read in the clean copy
/// through the slot's container ([`tooling_manifest`]). `None` when any of
/// it cannot be read — a script that names no version command, a command
/// that fails or says nothing.
pub fn tooling(
    project: &crate::project::Project,
    slot: &crate::slot::Slot,
    engine: std::sync::Arc<dyn crate::engine::Engine>,
    judged: &[crate::run::Judged],
) -> Result<Option<String>, MutantsError> {
    let mut stacks = Vec::new();
    for j in judged {
        let run = |argv: &[&str]| {
            crate::exec::run(
                project,
                slot,
                engine.clone(),
                &argv.iter().map(|a| a.to_string()).collect::<Vec<_>>(),
                crate::exec::On::Proof,
            )
        };
        let script = run(&["cat", &format!("{}/{SCRIPT}", j.scripts_at)])?;
        if !script.ok() {
            return Ok(None);
        }
        let version = match tool_version_command(&script.stdout) {
            Some(command) => {
                let said = run(&["sh", "-c", command])?;
                said.ok().then_some(said.stdout)
            }
            None => None,
        };
        stacks.push((j.stack.name.clone(), script.stdout, version));
    }
    match tooling_manifest(&stacks) {
        Some(manifest) => Ok(Some(hash_object(&slot.tree, &manifest)?)),
        None => Ok(None),
    }
}

/// The paths a partial campaign since `since` hands its stack: those the
/// branch touched whose content changed since then ([`changed_between`]).
/// A path changed since and back to the base's content is not the branch's,
/// and is not given.
pub fn touched_since(
    tree: &Path,
    since: &str,
    touched: &[String],
) -> Result<Vec<String>, MutantsError> {
    let changed = changed_between(tree, since, "HEAD")?;
    Ok(touched
        .iter()
        .filter(|path| changed.contains(*path))
        .cloned()
        .collect())
}

/// The campaign on file, when it answers for this content and the caller is
/// willing to take it.
///
/// Public, and its own function, because it **is** the whole of [`Replay`]:
/// reaching it through [`campaign`] needs a container, so a test that cannot
/// call it could only check that a campaign started, never why. A campaign costs
/// an hour (SPEC § 7), so one on the same content is not run again — unless a
/// human says something changed that the fingerprint cannot see, since the
/// fingerprint is over the touched files and not over `mutation.sh`, the
/// tool's version, or an exclusion added since.
pub fn already_answered(
    dir: &Path,
    want: &str,
    replay: Replay,
) -> Result<Option<Progress>, MutantsError> {
    if replay == Replay::Now {
        return Ok(None);
    }
    let Some(existing) = read(dir)? else {
        return Ok(None);
    };
    if existing.fingerprint != want {
        return Ok(None);
    }
    Ok(Some(Progress::Fresh {
        survivors: existing.survivors.len(),
    }))
}

/// Where a campaign is at, as a caller reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    /// Nothing to do: a campaign on this exact content is already on file.
    Fresh { survivors: usize },
    /// Launched just now, full or partial.
    Started { fingerprint: String, scope: Scope },
    /// Still going.
    Running { started_at: String, lines: usize },
    /// Ended, and `MUTANTS.json` is written. `chain` says each campaign of
    /// the chain in one line ([`chain_said`]).
    Finished {
        survivors: usize,
        chain: Vec<String>,
    },
    /// Past its deadline and stopped (SPEC 4.4, "un délai paramétré").
    Overrun { minutes: u32 },
    /// Ended without the line that says it finished, and its stderr says
    /// why: a test the branch carries failing inside `mutants/`, a crate that
    /// does not compile. That is the branch's to fix, and gate 7 reads the
    /// same stderr on the next `verify` and goes red, which sends a volet —
    /// so nothing here is waiting on a human.
    ///
    /// Kept apart from [`Progress::Lost`] because the monitor goes on after
    /// this one and stops after that one. A campaign that left no word on
    /// stderr leaves gate 7 unplayed, and a monitor that went on would start
    /// the same campaign on every tick.
    CouldNotRun(String),
    /// The container went away under it, or the engine could not be asked,
    /// or it ended having said nothing at all.
    Lost(String),
}

/// What a campaign that has ended says, once `FILE` is written.
///
/// Two sentences and not one: with nothing alive there is nobody to triage,
/// and "each needs one of the three outcomes" would then be a sentence about
/// no one. A clean campaign must not ask for outcomes it does not need — a
/// line that says the same thing whatever happened is a line a reader
/// learns to skip.
pub fn ended(survivors: usize) -> String {
    if survivors == 0 {
        return format!("finished — no survivor in {FILE}: gate 7 has nothing left to ask");
    }
    format!(
        "finished — {survivors} survivor(s) in {FILE}; each needs one of the three \
         outcomes before gate 7 is green"
    )
}

/// What a campaign that has just been launched says to the human whose verb
/// launched it.
///
/// Here rather than in `main.rs` for the same reason [`ended`] is: a sentence
/// a test can read is a sentence a test can catch. `cargo fmt` can join a
/// multi-line literal and keep the indentation inside the string, printing
/// fifty spaces in the middle of it, and only a test notices.
pub fn started(mission: &str) -> String {
    format!("started; it runs detached, and `nunki verify {mission}` reads it back")
}

/// Read back the campaign that owns this slot's clean copy, if one is filed.
///
/// `None` when nothing is filed. Otherwise what it is now — and the reading
/// **clears the record** whenever the campaign is over, however it ended, so
/// a caller that asks cannot be misled by a stale file.
///
/// Its own function because two callers need the answer and only one of them
/// may launch: [`campaign`] below, and `run::launch`, which recreates the
/// container a campaign lives in and would otherwise kill it blind.
pub fn read_back(
    project: &crate::project::Project,
    slot: &crate::slot::Slot,
    engine: std::sync::Arc<dyn crate::engine::Engine>,
    dir: &Path,
) -> Result<Option<Progress>, MutantsError> {
    use crate::harness::spawn::{Presence, Signal, Spawned, Spawner};

    let Some(running) = read_running(&project.hq_root, &slot.name)? else {
        return Ok(None);
    };
    let spawner = crate::engine::spawn::ContainerSpawner::new(
        engine,
        crate::run::profile_path(project, &slot.name),
        &crate::compose::project_name(&project.session(), &slot.name)
            .map_err(|e| MutantsError::Exec(crate::exec::ExecError::Compose(e)))?,
        crate::compose::AGENT_SERVICE,
    )
    .identified_by(&running.fingerprint);
    let spawned = Spawned {
        pid: running.pid,
        container: running.container.clone(),
    };
    let presence = spawner
        .alive(&spawned)
        .map_err(|e| MutantsError::Launch(e.to_string()))?;
    let text = std::fs::read_to_string(&running.log).unwrap_or_default();
    let progress = match presence {
        // A frozen campaign is still in flight, and its deadline must not run
        // against a clock the human stopped on purpose. Reported as running,
        // with the freeze named, rather than counted as an overrun.
        Presence::Paused => Progress::Running {
            started_at: running.started_at.clone(),
            lines: text.lines().count(),
        },
        Presence::Running => {
            if minutes_since(&running.started_at) > running.deadline_minutes {
                // Stopped rather than left: SPEC 4.4 gives the campaign a
                // configured delay, and a campaign past it is holding a slot,
                // not working in it.
                let _ = spawner.signal(&spawned, Signal::Terminate);
                forget_running(&project.hq_root, &slot.name)?;
                Progress::Overrun {
                    minutes: running.deadline_minutes,
                }
            } else {
                Progress::Running {
                    started_at: running.started_at.clone(),
                    lines: text.lines().count(),
                }
            }
        }
        // Ended, and it said so. Anything else is a campaign that stopped:
        // nothing is written, so gate 7 keeps asking rather than passing on a
        // log that happens to hold no survivor.
        Presence::Ended if completed(&text) => {
            // Finished, and still measured nothing: not recorded, and its
            // reason left where gate 7 reads it.
            let progress = match settle_unmeasured(&running.log, &text)? {
                Some(why) => Progress::CouldNotRun(why),
                None => match record_finished_with_registry(
                    dir,
                    &project.hq_root,
                    &slot.tree,
                    &running.fingerprint,
                    &running.head,
                    &text,
                    &running.chain,
                ) {
                    Ok(recorded) => Progress::Finished {
                        survivors: recorded.survivors,
                        chain: chain_on_file(dir)?,
                    },
                    // A partial campaign whose chain broke under it measured
                    // only what changed since, and is not recorded alone:
                    // forgotten, and said, rather than asked about for ever.
                    Err(MutantsError::Broken(why)) => Progress::Lost(why),
                    Err(e) => return Err(e),
                },
            };
            forget_running(&project.hq_root, &slot.name)?;
            progress
        }
        Presence::Ended => {
            forget_running(&project.hq_root, &slot.name)?;
            // Said it finished more than once: not a campaign that stopped,
            // and its reason is left where gate 7 reads it.
            if let Some(why) = settle_repeated(&running.log, &text)? {
                return Ok(Some(Progress::CouldNotRun(why)));
            }
            // The **stderr** file, not the log. A campaign that got nowhere
            // wrote nothing to stdout — that is what "after 0 line(s)" says —
            // so naming the log sends whoever reads this to an empty file,
            // precisely when they need the reason. Typically the log is
            // 0 bytes and the sibling `.err` holds the traceback that
            // explains it, which otherwise has to be found by hand.
            //
            // The log is still named when it holds something: a campaign that
            // printed survivors and then died says more there than in its
            // stderr.
            let stderr = running.log.with_extension("err");
            let said_why = std::fs::read_to_string(&stderr)
                .map(|t| !t.trim().is_empty())
                .unwrap_or(false);
            let diagnosis = if text.trim().is_empty() {
                stderr
            } else {
                running.log.clone()
            };
            let why = format!(
                "it stopped without saying it had finished, after {} line(s): a campaign \
                 that was killed, whose container went away, or that never compiled \
                 leaves exactly this, and none of them measured anything. What it said \
                 is in {}",
                text.lines().count(),
                diagnosis.display()
            );
            // A campaign whose stderr explains the failure (a guard failing
            // inside `mutants/`, say) is one gate 7 can go red on. Reporting
            // it as lost instead stops the monitor here, and the volet waits
            // for someone to type `nunki verify`.
            if said_why {
                Progress::CouldNotRun(why)
            } else {
                Progress::Lost(why)
            }
        }
        Presence::Vanished(why) | Presence::Unknown(why) => {
            forget_running(&project.hq_root, &slot.name)?;
            Progress::Lost(why)
        }
    };
    Ok(Some(progress))
}

/// End the campaign that owns this slot's clean copy, and forget it.
///
/// Nothing is written to [`FILE`]: a campaign cut short measured nothing, and
/// gate 7 must ask for another rather than read this one (SPEC 4.4).
pub fn end(
    project: &crate::project::Project,
    slot: &crate::slot::Slot,
    engine: std::sync::Arc<dyn crate::engine::Engine>,
) -> Result<(), MutantsError> {
    use crate::harness::spawn::{Signal, Spawned, Spawner};

    let Some(running) = read_running(&project.hq_root, &slot.name)? else {
        return Ok(());
    };
    let spawner = crate::engine::spawn::ContainerSpawner::new(
        engine,
        crate::run::profile_path(project, &slot.name),
        &crate::compose::project_name(&project.session(), &slot.name)
            .map_err(|e| MutantsError::Exec(crate::exec::ExecError::Compose(e)))?,
        crate::compose::AGENT_SERVICE,
    )
    .identified_by(&running.fingerprint);
    let _ = spawner.signal(
        &Spawned {
            pid: running.pid,
            container: running.container.clone(),
        },
        Signal::Terminate,
    );
    forget_running(&project.hq_root, &slot.name)
}

/// What a caller asks of [`campaign`], beyond where it runs.
#[derive(Debug, Clone, Copy)]
pub struct Asked<'a> {
    /// The mission's base, by **name**: [`campaign`] works out what the
    /// branch brought through the same call gate 7 makes.
    pub base: &'a str,
    /// How long it is given before `nunki` calls it hung.
    pub deadline_minutes: u32,
    pub replay: Replay,
    /// The mission's rigor: only `standard` chains its campaigns ([`scope`]).
    pub rigor: crate::mission::Rigor,
    /// The share gate 7 asks for at `standard`: the one the previous
    /// campaign is judged against before a partial one continues it.
    pub threshold: u32,
}

/// Start the campaign, or say where the one in flight is.
///
/// Long by nature, so it is launched **detached** and watched like a run: one
/// call starts it, later calls report on it, and the call that finds it ended
/// writes [`FILE`].
///
/// The stack's script is invoked as `mutation.sh <campaign-id> <path>…`. The
/// id is the content fingerprint, and it is first so that the campaign is
/// identifiable from its own command line — process ids inside a container
/// are recycled within seconds, and liveness here means "this campaign", not
/// "something holds that number". The fork point travels in [`BASE_ENV`].
///
/// A partial campaign has the same base, and as paths those the branch
/// touched whose content changed since the previous campaign's `HEAD`
/// ([`scope`], [`touched_since`]).
pub fn campaign(
    project: &crate::project::Project,
    slot: &crate::slot::Slot,
    engine: std::sync::Arc<dyn crate::engine::Engine>,
    dir: &Path,
    stack: &str,
    asked: &Asked<'_>,
) -> Result<Progress, MutantsError> {
    use crate::harness::spawn::Spawner;

    let head = git::head(&slot.tree)?;
    // The base's **name**, and the paths worked out here — not handed in.
    //
    // Gate 7 judges a campaign by the fingerprint of what it ran on, so the
    // gate and the launcher have to mean the same thing by "what this branch
    // brought". They were two calls in two files, and in a slot the base has
    // two readings. When the launcher and the gate computed the touched set
    // separately, their fingerprints could disagree, and gate 7 kept asking
    // for a campaign that had just run.
    //
    // One call, in the place that cannot be bypassed, rather than a rule the
    // next caller has to know.
    let touched = crate::gate::touched_since_base(&slot.tree, asked.base)
        .map_err(|e| MutantsError::Launch(e.to_string()))?;
    // The fingerprint stays over the touched files' content, not the fork
    // point: a base that moves without changing a touched file leaves the
    // diff of those files unchanged, and one that does change them changes
    // their content after the rebase.
    let fork = crate::gate::fork_point(&slot.tree, asked.base)
        .map_err(|e| MutantsError::Launch(e.to_string()))?;
    let want = fingerprint(&slot.tree, &touched)?;
    let compose_project = crate::compose::project_name(&project.session(), &slot.name)
        .map_err(|e| MutantsError::Exec(crate::exec::ExecError::Compose(e)))?;

    if let Some(progress) = read_back(project, slot, engine.clone(), dir)? {
        return Ok(progress);
    }

    // Nothing in flight. A campaign on this exact content is not run again:
    // SPEC 4.4 says it replays only when the touched files have changed, and
    // § 7 counts the hour it would otherwise spend.
    if let Some(fresh) = already_answered(dir, &want, asked.replay)? {
        return Ok(fresh);
    }

    let judged = crate::run::judged(project, stack);
    crate::exec::refresh(project, slot, engine.clone())?;
    // Absent or not executable is said here, where the message can be about
    // the campaign, rather than as a launch that times out waiting for a pid.
    for j in &judged {
        let script = format!("{}/{SCRIPT}", j.scripts_at);
        let probe = crate::exec::run(
            project,
            slot,
            engine.clone(),
            &[
                "sh".to_string(),
                "-c".to_string(),
                format!("test -x {script}"),
            ],
            crate::exec::On::Proof,
        )?;
        if !probe.ok() {
            return Err(MutantsError::Launch(format!(
                "there is no executable {script} in the container — the mutation campaign \
                 is a deterministic command the `{}` stack declares, in the project's home \
                 (SPEC 4.4)",
                j.stack.name
            )));
        }
    }

    // Full or partial (SPEC 4.4, the chain). The previous campaign is judged
    // as gate 7 would judge it now, the tests its outcomes name included.
    let tooling = match asked.rigor {
        crate::mission::Rigor::Standard => tooling(project, slot, engine.clone(), &judged)?,
        _ => None,
    };
    let coders = read_triage(dir)?;
    let scope = scope(
        asked.rigor,
        asked.replay,
        read(dir)?.as_ref(),
        tooling.as_deref(),
        |previous| {
            owed(previous, &coders, asked.rigor, asked.threshold)
                .or_else(|| crate::gate::named_test_missing(&slot.tree, None, previous, &coders))
        },
        |commit| git::run(&slot.tree, &["merge-base", "--is-ancestor", commit, "HEAD"]).is_ok(),
        |previous| {
            changed_between(&slot.tree, &previous.head, "HEAD")
                .err()
                .map(|e| e.to_string())
        },
    );
    let launched = launch_command(
        &slot.tree,
        &judged,
        &want,
        &touched,
        &fork,
        &scope,
        project.config.jobs(),
    )?;

    let runs = dir.join("runs");
    std::fs::create_dir_all(&runs).map_err(|e| MutantsError::Io(runs.clone(), e))?;
    let log = runs.join(format!("mutants-{}.log", &want[..7.min(want.len())]));
    // Emptied first. The spawner opens a log **in append mode**, which is
    // right for a harness run — one resumes into the same file and its
    // earlier turns must survive — and wrong for a campaign, which is one
    // measurement and not a stream. The name carries the fingerprint, so a
    // campaign replayed over content that has not changed writes into the
    // same file as the one before it, and `parse` reads the union.
    //
    // Appending instead, several campaigns on one fingerprint (5 survivors,
    // then 1, then none) leave `MUTANTS.json` reporting six, every one of
    // them dead. Gate 7 then sends a coder back for mutants that no longer
    // exist, and nothing in the flow can tell: the file says what it says.
    std::fs::write(&log, "").map_err(|e| MutantsError::Io(log.clone(), e))?;
    let spawner = crate::engine::spawn::ContainerSpawner::new(
        engine,
        crate::run::profile_path(project, &slot.name),
        &compose_project,
        crate::compose::AGENT_SERVICE,
    )
    .identified_by(&want);
    let spawned = spawner
        .spawn(&launched, &log)
        .map_err(|e| MutantsError::Launch(e.to_string()))?;

    write_running(
        &project.hq_root,
        &slot.name,
        &Running {
            fingerprint: want.clone(),
            head,
            started_at: crate::state::now_rfc3339(),
            container: spawned.container,
            pid: spawned.pid,
            log,
            deadline_minutes: asked.deadline_minutes,
            chain: Chain {
                scope: scope.clone(),
                tooling,
                earlier: Vec::new(),
                ran: None,
            },
        },
    )?;
    Ok(Progress::Started {
        fingerprint: want,
        scope,
    })
}

/// What a campaign of `scope` runs ([`command`]): a full one on every path
/// the branch touched; a partial one on those whose content changed since
/// the previous campaign's `HEAD` ([`touched_since`]). Both with the fork
/// point in [`BASE_ENV`]: in each file it is given, a partial campaign
/// mutates everything the branch changed, exactly as a full one would, so
/// the file's counts are the ones a full campaign would give.
pub fn launch_command(
    tree: &Path,
    judged: &[crate::run::Judged],
    campaign: &str,
    touched: &[String],
    fork: &str,
    scope: &Scope,
    jobs: u32,
) -> Result<crate::harness::spawn::CommandSpec, MutantsError> {
    let paths = match scope.since() {
        Some(since) => touched_since(tree, since, touched)?,
        None => touched.to_vec(),
    };
    Ok(command(judged, campaign, &paths, fork, jobs))
}

/// What a campaign runs in the container: each stack's `mutation.sh` on the
/// touched paths, from the clean copy, with the fork point in [`BASE_ENV`]
/// and the project's `mutation_jobs` in [`JOBS_ENV`].
///
/// `campaign` is the fingerprint, `fork` the commit the branch forked from.
/// One stack runs its script directly, as it always has; several run through
/// [`several_campaigns`], whose child scripts inherit the same environment.
pub fn command(
    judged: &[crate::run::Judged],
    campaign: &str,
    touched: &[String],
    fork: &str,
    jobs: u32,
) -> crate::harness::spawn::CommandSpec {
    let (program, args) = match judged {
        [one] => {
            let mut args = vec![campaign.to_string()];
            args.extend(touched.iter().cloned());
            (format!("{}/{SCRIPT}", one.scripts_at), args)
        }
        several => (
            "sh".to_string(),
            vec![
                "-c".to_string(),
                several_campaigns(several, campaign, touched),
            ],
        ),
    };
    crate::harness::spawn::CommandSpec {
        program,
        args,
        cwd: PathBuf::from(crate::exec::PROOF_AT),
        env: [
            (BASE_ENV.to_string(), fork.to_string()),
            (JOBS_ENV.to_string(), jobs.to_string()),
        ]
        .into_iter()
        .collect(),
    }
}

/// The campaign of a project carrying several stacks, as one shell script
/// (SPEC 4.2, "plusieurs stacks").
///
/// Each touched path goes to the stack whose directory holds it — the
/// deepest one, so `frontend/` beats the root — relative to that directory,
/// and each stack's `mutation.sh` runs there on its own paths; a stack this
/// branch did not touch has nothing to mutate and is not called. What each
/// prints keeps the contract `nunki` reads, made the project's: an id
/// prefixed with the stack's name, since two tools number their mutants
/// independently, and a file relative to the repository, like the touched
/// list. `{"campaign":"done"}` is said once, at the end, and only if every
/// stack said it: a campaign one stack did not finish measured nothing for it.
/// A stack that said it more than once did not finish either, by the rule
/// [`completed`] reads a log with.
/// A stack that found mutants and tried none measured nothing, and the
/// campaign is then not finished either, whatever the other stacks did.
/// It carries `tried` and `found`, each the stacks' counts added up, and
/// each only when every stack that ran gave one: a sum missing a stack's
/// mutants would be a share of the wrong whole. Without `tried` gate 7
/// judges as `critical` does; without `found` the not-measured rule has
/// nothing to compare, and the stacks' own scripts are what refuse.
///
/// Each stack's per-file counts pass through, their paths made the
/// repository's like the survivors' files, and the line says `by_file` only
/// when every stack that ran said it: one stack that counts no file leaves
/// the project's campaign with nothing a later partial one could be rebuilt
/// from.
///
/// Whether a stack said it finished is read from what `jq` **printed**, not
/// from `jq -e`: jq 1.6 — Debian bookworm's — exits 0 under `-e` on an
/// empty input, so a stack that printed nothing at all was counted as
/// finished.
///
/// `jq` reads the lines, as every stack image carries it; a line that is not
/// JSON is progress and is dropped from the result, which is where the
/// contract already puts it.
pub fn several_campaigns(
    judged: &[crate::run::Judged],
    campaign: &str,
    touched: &[String],
) -> String {
    use crate::exec::quote;
    let mut script = String::from(
        "set -u\ncomplete=1\ncounted=1\ntried=0\nsized=1\nfound=0\nbyfile=1\nout=\"$(mktemp)\"\n",
    );
    for j in judged {
        let mine: Vec<String> = touched
            .iter()
            .filter(|path| owner(judged, path).is_some_and(|o| o.stack == j.stack))
            .map(|path| match j.stack.dir.as_str() {
                "" => path.clone(),
                dir => path[dir.len() + 1..].to_string(),
            })
            .collect();
        if mine.is_empty() {
            continue;
        }
        let dir = if j.stack.dir.is_empty() {
            ".".to_string()
        } else {
            j.stack.dir.clone()
        };
        let prefix = if j.stack.dir.is_empty() {
            String::new()
        } else {
            format!("{}/", j.stack.dir)
        };
        script.push_str(&format!(
            "( cd {dir} && {at}/{SCRIPT} {campaign} {paths} ) > \"$out\"\n\
             jq -R -c --arg s {name} --arg d {prefix} 'fromjson? | select(type == \"object\" \
             and has(\"id\")) | .id = ($s + \":\" + (.id | tostring)) | .file = ($d + \
             (.file | tostring))' < \"$out\"\n\
             jq -R -c --arg d {prefix} 'fromjson? | select(type == \"object\" and \
             has(\"measured\")) | .measured = ($d + (.measured | tostring))' < \"$out\"\n\
             counts=$(jq -R -r 'fromjson? | select(type == \"object\" and .campaign == \
             \"done\") | (.by_file // false) | tostring' < \"$out\" | tail -n 1)\n\
             [ \"$counts\" = true ] || byfile=0\n\
             dones=$(jq -R -c 'fromjson? | select(type == \"object\" and .campaign == \"done\")' \
             < \"$out\" | grep -c .)\n\
             said=$(jq -R -r 'fromjson? | select(type == \"object\" and .campaign == \"done\") \
             | (.tried // \"none\") | tostring' < \"$out\" | tail -n 1)\n\
             [ \"$dones\" = 1 ] || said=''\n\
             case \"$said\" in\n\
             '') complete=0 ;;\n\
             *[!0-9]*) counted=0 ;;\n\
             *) tried=$((tried + said)) ;;\n\
             esac\n\
             seen=$(jq -R -r 'fromjson? | select(type == \"object\" and .campaign == \"done\") \
             | (.found // \"none\") | tostring' < \"$out\" | tail -n 1)\n\
             case \"$seen\" in\n\
             ''|*[!0-9]*) sized=0 ;;\n\
             *) found=$((found + seen)) ;;\n\
             esac\n\
             case \"$said:$seen\" in 0:[1-9]*) complete=0 ;; esac\n",
            dir = quote(&dir),
            at = j.scripts_at,
            campaign = quote(campaign),
            paths = mine.iter().map(|p| quote(p)).collect::<Vec<_>>().join(" "),
            name = quote(&j.stack.name),
            prefix = quote(&prefix),
        ));
    }
    script.push_str(
        "rm -f \"$out\"\n\
         if [ \"$complete\" = 1 ]; then\n\
         line='{\"campaign\":\"done\"'\n\
         if [ \"$counted\" = 1 ]; then line=\"$line,\\\"tried\\\":$tried\"; fi\n\
         if [ \"$sized\" = 1 ]; then line=\"$line,\\\"found\\\":$found\"; fi\n\
         if [ \"$counted\" = 1 ] && [ \"$byfile\" = 1 ]; then line=\"$line,\\\"by_file\\\":true\"; fi\n\
         printf '%s}\\n' \"$line\"\n\
         fi\n",
    );
    script
}

/// The stack a path of the repository belongs to: the one with the deepest
/// directory holding it, the root holding everything.
fn owner<'a>(judged: &'a [crate::run::Judged], path: &str) -> Option<&'a crate::run::Judged> {
    judged
        .iter()
        .filter(|j| {
            j.stack.dir.is_empty()
                || path
                    .strip_prefix(&j.stack.dir)
                    .is_some_and(|rest| rest.starts_with('/'))
        })
        .max_by_key(|j| j.stack.dir.len())
}

/// Minutes since an RFC 3339 stamp, read the way `nunki` writes them. A stamp it
/// cannot read counts as zero rather than as an overrun: the campaign is not
/// killed because a clock was unreadable.
fn minutes_since(stamp: &str) -> u32 {
    let now = crate::state::now_rfc3339();
    let (Some(then), Some(now)) = (epoch_minutes(stamp), epoch_minutes(&now)) else {
        return 0;
    };
    now.saturating_sub(then)
}

/// `YYYY-MM-DDTHH:MM:SSZ` into minutes, on a calendar simple enough to be
/// right for a difference of hours.
fn epoch_minutes(stamp: &str) -> Option<u32> {
    let bytes = stamp.as_bytes();
    if bytes.len() < 16 {
        return None;
    }
    let n = |from: usize, to: usize| stamp.get(from..to)?.parse::<u32>().ok();
    let (y, mo, d) = (n(0, 4)?, n(5, 7)?, n(8, 10)?);
    let (h, mi) = (n(11, 13)?, n(14, 16)?);
    // Days since an arbitrary epoch, counting months as they come. Leap years
    // matter only across a February, and a campaign does not run for a month.
    let days = y * 366 + mo * 31 + d;
    Some(days * 24 * 60 + h * 60 + mi)
}
