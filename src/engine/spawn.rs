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
//! structured output, and it is captured on the host, where `nunki` reads it.
//! The mission folder cannot serve for that — it is mounted read-only but
//! for the agent's own files (SPEC 4.1) — which is why a run's log lives in
//! the HQ's
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
const IS_RUNNING: &str = "nunki-run-running";
const HAS_ENDED: &str = "nunki-run-ended";

/// Where the agent may write its own runtime files. A tmpfs of its own,
/// mounted in every profile because a read-only tree and a read-only mission
/// folder leave nowhere else (see [`crate::compose`]).
pub const RUN_DIR: &str = "/run/nunki";

/// The variable every process of a run carries, set by the wrapper and
/// inherited by whatever the run starts — in the background too, and by a
/// process that leaves its group or its session. What [`reap_script`] finds
/// a run's leftovers by.
///
/// A convenience and not a guard: a process may clear its own environment,
/// and one that does is not found. What contains an agent is its container.
pub const RUN_TAG: &str = "NUNKI_RUN";

/// What [`reap_script`] prints once it has looked, so that a script that did
/// not run is never read as one that found nothing.
const REAPED: &str = "nunki-reaped";

/// The shell that stops every process still carrying `NUNKI_RUN=<run>`:
/// `TERM` first, and `KILL` for what is left a few seconds later — again,
/// until none is left or a few rounds have passed, since a process killed is
/// not gone at once, and one that ignored `TERM` may have started another
/// meanwhile. It prints [`REAPED`] and the pids it signalled first.
///
/// Read out of `/proc/<pid>/environ`, with nothing but a POSIX shell, `tr`
/// and `grep`: no `pkill` or `ps`, which an image need not ship. A process
/// it cannot read — another user's, or a zombie, whose environment is empty
/// — is not one of the run's.
pub fn reap_script(run: &str) -> String {
    format!(
        "{}\
         pids=$(mine)\n\
         if [ -n \"$pids\" ]; then kill -TERM $pids 2>/dev/null; fi\n\
         for i in 1 2 3; do [ -z \"$(mine)\" ] && break; sleep 1; done\n\
         for i in 1 2 3; do left=$(mine); [ -z \"$left\" ] && break; \
         kill -KILL $left 2>/dev/null; sleep 1; done\n\
         echo {REAPED} $pids\n",
        mine(run)
    )
}

/// What [`left_script`] prints once it has looked, followed by what it
/// found: a word, so that a check that did not run is never read as one that
/// found nothing.
const LEFT: &str = "nunki-left";

/// The shell function both scripts find a run's processes with: `mine`
/// prints the pid of every process whose environment carries
/// `NUNKI_RUN=<run>`, one per line.
fn mine(run: &str) -> String {
    let wanted = crate::exec::quote(&format!("{RUN_TAG}={run}"));
    format!(
        "mine() {{ for p in /proc/[0-9]*; do \
         if tr '\\0' '\\n' < \"$p/environ\" 2>/dev/null | grep -qxF -- {wanted}; then \
         echo \"${{p#/proc/}}\"; fi; done; }}\n"
    )
}

/// The shell that says what still carries `NUNKI_RUN=<run>`, and stops
/// nothing: [`LEFT`] and the pids, on one line. It reads the way
/// [`reap_script`] reads, so a zombie — whose environment is empty — is not
/// counted: it runs nothing, and it is gone once its parent reaps it.
pub fn left_script(run: &str) -> String {
    format!("{}echo {LEFT} $(mine)\n", mine(run))
}

