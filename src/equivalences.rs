//! The project's registry of equivalences (SPEC 4.4, gate 7).
//!
//! A ruling that a surviving mutant is equivalent lives in the mission's
//! `MUTANTS.json`, and [`crate::mutants::carry`] takes it from one campaign to
//! the next **inside that mission**. The next mission that touches the same
//! file meets the same mutant on the same unchanged line, and without this
//! registry the HQ is asked the same question again.
//!
//! **The HQ's file.** It lives under the project's `hq/`, which no agent
//! container mounts, and only the HQ's verbs write it: `--equivalent` and
//! `--ratify` enter a ruling, `--lift` takes one out. A coder's proposal never
//! enters it, and a refusal is not an equivalence.
//!
//! **A ruling is about a line's content.** Each entry carries the mutant's
//! file and description — what the mutation changes, never its line number,
//! which the smallest commit above it moves — and a digest of the content of
//! the source line it sat on, whitespace at both ends ignored. A later
//! campaign applies it while that line is unchanged, wherever it moved to,
//! and stops the moment its content changes: the ruling was about other code.
//!
//! **Fail closed.** A registry that cannot be read applies nothing and says
//! so; a source line that cannot be read — file gone, line out of range —
//! matches nothing; two survivors of a campaign, or two entries, sharing a
//! file and description match nothing either.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::mutants::{Campaign, Survivor, Triage};

/// The registry, under the project's `hq/`.
pub const FILE: &str = "equivalences.json";

/// The lock the registry's writers take, beside the slots' locks. Two HQ
/// verbs on two missions hold two different slot locks, and both may write
/// this one file. A leading dot, so that no slot can be called the same.
const LOCK: &str = ".equivalences";

/// Where a project's registry lives.
pub fn path(hq_root: &Path) -> PathBuf {
    hq_root.join(FILE)
}

/// One ruling, as the registry keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// The mutant's file, relative to the repository.
    pub file: String,
    /// What the mutation changes, in the tool's words. With the file, what
    /// the ruling is about.
    pub description: String,
    /// [`line_digest`] of the source line the mutant sat on, at [`Self::commit`].
    pub line: String,
    /// The HQ's sentence.
    pub why: String,
    /// Who ruled.
    pub by: String,
    /// The mission it was ruled on.
    pub mission: String,
    /// The commit of the campaign it was ruled on.
    pub commit: String,
    pub date: String,
}

/// `equivalences.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default)]
    pub entries: Vec<Entry>,
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error(
        "{0} is not a registry of equivalences `nunki` can read: {1} — no ruling is applied \
         from it until it is repaired or removed"
    )]
    Unreadable(PathBuf, String),
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error(transparent)]
    Lock(#[from] crate::state::LockError),
}

/// Read the registry. No file, or an empty one, is an empty registry — a
/// project that never ruled behaves exactly as before the registry existed.
/// One that cannot be parsed is an error, never an empty registry: a file
/// the HQ wrote and `nunki` cannot read must be said, and writing over it
/// would lose every ruling it holds.
pub fn read(hq_root: &Path) -> Result<Registry, RegistryError> {
    let file = path(hq_root);
    let text = match std::fs::read_to_string(&file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Registry::default()),
        Err(e) => return Err(RegistryError::Unreadable(file, e.to_string())),
    };
    if text.trim().is_empty() {
        return Ok(Registry::default());
    }
    serde_json::from_str(&text).map_err(|e| RegistryError::Unreadable(file, e.to_string()))
}

/// Write the registry whole, through a file renamed into place: a reader —
/// a campaign being recorded on another mission — sees the old registry or
/// the new one, never half of one.
pub fn write(hq_root: &Path, registry: &Registry) -> Result<(), RegistryError> {
    std::fs::create_dir_all(hq_root).map_err(|e| RegistryError::Io(hq_root.to_path_buf(), e))?;
    let file = path(hq_root);
    let staged = hq_root.join(format!("{FILE}.tmp"));
    let body = serde_json::to_string_pretty(registry).expect("a registry serialises");
    std::fs::write(&staged, format!("{body}\n"))
        .map_err(|e| RegistryError::Io(staged.clone(), e))?;
    std::fs::rename(&staged, &file).map_err(|e| RegistryError::Io(file, e))
}

