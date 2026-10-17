//! The service definitions a human approved (SPEC 4.2, 4.5).
//!
//! A services file is in the repository, so whoever writes the slot's tree
//! writes it — the coder included. The closed model of
//! [`crate::compose::services`] keeps the power out of it; this module keeps
//! out what nobody looked at. What is approved is a **digest of nunki's
//! rendering**, never of the file's bytes, and a human approves it after
//! reading that rendering (`nunki services --show`). A definition approved
//! once serves every mission of the project until it changes, and a change
//! — a coder's edit included — starts nothing until a human approves it in
//! turn.
//!
//! The list lives in the project's HQ ([`FILE`]), which is never mounted: no
//! agent can approve its own edit. For a mission, the file is read at the
//! commit its branch starts from ([`At::Mission`]), through the host's
//! mirror, and never from the tree or from the branch's own commits: the
//! file is the human's, an agent's edit is refused at gate 4, and it is
//! never what a system profile lifts (#141).

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::check::{Check, Verdict};
use crate::compose::services::{
    LABEL, Lifted, RESERVED_SERVICES, ServicesError, ServicesFile, digest,
};
use crate::engine::Container;
use crate::git::GitError;
use crate::project::Project;

/// Where the approved digests live, inside the project's HQ.
pub const FILE: &str = "services.json";

#[derive(Debug, thiserror::Error)]
pub enum ApprovalError {
    #[error("nunki.yaml declares no services_file, so there is nothing to show or approve")]
    NoServicesFile,
    #[error(
        "nunki.yaml's services_file {0:?} is not a path inside the repository: a relative \
         path, without `..`"
    )]
    Path(PathBuf),
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(
        "{path} is not on {on}: nunki reads the services file from a commit, never from \
         the tree, so it has to be committed there"
    )]
    NotOnCommit { path: String, on: &'static str },
    #[error("{path} is not a regular file on {on} (mode {mode})")]
    NotAFile {
        path: String,
        on: &'static str,
        mode: String,
    },
    #[error("{path} is not UTF-8 text")]
    NotText { path: String },
    #[error("{path} is not a services file nunki lifts, so no service was started: {source}")]
    Refused {
        path: String,
        #[source]
        source: ServicesError,
    },
    #[error(
        "{path} renders to {digest}, which no human has approved, so no service was \
         started: `nunki services --show` prints it, and `nunki services --approve \
         {digest}` approves it"
    )]
    NotApproved { path: String, digest: String },
    #[error(
        "{digest} is not the digest of the services file as it stands now ({current}): \
         the file changed since it was shown. `nunki services --show` shows it again"
    )]
    Stale { digest: String, current: String },
    #[error("mission {0} has not started, so it has no slot to read the services file from")]
    NoMission(String),
    #[error(
        "the commit the mission's branch starts from cannot be read, so no services file \
         was read: {0}"
    )]
    NoForkPoint(String),
    #[error("the mission's slot cannot be found: {0}")]
    Slot(String),
    #[error("{0} could not be read: {1}")]
    Unreadable(PathBuf, #[source] std::io::Error),
    #[error("{0} is not a list of approvals nunki can read: {1}")]
    Corrupt(PathBuf, String),
    #[error("{0} could not be written: {1}")]
    Unwritable(PathBuf, #[source] std::io::Error),
    #[error("{0} is not a profile nunki wrote, so nothing was started: {1}")]
    NotAProfile(PathBuf, String),
    #[error(
        "{path} would start the service {service:?} from a definition no human approved — \
         a profile written before approvals, or from a rendering no longer approved — so \
         nothing was started. The next launch writes the profile again from the approved \
         services file; `nunki services --show` says what is approved"
    )]
    UnapprovedProfile { path: PathBuf, service: String },
}

/// Where this project's approvals live.
pub fn file(project: &Project) -> PathBuf {
    project.hq_root.join(FILE)
}

/// Every approval of a project.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approvals {
    #[serde(default)]
    pub approved: Vec<Approval>,
}

/// One rendering a human approved: its digest, who, when, and the rendering
/// itself, so the list says what was approved and not only that something
/// was.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approval {
    pub digest: String,
    pub by: String,
    pub at: String,
    pub rendering: String,
}

