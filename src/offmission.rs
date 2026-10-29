//! Campaigns outside missions (SPEC 4.4, gate 7: the campaigns outside
//! missions).
//!
//! A mission measures what its branch changed, and from rule 2 on, a sample
//! of it. What neither sees — a mutant left outside a sample, a mutant of a
//! file no partial campaign measured again after another change revived
//! it — is caught here, by a campaign the human launches on the project's
//! base, with no scheduler: `nunki mutants run`, on the files changed since
//! the last completed campaign outside missions, each mutated whole, or on
//! the whole code with `--whole`. `nunki mutants status` reads it back into the project's
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
    /// Each survivor still owed, dated by the commit that found it. Those of
    /// one file all come from the last campaign that measured that file
    /// ([`settled`]).
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
    /// On the whole code, or on the files changed since the one before.
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
    /// The files changed since the last completed campaign outside
    /// missions, each mutated whole.
    SinceLast,
    /// The debt's survivors, by name, and nothing else ([`measurable`]): what
    /// settles the survivors a hardening mission's tests kill, once they are
    /// on the base. A test file is never mutated, so no other form measures a
    /// survivor whose own file did not change again.
    Owed,
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
    /// For one on the debt's survivors by name: how many it measures.
    pub owed: Option<Owing>,
}

/// What a campaign on the debt's survivors by name took from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owing {
    /// How many survivors it measures.
    pub named: usize,
    /// How many it left: those it cannot measure by name, and those beyond
    /// what one campaign is handed.
    pub left: usize,
    /// Why, for the first of those it cannot measure.
    pub why: Option<String>,
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
    #[error("no survivor called {0:?} is owed: `nunki mutants debt` lists the debt")]
    NotOwed(String),
    #[error(
        "a ruling needs its sentence: `--because <why>` says why the mutant changes nothing \
         anyone can observe"
    )]
    Blank,
    #[error(
        "the registry of equivalences is unchanged — {why}. Nothing is ruled, and the debt still \
         owes {ids}: a test that kills a survivor settles it"
    )]
    NotRegistered { ids: String, why: String },
    #[error(
        "the registry proposes nothing on {0:?}: `nunki mutants debt --equivalent <survivor> \
         --because <why>` rules on it yourself"
    )]
    NoProposal(String),
    #[error(
        "the proposal on {0:?} changed since it was listed, so nothing was ratified — list them \
         again"
    )]
    Changed(String),
    #[error(
        "the registry of equivalences took the ruling on {ids}, but the debt could not be \
         written ({why}): it still lists them as owed, and the same command given again settles \
         them without ruling twice"
    )]
    DebtNotWritten { ids: String, why: String },
    #[error("the project's debt could not be locked: {0}")]
    Locked(String),
    #[error(
        "the project's debt owes nothing, so there is nothing to measure by name: `nunki \
         mutants run` measures the code first"
    )]
    NothingOwed,
    #[error("none of the debt's {owed} survivor(s) {what}: {why}")]
    NoneTaken {
        owed: usize,
        what: &'static str,
        why: String,
    },
    #[error(transparent)]
    Registry(#[from] crate::equivalences::RegistryError),
}

/// Where the project's debt is.
pub fn debt_path(hq_root: &Path) -> PathBuf {
    hq_root.join(DEBT_FILE)
}

/// The lock the debt's writers take, beside the slots' and the registry's:
/// `nunki mutants status`, reading a campaign back into it, and the HQ's
/// rulings on it. A leading dot, so that no slot can be called the same.
const DEBT_LOCK: &str = ".debt";

fn lock_debt(hq_root: &Path, verb: &str) -> Result<crate::state::SlotLock, String> {
    crate::equivalences::hq_lock(hq_root, DEBT_LOCK, verb)
}

