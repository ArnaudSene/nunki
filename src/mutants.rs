//! The mutation campaign and its file (SPEC 4.4, gate 7).
//!
//! Gate 7 is deterministic and reads a file — SPEC says so in as many words:
//! *"Son résultat est un fichier du dossier de mission que le HQ lit."* The
//! campaign that produces that file is a separate, long thing: it runs in the
//! slot's container, on the clean copy of `HEAD`, launched detached and
//! watched like a run. This module owns both halves; the gate itself lives in
//! [`crate::gate`].
//!
//! **No threshold.** The gate is green when every survivor has received one
//! of three outcomes, not when a score clears a bar. A threshold and a triage
//! pull in opposite directions, and Google, whose practice this borrows, keeps
//! neither score nor bar.
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
/// hand a line came from (SPEC 4.1, decided 2026-09-10). The agent may write
/// the two outcomes that rest on a committed test, and physically cannot
/// write the third.
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
}

/// The three outcomes SPEC 4.4 allows, and there is no fourth.
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
    /// empties itself (SPEC 4.4, decided 2026-09-10).
    Equivalent { why: String },
    /// Recognised as a bug and frozen in a named test.
    Bug { test: String },
}

impl Triage {
    /// Whether the coder may write this outcome itself. The two that rest on
    /// a committed test, yes; the judgement, no.
    pub fn is_the_coders_to_give(&self) -> bool {
        self.test().is_some()
    }

    /// The name this outcome goes by in a refusal.
    pub fn kind(&self) -> &'static str {
        match self {
            Triage::Killed { .. } => "killed",
            Triage::Equivalent { .. } => "equivalent",
            Triage::Bug { .. } => "bug",
        }
    }

    /// The test this outcome rests on, when it rests on one.
    pub fn test(&self) -> Option<&str> {
        match self {
            Triage::Killed { test } | Triage::Bug { test } => Some(test),
            Triage::Equivalent { .. } => None,
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
}

/// Whether the campaign said it finished.
///
/// The whole log, not its last line: a campaign is read back through a file
/// the engine is still writing, and asking for the last line would turn a
/// half-flushed newline into "it did not finish". The line is printed last,
/// so a truncated log has lost it either way.
pub fn completed(text: &str) -> bool {
    text.lines().any(|line| {
        serde_json::from_str::<Terminal>(line.trim())
            .map(|t| t.campaign == "done")
            .unwrap_or(false)
    })
}

pub fn parse(text: &str) -> Vec<Survivor> {
    text.lines()
        .filter_map(|line| serde_json::from_str::<Survivor>(line.trim()).ok())
        .collect()
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
    let found = campaign
        .survivors
        .iter_mut()
        .find(|s| s.id == id)
        .ok_or_else(|| {
            MutantsError::Unreadable(
                dir.join(FILE),
                format!("no survivor is called {id:?} in this campaign"),
            )
        })?;
    found.outcome = Some(Triage::Equivalent {
        why: why.to_string(),
    });
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
/// Before this existed the only way past it was to delete `MUTANTS.json` by
/// hand, and on 2026-09-17 that took `MUTANTS.triage.json` with it — a file
/// the engine then replaced with a directory, which brought the mission down.
/// A verb is cheaper than the workaround it replaces.
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
    /// The container went away under it, or the engine could not be asked.
    Lost(String),
}

/// What a campaign that has ended says, once `FILE` is written.
///
/// Two sentences and not one: with nothing alive there is nobody to triage,
/// and "each needs one of the three outcomes" would then be a sentence about
/// no one. Measured on `notes-api` on 2026-09-16, where a clean campaign
/// still asked for outcomes it did not need — a line that says the same
/// thing whatever happened is a line a reader learns to skip.
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
/// a test can read is a sentence a test can catch. This one was printed with
/// fifty spaces in the middle of it for half a day — `cargo fmt` had joined a
/// multi-line literal and kept the indentation inside the string, and nothing
/// was looking.
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
/// "something holds that number".
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
            let survivors = parse(&text);
            write(
                dir,
                &Campaign {
                    fingerprint: running.fingerprint.clone(),
                    head: running.head.clone(),
                    date: crate::state::now_rfc3339(),
                    survivors: survivors.clone(),
                },
            )?;
            forget_running(&project.hq_root, &slot.name)?;
            Progress::Finished {
                survivors: survivors.len(),
            }
        }
        Presence::Ended => {
            forget_running(&project.hq_root, &slot.name)?;
            Progress::Lost(format!(
                "it stopped without saying it had finished, after {} line(s): a campaign \
                 that was killed, whose container went away, or that never compiled \
                 leaves exactly this, and none of them measured anything. Its log is {}",
                text.lines().count(),
                running.log.display()
            ))
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
    use crate::harness::spawn::{CommandSpec, Spawner};

    let head = git::head(&slot.tree)?;
    // The base's **name**, and the paths worked out here — not handed in.
    //
    // Gate 7 judges a campaign by the fingerprint of what it ran on, so the
    // gate and the launcher have to mean the same thing by "what this branch
    // brought". They were two calls in two files, and in a slot the base has
    // two readings: measured on `notes-4` on 2026-09-18, the launcher's set
    // held eight paths and the gate's four, their fingerprints never agreed,
    // and gate 7 asked for a campaign that had just run — fifty-seven turns
    // of `verify` went round it.
    //
    // One call, in the place that cannot be bypassed, rather than a rule the
    // next caller has to know.
    let touched = crate::gate::touched_since_base(&slot.tree, base)
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

    let script = format!("{}/{SCRIPT}", crate::run::STACK_AT);
    crate::exec::refresh(project, slot, engine.clone())?;
    // Absent or not executable is said here, where the message can be about
    // the campaign, rather than as a launch that times out waiting for a pid.
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
            "there is no executable {script} in the container — the mutation campaign is \
             a deterministic command the `{stack}` stack declares, in the project's home \
             (SPEC 4.4)"
        )));
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
    // Measured on `notes-3` on 2026-09-16, four campaigns on one fingerprint:
    // 5 survivors, then 1, then none, then none — and `MUTANTS.json` said
    // six, every one of them dead. Gate 7 sent a coder back three times for
    // mutants that no longer existed, and nothing in the flow could tell:
    // the file said what the file said.
    std::fs::write(&log, "").map_err(|e| MutantsError::Io(log.clone(), e))?;
    let spawner = crate::engine::spawn::ContainerSpawner::new(
        engine,
        crate::run::profile_path(project, &slot.name),
        &compose_project,
        crate::compose::AGENT_SERVICE,
    )
    .identified_by(&want);
    let mut args = vec![want.clone()];
    args.extend(touched.iter().cloned());
    let spawned = spawner
        .spawn(
            &CommandSpec {
                program: script.clone(),
                args,
                cwd: PathBuf::from(crate::exec::PROOF_AT),
                env: Default::default(),
            },
            &log,
        )
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