impl Approvals {
    /// The approvals in `path`, or none when the file does not exist yet.
    /// A file that exists and cannot be read approves nothing: an error, so
    /// that nothing is lifted on it.
    pub fn load(path: &Path) -> Result<Self, ApprovalError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(ApprovalError::Unreadable(path.to_path_buf(), e)),
        };
        serde_json::from_str(&text)
            .map_err(|e| ApprovalError::Corrupt(path.to_path_buf(), e.to_string()))
    }

    /// The approval of `digest`, if a human gave one.
    pub fn find(&self, digest: &str) -> Option<&Approval> {
        self.approved.iter().find(|a| a.digest == digest)
    }

    fn save(&self, path: &Path) -> Result<(), ApprovalError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| ApprovalError::Unwritable(path.to_path_buf(), e))?;
        }
        let text = serde_json::to_string_pretty(self).expect("approvals serialise to JSON");
        let staged = path.with_extension("json.new");
        std::fs::write(&staged, format!("{text}\n"))
            .and_then(|()| std::fs::rename(&staged, path))
            .map_err(|e| ApprovalError::Unwritable(path.to_path_buf(), e))
    }
}

/// The services file as a commit holds it, read into the model and
/// rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Current {
    /// The file's path in the repository, as `nunki.yaml` names it.
    pub path: String,
    pub services: ServicesFile,
    pub rendering: String,
    pub digest: String,
}

/// Where a services file is read from: always a commit, never a tree.
#[derive(Debug, Clone, Copy)]
pub enum At<'a> {
    /// The commit a slot's `HEAD` names, read through the host's mirror.
    Slot(&'a Path),
    /// The commit `HEAD` names in the human's own repository.
    Repository(&'a Path),
    /// The commit a mission's branch, checked out in the slot at `tree`,
    /// starts from: where `HEAD` meets the base `base` as the host recorded
    /// it ([`crate::gate::fork_point`]). What a system profile lifts.
    ///
    /// Not the branch's `HEAD`, which holds whatever an agent committed: an
    /// edit that gate 4 refuses would otherwise stop the next system
    /// profile on a rendering no human approved, hours into a mission. The
    /// fork point is an ancestor of the host's record, so it is a commit of
    /// the human's base, and its rendering still needs a human's approval.
    /// And no fallback to `HEAD` when the record is missing: that is the
    /// read this replaces.
    Mission { tree: &'a Path, base: &'a str },
}

/// The project's services file at `at`, rendered, or `None` when
/// `nunki.yaml` declares none.
pub fn read(project: &Project, at: At) -> Result<Option<Current>, ApprovalError> {
    let Some(declared) = &project.config.services_file else {
        return Ok(None);
    };
    let path = repository_path(declared)?;
    let blob = match at {
        At::Slot(tree) => crate::git::SlotGit::open(tree)?.blob_at_head(&path)?,
        At::Repository(root) => crate::git::blob_at_head(root, &path)?,
        At::Mission { tree, base } => {
            let fork = crate::gate::fork_point(tree, base)
                .map_err(|e| ApprovalError::NoForkPoint(e.to_string()))?;
            crate::git::SlotGit::open(tree)?.blob_at(&fork, &path)?
        }
    };
    let on = match at {
        At::Slot(_) => "the commit the slot's HEAD names",
        At::Repository(_) => "the commit HEAD names in the repository",
        At::Mission { .. } => "the commit the mission's branch starts from, on its base",
    };
    let Some(blob) = blob else {
        return Err(ApprovalError::NotOnCommit { path, on });
    };
    if !blob.is_file() {
        return Err(ApprovalError::NotAFile {
            path,
            on,
            mode: blob.mode,
        });
    }
    let Ok(text) = String::from_utf8(blob.bytes) else {
        return Err(ApprovalError::NotText { path });
    };
    let services = ServicesFile::parse(&text).map_err(|source| ApprovalError::Refused {
        path: path.clone(),
        source,
    })?;
    let rendering = services.render();
    Ok(Some(Current {
        path,
        digest: digest(&rendering),
        services,
        rendering,
    }))
}

/// The services a profile may lift from `at`: `None` when the project
/// declares no file, the model when a human approved its rendering, and
/// [`ApprovalError::NotApproved`] otherwise.
pub fn approved(project: &Project, at: At) -> Result<Option<ServicesFile>, ApprovalError> {
    let Some(current) = read(project, at)? else {
        return Ok(None);
    };
    if Approvals::load(&file(project))?
        .find(&current.digest)
        .is_none()
    {
        return Err(ApprovalError::NotApproved {
            path: current.path,
            digest: current.digest,
        });
    }
    Ok(Some(current.services))
}

/// What `nunki services --approve` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Approved {
    /// Added to the list now.
    Now(Approval),
    /// A human had approved it already.
    Already(Approval),
}

