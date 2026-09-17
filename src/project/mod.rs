//! A project `nunki` can orchestrate: where it lives, and what it declares
//! (SPEC 4.1, `nunki.yaml`).
//!
//! Two places, and they are never the same. The **repository** is the
//! human's, and `nunki` writes almost nothing in it: `AGENTS.md`, the import
//! that makes a harness read it, and a `.gitattributes` when none exists
//! (SPEC 3.3). Development tooling is not a project's to carry in its history.
//!
//! Everything else lives in the project's **home**, `~/.nunki/<project>/`,
//! laid out so that the separation can be seen:
//!
//! - `nunki.yaml`, the project's configuration, which names the repository
//!   it belongs to;
//! - `hq/`, the HQ — state, locks, missions, profiles. Never mounted into a
//!   container, but for each mission's own folder;
//! - `stacks/<stack>/`, the stack fragments. What an image is built from and
//!   what a firewall is fenced by is read on the host; the scripts a gate or
//!   a launch runs are mounted read-only into the agent's container, one file
//!   at a time.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::mission::Bounds;

/// The file in the project's home. What it declares beats a stack default;
/// a mission header beats it in turn (SPEC 4.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// The repository this configuration belongs to. The home is named after
    /// the repository's directory, so two repositories with the same name
    /// would otherwise share one configuration and one HQ without a word.
    #[serde(default)]
    pub root: Option<PathBuf>,
    /// Which harness runs the agents.
    pub harness: String,
    /// The project's forge. No agent may reach it: the clone's origin is
    /// unreachable, and an allowlist naming the forge would undo that
    /// (SPEC 3.1, 4.1 bis rule 6).
    #[serde(default)]
    pub forge: Vec<String>,
    /// Stack fragments this project uses, under the home's `stacks/<name>/`.
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
    /// Which model the agents run on, when the harness takes one. Absent,
    /// the harness keeps its own default. A mission header beats it, and no
    /// name is checked against a list: what a model name means belongs to
    /// the harness, not to `nunki`, and a list here would rot with every
    /// release. An unknown name fails in the container, where the harness
    /// says what it accepts.
    #[serde(default)]
    pub model: Option<String>,
    /// How the harness answers a permission it would otherwise ask a human
    /// about. There is nobody to ask in an autonomous container, so the
    /// question is only which way the silence falls (SPEC 4.3).
    ///
    /// `auto` — the default — lets the harness's own safety checks decide
    /// and nudges the agent to keep working rather than stop for a
    /// clarification. `dontAsk` allows only what is pre-approved and denies
    /// everything else, which is safer and stricter: an agent refused a tool
    /// it needed is a run that ends having done nothing.
    ///
    /// Not checked against a list, for the same reason as `model`: these
    /// names belong to the harness's vocabulary, not to `nunki`.
    #[serde(default = "default_permission_mode")]
    pub permission_mode: String,
    #[serde(default)]
    pub bounds: Bounds,
    /// Where the test credentials live, mounted read-only on a system
    /// profile and never anywhere else (SPEC 3.1).
    #[serde(default)]
    pub credentials: Option<PathBuf>,
    /// How the application is started, overriding the stack's `run.sh`: a
    /// path inside the tree, or `none` for a library (SPEC 4.2).
    #[serde(default)]
    pub run: Option<String>,
    /// The project's own Compose file, whose `services:` and `networks:` are
    /// merged verbatim into every system profile (SPEC 4.2, engine table:
    /// `include:` is unusable on one of the two engines, so `nunki` merges the
    /// YAML itself). A path inside the tree, because the file is the
    /// project's and travels with the commit; absent means the mission's
    /// system profile lifts no service of its own.
    #[serde(default)]
    pub services_file: Option<PathBuf>,
    /// Who refuses a push to a protected branch besides gate 2: the forge,
    /// or the human (SPEC 4.1 bis, "branche protégée").
    #[serde(default)]
    pub forge_protection: ForgeProtection,
}

fn default_protected_branches() -> Vec<String> {
    vec!["main".to_string(), "master".to_string(), "dev".to_string()]
}

/// Chosen by Arnaud on 2026-09-12: an agent that keeps working is worth more
/// than one refused a tool it needed, now that the container, the firewall
/// and git are what actually restrain it (SPEC 3.2).
fn default_permission_mode() -> String {
    "auto".to_string()
}

