//! A fake harness for the engine's tests (SPEC 4.3): it answers scripted
//! states without a container, so the flow and the checks can be tested
//! with no Docker and no model.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use super::{
    Exposure, GuardSetup, Harness, HarnessError, Outcome, Provisioning, Role, RunHandle,
    RunRequest, RunState, SessionId,
};

/// Scripted answers for one session: each call to `state` pops the next one;
/// the last answer sticks.
#[derive(Default)]
pub struct FakeHarness {
    scripts: Mutex<HashMap<SessionId, Vec<RunState>>>,
    launched: Mutex<Vec<RunRequest>>,
    stopped: Mutex<Vec<SessionId>>,
}

impl FakeHarness {
    pub fn new() -> Self {
        Self::default()
    }

    /// Script the states a session will report, in order.
    pub fn script(&self, session: SessionId, states: Vec<RunState>) {
        self.scripts.lock().unwrap().insert(session, states);
    }

    /// Convenience: a session that finishes at once with `outcome`.
    pub fn finish_with(&self, session: SessionId, outcome: Outcome) {
        self.script(session, vec![RunState::Finished(outcome)]);
    }

    pub fn launched(&self) -> Vec<RunRequest> {
        self.launched.lock().unwrap().clone()
    }

    pub fn stopped(&self) -> Vec<SessionId> {
        self.stopped.lock().unwrap().clone()
    }
}

impl Harness for FakeHarness {
    fn name(&self) -> &'static str {
        "fake"
    }

    fn token_env(&self) -> &'static str {
        "HQ_FAKE_TOKEN"
    }

    fn provision(&self) -> Provisioning {
        Provisioning::default()
    }

    fn launch(
        &self,
        request: &RunRequest,
        _guards: &GuardSetup,
        _exposure: &Exposure,
    ) -> Result<RunHandle, HarnessError> {
        self.launched.lock().unwrap().push(request.clone());
        Ok(RunHandle {
            session: request.session.clone(),
            container: format!("fake-{}", request.session.0),
            pid: Some(1),
            log: request
                .workspace
                .mission_dir
                .join("runs")
                .join(format!("{}.jsonl", request.session.0)),
        })
    }

    fn state(&self, handle: &RunHandle) -> Result<RunState, HarnessError> {
        let mut scripts = self.scripts.lock().unwrap();
        let states = scripts
            .get_mut(&handle.session)
            .ok_or_else(|| HarnessError::UnknownRun(handle.session.clone()))?;
        if states.len() > 1 {
            Ok(states.remove(0))
        } else {
            states
                .first()
                .cloned()
                .ok_or_else(|| HarnessError::UnknownRun(handle.session.clone()))
        }
    }

    fn stop(&self, handle: &RunHandle) -> Result<(), HarnessError> {
        self.stopped.lock().unwrap().push(handle.session.clone());
        Ok(())
    }

    fn expose(&self, _role: Role, prompt_file: PathBuf) -> Exposure {
        Exposure::SystemPromptFile(prompt_file)
    }
}
