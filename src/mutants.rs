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

use std::collections::BTreeMap;
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

/// A mutant the campaign could not kill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Survivor {
    /// What the tool calls it — enough to find it again.
    pub id: String,
    pub file: String,
    pub line: u32,
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
    /// The HQ's ruling, given on another mission and applied here from the
    /// project's registry of equivalences ([`crate::equivalences`]): the same
    /// mutation, on a line whose content has not changed since. An
    /// `equivalent` in every way that counts — the HQ's, never the coder's —
    /// and kept apart so that gate 7 and `mission status` can say the HQ did
    /// not rule on it on this mission. Never carried by [`carry`]: the next
    /// campaign asks the registry again, against the line as it stands then.
    EquivalentRegistered {
        why: String,
        /// The mission the ruling was given on.
        mission: String,
        /// The commit of the campaign it was given on.
        commit: String,
    },
}

impl Triage {
    /// Whether the coder may write this outcome itself. The two that rest on
    /// a committed test, yes, and the proposal of the third; the judgement
    /// itself, no.
    pub fn is_the_coders_to_give(&self) -> bool {
        match self {
            Triage::Killed { .. } | Triage::Bug { .. } | Triage::EquivalentProposed { .. } => true,
            Triage::Equivalent { .. } | Triage::EquivalentRegistered { .. } => false,
        }
    }

    /// The name this outcome goes by in a refusal.
    pub fn kind(&self) -> &'static str {
        match self {
            Triage::Killed { .. } => "killed",
            Triage::Equivalent { .. } => "equivalent",
            Triage::Bug { .. } => "bug",
            Triage::EquivalentProposed { .. } => "equivalent_proposed",
            Triage::EquivalentRegistered { .. } => "equivalent_registered",
        }
    }

    /// Whether this is the HQ's `equivalent` ruling — given on this mission,
    /// or applied from the project's registry. What `--refuse` will not undo
    /// and `--lift` takes back.
    pub fn is_a_ruling(&self) -> bool {
        matches!(
            self,
            Triage::Equivalent { .. } | Triage::EquivalentRegistered { .. }
        )
    }

    /// The test this outcome rests on, when it rests on one.
    pub fn test(&self) -> Option<&str> {
        match self {
            Triage::Killed { test } | Triage::Bug { test } => Some(test),
            Triage::Equivalent { .. }
            | Triage::EquivalentProposed { .. }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tried: Option<u32>,
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
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| MutantsError::Unreadable(file, e.to_string()))
}