/// What a campaign outside missions is called — its log, its record, and
/// where a ruling on its survivors names its mission in the registry of
/// equivalences: `off-mission-` and the first twelve characters of the
/// base's commit it ran on.
pub fn campaign_id(commit: &str) -> String {
    format!("off-mission-{}", short(commit))
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
/// changed since measured each file in `changed` again **whole** ([`run`]),
/// so the debt drops what it owed in those files — removed and renamed ones
/// included — for what the campaign found there, and keeps the survivors of
/// every other file as they were: the chain's rule (SPEC 4.4), and its
/// assumption, that a mutant measured in a file nobody changed since still
/// exists and still survives. No hunk is read. Measured again on the
/// changed lines only, an owed survivor nobody measured again stayed owed
/// at a line an edit had moved, and a new survivor could take its entry
/// under the same key — the description names the function, not the
/// position: measured on cargo-mutants 27.1.0, `src/lib.rs:2:7: replace >
/// with == in keep` and `src/lib.rs:2:16: replace > with == in keep` both
/// survive. So the survivors of one file all come from one campaign, and
/// two under the same key stay two.
///
/// `changed` is `None` in a record written while such a campaign measured
/// only the lines changed: nothing is dropped by file then. Either way, a
/// survivor found under an id already owed replaces that entry, so no two
/// entries of the debt share an id.
pub fn settled(
    mut debt: Debt,
    found: Vec<Survivor>,
    measured: Measured,
    changed: Option<&[String]>,
) -> Debt {
    if measured.whole {
        debt.owed.clear();
    } else if let Some(changed) = changed {
        let changed: std::collections::BTreeSet<&String> = changed.iter().collect();
        debt.owed.retain(|o| !changed.contains(&o.survivor.file));
    }
    debt.owed
        .retain(|o| !found.iter().any(|s| s.id == o.survivor.id));
    debt.owed.extend(found.into_iter().map(|survivor| Owed {
        survivor,
        found_at: measured.commit.clone(),
    }));
    debt.last = Some(measured);
    debt
}

/// The debt after a completed campaign that measured the survivors `named`
/// by name at `commit` (`nunki mutants run --owed`): each of them is dropped
/// for what the campaign found — one still surviving comes back, found at
/// `commit` — and every other survivor stays as it was. What the debt last
/// measured ([`Debt::last`]) does not move: this campaign measured no file,
/// and the next one on what changed since starts where the last one ended.
pub fn settled_owed(mut debt: Debt, found: Vec<Survivor>, commit: &str, named: &[String]) -> Debt {
    debt.owed.retain(|o| {
        !named.contains(&o.survivor.id) && !found.iter().any(|s| s.id == o.survivor.id)
    });
    debt.owed.extend(found.into_iter().map(|survivor| Owed {
        survivor,
        found_at: commit.to_string(),
    }));
    debt
}

/// What [`measurable`] takes from the debt, and what it leaves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Measurable {
    /// The survivors taken, in the debt's order: oldest first.
    pub taken: Vec<Owed>,
    /// Each survivor left, by id, and why.
    pub left: Vec<(String, String)>,
    /// How many more could have been taken, beyond what fits.
    pub beyond: usize,
}

impl Measurable {
    /// What was left, in a few words: the first three, and how many more.
    pub fn left_said(&self) -> String {
        let mut said: Vec<String> = self
            .left
            .iter()
            .take(3)
            .map(|(id, why)| format!("{}: {why}", crate::text::one_line(id)))
            .collect();
        if self.left.len() > 3 {
            said.push(format!("and {} more", self.left.len() - 3));
        }
        said.join("; ")
    }
}

