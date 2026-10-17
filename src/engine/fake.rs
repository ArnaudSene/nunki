//! A scripted engine, so the flow can be tested without a container.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::{Container, Dialect, Engine, EngineError, ExecOutput, Liveness, Netns};

/// What was asked of the engine, in order, for a test to assert on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    Up(String),
    Stop(String, Vec<String>),
    Pause(String, Vec<String>),
    Unpause(String, Vec<String>),
    Kill(String, Vec<String>),
    Down(String, bool),
    Exec(String, String, Vec<String>),
    ContainerOf(String, String),
    Liveness(String),
    Containers(String),
    Remove(Vec<String>),
    RemoveVolume(String),
}

pub struct FakeEngine {
    dialect: Dialect,
    pub calls: Mutex<Vec<Call>>,
    containers: Mutex<BTreeMap<String, String>>,
    liveness: Mutex<BTreeMap<String, Liveness>>,
    /// Answers given in turn for a container, the last kept, before
    /// `liveness` is looked at.
    liveness_queue: Mutex<BTreeMap<String, Vec<Liveness>>>,
    exec_result: Mutex<ExecOutput>,
    exec_queue: Mutex<Vec<ExecOutput>>,
    /// Answers given to an `exec` whose argv holds a fragment, before the
    /// queue and the default are looked at: the first that matches.
    exec_when: Mutex<Vec<(String, ExecOutput)>>,
    fail_up: Mutex<Option<String>>,
    fail_stop: Mutex<Option<String>>,
    lifted: Mutex<Vec<Container>>,
    /// The named volumes the engine holds. One is **in use** while any
    /// container of the project remains, as Docker refuses `volume rm` on a
    /// volume a container mounts, stopped or not.
    volumes: Mutex<std::collections::BTreeSet<String>>,
    /// Volumes some other project's container mounts: never removable here.
    held: Mutex<std::collections::BTreeSet<String>>,
    /// Containers the engine is asked about and does not answer for.
    unanswered: Mutex<std::collections::BTreeSet<String>>,
    /// Containers an `up` makes, listed from then on.
    made_on_up: Mutex<Vec<Container>>,
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
            liveness_queue: Mutex::new(BTreeMap::new()),
            exec_result: Mutex::new(ExecOutput {
                status: 0,
                stdout: String::new(),
                stderr: String::new(),
            }),
            exec_queue: Mutex::new(Vec::new()),
            exec_when: Mutex::new(Vec::new()),
            fail_up: Mutex::new(None),
            fail_stop: Mutex::new(None),
            lifted: Mutex::new(Vec::new()),
            volumes: Mutex::new(Default::default()),
            held: Mutex::new(Default::default()),
            unanswered: Mutex::new(Default::default()),
            made_on_up: Mutex::new(Vec::new()),
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

    /// A container already in the project before anything is asked: what a
    /// previous profile left there.
    pub fn with_lifted(self, container: Container) -> Self {
        self.lifted.lock().unwrap().push(container);
        self
    }

    /// A named volume the engine holds.
    pub fn with_volume(self, name: &str) -> Self {
        self.volumes.lock().unwrap().insert(name.to_string());
        self
    }

    /// A named volume a container outside the project mounts: it exists,
    /// and removing it fails whatever is taken down.
    pub fn with_volume_held(self, name: &str) -> Self {
        self.held.lock().unwrap().insert(name.to_string());
        self.with_volume(name)
    }

    /// The named volumes the engine still holds.
    pub fn volumes(&self) -> Vec<String> {
        self.volumes.lock().unwrap().iter().cloned().collect()
    }

    pub fn with_liveness(self, container: &str, state: Liveness) -> Self {
        self.liveness
            .lock()
            .unwrap()
            .insert(container.to_string(), state);
        self
    }

    /// Have the next `up` make `container`, whether it then fails or not:
    /// what Compose does to a service it finds no container for.
    pub fn making_on_up(self, container: Container) -> Self {
        self.made_on_up.lock().unwrap().push(container);
        self
    }

    /// Fail every question about `container`: an engine that cannot be
    /// reached, rather than one that knows nothing of it.
    pub fn with_unanswered(self, container: &str) -> Self {
        self.unanswered
            .lock()
            .unwrap()
            .insert(container.to_string());
        self
    }

    /// Answer each question about `container` in turn, and keep answering
    /// with the last: a container that changes state between two questions,
    /// as one paused by a human does.
    pub fn with_livenesses(self, container: &str, states: Vec<Liveness>) -> Self {
        self.liveness_queue
            .lock()
            .unwrap()
            .insert(container.to_string(), states);
        self
    }

    pub fn with_exec(self, out: ExecOutput) -> Self {
        *self.exec_result.lock().unwrap() = out;
        self
    }

    /// Answer each `exec` in turn, and keep answering with the last.
    ///
    /// A gate that runs a script first refreshes the clean copy of `HEAD`,
    /// so a single answer is given to both — and a test about the script's
    /// own status would be a test about the refresh's. This is what lets one
    /// be said without the other.
    pub fn with_execs(self, outs: Vec<ExecOutput>) -> Self {
        *self.exec_queue.lock().unwrap() = outs;
        self
    }