/// A private repository on GitHub's free plan can neither protect a branch
/// nor say it does, and a red `nunki check` cannot fix that: a red that cannot
/// be fixed teaches to ignore red. `by_hand` is the written decision that the
/// human holds the rule instead.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForgeProtection {
    /// The forge refuses the push, and `nunki check` asks it whether it would.
    #[default]
    Forge,
    /// The human holds the rule; `nunki check` does not ask the forge, and says
    /// so for every protected branch.
    ByHand,
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

/// The file `nunki init` writes in the project's home, and every verb reads.
pub const CONFIG_FILE: &str = "nunki.yaml";
/// The HQ, inside the project's home.
pub const HQ_DIR: &str = "hq";
/// The stack fragments, inside the project's home.
pub const STACKS_DIR: &str = "stacks";
/// Where a stack fragment declares the directories an execution must be able
/// to write when the tree is read-only.
pub const WRITABLE_FILE: &str = "writable.txt";
/// Where a stack fragment declares the caches its toolchain keeps outside the
/// tree — a package registry, a compiler cache — each kept as a named volume
/// per slot.
pub const CACHES_FILE: &str = "caches.txt";

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("{0} is not inside a git repository, and nunki orchestrates a repository")]
    NotARepository(PathBuf),
    #[error("git could not answer for {at}, so nunki cannot say which repository this is: {said}")]
    GitSilent { at: PathBuf, said: String },
    #[error(
        "the repository at {0} is in no project of this machine — `nunki init` makes it one, \
         and `nunki adopt <id>` points an existing project here if you moved it"
    )]
    NotRegistered(PathBuf),
    #[error(transparent)]
    Sessions(#[from] crate::sessions::SessionsError),
    #[error("no {} for the repository at {root} — run `nunki init` there first", config.display())]
    NotAProject { root: PathBuf, config: PathBuf },
    #[error(
        "{config} belongs to the repository at {claimed}, not to {here}: two repositories \
         with the same directory name cannot share one home — rename one of them"
    )]
    ClaimedBy {
        config: PathBuf,
        claimed: PathBuf,
        here: PathBuf,
    },
    #[error("{config} does not say which repository it belongs to — add `root: {here}` to it")]
    Unclaimed { config: PathBuf, here: PathBuf },
    #[error(
        "{found} is in the repository, and nunki's configuration now lives outside it: move \
         it to {home}/{CONFIG_FILE} with `root: {root}` added, and move `.nunki/stacks/` \
         to {home}/{STACKS_DIR}/"
    )]
    ConfigInTheTree {
        found: PathBuf,
        home: PathBuf,
        root: PathBuf,
    },
    #[error(
        "{home} holds the HQ at its top level, and the HQ now lives in {home}/{HQ_DIR}/: move \
         state/, locks/, missions/ and whatever else nunki wrote there into it"
    )]
    FlatHq { home: PathBuf },
    #[error("{0} could not be read: {1}")]
    Unreadable(PathBuf, String),
    #[error("{0} is not valid: {1}")]
    Invalid(PathBuf, String),
    #[error("no home directory: every project's home lives under ~/.nunki")]
    NoHome,
}

/// An opened project: the repository, its configuration, its home and the
/// HQ inside it.
#[derive(Debug, Clone)]
pub struct Project {
    pub root: PathBuf,
    pub config: Config,
    /// `~/.nunki/<project>/`.
    pub home: PathBuf,
    /// `~/.nunki/<project>/hq/`.
    pub hq_root: PathBuf,
}

impl Project {
    /// Open the project whose repository contains `start`: the repository's
    /// top level, the way git finds it, then its home under `~/.nunki`.
    pub fn open(start: &Path) -> Result<Self, ProjectError> {
        let root = Self::find_root(start)?;
        let home = Self::home_for(&root)?;
        Self::open_at(root, home)
    }