/// The debt's survivors that can be measured by name at `commit`, in the
/// debt's order, oldest first, and why each other one cannot (SPEC 4.4, the
/// debt): what a hardening mission takes ([`hardening`]) and `nunki mutants
/// run --owed` measures. One is taken when
///
/// - a stack names it ([`mutants::named_by`]) and its script, in the
///   project's home, says it measures by name ([`mutants::measures_by_name`]).
///   The Python and Next.js scripts do not, and their survivors wait for a
///   campaign on the whole code;
/// - its file is unchanged in the human's repository between the commit
///   that found it and `commit`. A name holds a line and a column: in a file
///   changed since, it may have moved, and a name no longer in the code makes
///   the campaign measure nothing. Such a survivor waits for `nunki mutants
///   run`, which measures its file whole again;
///
/// and as long as the names fit in [`mutants::NAMES_MAX_BYTES`], and in
/// `count` when one is given. The rest is [`Measurable::beyond`].
pub fn measurable(
    project: &Project,
    judged: &[crate::run::Judged],
    owed: &[Owed],
    commit: &str,
    count: Option<usize>,
) -> Measurable {
    let by_name: std::collections::BTreeSet<String> = judged
        .iter()
        .filter(|j| {
            std::fs::read_to_string(project.fragment(&j.stack.name).join(mutants::SCRIPT))
                .is_ok_and(|script| mutants::measures_by_name(&script))
        })
        .map(|j| j.stack.name.clone())
        .collect();
    let mut taken = Vec::new();
    let mut left = Vec::new();
    let mut beyond = 0;
    let mut bytes: usize = 2;
    for o in owed {
        let name = match mutants::named_by(judged, &o.survivor) {
            Ok((j, _)) if !by_name.contains(&j.stack.name) => {
                left.push((
                    o.survivor.id.clone(),
                    format!(
                        "the `{}` stack's {} does not measure mutants by name",
                        j.stack.name,
                        mutants::SCRIPT
                    ),
                ));
                continue;
            }
            Ok((_, name)) => name,
            Err(why) => {
                left.push((o.survivor.id.clone(), why));
                continue;
            }
        };
        match file_changed(&project.root, &o.found_at, commit, &o.survivor.file) {
            Ok(false) => {}
            Ok(true) => {
                left.push((
                    o.survivor.id.clone(),
                    format!(
                        "its file changed since the campaign that found it, at {}: `nunki \
                         mutants run` measures it again",
                        short(&o.found_at)
                    ),
                ));
                continue;
            }
            Err(why) => {
                left.push((
                    o.survivor.id.clone(),
                    format!(
                        "whether its file changed since {} could not be read: {}",
                        short(&o.found_at),
                        crate::text::one_line(&why)
                    ),
                ));
                continue;
            }
        }
        let size = serde_json::to_string(&name).map_or(usize::MAX, |n| n.len() + 1);
        if count.is_some_and(|n| taken.len() >= n)
            || bytes.saturating_add(size) > mutants::NAMES_MAX_BYTES
        {
            beyond += 1;
            continue;
        }
        bytes += size;
        taken.push(o.clone());
    }
    Measurable {
        taken,
        left,
        beyond,
    }
}

/// Whether `file` differs in the human's repository at `root` between
/// `from` and `to`, read without rename detection, and as a literal path.
fn file_changed(root: &Path, from: &str, to: &str, file: &str) -> Result<bool, String> {
    crate::git::run(
        root,
        &[
            "--literal-pathspecs",
            "diff",
            "--name-only",
            "--no-renames",
            from,
            to,
            "--",
            file,
        ],
    )
    .map(|out| !out.trim().is_empty())
    .map_err(|e| e.to_string())
}

/// What a hardening mission framed now on `base` takes from the debt (SPEC
/// 4.4, the debt): its survivors measurable by name at `base` as the human's
/// repository has it ([`measurable`]), less those the registry proposes as
/// equivalent — no test kills those, and `nunki mutants debt --ratify`
/// settles them — and no more than its campaign's budget of
/// `budget_minutes` measures at the project's pace ([`mutants::fits`]), so
/// that its gate 7 measures each of them rather than a sample. At the pace
/// on file now: one that rises before the campaign runs, or code the branch
/// adds, makes the union too big for the budget, and it is sampled like any
/// campaign's.
pub fn hardening(
    project: &Project,
    base: &str,
    budget_minutes: u32,
) -> Result<Measurable, OffMissionError> {
    let debt = read_debt(&project.hq_root)?;
    if debt.owed.is_empty() {
        return Err(OffMissionError::NothingOwed);
    }
    let commit = crate::git::head_of(&project.root, base).map_err(|_| OffMissionError::NoBase {
        root: project.root.clone(),
        base: base.to_string(),
    })?;
    // A registry that cannot be read cannot say a proposal no longer holds:
    // every proposal is then taken to hold, and left.
    let registry = crate::equivalences::read_locked(&project.hq_root).ok();
    let (proposed, rest): (Vec<Owed>, Vec<Owed>) =
        debt.owed.iter().cloned().partition(|o| {
            proposal(o).is_some() && registry.as_ref().is_none_or(|r| standing(r, o))
        });
    let judged = crate::run::judged(project, &crate::run::stack_of(project));
    let pace = mutants::read_pace(&project.hq_root).unwrap_or(mutants::DEFAULT_PACE);
    let mut measured = measurable(
        project,
        &judged,
        &rest,
        &commit,
        Some(mutants::fits(budget_minutes, pace) as usize),
    );
    measured.left.extend(proposed.into_iter().map(|o| {
        (
            o.survivor.id,
            "the registry proposes it equivalent: `nunki mutants debt --ratify` settles it"
                .to_string(),
        )
    }));
    if measured.taken.is_empty() {
        return Err(OffMissionError::NoneTaken {
            owed: debt.owed.len(),
            what: "can be taken by a hardening mission",
            why: measured.left_said(),
        });
    }
    Ok(measured)
}

