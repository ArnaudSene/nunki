//! Slots: a local clone, its volumes, and one agent container at a time
//! (SPEC 4.2, "slots et branches").
//!
//! A slot is where a mission happens. Three properties make it a slot rather
//! than a copy of the repository:
//!
//! 1. **The clone has no hard links.** A local clone shares its `.git`
//!    objects with the repository it came from, and a raw write in the
//!    container corrupts both (SPEC 3.2, decided after a review found it).
//! 2. **Its `origin` is a host path that does not exist in the container.**
//!    That is what makes pushing impossible without relying on any harness.
//! 3. **It is created beside the repository**, never on another volume: a
//!    slot on a different filesystem is a slot the engine may not share
//!    (SPEC 4.2 bis).

use std::path::{Path, PathBuf};

use crate::git::{self, GitError};
use crate::project::Project;

/// Where a project's slots live: a sibling of the repository, so the clone
/// stays on the same filesystem.
pub fn slots_dir(project: &Project) -> PathBuf {
    let parent = project
        .root
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    parent.join(format!("{}-slots", project.name()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    pub name: String,
    pub tree: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum SlotError {
    #[error("slot {0:?} already exists at {1}")]
    Exists(String, PathBuf),
    #[error("no slot {0:?} under {1}")]
    Unknown(String, PathBuf),
    #[error(
        "slot {name:?} carries {count} commit(s) on {branch} that {repository} does not have; \
         `nunki mission fetch` first, or say so explicitly"
    )]
    Unfetched {
        name: String,
        branch: String,
        count: usize,
        repository: String,
    },
    #[error("slot {0:?} has uncommitted work; commit it or discard it first")]
    Dirty(String),
    #[error("a slot name may hold letters, digits, - and _ only, and {0:?} does not")]
    BadName(String),
    #[error(transparent)]
    Git(#[from] GitError),
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
}

/// Create a slot: a clone of the project, without hard links.
pub fn add(project: &Project, name: &str) -> Result<Slot, SlotError> {
    check_name(name)?;
    let dir = slots_dir(project);
    let tree = dir.join(name);
    if tree.exists() {
        return Err(SlotError::Exists(name.to_string(), tree));
    }
    std::fs::create_dir_all(&dir).map_err(|e| SlotError::Io(dir.clone(), e))?;

    // `--no-hardlinks` is the whole point: without it the clone shares its
    // objects with the repository, and a raw write inside a container
    // corrupts the human's history along with the slot's.
    git::run_anywhere(&[
        "clone",
        "--no-hardlinks",
        &project.root.display().to_string(),
        &tree.display().to_string(),
    ])?;

    Ok(Slot {
        name: name.to_string(),
        tree,
    })
}

/// The slots a project has, in name order.
pub fn list(project: &Project) -> Vec<Slot> {
    let dir = slots_dir(project);
    let mut slots: Vec<Slot> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join(".git").exists())
        .map(|e| Slot {
            name: e.file_name().to_string_lossy().into_owned(),
            tree: e.path(),
        })
        .collect();
    slots.sort_by(|a, b| a.name.cmp(&b.name));
    slots
}

pub fn find(project: &Project, name: &str) -> Result<Slot, SlotError> {
    list(project)
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| SlotError::Unknown(name.to_string(), slots_dir(project)))
}

/// Remove a slot — and refuse while it holds work the repository does not
/// have (SPEC 3.3, rule 2). `force` is the human saying it anyway.
/// Put a slot back to a clean state without destroying it.
///
/// What it removes, and what it deliberately does not. The **clone stays**:
/// `nunki slot rm` is the verb that deletes one, and a reset that quietly did
/// the same would be a name lying about a destructive act. What goes is the
/// work in progress — uncommitted changes, untracked files — and the slot's
/// **named volumes**, which is the real reason to reset: a build cache or a
/// harness state directory that has gone bad outlives every rebuild of the
/// image, and nothing else can reach it.
///
/// It refuses on the same grounds as `rm`, and for the same reason: commits
/// the repository does not have are work nobody else holds, and a verb that
/// discarded them because its name sounds mild would be the worst kind of
/// verb (SPEC 3.3).
pub fn reset(
    project: &Project,
    name: &str,
    engine_bin: &str,
    force: bool,
) -> Result<Reset, SlotError> {
    let slot = find(project, name)?;
    if !force {
        let missing = git::commits_not_in(&slot.tree, &project.root)?;
        if !missing.is_empty() {
            return Err(SlotError::Unfetched {
                name: name.to_string(),
                branch: git::current_branch(&slot.tree)?,
                count: missing.len(),
                repository: project.root.display().to_string(),
            });
        }
    }

    let discarded = !git::is_clean(&slot.tree)?;
    git::run(&slot.tree, &["reset", "--hard", "--quiet", "HEAD"])?;
    // `-x` here, unlike the refresh of the proof copy: that one keeps the
    // build cache on purpose, and this one exists to throw it away.
    git::run(&slot.tree, &["clean", "-qxdff"])?;

    let mut volumes = Vec::new();
    for volume in volumes_of(project, &slot) {
        let out = std::process::Command::new(engine_bin)
            .args(["volume", "rm", "-f", &volume])
            .output();
        // A volume that was never created is not an error: a slot reset
        // before its first run has none, and saying so as a failure would
        // make the ordinary case look broken.
        if out.map(|o| o.status.success()).unwrap_or(false) {
            volumes.push(volume);
        }
    }
    Ok(Reset {
        slot,
        discarded,
        volumes,
    })
}

/// What a reset threw away, for the caller to print.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reset {
    pub slot: Slot,
    /// Whether the tree had uncommitted work.
    pub discarded: bool,
    /// The named volumes that were actually removed.
    pub volumes: Vec<String>,
}

/// Every named volume a slot owns, whichever profile put it there.
///
/// Listed here rather than derived from a profile file: a profile is
/// regenerated at every launch and may not exist at all, and a reset must be
/// able to clean a slot whose last profile is gone.
pub fn volumes_of(project: &Project, slot: &Slot) -> Vec<String> {
    let mut names = vec![
        crate::exec::proof_volume(&slot.name),
        format!("nunki-{}-harness", slot.name),
        format!("nunki-{}-cargo", slot.name),
    ];
    for stack in &project.config.stacks {
        for path in project.stack_writable(stack) {
            names.push(crate::run::writable_volume(&slot.name, &path));
        }
    }
    names.sort();
    names.dedup();
    names
}

pub fn rm(project: &Project, name: &str, force: bool) -> Result<Slot, SlotError> {
    let slot = find(project, name)?;
    if !force {
        if !git::is_clean(&slot.tree)? {
            return Err(SlotError::Dirty(name.to_string()));
        }
        let missing = git::commits_not_in(&slot.tree, &project.root)?;
        if !missing.is_empty() {
            return Err(SlotError::Unfetched {
                name: name.to_string(),
                branch: git::current_branch(&slot.tree)?,
                count: missing.len(),
                repository: project.root.display().to_string(),
            });
        }
    }
    std::fs::remove_dir_all(&slot.tree).map_err(|e| SlotError::Io(slot.tree.clone(), e))?;
    Ok(slot)
}

/// A name that survives being a directory, a Compose project and a container
/// name (SPEC 4.2: Compose refuses anything but lowercase letters, digits,
/// `-` and `_`).
fn check_name(name: &str) -> Result<(), SlotError> {
    let ok = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric());
    if ok {
        Ok(())
    } else {
        Err(SlotError::BadName(name.to_string()))
    }
}