/// The digest of a source line's content: git's blob id of the line with
/// whitespace at both ends taken off — the digest the campaign fingerprint
/// uses ([`crate::mutants::fingerprint`]), so a human can reproduce it with
/// `printf '%s' '<line>' | git hash-object --stdin`.
///
/// Worked out here rather than by spawning git once per survivor, and the
/// same whatever hash a repository's objects use: an entry must not change
/// meaning because one repository moved to SHA-256.
pub fn line_digest(line: &str) -> String {
    let content = line.trim();
    let mut digest = ring::digest::Context::new(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY);
    digest.update(format!("blob {}\0", content.len()).as_bytes());
    digest.update(content.as_bytes());
    digest
        .finish()
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The content of `file` at `commit` in the repository at `tree`, exactly as
/// committed, or `None` when it cannot be read: a commit the repository does
/// not hold, a file that is not there, bytes that are not text.
pub fn source_at(tree: &Path, commit: &str, file: &str) -> Option<String> {
    let out = Command::new("git")
        .env("LC_ALL", "C")
        .arg("-C")
        .arg(tree)
        .args(["cat-file", "blob", &format!("{commit}:{file}")])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

/// The 1-based `line` of `source`, if it has one.
pub fn nth_line(source: &str, line: u32) -> Option<&str> {
    let index = usize::try_from(line).ok()?.checked_sub(1)?;
    source.lines().nth(index)
}

/// [`line_digest`] of `file`'s `line` at `commit`, or `None` when that line
/// cannot be read — and `None` matches nothing.
pub fn digest_at(tree: &Path, commit: &str, file: &str, line: u32) -> Option<String> {
    let source = source_at(tree, commit, file)?;
    nth_line(&source, line).map(line_digest)
}

/// What [`apply`] matches on: the mutant's file and what it changes.
fn pair(file: &str, description: &str) -> (String, String) {
    (file.to_string(), description.to_string())
}

/// Give each survivor still without an outcome the registry's ruling on it,
/// when there is exactly one and it is about the code the survivor sits on:
/// the same file and description, and the same [`line_digest`] for the line
/// it sits on now, as `digest` reads it. Returns how many were given one.
///
/// Nothing is applied to a survivor when its file and description name two
/// survivors of the campaign, or two entries: which ruling speaks for which
/// is then a guess. Nor to a survivor carrying the HQ's refusal of a
/// proposal on this mission — the HQ said no to it here — nor to one whose
/// line `digest` cannot read.
pub fn apply(
    registry: &Registry,
    survivors: &mut [Survivor],
    digest: impl Fn(&Survivor) -> Option<String>,
) -> usize {
    let mut now: BTreeMap<(String, String), usize> = BTreeMap::new();
    for s in survivors.iter() {
        *now.entry(pair(&s.file, &s.description)).or_default() += 1;
    }
    let mut entries: BTreeMap<(String, String), Vec<&Entry>> = BTreeMap::new();
    for e in &registry.entries {
        entries
            .entry(pair(&e.file, &e.description))
            .or_default()
            .push(e);
    }
    let mut applied = 0;
    for survivor in survivors.iter_mut() {
        if survivor.outcome.is_some() || survivor.refused.is_some() {
            continue;
        }
        let key = pair(&survivor.file, &survivor.description);
        if now.get(&key) != Some(&1) {
            continue;
        }
        let Some([entry]) = entries.get(&key).map(Vec::as_slice) else {
            continue;
        };
        if digest(survivor).as_deref() != Some(entry.line.as_str()) {
            continue;
        }
        survivor.outcome = Some(Triage::EquivalentRegistered {
            why: entry.why.clone(),
            mission: entry.mission.clone(),
            commit: entry.commit.clone(),
        });
        applied += 1;
    }
    applied
}

/// What became of the registry when the HQ ruled or lifted on a mission.
///
/// The mission's own file is written either way: the ruling stands on the
/// mission, and this says whether it also stands for the next one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Registered {
    /// The registry now says so — `n` entries entered or taken out.
    Done(usize),
    /// The registry was left as it was, and why.
    Not(String),
}

impl Registered {
    /// The sentence a verb prints when the registry was left as it was.
    pub fn warning(&self) -> Option<String> {
        match self {
            Registered::Done(_) => None,
            Registered::Not(why) => Some(why.clone()),
        }
    }
}

/// Take the registry's lock, read it, change it with `change`, and write it
/// back when `change` changed something. Any failure becomes
/// [`Registered::Not`], and an unreadable registry is never written over.
fn under_lock(
    hq_root: &Path,
    verb: &str,
    change: impl FnOnce(&mut Registry) -> Result<usize, String>,
) -> Registered {
    let locks = hq_root.join("locks");
    if let Err(e) = std::fs::create_dir_all(&locks) {
        return Registered::Not(format!("{}: {e}", locks.display()));
    }
    let _lock = match crate::state::SlotLock::acquire(&locks, LOCK, verb) {
        Ok(lock) => lock,
        Err(e) => return Registered::Not(e.to_string()),
    };
    let mut registry = match read(hq_root) {
        Ok(r) => r,
        Err(e) => return Registered::Not(e.to_string()),
    };
    let changed = match change(&mut registry) {
        Ok(n) => n,
        Err(why) => return Registered::Not(why),
    };
    if changed == 0 {
        return Registered::Done(0);
    }
    match write(hq_root, &registry) {
        Ok(()) => Registered::Done(changed),
        Err(e) => Registered::Not(e.to_string()),
    }
}

/// Who ruled, on which mission, and in which repository the campaign's
/// commit can be read — what [`enter`] and [`remove`] need beyond the
/// campaign.
#[derive(Debug, Clone)]
pub struct Ruling<'a> {
    pub hq_root: &'a Path,
    /// A repository holding the campaign's commit: the mission's slot.
    pub tree: &'a Path,
    pub mission: &'a str,
    pub by: &'a str,
}

