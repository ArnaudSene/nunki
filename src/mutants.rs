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

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::git;

/// The campaign's result, in the mission folder, read by the HQ and by gate 7.
pub const FILE: &str = "MUTANTS.json";

/// The campaign in flight, beside it. Its own record on purpose: the mission
/// state holds one run handle and that one belongs to the agent — a campaign
/// filed there would show up in `hq mission status` as an agent's run.
pub const RUN_FILE: &str = "MUTANTS.run.json";

/// What the stack fragment declares. It prints the survivors on stdout, one
/// JSON object per line, and `hq` never parses a mutation tool itself: which
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
    /// Shown equivalent, in one sentence. This is the one outcome no gate can
    /// check: SPEC gives the counter-check to the HQ, so gate 7 accepts it
    /// and **says how many rode on it**. Left silent it would be the escape
    /// hatch that empties the gate.
    Equivalent { why: String },
    /// Recognised as a bug and frozen in a named test.
    Bug { test: String },
}

impl Triage {
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
    /// How long it is given before `hq` calls it hung (SPEC 4.4, "un délai
    /// paramétré").
    pub deadline_minutes: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum MutantsError {
    #[error(transparent)]
    Git(#[from] git::GitError),
    #[error("{0} is not a campaign `hq` can read: {1}")]
    Unreadable(PathBuf, String),
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error(transparent)]
    Exec(#[from] crate::exec::ExecError),
    #[error("the campaign could not be launched: {0}")]
    Launch(String),
}

/// Where the two files live for a mission.
pub fn paths(dir: &Path) -> (PathBuf, PathBuf) {
    (dir.join(FILE), dir.join(RUN_FILE))
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
    let (file, _) = paths(dir);
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

pub fn read_running(dir: &Path) -> Result<Option<Running>, MutantsError> {
    let (_, file) = paths(dir);
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
    let (file, _) = paths(dir);
    let body = serde_json::to_string_pretty(campaign).expect("a campaign serialises");
    std::fs::write(&file, format!("{body}\n")).map_err(|e| MutantsError::Io(file, e))
}

pub fn write_running(dir: &Path, running: &Running) -> Result<(), MutantsError> {
    let (_, file) = paths(dir);
    let body = serde_json::to_string_pretty(running).expect("a record serialises");
    std::fs::write(&file, format!("{body}\n")).map_err(|e| MutantsError::Io(file, e))
}

pub fn forget_running(dir: &Path) -> Result<(), MutantsError> {
    let (_, file) = paths(dir);
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
pub fn parse(text: &str) -> Vec<Survivor> {
    text.lines()
        .filter_map(|line| serde_json::from_str::<Survivor>(line.trim()).ok())
        .collect()
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
pub fn campaign(
    project: &crate::project::Project,
    slot: &crate::slot::Slot,
    engine: std::sync::Arc<dyn crate::engine::Engine>,
    dir: &Path,
    stack: &str,
    touched: &[String],
    deadline_minutes: u32,
) -> Result<Progress, MutantsError> {
    use crate::harness::spawn::{CommandSpec, Presence, Signal, Spawned, Spawner};

    let head = git::head(&slot.tree)?;
    let want = fingerprint(&slot.tree, touched)?;
    let file = crate::run::profile_path(project, &slot.name);
    let compose_project = crate::compose::project_name(&slot.name)
        .map_err(|e| MutantsError::Exec(crate::exec::ExecError::Compose(e)))?;

    if let Some(running) = read_running(dir)? {
        let spawner = crate::engine::spawn::ContainerSpawner::new(
            engine.clone(),
            file,
            &compose_project,
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
        return match presence {
            Presence::Running => {
                if minutes_since(&running.started_at) > running.deadline_minutes {
                    // Stopped rather than left: SPEC 4.4 gives the campaign a
                    // configured delay, and a campaign past it is holding a
                    // slot, not working in it.
                    let _ = spawner.signal(&spawned, Signal::Terminate);
                    forget_running(dir)?;
                    Ok(Progress::Overrun {
                        minutes: running.deadline_minutes,
                    })
                } else {
                    Ok(Progress::Running {
                        started_at: running.started_at.clone(),
                        lines: text.lines().count(),
                    })
                }
            }
            Presence::Ended => {
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
                forget_running(dir)?;
                Ok(Progress::Finished {
                    survivors: survivors.len(),
                })
            }
            Presence::Vanished(why) | Presence::Unknown(why) => {
                forget_running(dir)?;
                Ok(Progress::Lost(why))
            }
        };
    }

    // Nothing in flight. A campaign on this exact content is not run again:
    // SPEC 4.4 says it replays only when the touched files have changed, and
    // § 7 counts the hour it would otherwise spend.
    if let Some(existing) = read(dir)?
        && existing.fingerprint == want
    {
        return Ok(Progress::Fresh {
            survivors: existing.survivors.len(),
        });
    }

    let script = format!("{}/stacks/{stack}/{SCRIPT}", crate::project::FRAGMENTS_DIR);
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
            "there is no executable {script} on this commit — the mutation campaign is \
             a deterministic command the stack fragment declares (SPEC 4.4)"
        )));
    }

    let runs = dir.join("runs");
    std::fs::create_dir_all(&runs).map_err(|e| MutantsError::Io(runs.clone(), e))?;
    let log = runs.join(format!("mutants-{}.log", &want[..7.min(want.len())]));
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
                program: format!("./{script}"),
                args,
                cwd: PathBuf::from(crate::exec::PROOF_AT),
                env: Default::default(),
            },
            &log,
        )
        .map_err(|e| MutantsError::Launch(e.to_string()))?;

    write_running(
        dir,
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

/// Minutes since an RFC 3339 stamp, read the way `hq` writes them. A stamp it
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
