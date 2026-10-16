//! Campaigns outside missions (SPEC 4.4, gate 7: the campaigns outside
//! missions).
//!
//! A mission measures what its branch changed, and from rule 2 on, a sample
//! of it. What neither sees — a mutant left outside a sample, a mutant of a
//! file no partial campaign measured again after another change revived
//! it — is caught here, by a campaign the human launches on the project's
//! base, with no scheduler: `nunki mutants run`, on what changed since the
//! last completed campaign outside missions, or on the whole code with
//! `--whole`. `nunki mutants status` reads it back into the project's
//! **debt**, the survivors no mission owns, kept in the project's HQ.
//!
//! It runs in a slot no mission holds, in that slot's clean copy, put on
//! the base as the human's repository last had it — the commit the host
//! records, never a ref of the slot — while the slot's own checkout is left
//! alone. Its in-flight record is the slot's, the one every door into the
//! copy checks, marked as outside missions ([`crate::mutants::OffMission`]):
//! a mission's reading of the slot refuses it, so it is never recorded as a
//! mission's campaign.
//!
//! **Starting is never a side effect of asking.** `run` starts one, and
//! refuses while any campaign is filed in the slot; `status` reads one back
//! and never starts one, so a loop polling it waits without launching.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::engine::Engine;
use crate::mutants::{self, MutantsError, OffMission, Progress, Running, Survivor};
use crate::project::Project;

/// The project's debt, under its HQ.
pub const DEBT_FILE: &str = "mutants-debt.json";

/// Where a campaign outside missions keeps its logs, under the HQ.
pub const DIR: &str = "off-mission";

/// The deadline `nunki mutants run` gives a campaign unless told otherwise,
/// in minutes. A campaign outside missions measures no sample — it is
/// where the whole is measured — so its deadline only stops one that hangs.
/// A choice, not a measurement: the whole of a crate whose mutant takes
/// 47 s, at 500 mutants, takes about 6.5 hours.
pub const DEFAULT_MINUTES: u32 = 720;

/// The project's debt: the survivors of the campaigns outside missions, as
/// the last of them left them, and what that campaign ran on.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Debt {
    /// Each survivor still owed, keyed — as rulings are carried — by its
    /// file and its description, and dated by the commit that found it.
    #[serde(default)]
    pub owed: Vec<Owed>,
    /// The last campaign outside missions that completed: what a campaign
    /// on what changed since starts from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<Measured>,
}

/// A survivor in the debt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owed {
    #[serde(flatten)]
    pub survivor: Survivor,
    /// The commit of the campaign that found it.
    pub found_at: String,
}

/// What a completed campaign outside missions ran on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Measured {
    pub base: String,
    pub commit: String,
    pub date: String,
    /// On the whole code, or on what changed since the one before.
    pub whole: bool,
    /// How many mutants it tried, as its script said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tried: Option<u32>,
}

/// What `nunki mutants run` measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    /// Every file of the code at the base's commit.
    Whole,
    /// What changed since the last completed campaign outside missions.
    SinceLast,
}

/// What a caller asks of [`run`].
#[derive(Debug, Clone, Copy)]
pub struct Asked<'a> {
    /// The base branch, by name, as `nunki mission new` defaults it.
    pub base: &'a str,
    pub form: Form,
    /// The deadline, in minutes.
    pub minutes: u32,
}

