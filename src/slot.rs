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
         `hq mission fetch` first, or say so explicitly"
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