/// Approve `wanted`, only if it is the digest of a rendering as it stands
/// now in one of `current` — the file that was shown. A digest that is in
/// none of them is refused: the file changed since it was shown, and what is
/// approved has to be what a human read.
pub fn approve(
    project: &Project,
    wanted: &str,
    current: &[Current],
) -> Result<Approved, ApprovalError> {
    let path = file(project);
    let mut approvals = Approvals::load(&path)?;
    if let Some(approval) = approvals.find(wanted) {
        return Ok(Approved::Already(approval.clone()));
    }
    let Some(shown) = current.iter().find(|c| c.digest == wanted) else {
        let mut digests: Vec<&str> = current.iter().map(|c| c.digest.as_str()).collect();
        digests.sort_unstable();
        digests.dedup();
        return Err(ApprovalError::Stale {
            digest: wanted.to_string(),
            current: if digests.is_empty() {
                "no services file could be read".to_string()
            } else {
                digests.join(", ")
            },
        });
    };
    let human = crate::human::me(&project.nunki_home(), Some(&project.root));
    let unreadable = human.unreadable();
    let approval = Approval {
        digest: shown.digest.clone(),
        by: match (human.name, human.email) {
            (Some(name), Some(email)) => format!("{name} <{email}>"),
            (Some(name), None) => name,
            (None, _) => match unreadable {
                Some(why) => format!("a human nunki has no name for ({why})"),
                None => "a human nunki has no name for".to_string(),
            },
        },
        at: crate::state::now_rfc3339(),
        rendering: shown.rendering.clone(),
    };
    approvals.approved.push(approval.clone());
    approvals.save(&path)?;
    Ok(Approved::Now(approval))
}

/// What `nunki services --show` prints: where the file was read, its
/// digest, whether a human approved it, and the rendering, with anything a
/// terminal would act on escaped.
pub fn show(current: &Current, from: &str, approvals: &Approvals) -> String {
    let status = match approvals.find(&current.digest) {
        Some(a) => format!("approved by {} at {}", a.by, a.at),
        None => format!(
            "not approved: `nunki services --approve {}` approves it",
            current.digest
        ),
    };
    let mut out = format!(
        "{} on {}\ndigest {}\n{}\n",
        current.path,
        crate::text::one_line(from),
        current.digest,
        crate::text::one_line(&status)
    );
    let dropped = current.services.dropped_ports();
    if !dropped.is_empty() {
        out.push_str(&ports_dropped(&dropped));
        out.push('\n');
    }
    out.push_str("---\n");
    out.push_str(&crate::text::printable(&current.rendering));
    out
}

/// The containers to take down before a profile is started: every
/// container of the slot's Compose project backing a service nunki did not
/// write itself, whose definition is not the approved one or which the
/// profile does not lift.
///
/// `lifting` is what the profile lifts. A system profile lifts the services
/// its mission declares, under one approved digest, and every other
/// container goes — an older approved definition, an unlabelled one lifted
/// before approvals existed, a service the file no longer names, one the
/// mission does not declare. A mission profile lifts none, and leaves up
/// what a human approved: the services stay up between profiles (SPEC 4.2).
pub fn stale(
    containers: &[Container],
    lifting: Option<&Lifted>,
    approvals: &Approvals,
) -> Vec<String> {
    let labels = lifting.map(|lifted| {
        lifted
            .services
            .services
            .keys()
            .map(|name| (name.clone(), lifted.approved.clone()))
            .collect()
    });
    stale_labelled(containers, labels.as_ref(), approvals)
}

