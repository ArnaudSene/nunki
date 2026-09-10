//! A scripted engine, so the flow can be tested without a container.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::{Dialect, Engine, EngineError, ExecOutput, Liveness, Netns};

/// What was asked of the engine, in order, for a test to assert on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    Up(String),
    Stop(String, Vec<String>),
    Down(String, bool),
    Exec(String, String, Vec<String>),
    ContainerOf(String, String),
    Liveness(String),
}

pub struct FakeEngine {
    dialect: Dialect,
    pub calls: Mutex<Vec<Call>>,
    containers: Mutex<BTreeMap<String, String>>,
    liveness: Mutex<BTreeMap<String, Liveness>>,
    exec_result: Mutex<ExecOutput>,
    fail_up: Mutex<Option<String>>,
}

impl Default for FakeEngine {
    fn default() -> Self {
        Self {
            dialect: Dialect {
                netns: Netns::Service,
                host_alias: "host.docker.internal".to_string(),
                userns: None,
            },
            calls: Mutex::new(Vec::new()),
            containers: Mutex::new(BTreeMap::new()),
            liveness: Mutex::new(BTreeMap::new()),
            exec_result: Mutex::new(ExecOutput {
                status: 0,
                stdout: String::new(),
                stderr: String::new(),
            }),
            fail_up: Mutex::new(None),
        }
    }
}

impl FakeEngine {
    pub fn with_container(self, service: &str, container: &str) -> Self {
        self.containers
            .lock()
            .unwrap()
            .insert(service.to_string(), container.to_string());
        self
    }

    pub fn with_liveness(self, container: &str, state: Liveness) -> Self {
        self.liveness
            .lock()
            .unwrap()
            .insert(container.to_string(), state);
        self
    }

    pub fn with_exec(self, out: ExecOutput) -> Self {
        *self.exec_result.lock().unwrap() = out;
        self
    }

    /// Make `up` fail, the way a sidecar that never becomes healthy does.
    pub fn failing_up(self, why: &str) -> Self {
        *self.fail_up.lock().unwrap() = Some(why.to_string());
        self
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, call: Call) {
        self.calls.lock().unwrap().push(call);
    }
}

impl Engine for FakeEngine {
    fn name(&self) -> &'static str {
        "fake"
    }

    fn dialect(&self) -> &Dialect {
        &self.dialect
    }

    fn up(&self, _file: &Path, project: &str) -> Result<(), EngineError> {
        self.record(Call::Up(project.to_string()));
        match self.fail_up.lock().unwrap().clone() {
            Some(stderr) => Err(EngineError::Command {
                verb: "up",
                status: "1".to_string(),
                stderr,
            }),
            None => Ok(()),
        }
    }

    fn stop(&self, _file: &Path, project: &str, services: &[&str]) -> Result<(), EngineError> {
        self.record(Call::Stop(
            project.to_string(),
            services.iter().map(|s| s.to_string()).collect(),
        ));
        Ok(())
    }

    fn down(&self, _file: &Path, project: &str, volumes: bool) -> Result<(), EngineError> {
        self.record(Call::Down(project.to_string(), volumes));
        Ok(())
    }

    fn exec(
        &self,
        _file: &Path,
        project: &str,
        service: &str,
        argv: &[String],
    ) -> Result<ExecOutput, EngineError> {
        self.record(Call::Exec(
            project.to_string(),
            service.to_string(),
            argv.to_vec(),
        ));
        Ok(self.exec_result.lock().unwrap().clone())
    }

    fn container_of(
        &self,
        _file: &Path,
        project: &str,
        service: &str,
    ) -> Result<Option<String>, EngineError> {
        self.record(Call::ContainerOf(project.to_string(), service.to_string()));
        Ok(self.containers.lock().unwrap().get(service).cloned())
    }

    fn liveness(&self, container: &str) -> Result<Liveness, EngineError> {
        self.record(Call::Liveness(container.to_string()));
        Ok(self
            .liveness
            .lock()
            .unwrap()
            .get(container)
            .cloned()
            .unwrap_or(Liveness::Gone))
    }
}

/// A path any test can hand to an engine that never reads it.
pub fn nowhere() -> PathBuf {
    PathBuf::from("/nowhere/profile.yml")
}