    /// Open the project rooted at `root` whose home is `home` — what `open`
    /// does once both are known, and what a test does to keep `~/.nunki` out
    /// of it.
    pub fn open_at(root: PathBuf, home: PathBuf) -> Result<Self, ProjectError> {
        let file = home.join(CONFIG_FILE);
        if !file.is_file() {
            // Said by name rather than as "not a project": a repository that
            // still carries the old layout is one a human has to move once,
            // and nunki moves nothing silently.
            let in_tree = root.join(CONFIG_FILE);
            if in_tree.is_file() {
                return Err(ProjectError::ConfigInTheTree {
                    found: in_tree,
                    home,
                    root,
                });
            }
            return Err(ProjectError::NotAProject { root, config: file });
        }
        if home.join("state").is_dir() && !home.join(HQ_DIR).is_dir() {
            return Err(ProjectError::FlatHq { home });
        }

        let text = std::fs::read_to_string(&file)
            .map_err(|e| ProjectError::Unreadable(file.clone(), e.to_string()))?;
        let config: Config = serde_yaml_ng::from_str(&text)
            .map_err(|e| ProjectError::Invalid(file.clone(), e.to_string()))?;
        let Some(claimed) = config.root.clone() else {
            return Err(ProjectError::Unclaimed {
                config: file,
                here: root,
            });
        };
        if canonical(&claimed) != canonical(&root) {
            return Err(ProjectError::ClaimedBy {
                config: file,
                claimed,
                here: root,
            });
        }
        Ok(Self::at(root, config, home))
    }

    /// A project rooted exactly here, with its home somewhere chosen — the
    /// form tests use, and the one `nunki init` uses before a home exists.
    pub fn at(root: PathBuf, config: Config, home: PathBuf) -> Self {
        let hq_root = home.join(HQ_DIR);
        Self {
            root,
            config,
            home,
            hq_root,
        }
    }

