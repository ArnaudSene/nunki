//! The Docker adapter (SPEC 4.2, "la première version ne vise que Docker").

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::cli::{Cli, RealCli};
use super::{Dialect, Engine, EngineError, ExecOutput, Liveness, Netns};
use crate::harness::spawn::CommandSpec;

/// How to invoke the engine. Both halves are overridable, because the
/// machines this runs on are further apart than they look: the development
/// machine measured Compose v5.1.2 against a CI runner on v2.38.2 (SPEC 4.2
/// bis), and the doctrine is checked under both.
#[derive(Debug, Clone)]
pub struct Config {
    /// The Compose command line, split into words: `docker compose` by
    /// default, `HQ_COMPOSE` if set.
    pub compose: Vec<String>,
    /// The engine binary used for what Compose does not answer — container
    /// inspection. `docker` by default, `HQ_ENGINE` if set.
    pub engine: String,
    /// Seconds an engine is given to stop a container before it is killed.
    pub stop_timeout: u32,
}

impl Default for Config {
    fn default() -> Self {
        let compose = std::env::var("HQ_COMPOSE")
            .unwrap_or_else(|_| "docker compose".to_string())
            .split_whitespace()
            .map(str::to_string)
            .collect::<Vec<_>>();
        Self {
            compose: if compose.is_empty() {
                vec!["docker".to_string(), "compose".to_string()]
            } else {
                compose
            },
            engine: std::env::var("HQ_ENGINE").unwrap_or_else(|_| "docker".to_string()),
            // Long enough for an agent to finish writing its journal after a
            // SIGINT, short enough that a slot is not held hostage.
            stop_timeout: 20,
        }
    }
}

pub struct Docker {
    config: Config,
    dialect: Dialect,
    cli: Box<dyn Cli>,
}

impl Docker {
    pub fn new(config: Config, cli: Box<dyn Cli>) -> Self {
        Self {
            config,
            dialect: Dialect {
                netns: Netns::Service,
                // Docker Desktop, OrbStack and Docker Engine with an explicit
                // `host-gateway` all answer to this name; Podman does not,
                // which is why it is here and not in the generator.
                host_alias: "host.docker.internal".to_string(),
                userns: None,
            },
            cli,
        }
    }

    pub fn real() -> Self {
        Self::new(Config::default(), Box::new(RealCli))
    }

    /// A Compose invocation as data, so a test can read it without an engine.
    pub fn compose_command(&self, file: &Path, project: &str, args: &[&str]) -> CommandSpec {
        let mut words = self.config.compose.iter();
        let program = words
            .next()
            .cloned()
            .unwrap_or_else(|| "docker".to_string());
        let mut all: Vec<String> = words.cloned().collect();
        all.extend([
            "-p".to_string(),
            project.to_string(),
            "-f".to_string(),
            file.display().to_string(),
        ]);
        all.extend(args.iter().map(|a| a.to_string()));
        CommandSpec {
            program,
            args: all,
            cwd: file.parent().unwrap_or(Path::new(".")).to_path_buf(),
            env: BTreeMap::new(),
        }
    }

    fn engine_command(&self, args: &[&str]) -> CommandSpec {
        CommandSpec {
            program: self.config.engine.clone(),
            args: args.iter().map(|a| a.to_string()).collect(),
            cwd: PathBuf::from("."),
            env: BTreeMap::new(),
        }
    }

    fn run(&self, verb: &'static str, spec: &CommandSpec) -> Result<String, EngineError> {
        let out = self.cli.run(spec)?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(EngineError::Command {
                verb,
                status: out
                    .status
                    .code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "signalled".to_string()),
                stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
            })
        }
    }
}