/// Enter the HQ's ruling on survivor `id` of `campaign` — the one it has
/// just written in the mission's file — in the registry.
///
/// An entry already there on the same file and description is replaced:
/// it is a ruling on the same mutation, given again, and two entries on one
/// mutation would make every later match ambiguous, so the mutation would
/// be asked again forever. A survivor whose line cannot be read at the
/// campaign's commit is not entered, and the answer says so.
pub fn enter(ruling: &Ruling, campaign: &Campaign, id: &str) -> Registered {
    let mut fresh = Vec::new();
    for s in campaign.survivors.iter().filter(|s| s.id == id) {
        let why = match &s.outcome {
            Some(Triage::Equivalent { why, .. }) => why.clone(),
            _ => continue,
        };
        let Some(line) = digest_at(ruling.tree, &campaign.head, &s.file, s.line) else {
            return Registered::Not(format!(
                "{}:{} cannot be read at {} in {}, so the ruling holds on this mission only",
                s.file,
                s.line,
                short(&campaign.head),
                ruling.tree.display()
            ));
        };
        fresh.push(Entry {
            file: s.file.clone(),
            description: s.description.clone(),
            line,
            why,
            by: ruling.by.to_string(),
            mission: ruling.mission.to_string(),
            commit: campaign.head.clone(),
            date: crate::state::now_rfc3339(),
        });
    }
    if fresh.is_empty() {
        return Registered::Done(0);
    }
    under_lock(ruling.hq_root, "mission mutants (registry)", |registry| {
        registry.entries.retain(|e| {
            !fresh
                .iter()
                .any(|f| f.file == e.file && f.description == e.description)
        });
        let n = fresh.len();
        registry.entries.extend(fresh);
        Ok(n)
    })
}

