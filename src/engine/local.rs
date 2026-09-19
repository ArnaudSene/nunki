//! An engine that really runs what it is given, on this machine.
//!
//! Not a second Docker adapter, and not a replacement for the `#[ignore]`d
//! live tests. Those exercise the real thing and are the only proof that
//! counts — and CI never plays them, because CI runs on a macOS runner with
//! no container engine. This one runs **in CI**, on both runners, and it has
//! the one property [`super::fake::FakeEngine`] lacks: state.
//!
//! The fake answers every `exec` with the next canned [`ExecOutput`],
//! whatever the argv. So it has no directory a `git reset --hard` could
//! destroy, no process a `stop` could kill, and no exit status anything could
//! come from — and six orchestration defects went through a suite of five
//! hundred tests built on it (SPEC 4.4, the campaign and the copy of `HEAD`).
//! Every one of them was an interaction between a real command and real
//! filesystem state.
//!
//! **What it does not do, it refuses rather than pretends.** A double that
//! silently succeeds at what it does not implement is the same defect as the
//! one it replaces, one layer down: `pause`, `unpause` and `kill` return an
//! error naming themselves.
//!
//! Paths are the whole trick. Commands arrive written for a container —
//! `/work/proof`, `/work/tree` — and are rewritten to the temporary
//! directories standing in for them before anything runs. Nothing else about
//! a container is simulated: there is no namespace, no user, no image.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

use super::{Dialect, Engine, EngineError, ExecOutput, Liveness, Netns};
use crate::harness::spawn::CommandSpec;

/// The container id this engine answers with. Fixed: there is one.
pub const CONTAINER: &str = "local";

pub struct LocalEngine {
    dialect: Dialect,
    /// Container path first, host path second — longest first, so that
    /// `/work/tree/target` is rewritten before `/work/tree`.
    mounts: Vec<(String, String)>,
    up: Mutex<bool>,
    /// Detached processes this engine started, so a `stop` really ends them
    /// and `Drop` leaves none behind. A leaked `sleep` in CI is a flake that
    /// looks like something else.
    spawned: Mutex<Vec<u32>>,
}

impl LocalEngine {
    /// `mounts` maps a container path to a host directory. Order does not
    /// matter: the longest path is applied first, whatever it is given in.
    pub fn new(mounts: &[(&str, &Path)]) -> Self {
        let mut mounts: Vec<(String, String)> = mounts
            .iter()
            .map(|(at, host)| ((*at).to_string(), host.display().to_string()))
            .collect();
        mounts.sort_by_key(|(at, _)| std::cmp::Reverse(at.len()));
        Self {
            dialect: Dialect {
                netns: Netns::Service,
                host_alias: "host.docker.internal".to_string(),
                userns: None,
            },
            mounts,
            up: Mutex::new(false),
            spawned: Mutex::new(Vec::new()),
        }
    }

    /// A command written for the container, rewritten for this machine.
    pub fn on_the_host(&self, text: &str) -> String {
        let mut text = text.to_string();
        for (at, host) in &self.mounts {
            text = text.replace(at, host);
        }
        text
    }

    /// What this engine has been asked to start and not yet ended.
    pub fn running(&self) -> Vec<u32> {
        self.spawned
            .lock()
            .unwrap()
            .iter()
            .copied()
            .filter(|pid| alive(*pid))
            .collect()
    }

    pub fn remember(&self, pid: u32) {
        self.spawned.lock().unwrap().push(pid);
    }

    fn end_everything(&self) {
        for pid in self.spawned.lock().unwrap().drain(..) {
            // Negative, so the shell's children go with it: a campaign is a
            // script that spawns, and killing only the shell leaves the work
            // running — which is exactly the shape `stop` has to reproduce.
            unsafe {
                libc_kill(-(pid as i32), 15);
                libc_kill(pid as i32, 15);
            }
        }
    }
}

impl Drop for LocalEngine {
    fn drop(&mut self) {
        self.end_everything();
    }
}

fn alive(pid: u32) -> bool {
    unsafe { libc_kill(pid as i32, 0) == 0 }
}

// `kill(2)`, without a dependency for two calls. The project prefers a few
// lines of its own to a crate nobody would read (CLAUDE.md, section 7).
unsafe extern "C" {
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
}

