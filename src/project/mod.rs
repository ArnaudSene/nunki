//! A project `hq` can orchestrate: where it lives, and what it declares
//! (SPEC 4.1, `hq.yaml`).
//!
//! Two roots, and they are never the same place. The **repository** is the
//! human's, and `hq` writes almost nothing in it (SPEC 3.3). The **HQ** is
//! `~/.hq/<project>/`: journal, dashboard, state, missions — everything
//! `hq` owns lives there, outside the tree an agent can write.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::mission::Bounds;

/// The file at the repository root. What it declares beats a stack default;
/// a mission header beats it in turn (SPEC 4.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// Which harness runs the agents.
    pub harness: String,
    /// The project's forge. No agent may reach it: the clone's origin is
    /// unreachable, and an allowlist naming the forge would undo that
    /// (SPEC 3.1, 4.1 bis rule 6).
    #[serde(default)]
    pub forge: Vec<String>,
    /// Stack fragments this project uses, under `.hq/stacks/<name>/`.
    #[serde(default)]
    pub stacks: Vec<String>,
    /// Branches no mission may target or push to.
    #[serde(default = "default_protected_branches")]
    pub protected_branches: Vec<String>,
    #[serde(default)]
    pub protected_paths: ProtectedPaths,
    /// The account this project's missions spend unless one says otherwise.
    #[serde(default)]
    pub account: Option<String>,
    #[serde(default)]
    pub bounds: Bounds,
    /// Where the test credentials live, mounted read-only on a system
    /// profile and never anywhere else (SPEC 3.1).
    #[serde(default)]
    pub credentials: Option<PathBuf>,
    /// How the application is started, overriding the stack's `run.sh`.
    /// `none` for a library (SPEC 4.2).
    #[serde(default)]
    pub run: Option<String>,
}

fn default_protected_branches() -> Vec<String> {
    vec!["main".to_string(), "master".to_string(), "dev".to_string()]
}

/// The two modes of SPEC 4.1. There is no "ask": nothing can ask in an
/// autonomous run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtectedPaths {
    /// Refused outright.
    #[serde(default)]
    pub refuse: Vec<String>,
    /// Refused only where the file already exists on the base.
    #[serde(default)]
    pub refuse_if_exists: Vec<String>,
}

/// The file `hq init` writes and every verb reads.
pub const CONFIG_FILE: &str = "hq.yaml";
/// Where a project's stack fragments live, inside the project.
pub const FRAGMENTS_DIR: &str = ".hq";

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("no {CONFIG_FILE} in {0} or any directory above it — run `hq init` first")]
    NotAProject(PathBuf),
    #[error("{0} could not be read: {1}")]
    Unreadable(PathBuf, String),
    #[error("{0} is not valid: {1}")]
    Invalid(PathBuf, String),
    #[error("no home directory: the HQ lives under ~/.hq")]
    NoHome,
}

/// An opened project: the repository, its configuration, and its HQ.
#[derive(Debug, Clone)]
pub struct Project {
    pub root: PathBuf,
    pub config: Config,
    pub hq_root: PathBuf,
}

impl Project {
    /// Find the project containing `start`, the way git finds a repository:
    /// by walking up until the file is there.
    pub fn open(start: &Path) -> Result<Self, ProjectError> {
        let root =
            Self::find_root(start).ok_or_else(|| ProjectError::NotAProject(start.to_path_buf()))?;
        let file = root.join(CONFIG_FILE);
        let text = std::fs::read_to_string(&file)
            .map_err(|e| ProjectError::Unreadable(file.clone(), e.to_string()))?;
        let config: Config = serde_yaml_ng::from_str(&text)
            .map_err(|e| ProjectError::Invalid(file.clone(), e.to_string()))?;
        let hq_root = Self::hq_root_for(&root)?;
        Ok(Self {
            root,
            config,
            hq_root,
        })
    }

    /// Open a project rooted exactly here, with its HQ somewhere chosen — the
    /// form tests use, and the one `hq init` uses before a HQ exists.
    pub fn at(root: PathBuf, config: Config, hq_root: PathBuf) -> Self {
        Self {
            root,
            config,
            hq_root,
        }
    }

    /// `~/.hq/` — the directory every project's HQ sits under, and where
    /// accounts live. Derived from the HQ rather than from `HOME`, so a test
    /// or a second machine can point the whole thing elsewhere by saying so
    /// once.
    pub fn hq_home(&self) -> PathBuf {
        self.hq_root
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.hq_root.clone())
    }

    pub fn name(&self) -> String {
        self.root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "project".to_string())
    }

    /// The stack fragment directory for `stack`, whether or not it exists.
    pub fn fragment(&self, stack: &str) -> PathBuf {
        self.root.join(FRAGMENTS_DIR).join("stacks").join(stack)
    }

    /// The domains a stack declares for its dependencies, if it declares any.
    pub fn stack_domains(&self, stack: &str) -> Option<Vec<String>> {
        let file = self.fragment(stack).join("allow.txt");
        let text = std::fs::read_to_string(file).ok()?;
        Some(
            text.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(str::to_string)
                .collect(),
        )
    }

    fn find_root(start: &Path) -> Option<PathBuf> {
        let mut here = if start.is_dir() {
            start.to_path_buf()
        } else {
            start.parent()?.to_path_buf()
        };
        loop {
            if here.join(CONFIG_FILE).is_file() {
                return Some(here);
            }
            if !here.pop() {
                return None;
            }
        }
    }

    fn hq_root_for(root: &Path) -> Result<PathBuf, ProjectError> {
        let home = std::env::var_os("HOME").ok_or(ProjectError::NoHome)?;
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "project".to_string());
        Ok(PathBuf::from(home).join(".hq").join(name))
    }
}