/// Where the ruling a survivor holds was first given: the mission and the
/// campaign commit. `None` for a survivor holding no ruling.
fn origin<'a>(
    survivor: &'a Survivor,
    campaign: &'a Campaign,
    mission: &'a str,
) -> Option<(&'a str, &'a str)> {
    match &survivor.outcome {
        Some(Triage::EquivalentRegistered {
            mission, commit, ..
        }) => Some((mission, commit)),
        Some(Triage::Equivalent { carried_from, .. }) => {
            Some((mission, carried_from.as_deref().unwrap_or(&campaign.head)))
        }
        _ => None,
    }
}

/// Take out of the registry the rulings survivor `id` of `campaign` holds,
/// read **before** `--lift` took them off the mission's file: each entry on
/// the same file and description that was given where the survivor's ruling
/// was (the same mission and commit), or that is about the line the survivor
/// sits on now.
pub fn remove(ruling: &Ruling, campaign: &Campaign, id: &str) -> Registered {
    let held: Vec<(&Survivor, (&str, &str))> = campaign
        .survivors
        .iter()
        .filter(|s| s.id == id)
        .filter_map(|s| origin(s, campaign, ruling.mission).map(|o| (s, o)))
        .collect();
    if held.is_empty() {
        return Registered::Done(0);
    }
    let lines: Vec<Option<String>> = held
        .iter()
        .map(|(s, _)| digest_at(ruling.tree, &campaign.head, &s.file, s.line))
        .collect();
    under_lock(
        ruling.hq_root,
        "mission mutants --lift (registry)",
        |registry| {
            let before = registry.entries.len();
            registry.entries.retain(|e| {
                !held
                    .iter()
                    .zip(&lines)
                    .any(|((s, (mission, commit)), line)| {
                        e.file == s.file
                            && e.description == s.description
                            && ((e.mission == *mission && e.commit == *commit)
                                || line.as_deref() == Some(e.line.as_str()))
                    })
            });
            Ok(before - registry.entries.len())
        },
    )
}

/// Whether an entry's line still stands in a repository's `HEAD`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Standing {
    /// A line of the file at `HEAD` has the entry's digest.
    Stands,
    /// The file is there, and no line of it has the entry's digest.
    Changed,
    /// The file cannot be read at `HEAD`.
    Gone,
}

impl Standing {
    pub fn said(&self) -> &'static str {
        match self {
            Standing::Stands => "its line stands at HEAD",
            Standing::Changed => "its line has changed at HEAD: it applies to nothing there",
            Standing::Gone => "its file cannot be read at HEAD: it applies to nothing there",
        }
    }
}

/// Where `entry`'s line is in the repository at `tree`, at `HEAD`. Any line
/// of the file counts, since a line that moved still matches.
pub fn standing(tree: &Path, entry: &Entry) -> Standing {
    match source_at(tree, "HEAD", &entry.file) {
        None => Standing::Gone,
        Some(source) if source.lines().any(|l| line_digest(l) == entry.line) => Standing::Stands,
        Some(_) => Standing::Changed,
    }
}

/// `nunki mission mutants --registry`: one paragraph per entry — file,
/// mutation, sentence, mission, date, and whether its line still stands at
/// `HEAD` of the repository at `tree`.
pub fn listing(registry: &Registry, tree: &Path) -> String {
    if registry.entries.is_empty() {
        return "the registry of equivalences holds no ruling".to_string();
    }
    let one = crate::text::one_line;
    registry
        .entries
        .iter()
        .map(|e| {
            format!(
                "{file} — {description}\n  {why}\n  ruled by {by} on mission {mission} at {commit}, \
                 {date}; {standing}\n",
                file = one(&e.file),
                description = one(&e.description),
                why = one(&e.why),
                by = one(&e.by),
                mission = one(&e.mission),
                commit = short(&e.commit),
                date = one(&e.date),
                standing = standing(tree, e).said(),
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The first twelve characters of a commit, as every other message names
/// one — and the whole of a hand-edited value that is not one.
fn short(commit: &str) -> &str {
    commit.get(..12).unwrap_or(commit)
}
