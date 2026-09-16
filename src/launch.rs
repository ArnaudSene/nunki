//! Starting the application (SPEC 4.2, "les services et le lancement de
//! l'application", rule 2).
//!
//! Launching the application is **no agent's job**. `nunki` starts it, in the
//! profile of the role that will test or attack it, before launching that
//! role — so the integrator wires a deliverable that is already running, and
//! the security agent attacks exactly what the integrator validated.
//!
//! It starts it from a **script that belongs to the project**. Three places
//! may declare that script, and they are read in this order:
//!
//! 1. the frozen mission header (`run:`), which refines it for one mission;
//! 2. `nunki.yaml` (`run:`), which replaces the stack's default for the project;
//! 3. the stack fragment's `run.sh`, which says how an application of this
//!    stack is normally started.
//!
//! `none` at any of the three means there is nothing to start — a library,
//! whose security agent works on the code and the build artefact.
//!
//! The **choice** comes from those three declarations as `nunki` holds them; the
//! **file** is read from the slot's tree, at the commit the slot is on. That
//! split is the one SPEC asks for: the integrator may amend the launch script
//! and commit it, "et c'est cette version que `nunki` utilise ensuite" — so the
//! security run must read the file again rather than reuse anything cached
//! when the integrator started.
//!
//! The application runs **inside the agent's container**, which is what puts
//! it inside the perimeter the firewall owns. It therefore needs no stopping
//! of its own: a profile switch removes that container, and `nunki` starts the
//! application again in the next one.

use std::path::PathBuf;
use std::sync::Arc;

use crate::engine::Engine;
use crate::harness::spawn::{CommandSpec, Spawner};
use crate::harness::{Role, RunHandle, SessionId};
use crate::project::Project;
use crate::slot::Slot;

/// The launch script a stack fragment ships, in the project's home under
/// `stacks/<name>/`, and mounted read-only at [`crate::run::STACK_AT`].
pub const SCRIPT: &str = "run.sh";

/// What a declaration says when there is nothing to start.
pub const NOTHING: &str = "none";

/// Which of the three declarations decided, kept so an error can name the
/// file a human has to edit rather than the value it resolved to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Declared {
    /// The mission header's `run:`.
    Header,
    /// `nunki.yaml`'s `run:`.
    Config,
    /// The stack fragment's default.
    Stack,
}

impl Declared {
    pub fn where_from(self) -> &'static str {
        match self {
            Declared::Header => "the mission header's `run:`",
            Declared::Config => "nunki.yaml's `run:`",
            Declared::Stack => "the stack fragment's default",
        }
    }
}

/// What starting the application means for this mission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    /// Nothing to start.
    Nothing { declared: Declared },
    /// This script, as the container runs it: relative to the tree's root
    /// when the project declared it, absolute under
    /// [`crate::run::STACK_AT`] when it is the stack's default.
    Script { path: String, declared: Declared },
}

