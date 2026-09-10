//! Running the harness inside its container (SPEC 4.1, profiles; 4.3, runs).
//!
//! The local spawner starts a process on this machine and holds its pid. That
//! does not survive the move into a container: what the host holds then is
//! the engine's client, and signalling the client kills the client while the
//! agent keeps going. So the run records the pid **inside** the container,
//! and everything asked of it afterwards — is it alive, stop it — is asked
//! from inside too.
//!
//! The stream is the other way round: the client's stdout is the harness's
//! structured output, and it is captured on the host, where `hq` reads it.
//! The mission folder cannot serve for that — it is mounted read-only but
//! for three files (SPEC 4.1) — which is why a run's log lives in the HQ's
//! own directory and never in the container.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::Engine;
use crate::harness::spawn::{CommandSpec, LocalSpawner, Signal, Spawned, Spawner};

/// Where the agent may write its own runtime files. A tmpfs of its own,
/// mounted in every profile because a read-only tree and a read-only mission
/// folder leave nowhere else (see [`crate::compose`]).
pub const RUN_DIR: &str = "/run/hq";

pub struct ContainerSpawner {
    engine: Arc<dyn Engine>,
    /// The profile file currently up, as the engine reads it.
    file: PathBuf,
    project: String,
    service: String,
    /// How long to wait for the wrapper to publish its pid.
    patience: Duration,
    host: LocalSpawner,
}

impl ContainerSpawner {
    pub fn new(engine: Arc<dyn Engine>, file: PathBuf, project: &str, service: &str) -> Self {
        Self {
            engine,
            file,
            project: project.to_string(),
            service: service.to_string(),
            patience: Duration::from_secs(10),
            host: LocalSpawner,
        }
    }

    /// The pid file a run publishes, named after its log — which is named
    /// after the session.
    fn pid_file(log: &Path) -> String {
        let stem = log
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "run".to_string());
        format!("{RUN_DIR}/{stem}.pid")
    }

    /// The host-side command that runs `cmd` in the container.
    ///
    /// The wrapper publishes the in-container pid and then `exec`s, so the
    /// pid it wrote is the harness's own and not a shell's. Arguments travel
    /// as arguments — `"$@"` — so nothing has to be quoted into a script.
    pub fn command(&self, cmd: &CommandSpec, log: &Path) -> CommandSpec {
        let mut options = vec!["-w".to_string(), cmd.cwd.display().to_string()];
        for (key, value) in &cmd.env {
            options.push("-e".to_string());
            options.push(format!("{key}={value}"));
        }
        let mut argv = Vec::new();
        argv.extend([
            "sh".to_string(),
            "-c".to_string(),
            "echo $$ > \"$0\"; exec \"$@\"".to_string(),
            Self::pid_file(log),
            cmd.program.clone(),
        ]);
        argv.extend(cmd.args.iter().cloned());
        self.engine
            .detached_command(&self.file, &self.project, &self.service, &options, &argv)
    }

    fn in_container(&self, argv: &[&str]) -> io::Result<crate::engine::ExecOutput> {
        let owned: Vec<String> = argv.iter().map(|a| a.to_string()).collect();
        self.engine
            .exec(&self.file, &self.project, &self.service, &owned)
            .map_err(io::Error::other)
    }

    fn read_pid(&self, log: &Path) -> io::Result<Option<u32>> {
        let file = Self::pid_file(log);
        let out = self.in_container(&["cat", &file])?;
        if !out.ok() {
            return Ok(None);
        }
        Ok(out.stdout.trim().parse().ok())
    }
}

impl Spawner for ContainerSpawner {
    fn spawn(&self, cmd: &CommandSpec, log: &Path) -> io::Result<Spawned> {
        let client = self.command(cmd, log);
        // The client runs on the host so that its stdout — the harness's
        // structured output — lands in a file `hq` can read.
        self.host.spawn(&client, log)?;

        let container = self
            .engine
            .container_of(&self.file, &self.project, &self.service)
            .map_err(io::Error::other)?
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("service {:?} has no container", self.service),
                )
            })?;

        // Wait for the wrapper to publish its pid rather than record the
        // client's: a run whose pid we cannot learn is a run we could not
        // stop, and that must fail here, loudly, not later.
        let deadline = Instant::now() + self.patience;
        loop {
            if let Some(pid) = self.read_pid(log)? {
                return Ok(Spawned {
                    pid: Some(pid),
                    container,
                });
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the run never published its process id in the container",
                ));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn alive(&self, spawned: &Spawned) -> io::Result<bool> {
        let Some(pid) = spawned.pid else {
            return Ok(false);
        };
        let pid = pid.to_string();
        Ok(self.in_container(&["kill", "-0", &pid])?.ok())
    }

    fn signal(&self, spawned: &Spawned, signal: Signal) -> io::Result<()> {
        let Some(pid) = spawned.pid else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "the run has no process id",
            ));
        };
        let flag = format!("-{}", signal.name());
        let pid = pid.to_string();
        let out = self.in_container(&["kill", &flag, &pid])?;
        if out.ok() {
            Ok(())
        } else {
            Err(io::Error::other(out.stderr))
        }
    }
}