/// [`stale`], for a profile known by the label each service it lifts
/// carries — service name to digest — as [`labels_of_profile`] reads them
/// back from a profile already written.
pub fn stale_labelled(
    containers: &[Container],
    lifting: Option<&BTreeMap<String, String>>,
    approvals: &Approvals,
) -> Vec<String> {
    containers
        .iter()
        .filter(|c| !RESERVED_SERVICES.contains(&c.service.as_str()))
        .filter(|c| match (lifting, c.digest.as_deref()) {
            (Some(lifted), Some(label)) => {
                lifted.get(&c.service).map(String::as_str) != Some(label)
            }
            (None, Some(label)) => approvals.find(label).is_none(),
            (_, None) => true,
        })
        .map(|c| c.id.clone())
        .collect()
}

/// What the profile already written at `path` lifts from the project's
/// file, read back from it: each service but nunki's own, with the digest
/// its label carries. `None` when it lifts none — a mission profile, or a
/// project without a services file.
///
/// Every one of them has to carry the label of a rendering a human
/// approved, or the whole profile is refused: a profile written before
/// approvals existed lifts its services unlabelled and unfenced, and one
/// written from a rendering since withdrawn from the list is no longer
/// approved. Read for the paths that lift a profile without writing it
/// ([`crate::run::relift`]), so that none of them is a second way to lift.
pub fn labels_of_profile(
    path: &Path,
    approvals: &Approvals,
) -> Result<Option<BTreeMap<String, String>>, ApprovalError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| ApprovalError::Unreadable(path.to_path_buf(), e))?;
    let document: crate::yaml::Value = crate::yaml::from_str(&text)
        .map_err(|e| ApprovalError::NotAProfile(path.to_path_buf(), e.to_string()))?;
    let Some(services) = document.get("services").and_then(|s| s.as_mapping()) else {
        return Err(ApprovalError::NotAProfile(
            path.to_path_buf(),
            "it has no services".to_string(),
        ));
    };
    let mut labels = BTreeMap::new();
    for (name, service) in services {
        let name = name.as_str().unwrap_or_default();
        if RESERVED_SERVICES.contains(&name) {
            continue;
        }
        let label = service
            .get("labels")
            .and_then(|l| l.get(LABEL))
            .and_then(|l| l.as_str());
        match label {
            Some(label) if approvals.find(label).is_some() => {
                labels.insert(name.to_string(), label.to_string());
            }
            _ => {
                return Err(ApprovalError::UnapprovedProfile {
                    path: path.to_path_buf(),
                    service: name.to_string(),
                });
            }
        }
    }
    Ok((!labels.is_empty()).then_some(labels))
}
/// `nunki check`'s line on the services file of `at`: green when it is
/// approved or there is none, red otherwise, saying why.
pub fn check(project: &Project, at: At, whose: &str) -> Check {
    let what = format!("the services file on {whose} is one a human approved");
    let verdict = match read(project, at) {
        Ok(None) => Verdict::Green("the project declares no services file".to_string()),
        Ok(Some(current)) => match Approvals::load(&file(project)) {
            Ok(approvals) => match approvals.find(&current.digest) {
                Some(a) => Verdict::Green(format!(
                    "{} renders to {}, approved by {} at {}",
                    current.path, current.digest, a.by, a.at
                )),
                None => Verdict::Red(
                    ApprovalError::NotApproved {
                        path: current.path,
                        digest: current.digest,
                    }
                    .to_string(),
                ),
            },
            Err(e) => Verdict::Red(e.to_string()),
        },
        Err(e) => Verdict::Red(e.to_string()),
    };
    Check { what, verdict }
}

/// The services file's path in the repository, as gate 4 matches it: `None`
/// when the project declares none, or declares a path outside the
/// repository, which no commit can touch and which every read refuses on
/// its own ([`ApprovalError::Path`]).
pub fn declared_path(project: &Project) -> Option<String> {
    project
        .config
        .services_file
        .as_deref()
        .and_then(|p| repository_path(p).ok())
}