/// The coder's answers, by survivor id. Absent until it writes one, and an
/// unreadable one is an error rather than an empty triage: a file the coder
/// wrote and `nunki` cannot parse must be said, not silently ignored.
pub fn read_triage(dir: &Path) -> Result<BTreeMap<String, Triage>, MutantsError> {
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
/// The coder's two outcomes that rest on a test come first, then the HQ's
/// own ruling, then the coder's proposal — so a ratified proposal answers as
/// the HQ's `equivalent`, whatever the coder's file still says. A proposal is
/// an outcome only when it says why ([`crate::text::blank`] says it does
/// not) and the HQ has not refused it on this survivor. Anything else the
/// coder wrote — an `equivalent` of its own — is returned as it is, for
/// gate 7 to refuse by name.
pub fn answer(survivor: &Survivor, coders: &BTreeMap<String, Triage>) -> Option<Triage> {
    match coders.get(&survivor.id) {
        Some(Triage::EquivalentProposed { why }) => survivor.outcome.clone().or_else(|| {
            (!crate::text::blank(why) && survivor.refused.is_none())
                .then(|| Triage::EquivalentProposed { why: why.clone() })
        }),
        Some(other) => Some(other.clone()),
        None => survivor.outcome.clone(),
    }
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

/// An equivalence the coder proposed and the HQ has neither ratified nor
/// refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    pub id: String,
    pub file: String,
    pub line: u32,
    /// The coder's sentence.
    pub why: String,
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

/// How `mission status` and `mission wait` say that `count` proposals await
/// the HQ — nothing when none does.
pub fn proposals_await(count: usize) -> Option<String> {
    (count > 0).then(|| {
        format!(
            "{count} equivalence proposal(s) await the HQ's ruling, and `nunki push` \
             refuses until each is ratified or refused"
        )
    })
}

/// What `mission status` says of the survivors of `campaign` that the
/// project's registry of equivalences answered — nothing when none did. One
/// line naming how many, then one per survivor: its id, the sentence, and the
/// mission and commit the HQ ruled on. Listed because the HQ did not rule on
/// them on this mission, and `--lift` is how it takes one back.
pub fn registered(campaign: &Campaign) -> Option<String> {
    let lines: Vec<String> = campaign
        .survivors
        .iter()
        .filter_map(|s| match &s.outcome {
            Some(Triage::EquivalentRegistered {
                why,
                mission,
                commit,
            }) => Some(format!(
                "          {} — {} (ruled on mission {}, at {})",
                crate::text::one_line(&s.id),
                crate::text::one_line(why),
                crate::text::one_line(mission),
                crate::text::one_line(commit.get(..12).unwrap_or(commit)),
            )),
            _ => None,
        })
        .collect();
    (!lines.is_empty()).then(|| {
        format!(
            "{} equivalence(s) applied from the project's registry, ruled on another \
             mission — `nunki mission mutants <id> --lift <survivor>` takes one back\n{}",
            lines.len(),
            lines.join("\n")
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
        .filter_map(|s| match answer(s, &coders) {
            Some(Triage::EquivalentProposed { why }) => Some(Proposal {
                id: s.id.clone(),
                file: s.file.clone(),
                line: s.line,
                why,
            }),
            _ => None,
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
    record(dir, fingerprint, head, text, |_| {})
}

/// What [`record_finished_with_registry`] did with the project's registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    /// How many survivors the new campaign holds.
    pub survivors: usize,
    /// How many of them received a ruling from the registry.
    pub from_registry: usize,
    /// Why the registry could not be read, when it could not: nothing was
    /// applied from it, and `FOLLOWUP_HQ.md` says so.
    pub registry_unread: Option<String>,
}

/// [`record_finished`], and then the project's registry of equivalences
/// ([`crate::equivalences`]) applied to every survivor still without an
/// outcome, reading each survivor's line at `head` in the repository at
/// `tree` — the slot, which holds the campaign's commit.
///
/// After [`carry`], so that a ruling given on this mission keeps its own
/// origin. **Fails closed**: a registry that cannot be read applies nothing,
/// and the reason is appended to the mission's `FOLLOWUP_HQ.md`, the file the
/// HQ reads, rather than lost in a log.
pub fn record_finished_with_registry(
    dir: &Path,
    hq_root: &Path,
    tree: &Path,
    fingerprint: &str,
    head: &str,
    text: &str,
) -> Result<Recorded, MutantsError> {
    let registry = crate::equivalences::read(hq_root);
    let mut from_registry = 0;
    let survivors = record(dir, fingerprint, head, text, |survivors| {
        if let Ok(registry) = &registry {
            from_registry = crate::equivalences::apply(registry, survivors, |s| {
                crate::equivalences::digest_at(tree, head, &s.file, s.line)
            });
        }
    })?;
    let registry_unread = match registry {
        Ok(_) => None,
        Err(e) => {
            let why = e.to_string();
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

/// Parse, carry, let `then` add what it has, write.
fn record(
    dir: &Path,
    fingerprint: &str,
    head: &str,
    text: &str,
    then: impl FnOnce(&mut [Survivor]),
) -> Result<usize, MutantsError> {
    let mut survivors = parse(text);
    if let Some(previous) = read(dir)? {
        carry(&previous, &mut survivors);
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
            tried: tried(text),
        },
    )?;
    Ok(count)
}

/// Give each new survivor the `equivalent` ruling its twin held in the
/// previous campaign, when the twin can be told apart without a doubt — and
/// likewise the HQ's refusal of a proposal on it.
///
/// **The line is never part of the match**: it is exactly what a commit
/// above the mutant moves. The mutation itself — its file and what it
/// changed, the `description` — is what the ruling was about. Two tiers:
///
/// - the same id, file and description: the same mutant;
/// - otherwise the same file and description, when that pair names exactly
///   one ruled survivor before and exactly one survivor now. The same
///   `x = False -> x = None` twice in one file is two mutants the ruling may
///   not speak for alike, and it is ruled again rather than guessed.
///
/// A refusal is carried on the same two tiers, among the refused survivors:
/// without it, the same proposal written again on the same mutation after a
/// new campaign would count as an outcome again, though the HQ has said no
/// to it — and the prompt and the follow-up both tell the coder it does not
/// (HQ review of the pull request, item 1). A proposal is never carried.
///
/// A changed description — including a status that moved from `survived` to
/// `no tests` — is a different mutant, and nothing is carried. No id is
/// parsed: which tool named it is the stack's business, not `nunki`'s.
pub fn carry(previous: &Campaign, survivors: &mut [Survivor]) {
    let ruled: Vec<&Survivor> = previous
        .survivors
        .iter()
        .filter(|s| matches!(s.outcome, Some(Triage::Equivalent { .. })))
        .collect();
    let refused: Vec<&Survivor> = previous
        .survivors
        .iter()
        .filter(|s| s.refused.is_some())
        .collect();
    let from = |old: &Survivor| match &old.outcome {
        Some(Triage::Equivalent {
            carried_from: Some(first),
            ..
        }) => first.clone(),
        _ => previous.head.clone(),
    };
    let mut now: BTreeMap<(String, String), usize> = BTreeMap::new();
    for s in survivors.iter() {
        *now.entry(pair(s)).or_default() += 1;
    }
    for survivor in survivors.iter_mut() {
        if let Some(old) = twin(&ruled, survivor, &now)
            && let Some(Triage::Equivalent { why, .. }) = &old.outcome
        {
            survivor.outcome = Some(Triage::Equivalent {
                why: why.clone(),
                carried_from: Some(from(old)),
            });
        } else if let Some(old) = twin(&refused, survivor, &now) {
            survivor.refused = old.refused.clone();
        }
    }
}

/// What [`carry`] matches a mutation on when the id has changed: its file
/// and what it changed, never its line.
fn pair(s: &Survivor) -> (String, String) {
    (s.file.clone(), s.description.clone())
}

/// The survivor of `before` that `survivor` is, on [`carry`]'s two tiers:
/// the same id, file and description; or else the only one of `before` with
/// its file and description, when that pair also names one survivor `now`.
fn twin<'a>(
    before: &[&'a Survivor],
    survivor: &Survivor,
    now: &BTreeMap<(String, String), usize>,
) -> Option<&'a Survivor> {
    let exact = before.iter().find(|old| {
        old.id == survivor.id
            && old.file == survivor.file
            && old.description == survivor.description
    });
    exact.copied().or_else(|| {
        let same: Vec<&&Survivor> = before
            .iter()
            .filter(|old| pair(old) == pair(survivor))
            .collect();
        (same.len() == 1 && now.get(&pair(survivor)) == Some(&1)).then(|| *same[0])
    })
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

/// The coder's file, and its valid proposal on survivor `id` — or an error
/// that says why there is none to rule on.
fn proposal_on(dir: &Path, id: &str) -> Result<(BTreeMap<String, Triage>, String), MutantsError> {
    let coders = read_triage(dir)?;
    let why = match coders.get(id) {
        Some(Triage::EquivalentProposed { why }) if !crate::text::blank(why) => why.clone(),
        Some(Triage::EquivalentProposed { .. }) => {
            return Err(MutantsError::NoProposal(format!(
                "the coder's proposal on {id:?} gives no reason, so it is no proposal — \
                 `--equivalent {id} --because <why>` rules on the survivor yourself"
            )));
        }
        _ => {
            return Err(MutantsError::NoProposal(format!(
                "{TRIAGE_FILE} proposes no equivalence on {id:?}"
            )));
        }
    };
    Ok((coders, why))
}

/// The HQ ratifies the coder's proposal on survivor `id`: exactly the
/// `equivalent` [`rule_equivalent`] writes, with the coder's sentence as its
/// reason unless `because` replaces it. The proposal is taken out of the
/// coder's file, since it is now a ruling — and from here on it is the
/// ruling, never the proposal, that [`carry`] takes to the next campaign.
///
/// Returns the reason the ruling was written with.
pub fn ratify(dir: &Path, id: &str, because: Option<&str>) -> Result<String, MutantsError> {
    let (mut coders, proposed) = proposal_on(dir, id)?;
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
    coders.remove(id);
    write_triage(dir, &coders)?;
    Ok(why)
}

/// The HQ refuses the coder's proposal on survivor `id`, because of
/// `because`: the proposal is taken out of the coder's file, the refusal is
/// recorded on the survivor in [`FILE`], and the survivor is open again — a
/// proposal written again on it is no outcome ([`answer`]).
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
    let (mut coders, proposed) = proposal_on(dir, id)?;
    let mut campaign = read(dir)?.ok_or_else(|| {
        MutantsError::Unreadable(
            dir.join(FILE),
            "there is no campaign to rule on — `nunki mission mutants` runs one".to_string(),
        )
    })?;
    let refusal = Refusal {
        proposed,
        because: because.to_string(),
    };
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
    for found in found {
        found.refused = Some(refusal.clone());
    }
    write(dir, &campaign)?;
    coders.remove(id);
    write_triage(dir, &coders)?;
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
/// Only an `equivalent` is lifted — given here, or applied from the
/// project's registry: the coder's two outcomes live in its own file, and
/// this one never held them. Taking it out of the registry too is the
/// caller's ([`crate::equivalences::remove`]), from the campaign as it stood
/// before the lift.
pub fn lift_equivalent(dir: &Path, id: &str) -> Result<(), MutantsError> {
    let mut campaign = read(dir)?.ok_or_else(|| {
        MutantsError::Unreadable(
            dir.join(FILE),
            "there is no campaign to lift a ruling from".to_string(),
        )
    })?;
    let found = called(&mut campaign, dir, id)?;
    if !found
        .iter()
        .any(|s| s.outcome.as_ref().is_some_and(Triage::is_a_ruling))
    {
        return Err(MutantsError::Unreadable(
            dir.join(FILE),
            format!("{id:?} holds no `equivalent` ruling to lift"),
        ));
    }
    for survivor in found {
        if survivor.outcome.as_ref().is_some_and(Triage::is_a_ruling) {
            survivor.outcome = None;
        }
    }
    write(dir, &campaign)
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
    /// Launched just now.
    Started { fingerprint: String },
    /// Still going.
    Running { started_at: String, lines: usize },
    /// Ended, and `MUTANTS.json` is written.
    Finished { survivors: usize },
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
// Eight, and the eighth is [`Replay`]. Folding them into a struct would be a
// second change riding on this one; `run::plan` carries the same allow for
// the same reason.
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
                None => Progress::Finished {
                    survivors: record_finished_with_registry(
                        dir,
                        &project.hq_root,
                        &slot.tree,
                        &running.fingerprint,
                        &running.head,
                        &text,
                    )?
                    .survivors,
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

#[allow(clippy::too_many_arguments)]
pub fn campaign(
    project: &crate::project::Project,
    slot: &crate::slot::Slot,
    engine: std::sync::Arc<dyn crate::engine::Engine>,
    dir: &Path,
    stack: &str,
    base: &str,
    deadline_minutes: u32,
    replay: Replay,
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
    let touched = crate::gate::touched_since_base(&slot.tree, base)
        .map_err(|e| MutantsError::Launch(e.to_string()))?;
    // The fingerprint stays over the touched files' content, not the fork
    // point: a base that moves without changing a touched file leaves the
    // diff of those files unchanged, and one that does change them changes
    // their content after the rebase.
    let fork = crate::gate::fork_point(&slot.tree, base)
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
    if let Some(fresh) = already_answered(dir, &want, replay)? {
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
        .spawn(&command(&judged, &want, &touched, &fork), &log)
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
            deadline_minutes,
        },
    )?;
    Ok(Progress::Started { fingerprint: want })
}

/// What a campaign runs in the container: each stack's `mutation.sh` on the
/// touched paths, from the clean copy, with the fork point in [`BASE_ENV`].
///
/// `campaign` is the fingerprint, `fork` the commit the branch forked from.
/// One stack runs its script directly, as it always has; several run through
/// [`several_campaigns`], whose child scripts inherit the same environment.
pub fn command(
    judged: &[crate::run::Judged],
    campaign: &str,
    touched: &[String],
    fork: &str,
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
        env: [(BASE_ENV.to_string(), fork.to_string())]
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
        "set -u\ncomplete=1\ncounted=1\ntried=0\nsized=1\nfound=0\nout=\"$(mktemp)\"\n",
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