/// A campaign [`run`] started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Started {
    pub slot: String,
    pub base: String,
    pub commit: String,
    pub since: Option<String>,
    /// How many paths it was handed.
    pub paths: usize,
    pub minutes: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum OffMissionError {
    #[error(
        "slot {slot} is held by mission(s) {missions}: a campaign outside missions runs only \
         in a slot no mission holds — `nunki slot list` says which are free"
    )]
    Occupied { slot: String, missions: String },
    #[error(
        "a campaign is filed in slot {slot}: `nunki mutants status` reads it back, and `nunki \
         mutants run` starts nothing there until it has"
    )]
    InFlight { slot: String },
    #[error(
        "slot {0} has no profile yet, so no container to run a campaign in: a mission must \
         have run in it once"
    )]
    NoProfile(String),
    #[error(
        "the repository at {root} has no branch {base} to measure: `--base` names the branch \
         a campaign outside missions runs on"
    )]
    NoBase { root: PathBuf, base: String },
    #[error(
        "no campaign outside missions has completed on this project yet, so there is nothing \
         to measure since: `nunki mutants run --whole` measures the whole code"
    )]
    NoPrevious,
    #[error(
        "nothing changed on {base} since the last campaign outside missions, at {commit}: \
         there is nothing to measure"
    )]
    Unchanged { base: String, commit: String },
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("{0} is not a debt nunki can read: {1}")]
    Unreadable(PathBuf, String),
    #[error("{0}")]
    Lifecycle(String),
    #[error(transparent)]
    Mutants(#[from] MutantsError),
    #[error(transparent)]
    Git(#[from] crate::git::GitError),
    #[error(transparent)]
    Slot(#[from] crate::slot::SlotError),
    #[error(transparent)]
    Exec(#[from] crate::exec::ExecError),
}

/// Where the project's debt is.
pub fn debt_path(hq_root: &Path) -> PathBuf {
    hq_root.join(DEBT_FILE)
}

/// The project's debt: empty when none is on file; an error, naming the
/// file, when one is and cannot be read — never an empty debt in its place.
pub fn read_debt(hq_root: &Path) -> Result<Debt, MutantsError> {
    let path = debt_path(hq_root);
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            serde_json::from_str(&text).map_err(|e| MutantsError::Unreadable(path, e.to_string()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Debt::default()),
        Err(e) => Err(MutantsError::Io(path, e)),
    }
}

fn write_debt(hq_root: &Path, debt: &Debt) -> Result<(), MutantsError> {
    let path = debt_path(hq_root);
    let body = serde_json::to_string_pretty(debt)
        .map_err(|e| MutantsError::Io(path.clone(), std::io::Error::other(e)))?;
    let staged = path.with_extension("json.new");
    std::fs::write(&staged, body + "\n").map_err(|e| MutantsError::Io(staged.clone(), e))?;
    std::fs::rename(&staged, &path).map_err(|e| MutantsError::Io(path, e))
}

/// The debt after a completed campaign that found `found` at `commit`.
///
/// One on the whole code replaces it: it measured everything. One on what
/// changed since **adds** to it, a survivor already owed under the same key
/// keeping its entry: its base moves with every campaign, so it measured
/// again only the lines changed since the last one, and replacing a file's
/// entries would drop survivors on lines it never measured again.
///
/// **Each entry owed before the campaign stands for one survivor it found,
/// never more:** the one with the same id first, then one with the same
/// key. Two survivors of one campaign can share a key — the description
/// names the function, not the position, so measured on cargo-mutants
/// 27.1.0 `src/lib.rs:2:7: replace > with == in keep` and `src/lib.rs:2:16:
/// replace > with == in keep` both survive as `replace > with == in keep` —
/// and each is owed.
pub fn settled(mut debt: Debt, found: Vec<Survivor>, measured: Measured) -> Debt {
    let key = |s: &Survivor| (s.file.clone(), s.description.clone());
    if measured.whole {
        debt.owed.clear();
    }
    let mut free = vec![true; debt.owed.len()];
    let mut take = |matches: &dyn Fn(&Survivor) -> bool| {
        let i = (0..free.len()).find(|&i| free[i] && matches(&debt.owed[i].survivor))?;
        free[i] = false;
        Some(i)
    };
    // The same id first: with [A] owed and [B, A] found under one key, a
    // match by key alone would let B stand for A, and A be owed twice.
    let unmatched: Vec<Survivor> = found
        .into_iter()
        .filter(|s| take(&|o| o.id == s.id && key(o) == key(s)).is_none())
        .collect();
    let fresh: Vec<Owed> = unmatched
        .into_iter()
        .filter(|s| take(&|o| key(o) == key(s)).is_none())
        .map(|survivor| Owed {
            survivor,
            found_at: measured.commit.clone(),
        })
        .collect();
    debt.owed.extend(fresh);
    debt.last = Some(measured);
    debt
}

/// Start a campaign outside missions in `slot` (module documentation).
pub fn run(
    project: &Project,
    slot: &crate::slot::Slot,
    engine: Arc<dyn Engine>,
    stack: &str,
    asked: &Asked<'_>,
) -> Result<Started, OffMissionError> {
    use crate::harness::spawn::Spawner;

    let occupants = crate::lifecycle::occupants(project)
        .map_err(|e| OffMissionError::Lifecycle(e.to_string()))?;
    if let Some(missions) = occupants.get(&slot.name).filter(|m| !m.is_empty()) {
        return Err(OffMissionError::Occupied {
            slot: slot.name.clone(),
            missions: missions.join(", "),
        });
    }
    if mutants::read_running(&project.hq_root, &slot.name)?.is_some() {
        return Err(OffMissionError::InFlight {
            slot: slot.name.clone(),
        });
    }
    let profile = crate::run::profile_path(project, &slot.name);
    if !profile.is_file() {
        return Err(OffMissionError::NoProfile(slot.name.clone()));
    }

    // The base as the human's repository has it now, recorded by the host
    // (gate 2's rule): never a ref of the slot, which an agent may have
    // moved.
    let git = crate::git::SlotGit::open(&slot.tree)?;
    git.fetch_origin(&project.root)?;
    let commit = git
        .origin_branch(asked.base)
        .ok_or_else(|| OffMissionError::NoBase {
            root: project.root.clone(),
            base: asked.base.to_string(),
        })?;
    let debt = read_debt(&project.hq_root)?;
    let since = match asked.form {
        Form::Whole => None,
        Form::SinceLast => {
            let last = debt.last.ok_or(OffMissionError::NoPrevious)?;
            if last.commit == commit {
                return Err(OffMissionError::Unchanged {
                    base: asked.base.to_string(),
                    commit: short(&commit),
                });
            }
            Some(last.commit)
        }
    };
    let paths = match &since {
        None => git.names(&["ls-tree", "-r", "-z", "--name-only", &commit])?,
        Some(since) => git.names(&["diff", "--name-only", "-z", "--no-renames", since, &commit])?,
    };

    crate::exec::refresh_to(
        project,
        slot,
        engine.clone(),
        &format!("refs/remotes/origin/{}", asked.base),
        &commit,
    )?;
    let judged = crate::run::judged(project, stack);
    let id = format!("off-mission-{}", short(&commit));
    // No budget, so no sample: this is where the whole is measured. The
    // fork point is the last campaign's commit, or none for the whole code,
    // which the script then reads as whole files.
    let launched = mutants::command(
        &judged,
        &id,
        &paths,
        since.as_deref().unwrap_or(""),
        project.config.jobs(),
    );
    let runs = project.hq_root.join(DIR).join("runs");
    std::fs::create_dir_all(&runs).map_err(|e| OffMissionError::Io(runs.clone(), e))?;
    let log = runs.join(format!("{id}.log"));
    std::fs::write(&log, "").map_err(|e| OffMissionError::Io(log.clone(), e))?;
    let compose_project = crate::compose::project_name(&project.session(), &slot.name)
        .map_err(|e| MutantsError::Exec(crate::exec::ExecError::Compose(e)))?;
    let spawner = crate::engine::spawn::ContainerSpawner::new(
        engine,
        profile,
        &compose_project,
        crate::compose::AGENT_SERVICE,
    )
    .identified_by(&id);
    let spawned = spawner
        .spawn(&launched, &log)
        .map_err(|e| MutantsError::Launch(e.to_string()))?;
    mutants::write_running(
        &project.hq_root,
        &slot.name,
        &Running {
            fingerprint: id,
            head: commit.clone(),
            started_at: crate::state::now_rfc3339(),
            container: spawned.container,
            pid: spawned.pid,
            log,
            deadline_minutes: asked.minutes,
            chain: mutants::Chain {
                scope: mutants::Scope::Full {
                    why: "a campaign outside missions".to_string(),
                },
                ..mutants::Chain::default()
            },
            off_mission: Some(OffMission {
                base: asked.base.to_string(),
                commit: commit.clone(),
                since: since.clone(),
            }),
        },
    )?;
    Ok(Started {
        slot: slot.name.clone(),
        base: asked.base.to_string(),
        commit,
        since,
        paths: paths.len(),
        minutes: asked.minutes,
    })
}

/// Read back a campaign outside missions whose script has ended and of
/// which nothing is left running, into the project's debt, and forget it
/// ([`mutants::read_back_off_mission`]).
///
/// Only one that said it finished, and measured, is recorded: through the
/// same [`mutants::completed`] and [`mutants::unmeasured`] a mission's
/// campaign goes through. The registry of equivalences applies to what it
/// found, as to a mission's campaign. Anything else leaves the debt as it
/// was, and says why.
pub(crate) fn read_ended(
    project: &Project,
    slot: &crate::slot::Slot,
    running: &Running,
) -> Result<Progress, MutantsError> {
    let text = std::fs::read_to_string(&running.log).unwrap_or_default();
    mutants::forget_running(&project.hq_root, &slot.name)?;
    let Some(off) = &running.off_mission else {
        return Err(MutantsError::MissionCampaign {
            slot: slot.name.clone(),
        });
    };
    if !mutants::completed(&text) {
        let stderr = running.log.with_extension("err");
        return Ok(Progress::CouldNotRun(format!(
            "it stopped without saying it had finished, after {} line(s), and the debt is \
             left as it was. What it said is in {}",
            text.lines().count(),
            stderr.display()
        )));
    }
    if let Some(why) = mutants::unmeasured(&text) {
        return Ok(Progress::CouldNotRun(format!(
            "{why}; the debt is left as it was"
        )));
    }
    let mut found = mutants::parse(&text);
    if let Ok(registry) = crate::equivalences::read_locked(&project.hq_root) {
        crate::equivalences::apply(&registry, &mut found, |s| {
            crate::equivalences::identify(&slot.tree, &off.commit, s).ok()
        });
    }
    let count = found.len();
    let debt = settled(
        read_debt(&project.hq_root)?,
        found,
        Measured {
            base: off.base.clone(),
            commit: off.commit.clone(),
            date: crate::state::now_rfc3339(),
            whole: off.since.is_none(),
            tried: mutants::tried(&text),
        },
    );
    write_debt(&project.hq_root, &debt)?;
    Ok(Progress::Finished {
        survivors: count,
        chain: vec![debt_said(&debt)],
    })
}

/// The debt in one line: how many survivors it owes, and from which
/// campaign.
pub fn debt_said(debt: &Debt) -> String {
    match &debt.last {
        None => "no campaign outside missions has completed on this project".to_string(),
        Some(last) => format!(
            "{} survivor(s) owed; the last campaign outside missions ran on {} at {}, {}, \
             tried {}",
            debt.owed.len(),
            last.base,
            short(&last.commit),
            if last.whole {
                "on the whole code"
            } else {
                "on what changed since the one before"
            },
            last.tried
                .map_or_else(|| "a number not said".to_string(), |t| t.to_string())
        ),
    }
}

fn short(commit: &str) -> String {
    commit.chars().take(12).collect()
}
