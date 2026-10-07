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
//! **A ruling is about the code the mutation replaces.** Each entry carries
//! the mutant's file and description — what the mutation changes, never its
//! line number, which the smallest commit above it moves — and a digest of
//! every line of the code it replaced, its whole span as the tool lists it,
//! whitespace at both ends of each line ignored. A later campaign matches it
//! while that code is unchanged, wherever it moved to, and stops the moment
//! any line of it changes: the ruling was about other code.
//!
//! **The registry proposes, it never rules** (HQ review 2). Which mutant a
//! ruling was about is a heuristic across campaigns, and three rounds of
//! review each found a narrower way for one to land on a mutant the HQ never
//! ruled on. A match therefore gives the survivor a proposal — the HQ's own
//! sentence, from the mission and commit it was given on — which gate 7
//! counts as a proposal and `nunki push` waits on until the HQ ratifies or
//! refuses it. A wrong match costs one refusal, never a wrong ruling.
//!
//! **Only what is identified without a doubt** (HQ review of the pull
//! request, the class rule). A ruling is entered, and later matched, only
//! when the span's text occurs exactly once in its file, and only when the
//! campaign it was given on holds exactly one survivor on its file and
//! description. The same code twice in one file, the same mutation twice in
//! one campaign, a span the tool did not give: the HQ's ruling stands on its
//! mission, and the registry says nothing about it.
//!
//! **Fail closed.** A registry that cannot be read proposes nothing and says
//! so; a span that cannot be read — file gone, lines out of range — matches
//! nothing; two survivors of a campaign, or two entries, sharing a file and
//! description match nothing either.

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
    /// [`span_digest`] of the code the mutation replaced, at
    /// [`Self::commit`]: every line of its span. The key keeps the name it
    /// had when the digest covered one line, which a span of one line still
    /// gives the same value for.
    pub line: String,
    /// How many lines that span has: one, for an entry written before spans
    /// were carried.
    #[serde(default = "one_line")]
    pub lines: u32,
    /// The HQ's sentence.
    pub why: String,
    /// Who ruled.
    pub by: String,
    /// The mission it was ruled on.
    pub mission: String,
    /// The commit of the campaign it was ruled on.
    pub commit: String,
    pub date: String,
    /// Each time the HQ ratified this entry's proposal on a later mission,
    /// recorded beside the origin above, which stays the ruling's (HQ review
    /// 3, item 5).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ratified: Vec<Ratification>,
}

/// The HQ ratified a registry entry's proposal on another mission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ratification {
    pub by: String,
    pub mission: String,
    /// The commit of the campaign it was ratified on.
    pub commit: String,
    pub date: String,
}

fn one_line() -> u32 {
    1
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
        "{0} is not a registry of equivalences `nunki` can read: {1} — nothing is proposed \
         from it until it is repaired or removed"
    )]
    Unreadable(PathBuf, String),
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error(transparent)]
    Lock(#[from] crate::state::LockError),
}

/// Read the registry. No file is an empty registry — a project that never
/// ruled behaves exactly as before the registry existed. Anything else that
/// does not parse is an error, never an empty registry — a zero-length file
/// included, which is what a write cut short by a crash leaves (HQ review,
/// item 5): a file the HQ wrote and `nunki` cannot read must be said, and
/// writing over it would lose every ruling it held.
pub fn read(hq_root: &Path) -> Result<Registry, RegistryError> {
    let file = path(hq_root);
    let text = match std::fs::read_to_string(&file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Registry::default()),
        Err(e) => return Err(RegistryError::Unreadable(file, e.to_string())),
    };
    serde_json::from_str(&text).map_err(|e| RegistryError::Unreadable(file, e.to_string()))
}