    /// `~/.nunki/` — the directory every project's home sits under, and where
    /// accounts live. Derived from the home rather than from `HOME`, so a test
    /// or a second machine can point the whole thing elsewhere by saying so
    /// once.
    pub fn nunki_home(&self) -> PathBuf {
        self.home
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.home.clone())
    }

    /// The session this project is registered under: the name of its home,
    /// and the ledger's key (SPEC 4.1).
    ///
    /// What tells two projects apart everywhere a directory name would not.
    /// Two repositories called `notes-api` have two sessions; two projects
    /// whose slots are both called `one` have two homes, and this is what
    /// says so.
    pub fn session(&self) -> String {
        self.home
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "session".to_string())
    }

    pub fn name(&self) -> String {
        self.root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "project".to_string())
    }

    /// The stack fragment directory for `stack`, whether or not it exists.
    pub fn fragment(&self, stack: &str) -> PathBuf {
        self.home.join(STACKS_DIR).join(stack)
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

    /// The directories a stack declares as writable when the tree is mounted
    /// read-only (SPEC 4.2, services and launch, rule 3). Relative paths
    /// inside the tree; what is not declared stays closed.
    pub fn stack_writable(&self, stack: &str) -> Vec<String> {
        let file = self.fragment(stack).join(WRITABLE_FILE);
        let Ok(text) = std::fs::read_to_string(file) else {
            return Vec::new();
        };
        text.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| l.trim_matches('/').to_string())
            .filter(|l| !l.is_empty() && l != "." && !l.split('/').any(|s| s == ".."))
            .collect()
    }

    /// The caches a stack keeps **outside the tree**, as absolute paths in
    /// the container (SPEC 4.2; the stack-agnostic principle names caches
    /// among the declared fragments).
    ///
    /// `nunki` used to mount `/home/agent/.cargo/registry` from its own code,
    /// for every stack and every role: a Python project carried an empty
    /// `cargo` volume and none for pip. A cache belongs to a toolchain, so it
    /// is declared where the toolchain is.
    ///
    /// Absolute, and `..` refused: the path becomes a mount point, and a
    /// relative one would land wherever the container's working directory
    /// happens to be.
    pub fn stack_caches(&self, stack: &str) -> Vec<String> {
        let file = self.fragment(stack).join(CACHES_FILE);
        let Ok(text) = std::fs::read_to_string(file) else {
            return Vec::new();
        };
        text.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .filter(|l| l.starts_with('/') && !l.split('/').any(|s| s == ".."))
            .map(|l| l.trim_end_matches('/').to_string())
            .filter(|l: &String| !l.is_empty())
            .collect()
    }

    /// The top level of the repository containing `start`, as git reports it.
    ///
    /// Two failures, and they send a human to different places. Git saying
    /// there is no repository here is one; git unable to run at all — a
    /// permission on the working directory, a missing binary — is the other,
    /// and it is reported in git's own words. Measured on 2026-09-15: macOS
    /// stopped git from reading its working directory, and `nunki` answered
    /// "not inside a git repository" for an hour.
    pub fn find_root(start: &Path) -> Result<PathBuf, ProjectError> {
        let dir = if start.is_dir() {
            start
        } else {
            start
                .parent()
                .ok_or_else(|| ProjectError::NotARepository(start.to_path_buf()))?
        };
        match crate::git::run(dir, &["rev-parse", "--show-toplevel"]) {
            Ok(top) => Ok(canonical(Path::new(&top))),
            Err(crate::git::GitError::Failed { stderr, .. })
                if stderr.contains("not a git repository") =>
            {
                Err(ProjectError::NotARepository(start.to_path_buf()))
            }
            Err(e) => Err(ProjectError::GitSilent {
                at: dir.to_path_buf(),
                said: e.to_string(),
            }),
        }
    }

    /// Where the project rooted at `root` keeps everything that is not the
    /// repository's: `~/.nunki/<id>/`, the identifier the ledger holds for it.
    ///
    /// Named by an identifier and not by the repository's directory: two
    /// repositories called `api` are two projects, and a name cannot tell
    /// them apart (SPEC 4.1, decided 2026-09-16).
    pub fn home_for(root: &Path) -> Result<PathBuf, ProjectError> {
        Self::home_in(&Self::nunki_dir()?, root)
    }

    /// The same, in a home that is given rather than read from the
    /// environment.
    ///
    /// The split is what keeps a test out of the human's `~/.nunki`.
    /// `home_for` reads `$HOME`, so a test calling it answers from whatever
    /// that machine happens to hold: it passed here because a temporary path
    /// was not in the real ledger, which is a probe that would pass with the
    /// rule removed. `nunki_dir` is now the one place the environment is
    /// read, and it is read at the edge.
    pub fn home_in(nunki_home: &Path, root: &Path) -> Result<PathBuf, ProjectError> {
        match crate::sessions::find(nunki_home, root)? {
            Some(id) => Ok(nunki_home.join(id)),
            None => Err(ProjectError::NotRegistered(root.to_path_buf())),
        }
    }

    /// The same, opening a session for the project when it has none: what
    /// `nunki init` needs, and the only verb that may hand an identifier out.
    pub fn home_for_new(root: &Path) -> Result<PathBuf, ProjectError> {
        Self::home_for_new_in(&Self::nunki_dir()?, root)
    }

    /// The same, in a home that is given. Split for the reason
    /// [`Project::home_in`] is: the environment is read at the edge, and
    /// nowhere a test can reach by accident.
    pub fn home_for_new_in(nunki_home: &Path, root: &Path) -> Result<PathBuf, ProjectError> {
        let id = crate::sessions::open(nunki_home, root)?;
        Ok(nunki_home.join(id))
    }

    /// Write `root:` into a home's configuration, so that the ledger and the
    /// home agree on which repository this is.
    ///
    /// The file is rewritten line by line rather than re-serialised: it is the
    /// human's, comments and all, and `nunki` changes the one line it owns
    /// (SPEC 3.3).
    pub fn claim(home: &Path, root: &Path) -> Result<(), ProjectError> {
        let file = home.join(CONFIG_FILE);
        let text = std::fs::read_to_string(&file)
            .map_err(|e| ProjectError::Unreadable(file.clone(), e.to_string()))?;
        let said = format!("root: {}", canonical(root).display());
        let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
        match lines.iter().position(|l| l.starts_with("root:")) {
            Some(at) => lines[at] = said,
            None => lines.insert(0, said),
        }
        std::fs::write(&file, format!("{}\n", lines.join("\n")))
            .map_err(|e| ProjectError::Unreadable(file, e.to_string()))
    }

    /// `~/.nunki/`: the homes, the accounts and the ledger.
    /// `~/.nunki`, and **the one place the environment is read**.
    ///
    /// Everything that needs a home takes one; only the verbs at the edge
    /// call this. That is what keeps a test out of the human's home: a test
    /// that reads it does not fail, it answers from whatever that machine
    /// happens to hold, which is the shape of a probe that passes with the
    /// rule removed.
    pub fn nunki_dir() -> Result<PathBuf, ProjectError> {
        let home = std::env::var_os("HOME").ok_or(ProjectError::NoHome)?;
        Ok(PathBuf::from(home).join(".nunki"))
    }
}

/// A path as the filesystem resolves it, or as given when it does not exist:
/// `/tmp` and `/private/tmp` are one directory on macOS, and a comparison of
/// spellings would call them two.
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