/// The survivors [`hardening`] took, as a hardening mission's header freezes
/// them.
pub fn hardened(measured: &Measurable) -> Vec<crate::mission::Hardened> {
    measured
        .taken
        .iter()
        .map(|o| crate::mission::Hardened {
            id: o.survivor.id.clone(),
            file: o.survivor.file.clone(),
            line: o.survivor.line,
            end_line: o.survivor.end_line,
            description: o.survivor.description.clone(),
            found_at: o.found_at.clone(),
        })
        .collect()
}

/// The brief of a hardening mission, under its header: the one line it
/// holds — kill these survivors — what its gate 7 measures, and the
/// survivors, each named as its campaign names it.
pub fn hardening_brief(measured: &Measurable) -> String {
    let mut brief = String::from(
        "This is a hardening mission (SPEC 4.4, the debt): write the tests that kill the \
         survivors of the project's debt listed below.\n\n\
         Gate 7 runs them by their names, besides whatever this branch changes — a sample \
         of them, as of any campaign, if they no longer fit its budget — and passes at the \
         `standard` threshold. A name holds a line and a column, so:\n\n\
         - do not change the code these mutants replace, and do not move it: a survivor whose \
         name is no longer in the code makes the campaign measure nothing;\n\
         - add the tests where no line above a survivor moves — under `tests/`, or in a test \
         module at the end of a file.\n\n\
         A survivor no test can kill is equivalent: propose it, in one sentence, as for any \
         campaign.\n\n",
    );
    let left = measured.left.len() + measured.beyond;
    brief.push_str(&format!(
        "The survivors, {} of the {} the debt owes",
        measured.taken.len(),
        measured.taken.len() + left
    ));
    if left > 0 {
        brief.push_str(&format!(
            " — {left} left for a later mission or a campaign outside missions"
        ));
    }
    brief.push_str(":\n\n");
    for o in &measured.taken {
        brief.push_str(&format!(
            "- `{}` — {}, line {}, found at {}\n",
            crate::text::name(&o.survivor.id),
            crate::text::name(&o.survivor.file),
            o.survivor.line,
            short(&o.found_at)
        ));
    }
    brief
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
    let judged = crate::run::judged(project, stack);
    // By name: the survivors measurable at the base's commit, their files,
    // and that commit itself as the fork point — the diff since it is empty,
    // so the script runs the names and nothing its scope reaches.
    let owed = match asked.form {
        Form::Owed => {
            if debt.owed.is_empty() {
                return Err(OffMissionError::NothingOwed);
            }
            let measured = measurable(project, &judged, &debt.owed, &commit, None);
            if measured.taken.is_empty() {
                return Err(OffMissionError::NoneTaken {
                    owed: debt.owed.len(),
                    what: "can be measured by name",
                    why: measured.left_said(),
                });
            }
            Some(measured)
        }
        Form::Whole | Form::SinceLast => None,
    };
    let since = match asked.form {
        Form::Whole | Form::Owed => None,
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
    let named: Vec<Survivor> = owed
        .iter()
        .flat_map(|m| m.taken.iter().map(|o| o.survivor.clone()))
        .collect();
    let paths = match (&since, &owed) {
        (_, Some(_)) => {
            let mut files: Vec<String> = named.iter().map(|s| s.file.clone()).collect();
            files.sort();
            files.dedup();
            files
        }
        (None, None) => git.names(&["ls-tree", "-r", "-z", "--name-only", &commit])?,
        (Some(since), None) => {
            git.names(&["diff", "--name-only", "-z", "--no-renames", since, &commit])?
        }
    };
    // On the whole code, no fork point: the script reads the paths as whole
    // files. On what changed since, the **empty tree**: every line of a file
    // handed counts as changed, so each is mutated whole ([`settled`]) —
    // measured on cargo-mutants 27.1.0, `--in-diff` against it lists the
    // same mutants as `--file` — and a file removed, or with nothing to
    // mutate, is a measurement of nothing: with no fork point, `--file` on
    // either ends with no `outcomes.json`, which the script reads as a
    // campaign that could not run, and the next one would measure the same
    // range again, for ever.
    let fork = match (&since, &owed) {
        (_, Some(_)) => commit.clone(),
        (None, None) => String::new(),
        (Some(_), None) => git.run_with_input(&["hash-object", "-t", "tree", "--stdin"], b"")?,
    };
    let names = mutants::names_by_stack(&judged, &named).map_err(MutantsError::Launch)?;

    crate::exec::refresh_to(
        project,
        slot,
        engine.clone(),
        &format!("refs/remotes/origin/{}", asked.base),
        &commit,
    )?;
    // The scripts in the home said they measure by name; the copy's are
    // what runs, and are asked again.
    if let Some(why) = mutants::unnamed(project, slot, engine.clone(), &judged, &names)? {
        return Err(MutantsError::Launch(why).into());
    }
    let id = campaign_id(&commit);
    // No budget, so no sample: this is where the whole is measured.
    let launched =
        mutants::command_naming(&judged, &id, &paths, &fork, project.config.jobs(), &names);
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
                changed: since.as_ref().map(|_| paths.clone()),
                owed: owed
                    .as_ref()
                    .map(|_| named.iter().map(|s| s.id.clone()).collect()),
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
        owed: owed.as_ref().map(|m| Owing {
            named: m.taken.len(),
            left: m.left.len() + m.beyond,
            why: (!m.left.is_empty()).then(|| m.left_said()),
        }),
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
///
/// Under the debt's lock, taken **before** the record is forgotten: when the
/// HQ is ruling on the debt, the campaign stays filed and the next `nunki
/// mutants status` reads it back, rather than being forgotten unrecorded.
/// The record is read again under it, and `None` when it is not `running`
/// any more: another `status` read it back first — twice into the debt
/// would undo a ruling given between the two — and the record now filed,
/// if any, is a later campaign's, which forgetting would lose. `nunki
/// mutants run` files none while this one is filed, so it cannot change
/// once read under the lock.
pub fn read_ended(
    project: &Project,
    slot: &crate::slot::Slot,
    running: &Running,
) -> Result<Option<Progress>, MutantsError> {
    let _lock = lock_debt(&project.hq_root, "mutants status").map_err(MutantsError::DebtLocked)?;
    if mutants::read_running(&project.hq_root, &slot.name)?.as_ref() != Some(running) {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&running.log).unwrap_or_default();
    mutants::forget_running(&project.hq_root, &slot.name)?;
    let Some(off) = &running.off_mission else {
        return Err(MutantsError::MissionCampaign {
            slot: slot.name.clone(),
        });
    };
    if !mutants::completed(&text) {
        let stderr = running.log.with_extension("err");
        return Ok(Some(Progress::CouldNotRun(format!(
            "it stopped without saying it had finished, after {} line(s), and the debt is \
             left as it was. What it said is in {}",
            text.lines().count(),
            stderr.display()
        ))));
    }
    if let Some(why) = mutants::unmeasured(&text) {
        return Ok(Some(Progress::CouldNotRun(format!(
            "{why}; the debt is left as it was"
        ))));
    }
    let mut found = mutants::parse(&text);
    if let Ok(registry) = crate::equivalences::read_locked(&project.hq_root) {
        crate::equivalences::apply(&registry, &mut found, |s| {
            crate::equivalences::identify(&slot.tree, &off.commit, s).ok()
        });
    }
    let count = found.len();
    let debt = match &off.owed {
        Some(named) => settled_owed(read_debt(&project.hq_root)?, found, &off.commit, named),
        None => settled(
            read_debt(&project.hq_root)?,
            found,
            Measured {
                base: off.base.clone(),
                commit: off.commit.clone(),
                date: crate::state::now_rfc3339(),
                whole: off.since.is_none(),
                tried: mutants::tried(&text),
            },
            off.changed.as_deref(),
        ),
    };
    write_debt(&project.hq_root, &debt)?;
    Ok(Some(Progress::Finished {
        survivors: count,
        chain: vec![debt_said(&debt)],
    }))
}

/// The debt in one line: how many survivors it owes, and from which
/// campaign.
pub fn debt_said(debt: &Debt) -> String {
    match &debt.last {
        None => "no campaign outside missions has completed on this project".to_string(),
        Some(last) => format!(
            "{} survivor(s) owed{}; the last campaign outside missions ran on {} at {}, {}, \
             tried {}",
            debt.owed.len(),
            match debt.owed.iter().filter(|o| proposal(o).is_some()).count() {
                0 => String::new(),
                n => format!(", {n} of them proposed equivalent by the registry"),
            },
            last.base,
            short(&last.commit),
            if last.whole {
                "on the whole code"
            } else {
                "on the files changed since the one before"
            },
            last.tried
                .map_or_else(|| "a number not said".to_string(), |t| t.to_string())
        ),
    }
}

/// `nunki mutants debt`: one paragraph per survivor owed — its name, the
/// commit that found it, the registry's proposal on it when there is one,
/// and when it shares its mutation with another survivor of the debt, which
/// the registry cannot tell apart — then the debt in one line.
///
/// A proposal whose entry `registry` no longer holds is said to be one
/// `--ratify` refuses ([`crate::equivalences::ratified_owed`]); with no
/// registry read (`None`), that it cannot be ratified now.
pub fn listing(debt: &Debt, registry: Option<&crate::equivalences::Registry>) -> String {
    let key = |s: &Survivor| (s.file.clone(), s.description.clone());
    let mut out = String::new();
    for owed in &debt.owed {
        let s = &owed.survivor;
        out.push_str(&crate::text::name(&s.id));
        out.push('\n');
        let lines = match s.span() {
            Some((start, end)) if start == end => format!("line {start}"),
            Some((start, end)) => format!("lines {start} to {end}"),
            None => format!("line {}, span not known", s.line),
        };
        out.push_str(&format!("  found at {}, {lines}\n", short(&owed.found_at)));
        if let Some(p) = proposal(owed) {
            out.push_str(&format!(
                "  proposed equivalent, {} — {}\n",
                p.from.as_ref().map(|f| f.said()).unwrap_or_default(),
                crate::text::one_line(&p.why)
            ));
            match registry {
                Some(registry) if standing(registry, owed) => {}
                Some(_) => out.push_str(
                    "  its entry was lifted or replaced since, so it cannot be ratified: \
                     `--equivalent` rules on it afresh\n",
                ),
                None => {
                    out.push_str("  the registry cannot be read, so it cannot be ratified now\n")
                }
            }
        }
        let twins = debt
            .owed
            .iter()
            .filter(|o| key(&o.survivor) == key(s))
            .count();
        if twins > 1 {
            out.push_str(&format!(
                "  {twins} survivors of the debt share this mutation: the registry cannot tell \
                 them apart, so only a test settles them\n"
            ));
        }
    }
    out.push_str(&format!("debt  {}", debt_said(debt)));
    out
}

/// The registry's proposal on a survivor of the debt, when the campaign
/// that found it matched it to an entry ([`read_ended`]).
fn proposal(owed: &Owed) -> Option<mutants::Proposal> {
    match &owed.survivor.outcome {
        Some(mutants::Triage::ProposedByNunki {
            why,
            from: from @ mutants::ProposedFrom::Registry { .. },
        }) => Some(mutants::Proposal {
            id: owed.survivor.id.clone(),
            file: owed.survivor.file.clone(),
            line: owed.survivor.line,
            why: why.clone(),
            from: Some(from.clone()),
        }),
        _ => None,
    }
}

/// Whether the registry still holds the entry the proposal on `owed` came
/// from, as it was when the proposal was made
/// ([`crate::equivalences::holds`]).
fn standing(registry: &crate::equivalences::Registry, owed: &Owed) -> bool {
    crate::equivalences::holds(registry, &owed.survivor)
}

/// The registry's proposals on `debt` that can be ratified, in the debt's
/// order: those whose entry `registry` still holds, on a survivor that
/// shares its mutation with no other survivor of the debt — two the
/// registry cannot tell apart, which only a test settles.
fn pending_in(debt: &Debt, registry: &crate::equivalences::Registry) -> Vec<mutants::Proposal> {
    let key = |s: &Survivor| (s.file.clone(), s.description.clone());
    debt.owed
        .iter()
        .filter(|o| standing(registry, o))
        .filter(|o| {
            debt.owed
                .iter()
                .filter(|t| key(&t.survivor) == key(&o.survivor))
                .count()
                == 1
        })
        .filter_map(proposal)
        .collect()
}

/// The registry's proposals the debt can ratify, in its order, for
/// `--ratify --all` to print before it ratifies them: those whose entry the
/// registry still holds. An unreadable registry is an error, never an empty
/// list.
pub fn pending(project: &Project) -> Result<Vec<mutants::Proposal>, OffMissionError> {
    let registry = crate::equivalences::read(&project.hq_root)?;
    Ok(pending_in(&read_debt(&project.hq_root)?, &registry))
}

/// `nunki mutants debt --equivalent <survivor> --because <why>`: the HQ
/// rules a survivor of the debt equivalent, **in the registry of
/// equivalences**, which every mission's campaign reads; then, and only
/// then, it leaves the debt.
///
/// The registry first, failing closed: when it cannot take the ruling —
/// the debt holds another survivor on the same file and description, the
/// code the mutation replaces occurs more than once in its file or cannot
/// be read at the commit that found it, the registry cannot be locked, read
/// or written — nothing is ruled, the debt is left as it was, and the error
/// says why. On a mission the ruling would still stand on the mission;
/// outside missions, nothing else would hold it.
pub fn rule_equivalent(project: &Project, id: &str, why: &str) -> Result<(), OffMissionError> {
    if crate::text::blank(why) {
        return Err(OffMissionError::Blank);
    }
    let _lock = lock_debt(&project.hq_root, "mutants debt --equivalent")
        .map_err(OffMissionError::Locked)?;
    let debt = read_debt(&project.hq_root)?;
    let at = debt
        .owed
        .iter()
        .position(|o| o.survivor.id == id)
        .ok_or_else(|| OffMissionError::NotOwed(id.to_string()))?;
    let found_at = debt.owed[at].found_at.clone();
    // The debt's survivors as the registry is to read them: the one ruled
    // holding the HQ's `equivalent`, every other as it is.
    let mut owed: Vec<Survivor> = debt.owed.iter().map(|o| o.survivor.clone()).collect();
    owed[at].outcome = Some(mutants::Triage::Equivalent {
        why: why.to_string(),
        carried_from: None,
    });
    let by = crate::human::me(&project.nunki_home(), Some(&project.root)).addressed();
    let owing = crate::equivalences::Owing {
        hq_root: &project.hq_root,
        root: &project.root,
        by: &by,
    };
    let registered =
        crate::equivalences::enter_owed(&owing, &owed, id, &found_at, &campaign_id(&found_at));
    taken_out(project, &debt, &[id], registered)
}

/// `nunki mutants debt --ratify <survivor>`: the HQ ratifies the registry's
/// proposal on a survivor of the debt. The entry it came from keeps its
/// origin and records the ratification beside it, as on a mission. When that
/// entry is not there any more as it was, or the survivor shares its
/// mutation with another of the debt, nothing is ratified
/// ([`crate::equivalences::ratified_owed`]). Then it leaves the debt.
/// Returns the sentence.
pub fn ratify(project: &Project, id: &str) -> Result<String, OffMissionError> {
    let _lock =
        lock_debt(&project.hq_root, "mutants debt --ratify").map_err(OffMissionError::Locked)?;
    let debt = read_debt(&project.hq_root)?;
    let mut said = ratified_by(project, &debt, &[id])?;
    Ok(said.remove(0))
}

/// `nunki mutants debt --ratify --all`: `listed` — the proposals the verb
/// printed, read by [`pending`] — ratified as [`ratify`] ratifies one, all
/// in **one** write of the registry and one of the debt, under one hold of
/// the debt's lock: what was printed is what is ruled, or nothing is. A
/// proposal that is not pending any more as it was listed fails the verb
/// before anything is written.
pub fn ratify_all(
    project: &Project,
    listed: &[mutants::Proposal],
) -> Result<Vec<String>, OffMissionError> {
    let _lock = lock_debt(&project.hq_root, "mutants debt --ratify --all")
        .map_err(OffMissionError::Locked)?;
    let debt = read_debt(&project.hq_root)?;
    let now = pending_in(&debt, &crate::equivalences::read(&project.hq_root)?);
    if let Some(gone) = listed.iter().find(|p| !now.contains(p)) {
        return Err(OffMissionError::Changed(gone.id.clone()));
    }
    let ids: Vec<&str> = listed.iter().map(|p| p.id.as_str()).collect();
    ratified_by(project, &debt, &ids)?;
    Ok(ids.into_iter().map(str::to_string).collect())
}

/// The proposals on survivors `ids` of `debt` ratified in the registry in
/// one write, then those survivors taken out of the debt in one more; the
/// debt left as it was when the registry did not take them all. The debt's
/// lock is the caller's. Returns each one's sentence.
fn ratified_by(
    project: &Project,
    debt: &Debt,
    ids: &[&str],
) -> Result<Vec<String>, OffMissionError> {
    let mut said = Vec::new();
    let mut campaigns = Vec::new();
    for id in ids {
        let owed = debt
            .owed
            .iter()
            .find(|o| o.survivor.id == *id)
            .ok_or_else(|| OffMissionError::NotOwed(id.to_string()))?;
        let p = proposal(owed).ok_or_else(|| OffMissionError::NoProposal(id.to_string()))?;
        said.push(p.why);
        campaigns.push((*id, owed.found_at.as_str(), campaign_id(&owed.found_at)));
    }
    let each: Vec<crate::equivalences::Ratifying> = campaigns
        .iter()
        .map(|(id, found_at, campaign)| crate::equivalences::Ratifying {
            id,
            found_at,
            campaign,
        })
        .collect();
    let survivors: Vec<Survivor> = debt.owed.iter().map(|o| o.survivor.clone()).collect();
    let by = crate::human::me(&project.nunki_home(), Some(&project.root)).addressed();
    let owing = crate::equivalences::Owing {
        hq_root: &project.hq_root,
        root: &project.root,
        by: &by,
    };
    let registered = crate::equivalences::ratified_owed(&owing, &survivors, &each);
    taken_out(project, debt, ids, registered)?;
    Ok(said)
}

/// Survivors `ids` taken out of `debt` once the registry took the ruling
/// on them — `registered` — and the debt left as it was when it did not.
/// The registry first: outside missions nothing else holds the ruling. A
/// debt that cannot be written then is said as such, with the ruling in
/// the registry: given again, the same command enters or records it once
/// more in its place, never beside it, and settles the debt.
fn taken_out(
    project: &Project,
    debt: &Debt,
    ids: &[&str],
    registered: crate::equivalences::Registered,
) -> Result<(), OffMissionError> {
    let named = || {
        ids.iter()
            .map(|id| format!("{:?}", id))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match registered {
        crate::equivalences::Registered::Done(n) if n > 0 => {}
        crate::equivalences::Registered::Done(_) => {
            return Err(OffMissionError::NotRegistered {
                ids: named(),
                why: "it entered nothing".to_string(),
            });
        }
        crate::equivalences::Registered::Not(why) => {
            return Err(OffMissionError::NotRegistered { ids: named(), why });
        }
    }
    let mut after = debt.clone();
    after
        .owed
        .retain(|o| !ids.contains(&o.survivor.id.as_str()));
    write_debt(&project.hq_root, &after).map_err(|e| OffMissionError::DebtNotWritten {
        ids: named(),
        why: e.to_string(),
    })
}

fn short(commit: &str) -> String {
    commit.chars().take(12).collect()
}
