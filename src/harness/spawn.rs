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
}