/// [`read`], under the registry's lock, the lock its writers take: a
/// campaign recorded while an HQ verb rewrites the registry reads it before
/// or after, never between (HQ review, item 6). Why it could not be read,
/// when it could not.
pub fn read_locked(hq_root: &Path) -> Result<Registry, String> {
    let _lock = lock(hq_root, "mission mutants (registry read)")?;
    read(hq_root).map_err(|e| e.to_string())
}

/// Write the registry whole, through a file renamed into place: a reader —
/// a campaign being recorded on another mission — sees the old registry or
/// the new one, never half of one. The staged file and its directory are
/// synced to disk before the rename, and the directory again after it, so
/// that a crash cannot leave the new name on a file whose bytes never
/// reached the disk (HQ review, item 5).
pub fn write(hq_root: &Path, registry: &Registry) -> Result<(), RegistryError> {
    use std::io::Write as _;
    std::fs::create_dir_all(hq_root).map_err(|e| RegistryError::Io(hq_root.to_path_buf(), e))?;
    let file = path(hq_root);
    let staged = hq_root.join(format!("{FILE}.tmp"));
    let body = serde_json::to_string_pretty(registry).expect("a registry serialises");
    let io = |path: &Path| {
        let path = path.to_path_buf();
        move |e| RegistryError::Io(path, e)
    };
    let mut out = std::fs::File::create(&staged).map_err(io(&staged))?;
    out.write_all(format!("{body}\n").as_bytes())
        .map_err(io(&staged))?;
    out.sync_all().map_err(io(&staged))?;
    let dir = std::fs::File::open(hq_root).map_err(io(hq_root))?;
    dir.sync_all().map_err(io(hq_root))?;
    std::fs::rename(&staged, &file).map_err(io(&file))?;
    dir.sync_all().map_err(io(hq_root))
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
    span_digest(&[line])
}

/// The digest of a span of source lines: [`line_digest`]'s, over the lines
/// each trimmed at both ends and joined by a newline. One line gives exactly
/// its [`line_digest`].
pub fn span_digest(lines: &[&str]) -> String {
    let content = lines
        .iter()
        .map(|l| l.trim())
        .collect::<Vec<_>>()
        .join("\n");
    let mut digest = ring::digest::Context::new(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY);
    digest.update(format!("blob {}\0", content.len()).as_bytes());
    digest.update(content.as_bytes());
    hex(digest.finish().as_ref())
}

/// The content of `file` at `commit` in the repository at `tree`, exactly as
/// committed, or `None` when it cannot be read: a commit the repository does
/// not hold, a file that is not there, bytes that are not text — or an
/// object that is not what its id says.
///
/// **The repository is not trusted.** It is the slot's, and the coder's
/// container writes its `.git`: a replace ref (`refs/replace/`) or a graft
/// would make git hand back the ruled content for a line that has changed,
/// and gate 7 would pass on a ruling the HQ never gave on this code
/// (security round 1, MEDIUM). So replace objects and grafts are switched off
/// for every read, and every object on the way — the commit, each tree
/// down the path, the blob — is hashed here and compared with the id it was
/// asked by. A loose object rewritten on disk reads as nothing, not as what
/// it was made to say. `commit` is resolved under the same switches: the
/// campaign's full id comes back as itself, and `HEAD` — for `--registry`,
/// in the human's repository — as the commit it names.
pub fn source_at(tree: &Path, commit: &str, file: &str) -> Option<String> {
    let commit = resolved(tree, commit)?;
    let body = object(tree, "commit", &commit)?;
    let mut at = String::from_utf8(body)
        .ok()?
        .lines()
        .next()?
        .strip_prefix("tree ")?
        .to_string();
    let parts: Vec<&str> = file.split('/').collect();
    for (i, name) in parts.iter().enumerate() {
        let listing = object(tree, "tree", &at)?;
        let (mode, oid) = entry(&listing, name, at.len() / 2)?;
        let last = i + 1 == parts.len();
        match (last, mode.as_str()) {
            (false, "40000") => at = oid,
            (true, "100644" | "100755") => {
                return String::from_utf8(object(tree, "blob", &oid)?).ok();
            }
            _ => return None,
        }
    }
    None
}