/// What [`ContainerSpawner::halt`] found once it had stopped a run by its
/// tag and asked again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Halt {
    /// Nothing carrying the tag is left. `signalled` is what the stop found
    /// to signal, which may be nothing.
    Done { signalled: Vec<u32> },
    /// The container is not running — gone, or exited — so nothing in it
    /// can be, and nothing was asked of it.
    NotRunning(String),
    /// The container is frozen: the engine refuses to exec into one, so
    /// nothing could be stopped, and nothing was.
    Paused,
    /// Still carrying the tag after the stop: these pids.
    Left(Vec<u32>),
    /// Whether anything is left could not be told, and why: the engine did
    /// not answer, or a script did not run. Never read as "nothing left".
    Unknown(String),
}

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
        format!("{RUN_DIR}/{}.pid", Self::run_of(log))
    }

    /// What names a run inside the container: its log's stem, which is its
    /// session — the pid file's name, and the value of [`RUN_TAG`].
    fn run_of(log: &Path) -> String {
        log.file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "run".to_string())
    }

    /// Stop whatever the run whose log is `log` left running in the
    /// container once it has ended: every process still carrying its
    /// [`RUN_TAG`] ([`reap_script`]). Returns the pids it signalled, or
    /// `None` when the container is not running — stopped, gone or frozen,
    /// nothing in it can outlive the run, or nothing can be asked.
    ///
    /// The run itself has ended: the caller asks only then. A mutation
    /// campaign in the same container carries its own tag, and is never one
    /// of the run's.
    pub fn reap(&self, container: &str, log: &Path) -> io::Result<Option<Vec<u32>>> {
        match self.engine.liveness(container) {
            Ok(Liveness::Running) => {}
            Ok(_) => return Ok(None),
            Err(e) => return Err(io::Error::other(e)),
        }
        let out = self.in_container(&["sh", "-c", &reap_script(&Self::run_of(log))])?;
        let Some(said) = out
            .stdout
            .lines()
            .find_map(|line| line.trim().strip_prefix(REAPED))
        else {
            return Err(io::Error::other(format!(
                "the check for what the run left behind did not run in the container ({}): {}",
                out.status,
                out.stderr.trim()
            )));
        };
        Ok(Some(
            said.split_whitespace()
                .filter_map(|pid| pid.parse().ok())
                .collect(),
        ))
    }

    /// Stop every process in the container carrying the [`RUN_TAG`] of the
    /// run whose log is `log` — whatever started it, at whatever depth —
    /// and then ask the container, apart from the stop, what still carries
    /// it ([`left_script`]).
    ///
    /// By the tag and never by a pid: a recorded pid is one process, and a
    /// script told to end leaves its children reparented and running. The
    /// stop is [`reap_script`]'s, `TERM` then `KILL`.
    ///
    /// Asked again because a stop that was sent is not a stop that took: a
    /// process can ignore `TERM`, outlast the rounds of `KILL`, or have been
    /// started meanwhile. And every step that could not be put is said as
    /// such ([`Halt::Unknown`]), never as "nothing left".
    ///
    /// Beside [`Spawner::signal`] rather than in it: that one is what stops a
    /// harness run — `nunki mission stop --now`, the emergency brake — and it
    /// keeps doing exactly what it did.
    pub fn halt(&self, container: &str, log: &Path) -> Halt {
        match self.engine.liveness(container) {
            Ok(Liveness::Running) => {}
            Ok(Liveness::Paused) => return Halt::Paused,
            Ok(Liveness::Exited(code)) => {
                return Halt::NotRunning(format!("container {} exited ({code})", short(container)));
            }
            Ok(Liveness::Gone) => {
                return Halt::NotRunning(format!(
                    "the engine no longer knows container {}",
                    short(container)
                ));
            }
            Err(e) => return Halt::Unknown(format!("the engine did not answer: {e}")),
        }
        let run = Self::run_of(log);
        let signalled = match self.said(&reap_script(&run), REAPED) {
            Ok(pids) => pids,
            Err(why) => return Halt::Unknown(format!("the stop {why}")),
        };
        match self.said(&left_script(&run), LEFT) {
            Ok(left) if left.is_empty() => Halt::Done { signalled },
            Ok(left) => Halt::Left(left),
            Err(why) => Halt::Unknown(format!("the check after the stop {why}")),
        }
    }

    /// Run `script` in the container and read the pids it printed after
    /// `word`, or say why that could not be done.
    fn said(&self, script: &str, word: &str) -> Result<Vec<u32>, String> {
        let out = self
            .in_container(&["sh", "-c", script])
            .map_err(|e| format!("could not be put: the engine did not answer: {e}"))?;
        let Some(pids) = out
            .stdout
            .lines()
            .find_map(|line| line.trim().strip_prefix(word))
        else {
            let said = out.stderr.trim();
            return Err(if said.is_empty() {
                format!("did not run in the container ({})", out.status)
            } else {
                format!("did not run in the container ({}): {said}", out.status)
            });
        };
        Ok(pids
            .split_whitespace()
            .filter_map(|pid| pid.parse().ok())
            .collect())
    }

    /// The host-side command that runs `cmd` in the container.
    ///
    /// The wrapper publishes the in-container pid and then `exec`s, so the
    /// pid it wrote is the harness's own and not a shell's. Arguments travel
    /// as arguments — `"$@"` — so nothing has to be quoted into a script.
    pub fn command(&self, cmd: &CommandSpec, log: &Path) -> CommandSpec {
        let mut options = vec!["-w".to_string(), cmd.cwd.display().to_string()];
        // Tagged, so that what the run leaves behind can be found once it
        // ends ([`ContainerSpawner::reap`]).
        options.push("-e".to_string());
        options.push(format!("{RUN_TAG}={}", Self::run_of(log)));
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
        // structured output — lands in a file `nunki` can read.
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
    /// carries the session id `nunki` imposed on it, and nothing else in the
    /// container does — the wrapper `exec`s the harness, so what the kernel
    /// reports for that pid is the harness's own command line. Against a
    /// real Claude Code run, the session id is in `/proc/<pid>/cmdline` for
    /// the whole run and gone the moment it ends.
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
            // Asked before the probe, because the probe cannot run: the
            // engine refuses to exec into a frozen container outright
            // ("Container … is paused, unpause the container before exec"),
            // and that refusal would arrive here as
            // `Unknown` — a silence about a run whose state is perfectly
            // known.
            Ok(Liveness::Paused) => return Ok(Presence::Paused),
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
        // Through a shell, as the liveness probe above already is: `kill` is a
        // builtin every POSIX shell has, and **not** a binary every image
        // ships. On a Debian agent image, `exec … kill -INT 7` answers
        // `executable file not found in $PATH` and a status of 127, and no
        // signal is ever sent — while `sh -c 'kill -INT 7'` in the same
        // container works.
        //
        // Every signal `nunki` sends went through this line: `nunki mission
        // stop --now`, the emergency brake, and the terminate a mutation
        // campaign is given when it runs past its deadline. None of them
        // reached a run on an image without `procps`.
        //
        // The live test above did not catch it, and could not: it lifts an
        // Alpine container, where busybox provides `/bin/kill`. A guard
        // proved against an image that happens to ship the tool says nothing
        // about the image the product actually runs on.
        let kill = format!("kill {flag} {pid}");
        let out = self.in_container(&["sh", "-c", &kill])?;
        let pid = pid.to_string();
        if out.ok() {
            Ok(())
        } else {
            // The status always, and what the shell said only if it said
            // anything. A `kill` that fails often writes nothing at all, and an
            // error carrying only that stderr is an error whose whole message
            // is empty: `nunki mission stop --now` would answer `nunki: io:`
            // and nothing else, while the run goes on. The status is then the
            // only thing anyone knows, so it is the one thing that must never
            // be dropped — the same reading `alive` already makes just above.
            let said = out.stderr.trim();
            let what = format!("kill {flag} {pid} in the container failed ({})", out.status);
            Err(io::Error::other(if said.is_empty() {
                what
            } else {
                format!("{what}: {said}")
            }))
        }
    }
}

/// A container id short enough to read, as every engine prints it.
fn short(container: &str) -> &str {
    container.get(..12).unwrap_or(container)
}