fn unsupported(verb: &str) -> EngineError {
    EngineError::Command {
        verb: "local",
        status: "unsupported".to_string(),
        stderr: format!(
            "the local engine does not implement {verb}: it runs commands and ends \
             processes, and says so rather than answering as if it had"
        ),
    }
}

impl Engine for LocalEngine {
    fn name(&self) -> &'static str {
        "local"
    }

    fn dialect(&self) -> &Dialect {
        &self.dialect
    }

    fn up(&self, _file: &Path, _project: &str) -> Result<(), EngineError> {
        *self.up.lock().unwrap() = true;
        Ok(())
    }

    fn pause(&self, _file: &Path, _project: &str, _services: &[&str]) -> Result<(), EngineError> {
        Err(unsupported("pause"))
    }

    fn unpause(&self, _file: &Path, _project: &str, _services: &[&str]) -> Result<(), EngineError> {
        Err(unsupported("unpause"))
    }

    fn kill(&self, _file: &Path, _project: &str, _services: &[&str]) -> Result<(), EngineError> {
        Err(unsupported("kill"))
    }

    /// The switch a profile change performs, and it really ends what is
    /// running: a detached campaign dies with the container it lives in, and
    /// a double where it survived would prove the opposite of the truth.
    fn stop(&self, _file: &Path, _project: &str, _services: &[&str]) -> Result<(), EngineError> {
        self.end_everything();
        *self.up.lock().unwrap() = false;
        Ok(())
    }

    fn down(&self, _file: &Path, _project: &str, _volumes: bool) -> Result<(), EngineError> {
        self.end_everything();
        *self.up.lock().unwrap() = false;
        Ok(())
    }

    fn exec(
        &self,
        _file: &Path,
        _project: &str,
        _service: &str,
        argv: &[String],
    ) -> Result<ExecOutput, EngineError> {
        let Some((program, args)) = argv.split_first() else {
            return Err(unsupported("an exec with no command"));
        };
        let out = Command::new(self.on_the_host(program))
            .args(args.iter().map(|a| self.on_the_host(a)))
            .output()
            .map_err(EngineError::Io)?;
        Ok(ExecOutput {
            status: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    fn container_of(
        &self,
        _file: &Path,
        _project: &str,
        _service: &str,
    ) -> Result<Option<String>, EngineError> {
        Ok(Some(CONTAINER.to_string()))
    }

    fn liveness(&self, _container: &str) -> Result<Liveness, EngineError> {
        Ok(match *self.up.lock().unwrap() {
            true => Liveness::Running,
            false => Liveness::Gone,
        })
    }

    fn detached_command(
        &self,
        _file: &Path,
        _project: &str,
        _service: &str,
        options: &[String],
        argv: &[String],
    ) -> CommandSpec {
        // `options` are the engine's own — `-w <dir>`, `-e K=V` — and the
        // host has no such flags, so they are read here and turned into what
        // a `Command` takes.
        let mut cwd = PathBuf::from(".");
        let mut env = BTreeMap::new();
        let mut rest = options.iter();
        while let Some(option) = rest.next() {
            match option.as_str() {
                "-w" => {
                    if let Some(dir) = rest.next() {
                        cwd = PathBuf::from(self.on_the_host(dir));
                    }
                }
                "-e" => {
                    if let Some(pair) = rest.next()
                        && let Some((key, value)) = pair.split_once('=')
                    {
                        env.insert(key.to_string(), self.on_the_host(value));
                    }
                }
                _ => {}
            }
        }
        let mut argv = argv.iter().map(|a| self.on_the_host(a));
        let program = argv.next().unwrap_or_default();
        CommandSpec {
            program,
            args: argv.collect(),
            cwd,
            env,
        }
    }
}

/// Start `spec` detached, the way the host spawner does, and remember it so
/// that a `stop` ends it. Used by tests that need a campaign to really be
/// running while something else asks about the directory it is rewriting.
pub fn detach(engine: &LocalEngine, spec: &CommandSpec, log: &Path) -> io::Result<u32> {
    let file = std::fs::File::create(log)?;
    let child = Command::new(&spec.program)
        .args(&spec.args)
        .current_dir(&spec.cwd)
        .envs(&spec.env)
        .stdout(Stdio::from(file.try_clone()?))
        .stderr(Stdio::from(file))
        .stdin(Stdio::null())
        .spawn()?;
    let pid = child.id();
    engine.remember(pid);
    Ok(pid)
}