impl Engine for Docker {
    fn name(&self) -> &'static str {
        "docker"
    }

    fn dialect(&self) -> &Dialect {
        &self.dialect
    }

    fn up(&self, file: &Path, project: &str) -> Result<(), EngineError> {
        // `--wait` is the whole point: it blocks until every service is
        // started and every healthcheck passes, so a firewall that fails to
        // pose its rules stops the profile here rather than letting an agent
        // run unfenced.
        self.run(
            "up",
            &self.compose_command(file, project, &["up", "-d", "--wait"]),
        )?;
        Ok(())
    }

    fn pause(&self, file: &Path, project: &str, services: &[&str]) -> Result<(), EngineError> {
        let mut args = vec!["pause"];
        args.extend_from_slice(services);
        self.run("pause", &self.compose_command(file, project, &args))?;
        Ok(())
    }

    fn unpause(&self, file: &Path, project: &str, services: &[&str]) -> Result<(), EngineError> {
        let mut args = vec!["unpause"];
        args.extend_from_slice(services);
        self.run("unpause", &self.compose_command(file, project, &args))?;
        Ok(())
    }

    fn kill(&self, file: &Path, project: &str, services: &[&str]) -> Result<(), EngineError> {
        // `kill` and not `stop --timeout 0`: nothing is asked and nothing is
        // waited for, which is the whole difference between the emergency
        // brake and a clean stop (SPEC 4.3).
        let mut args = vec!["kill"];
        args.extend_from_slice(services);
        self.run("kill", &self.compose_command(file, project, &args))?;
        Ok(())
    }

    fn stop(&self, file: &Path, project: &str, services: &[&str]) -> Result<(), EngineError> {
        let timeout = self.config.stop_timeout.to_string();
        let mut stop = vec!["stop", "--timeout", &timeout];
        stop.extend_from_slice(services);
        self.run("stop", &self.compose_command(file, project, &stop))?;

        // `rm -f` and never `down`: the project's services were levied once
        // for the slot and must survive the profile switch with the state the
        // previous role left in them.
        let mut remove = vec!["rm", "-f", "-v"];
        remove.extend_from_slice(services);
        self.run("rm", &self.compose_command(file, project, &remove))?;
        Ok(())
    }

    fn down(&self, file: &Path, project: &str, volumes: bool) -> Result<(), EngineError> {
        let timeout = self.config.stop_timeout.to_string();
        let mut args = vec!["down", "--timeout", &timeout];
        if volumes {
            args.push("--volumes");
        }
        // Never `--remove-orphans`: a profile file that omits a service must
        // not be a reason to delete it (SPEC 4.2, measured).
        self.run("down", &self.compose_command(file, project, &args))?;
        Ok(())
    }

    fn exec(
        &self,
        file: &Path,
        project: &str,
        service: &str,
        argv: &[String],
    ) -> Result<ExecOutput, EngineError> {
        let mut args = vec!["exec", "-T", service];
        args.extend(argv.iter().map(String::as_str));
        let spec = self.compose_command(file, project, &args);
        let out = self.cli.run(&spec)?;
        Ok(ExecOutput {
            status: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    fn container_of(
        &self,
        file: &Path,
        project: &str,
        service: &str,
    ) -> Result<Option<String>, EngineError> {
        let out = self.run(
            "ps",
            &self.compose_command(file, project, &["ps", "-q", service]),
        )?;
        Ok(out
            .lines()
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string))
    }

    fn detached_command(
        &self,
        file: &Path,
        project: &str,
        service: &str,
        options: &[String],
        argv: &[String],
    ) -> CommandSpec {
        // Order is not cosmetic: `exec [options] SERVICE COMMAND`. An option
        // after the service name becomes part of the command.
        let mut args = vec!["exec".to_string(), "-T".to_string()];
        args.extend(options.iter().cloned());
        args.push(service.to_string());
        args.extend(argv.iter().cloned());
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        self.compose_command(file, project, &borrowed)
    }

    fn liveness(&self, container: &str) -> Result<Liveness, EngineError> {
        let spec = self.engine_command(&[
            "inspect",
            "-f",
            "{{.State.Running}} {{.State.Paused}} {{.State.ExitCode}}",
            container,
        ]);
        let out = self.cli.run(&spec)?;
        if !out.status.success() {
            // The engine not knowing the container is an answer, not a
            // failure: a machine that slept, an engine that restarted without
            // its containers, a slot removed (SPEC 4.2).
            return Ok(Liveness::Gone);
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let mut fields = text.split_whitespace();
        // Measured on Docker 28: a paused container answers `true true 0`, so
        // reading only the first field reports it as running.
        match (fields.next(), fields.next(), fields.next()) {
            (Some("true"), Some("true"), _) => Ok(Liveness::Paused),
            (Some("true"), _, _) => Ok(Liveness::Running),
            (Some("false"), _, Some(code)) => code
                .parse()
                .map(Liveness::Exited)
                .map_err(|_| EngineError::Unreadable(text.trim().to_string())),
            _ => Err(EngineError::Unreadable(text.trim().to_string())),
        }
    }
}