#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    #[error(
        "{where_from} names {path:?}, and the slot's tree has no such file at {head} — \
         the launch script belongs to the project and is read from the commit the slot \
         is on (SPEC 4.2)"
    )]
    Missing {
        where_from: &'static str,
        path: String,
        head: String,
    },
    #[error(
        "the stack ships no launch script at {} — `nunki init --stack {stack}` writes one, \
         or say `run: none` when there is nothing to start",
        at.display()
    )]
    NoStackScript { stack: String, at: PathBuf },
    #[error(
        "{where_from} names {path:?}, which is not executable — `nunki` runs it, it does \
         not guess an interpreter for it"
    )]
    NotExecutable {
        where_from: &'static str,
        path: String,
    },
    #[error("the application could not be started: {0}")]
    Spawn(String),
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error(transparent)]
    Compose(#[from] crate::compose::ComposeError),
    #[error(transparent)]
    Git(#[from] crate::git::GitError),
}

/// Decide what starting the application means, and check the script is
/// really there. The check is here rather than at spawn time so the failure
/// names the declaration and the commit, instead of surfacing as a container
/// that exited 127 in a log nobody is reading.
pub fn resolve(
    project: &Project,
    slot: &Slot,
    stack: &str,
    header: &crate::mission::Header,
) -> Result<Launch, LaunchError> {
    let (declared, said) = match (header.run.as_deref(), project.config.run.as_deref()) {
        (Some(run), _) => (Declared::Header, run.to_string()),
        (None, Some(run)) => (Declared::Config, run.to_string()),
        (None, None) => {
            // The stack's default lives in the project's home and reaches the
            // container read-only: checked on the host, run where it is
            // mounted.
            let on_host = project.fragment(stack).join(SCRIPT);
            if !on_host.is_file() {
                return Err(LaunchError::NoStackScript {
                    stack: stack.to_string(),
                    at: on_host,
                });
            }
            if !executable(&on_host) {
                return Err(LaunchError::NotExecutable {
                    where_from: Declared::Stack.where_from(),
                    path: on_host.display().to_string(),
                });
            }
            return Ok(Launch::Script {
                path: format!("{}/{SCRIPT}", crate::run::STACK_AT),
                declared: Declared::Stack,
            });
        }
    };
    if said.trim() == NOTHING {
        return Ok(Launch::Nothing { declared });
    }

    let path = said.trim().trim_start_matches("./").to_string();
    let on_host = slot.tree.join(&path);
    if !on_host.is_file() {
        // A stack default that is absent is the ordinary case of a project
        // that never shipped one, and it is still an error: the mission
        // declares an integration, and nothing says how to start what is
        // being integrated. Saying `run: none` is how a project says there
        // is nothing.
        return Err(LaunchError::Missing {
            where_from: declared.where_from(),
            path,
            head: crate::git::head(&slot.tree)?,
        });
    }
    if !executable(&on_host) {
        return Err(LaunchError::NotExecutable {
            where_from: declared.where_from(),
            path,
        });
    }
    Ok(Launch::Script { path, declared })
}

/// Start the application, detached, in the slot's agent container.
///
/// The session identifier is passed to the script **as an argument** and not
/// only recorded: liveness is read from `/proc/<pid>/cmdline` inside the
/// container (see [`crate::engine::spawn`]), so a command line that does not
/// carry the identifier is a run that answers "ended" the moment it is
/// asked.
pub fn start(
    project: &Project,
    slot: &Slot,
    engine: Arc<dyn Engine>,
    role: Role,
    runs_dir: &std::path::Path,
    launch: &Launch,
) -> Result<Option<RunHandle>, LaunchError> {
    let Launch::Script { path, .. } = launch else {
        return Ok(None);
    };
    std::fs::create_dir_all(runs_dir).map_err(|e| LaunchError::Io(runs_dir.to_path_buf(), e))?;

    let session = SessionId(format!("nunki-app-{}", crate::role::slug(role)));
    let log = runs_dir.join(format!("{}.log", session.0));
    let compose_project = crate::compose::project_name(&project.session(), &slot.name)?;
    let spawner = crate::engine::spawn::ContainerSpawner::new(
        engine,
        crate::run::profile_path(project, &slot.name),
        &compose_project,
        crate::compose::AGENT_SERVICE,
    )
    .identified_by(&session.0);

    let spawned = spawner
        .spawn(
            &CommandSpec {
                // Absolute when it is the stack's default, mounted from the
                // project's home; relative to the tree when the project
                // declared its own.
                program: if path.starts_with('/') {
                    path.clone()
                } else {
                    format!("./{path}")
                },
                args: vec![session.0.clone()],
                // The tree, not the clean copy of `HEAD`: what is started is
                // what the integrator wired and committed in this slot, and
                // the copy exists for replaying proofs, not for serving them
                // (SPEC 4.2).
                cwd: PathBuf::from(crate::run::TREE_AT),
                env: Default::default(),
            },
            &log,
        )
        .map_err(|e| LaunchError::Spawn(e.to_string()))?;

    Ok(Some(RunHandle {
        session,
        container: spawned.container,
        pid: spawned.pid,
        log,
    }))
}

#[cfg(unix)]
fn executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn executable(path: &std::path::Path) -> bool {
    path.is_file()
}
