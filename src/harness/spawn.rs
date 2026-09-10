//! Where a harness command runs. The adapter builds a [`CommandSpec`] — plain
//! data — and a [`Spawner`] executes it: locally for tests and for a run on
//! the host, inside a container in production (the container engine's
//! spawner comes with the Compose work of SPEC 4.1 bis).

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A command to run, as data: nothing here is tied to a process API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
}

impl CommandSpec {
    /// The command line as a single string, for logs and assertions.
    pub fn display(&self) -> String {
        let mut parts = vec![self.program.clone()];
        parts.extend(self.args.iter().cloned());
        parts.join(" ")
    }
}

/// Which signal to send. Only two, and the difference matters: SIGINT ends
/// an agent's turn properly, SIGTERM leaves it unfinished (SPEC 4.3,
/// `pause`/`stop`/`kill`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    Interrupt,
    Terminate,
}

impl Signal {
    /// The name `kill` takes, for a spawner that has to go through a shell.
    pub fn name(self) -> &'static str {
        match self {
            Signal::Interrupt => "INT",
            Signal::Terminate => "TERM",
        }
    }
}

/// What a spawner reports back: enough to persist and to re-derive liveness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spawned {
    pub pid: Option<u32>,
    /// Container name, when the command runs in one.
    pub container: String,
}

pub trait Spawner: Send + Sync {
    /// Start `cmd` detached, with its stdout appended to `log` and its stderr
    /// to `<log>.err`. Returns as soon as the process is started.
    fn spawn(&self, cmd: &CommandSpec, log: &Path) -> io::Result<Spawned>;

    /// Is the process still there? Asked by the harness adapter to tell a run
    /// that is working from one that died without saying so.
    fn alive(&self, spawned: &Spawned) -> io::Result<bool>;

    /// Send it a signal. Where the process lives decides how: a pid on this
    /// machine is signalled directly, a pid inside a container is signalled
    /// from inside that container — signalling the client that started it
    /// would kill the client and leave the agent running.
    fn signal(&self, spawned: &Spawned, signal: Signal) -> io::Result<()>;
}

/// Runs the command on this machine, in its own process group so a signal
/// aimed at `hq` does not reach it.
#[derive(Debug, Default)]
pub struct LocalSpawner;

impl Spawner for LocalSpawner {
    fn spawn(&self, cmd: &CommandSpec, log: &Path) -> io::Result<Spawned> {
        if let Some(dir) = log.parent() {
            fs::create_dir_all(dir)?;
        }
        let out = File::options().create(true).append(true).open(log)?;
        let err = File::options()
            .create(true)
            .append(true)
            .open(log.with_extension("err"))?;
        let mut command = Command::new(&cmd.program);
        command
            .args(&cmd.args)
            .current_dir(&cmd.cwd)
            .envs(&cmd.env)
            .stdin(Stdio::null())
            .stdout(Stdio::from(out))
            .stderr(Stdio::from(err));
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let child = command.spawn()?;
        Ok(Spawned {
            pid: Some(child.id()),
            container: String::new(),
        })
    }

    fn alive(&self, spawned: &Spawned) -> io::Result<bool> {
        Ok(spawned.pid.is_some_and(crate::state::lock::process_alive))
    }

    fn signal(&self, spawned: &Spawned, signal: Signal) -> io::Result<()> {
        let Some(pid) = spawned.pid else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "the run has no process id",
            ));
        };
        let number = match signal {
            Signal::Interrupt => libc::SIGINT,
            Signal::Terminate => libc::SIGTERM,
        };
        // SAFETY: sending a signal to a pid we recorded ourselves.
        let rc = unsafe { libc::kill(pid as libc::pid_t, number) };
        if rc == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}