    /// Answer any `exec` one of whose arguments contains `fragment` with
    /// `out`, whatever its turn: what lets a test script the stop, the check
    /// after it and the refresh of the copy apart, rather than by an order
    /// the code under test is free to change.
    pub fn with_exec_when(self, fragment: &str, out: ExecOutput) -> Self {
        self.exec_when
            .lock()
            .unwrap()
            .push((fragment.to_string(), out));
        self
    }

    /// Make `stop` fail, the way an engine that cannot remove a container
    /// does.
    pub fn failing_stop(self, why: &str) -> Self {
        *self.fail_stop.lock().unwrap() = Some(why.to_string());
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
        let made: Vec<Container> = std::mem::take(&mut *self.made_on_up.lock().unwrap());
        self.lifted.lock().unwrap().extend(made);
        match self.fail_up.lock().unwrap().clone() {
            Some(stderr) => Err(EngineError::Command {
                verb: "up",
                status: "1".to_string(),
                stderr,
            }),
            None => Ok(()),
        }
    }

    fn pause(&self, _file: &Path, project: &str, services: &[&str]) -> Result<(), EngineError> {
        self.record(Call::Pause(
            project.to_string(),
            services.iter().map(|s| s.to_string()).collect(),
        ));
        Ok(())
    }

    fn unpause(&self, _file: &Path, project: &str, services: &[&str]) -> Result<(), EngineError> {
        self.record(Call::Unpause(
            project.to_string(),
            services.iter().map(|s| s.to_string()).collect(),
        ));
        Ok(())
    }

    fn kill(&self, _file: &Path, project: &str, services: &[&str]) -> Result<(), EngineError> {
        self.record(Call::Kill(
            project.to_string(),
            services.iter().map(|s| s.to_string()).collect(),
        ));
        Ok(())
    }

    fn stop(&self, _file: &Path, project: &str, services: &[&str]) -> Result<(), EngineError> {
        self.record(Call::Stop(
            project.to_string(),
            services.iter().map(|s| s.to_string()).collect(),
        ));
        match self.fail_stop.lock().unwrap().clone() {
            Some(stderr) => Err(EngineError::Command {
                verb: "stop",
                status: "1".to_string(),
                stderr,
            }),
            None => Ok(()),
        }
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
        if let Some((_, out)) = self
            .exec_when
            .lock()
            .unwrap()
            .iter()
            .find(|(fragment, _)| argv.iter().any(|a| a.contains(fragment.as_str())))
        {
            return Ok(out.clone());
        }
        let mut queue = self.exec_queue.lock().unwrap();
        if queue.len() > 1 {
            return Ok(queue.remove(0));
        }
        if let Some(last) = queue.first() {
            return Ok(last.clone());
        }
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

    fn detached_command(
        &self,
        _file: &Path,
        project: &str,
        service: &str,
        options: &[String],
        argv: &[String],
    ) -> crate::harness::spawn::CommandSpec {
        crate::harness::spawn::CommandSpec {
            program: "fake-engine".to_string(),
            args: [project.to_string()]
                .into_iter()
                .chain(options.iter().cloned())
                .chain([service.to_string()])
                .chain(argv.iter().cloned())
                .collect(),
            cwd: PathBuf::from("."),
            env: BTreeMap::new(),
        }
    }

    fn liveness(&self, container: &str) -> Result<Liveness, EngineError> {
        self.record(Call::Liveness(container.to_string()));
        if self.unanswered.lock().unwrap().contains(container) {
            return Err(EngineError::Unreadable(format!(
                "no answer about {container}"
            )));
        }
        if let Some(queue) = self.liveness_queue.lock().unwrap().get_mut(container) {
            if queue.len() > 1 {
                return Ok(queue.remove(0));
            }
            if let Some(last) = queue.first() {
                return Ok(last.clone());
            }
        }
        Ok(self
            .liveness
            .lock()
            .unwrap()
            .get(container)
            .cloned()
            .unwrap_or(Liveness::Gone))
    }

    fn containers(&self, project: &str) -> Result<Vec<Container>, EngineError> {
        self.record(Call::Containers(project.to_string()));
        Ok(self.lifted.lock().unwrap().clone())
    }

    fn removal(&self, container: &str) -> crate::harness::spawn::CommandSpec {
        crate::harness::spawn::CommandSpec {
            program: "fake-engine".to_string(),
            args: vec!["rm".to_string(), container.to_string()],
            cwd: PathBuf::from("."),
            env: BTreeMap::new(),
        }
    }

    fn remove(&self, containers: &[String]) -> Result<(), EngineError> {
        self.record(Call::Remove(containers.to_vec()));
        self.lifted
            .lock()
            .unwrap()
            .retain(|c| !containers.contains(&c.id));
        Ok(())
    }

    fn remove_volume(&self, name: &str) -> Result<bool, EngineError> {
        self.record(Call::RemoveVolume(name.to_string()));
        if !self.volumes.lock().unwrap().contains(name) {
            return Ok(false);
        }
        if !self.lifted.lock().unwrap().is_empty() || self.held.lock().unwrap().contains(name) {
            return Err(EngineError::Command {
                verb: "volume rm",
                status: "1".to_string(),
                stderr: format!("Error response from daemon: remove {name}: volume is in use"),
            });
        }
        self.volumes.lock().unwrap().remove(name);
        Ok(true)
    }
}

/// A path any test can hand to an engine that never reads it.
pub fn nowhere() -> PathBuf {
    PathBuf::from("/nowhere/profile.yml")
}
