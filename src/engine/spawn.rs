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

use super::{Engine, Liveness};
use crate::harness::spawn::{CommandSpec, LocalSpawner, Presence, Signal, Spawned, Spawner};

/// What the in-container check prints when the run is there, and when it is
/// not. Words rather than exit codes: an exit code cannot distinguish "the
/// process is not there" from "the engine could not run the check", and
/// those two must never be confused (see [`Presence`]).
const IS_RUNNING: &str = "hq-run-running";
const HAS_ENDED: &str = "hq-run-ended";

/// Where the agent may write its own runtime files. A tmpfs of its own,
/// mounted in every profile because a read-only tree and a read-only mission
/// folder leave nowhere else (see [`crate::compose`]).
pub const RUN_DIR: &str = "/run/hq";

pub struct ContainerSpawner {
    engine: Arc<dyn Engine>,
    /// What the harness's command line must still contain for the recorded
    /// pid to be the run and not a recycled number.
    identity: Option<String>,
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
            identity: None,
        }
    }

    /// Say what the run's command line carries — its session id — so that
    /// liveness can tell the run from whatever else has taken its pid.
    pub fn identified_by(mut self, session: &str) -> Self {
        self.identity = Some(session.to_string());
        self
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

    /// Alive **and still the same process** — or an admission that the
    /// question could not be put.
    ///
    /// `kill -0` alone is not enough here: process ids inside a container are
    /// low and recycled within seconds, so an unrelated `exec` becomes pid 7
    /// and a dead run reads as running. Measured — a finished run reported
    /// itself alive because the very shell asking the question had taken its
    /// number. So the identity is checked too: the harness's command line
    /// carries the session id `hq` imposed on it, and nothing else in the
    /// container does — the wrapper `exec`s the harness, so what the kernel
    /// reports for that pid is the harness's own command line. Measured
    /// again on 2026-09-10, against a real five-minute Claude Code run: the
    /// session id was in `/proc/<pid>/cmdline` for the whole run and gone
    /// the moment it ended.
    ///
    /// The other half is knowing when **not** to answer. Every step here can
    /// fail for a reason that has nothing to do with the run — the container
    /// is gone, the engine is not running, the profile was taken down — and
    /// each of those used to come back as `false`, which the adapter reads
    /// as "the harness died". So the engine is asked about the container
    /// first (SPEC 4.2, it is the authority), and the check itself reports
    /// by printing a word rather than by an exit code no failure can be
    /// told apart from.
    fn alive(&self, spawned: &Spawned) -> io::Result<Presence> {
        let Some(pid) = spawned.pid else {
            return Ok(Presence::Unknown(
                "the run recorded no process id, so nothing can be asked about it".into(),
            ));
        };
        if spawned.container.is_empty() {
            return Ok(Presence::Unknown(
                "the run recorded no container, so the engine cannot be asked about it".into(),
            ));
        }
        match self.engine.liveness(&spawned.container) {
            Ok(Liveness::Running) => {}
            Ok(Liveness::Exited(code)) => {
                return Ok(Presence::Vanished(format!(
                    "container {} exited ({code}) and took the run with it",
                    short(&spawned.container)
                )));
            }
            // The engine no longer knows it: a machine that slept, an engine
            // restarted without its containers, a profile taken down (SPEC
            // 4.2 names the first two). Worth knowing: the Docker adapter
            // also answers `Gone` when the daemon itself cannot be reached,
            // so a stopped engine arrives here dressed as a removed
            // container — a conflation of its own, and not this one to fix.
            Ok(Liveness::Gone) => {
                return Ok(Presence::Vanished(format!(
                    "the engine no longer knows container {}",
                    short(&spawned.container)
                )));
            }
            Err(e) => return Ok(Presence::Unknown(format!("the engine did not answer: {e}"))),
        }
        let check = match &self.identity {
            Some(session) => format!(
                "if tr '\\0' ' ' < /proc/{pid}/cmdline 2>/dev/null | grep -q -- {session}; \
                 then echo {IS_RUNNING}; else echo {HAS_ENDED}; fi"
            ),
            None => format!(
                "if kill -0 {pid} 2>/dev/null; then echo {IS_RUNNING}; else echo {HAS_ENDED}; fi"
            ),
        };
        let out = match self.in_container(&["sh", "-c", &check]) {
            Ok(out) => out,
            Err(e) => return Ok(Presence::Unknown(format!("the engine did not answer: {e}"))),
        };
        if out.stdout.contains(IS_RUNNING) {
            Ok(Presence::Running)
        } else if out.stdout.contains(HAS_ENDED) {
            Ok(Presence::Ended)
        } else {
            Ok(Presence::Unknown(format!(
                "the check did not run in the container ({}): {}",
                out.status,
                out.stderr.trim()
            )))
        }
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

/// A container id short enough to read, as every engine prints it.
fn short(container: &str) -> &str {
    container.get(..12).unwrap_or(container)
}