/// `services_file` as a path git names inside the repository.
fn repository_path(declared: &Path) -> Result<String, ApprovalError> {
    let mut parts = Vec::new();
    for component in declared.components() {
        match component {
            Component::Normal(part) => match part.to_str() {
                Some(part) => parts.push(part),
                None => return Err(ApprovalError::Path(declared.to_path_buf())),
            },
            Component::CurDir => {}
            _ => return Err(ApprovalError::Path(declared.to_path_buf())),
        }
    }
    if parts.is_empty() {
        return Err(ApprovalError::Path(declared.to_path_buf()));
    }
    Ok(parts.join("/"))
}

/// One place a services file was read from, and what was read there.
#[derive(Debug)]
pub struct Reading {
    /// Where, in words: the repository, or a slot and its mission.
    pub from: String,
    pub current: Result<Current, ApprovalError>,
}

/// The services file where `nunki services` reads it: where `mission`'s
/// system profile reads it when one is named ([`At::Mission`]); otherwise
/// the human's repository, and —
/// when `every_slot`, which is what an approval looks through — every slot
/// of the project as well, so a digest shown for any of them is one the
/// human can approve.
pub fn readings(
    project: &Project,
    mission: Option<&str>,
    every_slot: bool,
) -> Result<Vec<Reading>, ApprovalError> {
    if project.config.services_file.is_none() {
        return Err(ApprovalError::NoServicesFile);
    }
    let reading = |at: At, from: String| Reading {
        from,
        current: read(project, at).and_then(|c| c.ok_or(ApprovalError::NoServicesFile)),
    };
    if let Some(id) = mission {
        let (slot, base) = mission_slot(project, id)?;
        let from = format!(
            "the commit mission {id}'s branch starts from, in slot {}",
            slot.name
        );
        return Ok(vec![reading(
            At::Mission {
                tree: &slot.tree,
                base: &base,
            },
            from,
        )]);
    }
    let mut readings = vec![Reading {
        from: "the HEAD of the repository".to_string(),
        current: read(project, At::Repository(&project.root))
            .and_then(|c| c.ok_or(ApprovalError::NoServicesFile)),
    }];
    if every_slot {
        for slot in crate::slot::list(project) {
            let from = format!("the HEAD of slot {}", slot.name);
            readings.push(reading(At::Slot(&slot.tree), from));
        }
    }
    Ok(readings)
}

/// A recorded mission's slot, and the base its frozen header names.
fn mission_slot(project: &Project, id: &str) -> Result<(crate::slot::Slot, String), ApprovalError> {
    let state = crate::state::Store::open(&project.hq_root)
        .and_then(|s| s.load(id))
        .map_err(|_| ApprovalError::NoMission(id.to_string()))?;
    let slot =
        crate::slot::find(project, &state.slot).map_err(|e| ApprovalError::Slot(e.to_string()))?;
    Ok((slot, state.flow.header().base.clone()))
}

/// `nunki check`'s lines on the services files a check names: the HEAD of
/// the slot `--slot` names, and where the system profile of the mission
/// `--mission` names reads it. The same refusal a launch would meet, said as
/// a red line.
pub fn checks(project: &Project, slot: Option<&str>, mission: Option<&str>) -> Vec<Check> {
    let mut checks = Vec::new();
    let not_checked = |whose: &str, why: String| Check {
        what: format!("the services file on {whose} is one a human approved"),
        verdict: Verdict::NotChecked(why),
    };
    if let Some(name) = slot {
        let whose = format!("slot {name}'s HEAD");
        checks.push(match crate::slot::find(project, name) {
            Ok(slot) => check(project, At::Slot(&slot.tree), &whose),
            Err(e) => not_checked(&whose, e.to_string()),
        });
    }
    if let Some(id) = mission {
        let whose = format!("the commit mission {id}'s branch starts from");
        checks.push(match mission_slot(project, id) {
            Ok((slot, base)) => check(
                project,
                At::Mission {
                    tree: &slot.tree,
                    base: &base,
                },
                &whose,
            ),
            Err(e) => not_checked(&whose, e.to_string()),
        });
    }
    checks
}

/// The one line that says a launch, or a rendering, dropped the `ports` of
/// `services`, and why.
pub fn ports_dropped<S: AsRef<str>>(services: &[S]) -> String {
    format!(
        "ports dropped from {}: the agent reaches a service by its name, and a published \
         port would collide between slots",
        services
            .iter()
            .map(|s| s.as_ref())
            .collect::<Vec<_>>()
            .join(", ")
    )
}
