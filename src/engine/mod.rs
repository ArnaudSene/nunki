//! The container-engine boundary (SPEC 4.2).
//!
//! `nunki` generates the Compose file itself (see [`crate::compose`]) and an
//! engine adapter runs it. The boundary is not minuscule: the 4.2 table lists
//! what genuinely differs between `docker compose` and `podman-compose`, and
//! everything on that list lives behind [`Dialect`] and [`Engine`], nowhere
//! else. Docker is the first version's only target; Podman is documented and
//! second.

pub mod cli;
pub mod docker;
pub mod fake;
pub mod spawn;

use std::path::Path;

/// How a service says "share that service's network namespace" — the first
/// line of the 4.2 engine table, and the one the firewall doctrine rests on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Netns {
    /// Docker Compose: `service:<name>`, resolved by Compose itself.
    Service,
    /// podman-compose: only `container:<name>`, so the generated container
    /// name has to be known in advance.
    Container,
}

/// What an engine does differently. Held by the adapter, read by the
/// generator; no other module knows an engine exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dialect {
    pub netns: Netns,
    /// The name the host answers to from inside a container, when a mission
    /// declares it (SPEC 4.1 bis, rule 5).
    pub host_alias: String,
    /// A user-namespace mode the engine needs, if any (Podman rootless).
    pub userns: Option<String>,
}

impl Dialect {
    /// The value of `network_mode:` for a service joining `service`'s
    /// namespace, in `project`.
    pub fn netns_ref(&self, project: &str, service: &str) -> String {
        match self.netns {
            Netns::Service => format!("service:{service}"),
            // Compose names a container `<project>-<service>-<index>`; the
            // index is 1 for a service with no replicas.
            Netns::Container => format!("container:{project}-{service}-1"),
        }
    }
}

/// What the engine says about a container, which is the authority `nunki` asks
/// on restart before believing its own state (SPEC 4.2, "la reprise
/// re-dérive avant de décider").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Liveness {
    Running,
    /// Frozen by `nunki mission pause`: the process is there and makes no
    /// progress. Its own answer, because reporting it as running would tell a
    /// stall check that an agent stopped thinking, and reporting it as gone
    /// would tell the human their run died (measured: a paused container
    /// answers `Running=true Paused=true`, and `exec` into one is refused
    /// outright).
    Paused,
    /// It exists and is not running; the code is what the engine reports.
    Exited(i64),
    /// The engine knows nothing about it — a rebuilt machine, a pruned
    /// engine, a slot removed.
    Gone,
}

/// The output of a command run in a container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecOutput {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

impl ExecOutput {
    pub fn ok(&self) -> bool {
        self.status == 0
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("{verb} failed ({status}): {stderr}")]
    Command {
        verb: &'static str,
        status: String,
        stderr: String,
    },
    #[error("no container for service {0:?} in project {1:?}")]
    NoContainer(String, String),
    #[error("the engine's answer could not be read: {0}")]
    Unreadable(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Everything `nunki` asks of a container engine, and nothing more.
///
/// What is deliberately absent: building images, pulling, pruning, and
/// anything resembling orchestration. `nunki` orchestrates; the engine runs
/// containers.
pub trait Engine: Send + Sync {
    fn name(&self) -> &'static str;

    fn dialect(&self) -> &Dialect;

    /// Bring a profile up and wait until every service is started and, where
    /// declared, healthy. A firewall that never becomes healthy is an agent
    /// that never starts (SPEC 4.1 bis, rule 2), and this is where that is
    /// enforced.
    fn up(&self, file: &Path, project: &str) -> Result<(), EngineError>;

    /// Freeze named services where they are, and unfreeze them exactly there.
    ///
    /// The container, not a hook the agent could ignore (SPEC 4.3, "les trois
    /// gestes de l'humain sur un agent"). Nothing is lost: the processes are
    /// suspended by the engine. A model call in flight may time out during a
    /// long freeze, and the harness replays it.
    fn pause(&self, file: &Path, project: &str, services: &[&str]) -> Result<(), EngineError>;

    fn unpause(&self, file: &Path, project: &str, services: &[&str]) -> Result<(), EngineError>;

    /// The emergency brake: the containers are killed, nothing is waited for.
    ///
    /// Unlike [`stop`](Engine::stop), which asks and gives time, and unlike
    /// [`down`](Engine::down), which takes the project with it. What is
    /// killed here is the agent's pair; the project's services keep running.
    fn kill(&self, file: &Path, project: &str, services: &[&str]) -> Result<(), EngineError>;

    /// Stop and remove named services, leaving everything else in the project
    /// running. This is how a profile switch happens: the previous agent and
    /// its sidecar go, the project's services stay up with the state the
    /// integrator left in them (SPEC 4.2, "les services sont levés une fois
    /// par slot").
    fn stop(&self, file: &Path, project: &str, services: &[&str]) -> Result<(), EngineError>;

    /// Take the whole project down. `volumes` also deletes its named volumes,
    /// which is the only destructive thing an engine adapter can do.
    fn down(&self, file: &Path, project: &str, volumes: bool) -> Result<(), EngineError>;

    /// Run a command in a service's container and wait for it.
    fn exec(
        &self,
        file: &Path,
        project: &str,
        service: &str,
        argv: &[String],
    ) -> Result<ExecOutput, EngineError>;

    /// The container backing a service, if the engine has one.
    fn container_of(
        &self,
        file: &Path,
        project: &str,
        service: &str,
    ) -> Result<Option<String>, EngineError>;

    /// Ask the engine about a container by name or id.
    fn liveness(&self, container: &str) -> Result<Liveness, EngineError>;

    /// The command line that would run `argv` in a service's container, as
    /// data. [`exec`](Engine::exec) runs it and waits; a caller that needs
    /// the process to outlive the call — a run — takes this and detaches it
    /// itself (see [`spawn::ContainerSpawner`]). Building it stays here:
    /// command construction is the adapter's, whoever executes it.
    /// `options` are the engine's own exec options — a working directory,
    /// environment — and they must precede the service name, or the engine
    /// reads them as part of the command.
    fn detached_command(
        &self,
        file: &Path,
        project: &str,
        service: &str,
        options: &[String],
        argv: &[String],
    ) -> crate::harness::spawn::CommandSpec;
}