/// `git` in `tree`, with replace objects and grafts switched off.
fn git_untrusted(tree: &Path) -> Command {
    let mut git = Command::new("git");
    git.env("LC_ALL", "C")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_GRAFT_FILE", "/dev/null")
        .arg("-C")
        .arg(tree);
    git
}

/// `name` as a full commit id, resolved with replace objects off: a full id
/// comes back as itself, `HEAD` as the commit it names.
fn resolved(tree: &Path, name: &str) -> Option<String> {
    let out = git_untrusted(tree)
        .args([
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &format!("{name}^{{commit}}"),
        ])
        .output()
        .ok()?;
    let oid = String::from_utf8(out.stdout).ok()?.trim().to_string();
    out.status.success().then_some(oid)
}

/// The content of object `oid` of type `kind`, only when it hashes to `oid`
/// — which nothing but a full id of that very content can.
fn object(tree: &Path, kind: &str, oid: &str) -> Option<Vec<u8>> {
    let out = git_untrusted(tree)
        .args(["cat-file", kind, oid])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    (object_id(kind, &out.stdout, oid.len()) == oid).then_some(out.stdout)
}

/// Git's id of an object of type `kind` holding `content`, in the hash the
/// repository uses — SHA-1 for a 40-digit id, SHA-256 for a 64-digit one.
fn object_id(kind: &str, content: &[u8], digits: usize) -> String {
    let algorithm = if digits == 64 {
        &ring::digest::SHA256
    } else {
        &ring::digest::SHA1_FOR_LEGACY_USE_ONLY
    };
    let mut digest = ring::digest::Context::new(algorithm);
    digest.update(format!("{kind} {}\0", content.len()).as_bytes());
    digest.update(content);
    hex(digest.finish().as_ref())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The mode and id of `name` in a raw tree object: entries are
/// `<mode> <name>\0<id>`, the id `width` raw bytes.
///
/// The whole tree is read first, and a tree git itself would never write is
/// refused: one that names an entry twice, or whose entries are not strictly
/// in git's order (names compared as bytes, a directory's as if it ended in
/// `/`). A forged commit can list `lib.rs` twice — `git show` reads the
/// first, a checkout writes the last — and which of the two the campaign ran
/// on is then a guess (HQ review, item 2).
fn entry(listing: &[u8], name: &str, width: usize) -> Option<(String, String)> {
    let mut entries: Vec<(&[u8], &[u8], &[u8])> = Vec::new();
    let mut rest = listing;
    while !rest.is_empty() {
        let space = rest.iter().position(|&b| b == b' ')?;
        let nul = space + rest[space..].iter().position(|&b| b == 0)?;
        let end = nul + 1 + width;
        let id = rest.get(nul + 1..end)?;
        entries.push((&rest[..space], &rest[space + 1..nul], id));
        rest = &rest[end..];
    }
    let key = |(mode, name, _): &(&[u8], &[u8], &[u8])| {
        let mut key = name.to_vec();
        if *mode == b"40000" {
            key.push(b'/');
        }
        key
    };
    let names: std::collections::BTreeSet<&[u8]> = entries.iter().map(|e| e.1).collect();
    if names.len() != entries.len() || entries.windows(2).any(|w| key(&w[0]) >= key(&w[1])) {
        return None;
    }
    let (mode, _, id) = entries.iter().find(|e| e.1 == name.as_bytes())?;
    Some((std::str::from_utf8(mode).ok()?.to_string(), hex(id)))
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

/// The lines `start` to `end` of `source`, 1-based and inclusive, when the
/// file has them all.
fn span_of(source: &str, start: u32, end: u32) -> Option<Vec<&str>> {
    let lines: Vec<&str> = source.lines().collect();
    let from = usize::try_from(start).ok()?.checked_sub(1)?;
    let to = usize::try_from(end).ok()?;
    (from < to && to <= lines.len()).then(|| lines[from..to].to_vec())
}

/// How many times `span` occurs in `source`, line for line, each line
/// trimmed at both ends.
fn occurrences(source: &str, span: &[&str]) -> usize {
    let lines: Vec<&str> = source.lines().collect();
    lines
        .windows(span.len())
        .filter(|w| w.iter().zip(span).all(|(a, b)| a.trim() == b.trim()))
        .count()
}

/// [`span_digest`] of the code `survivor`'s mutation replaces, at `commit`
/// in the repository at `tree` — **only when it is identified without a
/// doubt**: its span is known, every line of it can be read, and that text
/// occurs exactly once in the file. Otherwise why not, in a sentence.
pub fn identify(tree: &Path, commit: &str, survivor: &Survivor) -> Result<String, String> {
    let at = format!("{}:{}", survivor.file, survivor.line);
    let (start, end) = survivor.span().ok_or_else(|| {
        format!("{at}: the campaign does not say which lines the mutation replaces")
    })?;
    let source = source_at(tree, commit, &survivor.file).ok_or_else(|| {
        format!(
            "{} cannot be read at {} in {}",
            survivor.file,
            short(commit),
            tree.display()
        )
    })?;
    identified(&source, start, end).map_err(|why| format!("{at}: {why}"))
}

/// [`span_digest`] of lines `start` to `end` of `source`, when that text
/// occurs exactly once in it.
pub fn identified(source: &str, start: u32, end: u32) -> Result<String, String> {
    let span = span_of(source, start, end)
        .ok_or_else(|| format!("lines {start} to {end} are not all in the file"))?;
    match occurrences(source, &span) {
        1 => Ok(span_digest(&span)),
        n => Err(format!(
            "the code the mutation replaces occurs {n} times in the file, so which one \
             was ruled on would be a guess"
        )),
    }
}

/// [`span_digest`] of the code `survivor`'s mutation replaces at `commit`,
/// whether or not it is unique — what a lift looks for, where taking out too
/// much is the safe side.
fn span_at(tree: &Path, commit: &str, survivor: &Survivor) -> Option<String> {
    let (start, end) = survivor.span()?;
    let source = source_at(tree, commit, &survivor.file)?;
    span_of(&source, start, end).map(|span| span_digest(&span))
}

/// What [`apply`] matches on: the mutant's file and what it changes.
fn pair(file: &str, description: &str) -> (String, String) {
    (file.to_string(), description.to_string())
}

/// Give each survivor still without an outcome a **proposal** of the
/// registry's ruling on it — never the ruling (HQ review 2): `nunki push`
/// waits for the HQ to ratify or refuse it. Only when there is exactly one
/// entry and it is about the code the survivor sits on:
/// the same file and description, and the same [`span_digest`] for the code
/// its mutation replaces now, as `digest` reads it — [`identify`], in a
/// campaign, which reads nothing for code that occurs more than once.
/// Returns how many were given one.
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
        survivor.outcome = Some(Triage::ProposedByNunki {
            why: entry.why.clone(),
            from: crate::mutants::ProposedFrom::Registry {
                mission: entry.mission.clone(),
                commit: entry.commit.clone(),
                date: entry.date.clone(),
            },
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

/// How many times the registry's lock is asked for before it is refused,
/// [`LOCK_WAIT`] apart: two seconds, where a ruling takes milliseconds.
const LOCK_TRIES: usize = 20;
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_millis(100);

/// The registry's lock. Asked again while another verb holds it, and then
/// refused, with why.
fn lock(hq_root: &Path, verb: &str) -> Result<crate::state::SlotLock, String> {
    let locks = hq_root.join("locks");
    std::fs::create_dir_all(&locks).map_err(|e| format!("{}: {e}", locks.display()))?;
    for _ in 0..LOCK_TRIES {
        if let Ok(lock) = crate::state::SlotLock::acquire(&locks, LOCK, verb) {
            return Ok(lock);
        }
        std::thread::sleep(LOCK_WAIT);
    }
    crate::state::SlotLock::acquire(&locks, LOCK, verb).map_err(|e| e.to_string())
}

/// Take the registry's lock, read it, change it with `change`, and write it
/// back when `change` changed something. Any failure becomes
/// [`Registered::Not`], and an unreadable registry is never written over.
fn under_lock(
    hq_root: &Path,
    verb: &str,
    change: impl FnOnce(&mut Registry) -> Result<usize, String>,
) -> Registered {
    let _lock = match lock(hq_root, verb) {
        Ok(lock) => lock,
        Err(why) => return Registered::Not(why),
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
/// be asked again forever.
///
/// Nothing is entered — the ruling holds on this mission only, and the
/// answer says why — unless the survivor is identified without a doubt: the
/// campaign holds no other survivor on its file and description, and
/// [`identify`] reads the code its mutation replaces, once, at the
/// campaign's commit.
pub fn enter(ruling: &Ruling, campaign: &Campaign, id: &str) -> Registered {
    let mut fresh = Vec::new();
    for s in campaign.survivors.iter().filter(|s| s.id == id) {
        let why = match &s.outcome {
            Some(Triage::Equivalent { why, .. }) => why.clone(),
            _ => continue,
        };
        let only = |why: String| {
            Registered::Not(format!("{why}, so the ruling holds on this mission only"))
        };
        let twins = campaign
            .survivors
            .iter()
            .filter(|t| pair(&t.file, &t.description) == pair(&s.file, &s.description))
            .count();
        if twins != 1 {
            return only(format!(
                "the campaign holds {twins} survivors on {} with this mutation, and the \
                 registry cannot tell them apart",
                s.file
            ));
        }
        let line = match identify(ruling.tree, &campaign.head, s) {
            Ok(digest) => digest,
            Err(why) => return only(why),
        };
        let (start, end) = s.span().expect("identify read the span");
        fresh.push(Entry {
            file: s.file.clone(),
            description: s.description.clone(),
            line,
            lines: end - start + 1,
            why,
            by: ruling.by.to_string(),
            mission: ruling.mission.to_string(),
            commit: campaign.head.clone(),
            date: crate::state::now_rfc3339(),
            ratified: Vec::new(),
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

/// The HQ ratified, on survivor `id` of `campaign`, the proposal the
/// registry made from the entry given on `origin`'s mission and commit:
/// that entry keeps its origin — mission, commit, date, sentence — and the
/// ratification is recorded beside it (HQ review 3, item 5). When the entry
/// is not there any more — lifted or replaced since — the ruling is entered
/// as any other ([`enter`]).
pub fn ratified(
    ruling: &Ruling,
    campaign: &Campaign,
    id: &str,
    origin: (&str, &str),
) -> Registered {
    let Some(survivor) = campaign.survivors.iter().find(|s| s.id == id) else {
        return Registered::Done(0);
    };
    let (mission, commit) = origin;
    let ratification = Ratification {
        by: ruling.by.to_string(),
        mission: ruling.mission.to_string(),
        commit: campaign.head.clone(),
        date: crate::state::now_rfc3339(),
    };
    let recorded = under_lock(
        ruling.hq_root,
        "mission mutants --ratify (registry)",
        |registry| {
            let mut n = 0;
            for e in registry.entries.iter_mut().filter(|e| {
                e.file == survivor.file
                    && e.description == survivor.description
                    && e.mission == mission
                    && e.commit == commit
            }) {
                e.ratified.push(ratification.clone());
                n += 1;
            }
            Ok(n)
        },
    );
    match recorded {
        Registered::Done(0) => enter(ruling, campaign, id),
        other => other,
    }
}

/// Where the ruling a survivor holds, or that `nunki` proposed from on this
/// mission, was first given: the mission and the campaign commit. `None`
/// otherwise — a proposal from the registry included: it was given only
/// where the survivor's code reads as the entry's, so [`remove`] finds its
/// entry by that code.
fn origin<'a>(
    survivor: &'a Survivor,
    campaign: &'a Campaign,
    mission: &'a str,
) -> Option<(&'a str, &'a str)> {
    match &survivor.outcome {
        Some(Triage::Equivalent { carried_from, .. }) => {
            Some((mission, carried_from.as_deref().unwrap_or(&campaign.head)))
        }
        Some(Triage::ProposedByNunki {
            from: crate::mutants::ProposedFrom::Carried { commit },
            ..
        }) => Some((mission, commit)),
        _ => None,
    }
}

/// Take out of the registry the rulings on survivor `id` of `campaign`,
/// read **before** `--lift` took them off the mission's file: each entry on
/// the same file and description that was given where the survivor's ruling
/// was (the same mission and commit), or that is about the code the
/// survivor's mutation replaces now — the second also for a survivor the
/// mission's file no longer holds a ruling on, so that a registry entry left
/// behind can still be lifted (HQ review, item 4).
pub fn remove(ruling: &Ruling, campaign: &Campaign, id: &str) -> Registered {
    let held: Vec<(&Survivor, Option<(&str, &str)>)> = campaign
        .survivors
        .iter()
        .filter(|s| s.id == id)
        .map(|s| (s, origin(s, campaign, ruling.mission)))
        .collect();
    if held.is_empty() {
        return Registered::Done(0);
    }
    let lines: Vec<Option<String>> = held
        .iter()
        .map(|(s, _)| span_at(ruling.tree, &campaign.head, s))
        .collect();
    under_lock(
        ruling.hq_root,
        "mission mutants --lift (registry)",
        |registry| {
            let before = registry.entries.len();
            registry.entries.retain(|e| {
                !held.iter().zip(&lines).any(|((s, origin), line)| {
                    e.file == s.file
                        && e.description == s.description
                        && (*origin == Some((e.mission.as_str(), e.commit.as_str()))
                            || line.as_deref() == Some(e.line.as_str()))
                })
            });
            Ok(before - registry.entries.len())
        },
    )
}

/// Whether an entry's code still stands in a repository's `HEAD`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Standing {
    /// The file at `HEAD` holds the entry's code once.
    Stands,
    /// The file is there, and does not hold the entry's code.
    Changed,
    /// The file holds the entry's code more than once: no campaign is
    /// answered from it there.
    Repeated(usize),
    /// The file cannot be read at `HEAD`.
    Gone,
}

impl Standing {
    pub fn said(&self) -> String {
        match self {
            Standing::Stands => "its code stands at HEAD".to_string(),
            Standing::Changed => {
                "its code has changed at HEAD: it applies to nothing there".to_string()
            }
            Standing::Repeated(n) => {
                format!("its code occurs {n} times at HEAD: it applies to nothing there")
            }
            Standing::Gone => {
                "its file cannot be read at HEAD: it applies to nothing there".to_string()
            }
        }
    }
}

/// Where `entry`'s code is in the repository at `tree`, at `HEAD`: any run
/// of its many lines counts, since code that moved still matches.
pub fn standing(tree: &Path, entry: &Entry) -> Standing {
    let Some(source) = source_at(tree, "HEAD", &entry.file) else {
        return Standing::Gone;
    };
    let lines: Vec<&str> = source.lines().collect();
    let width = usize::try_from(entry.lines.max(1)).unwrap_or(1);
    match lines
        .windows(width)
        .filter(|w| span_digest(w) == entry.line)
        .count()
    {
        0 => Standing::Changed,
        1 => Standing::Stands,
        n => Standing::Repeated(n),
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
