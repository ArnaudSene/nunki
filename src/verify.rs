//! `hq verify <mission>` (SPEC 4.2, 4.4, 4.5).
//!
//! One verb for a whole phase, because it is one state machine: the coder's
//! gates, then the integration mission, then the security mission, each with
//! its own gates, iteration rules applied at the first red, and the hand
//! back to the human for the push. It is **resumable** — the state is
//! persisted at every transition, and running it again picks up where it
//! stopped.
//!
//! What it does not do: launch an agent. Lifting the system profile and
//! starting the application belong to their own piece, so when the flow
//! reaches a stage that needs a run, `verify` says which role is owed one
//! and stops. Saying it is better than pretending the mission is stuck.
//!
//! The slot's lock is taken **once**, here, at the top: everything under it
//! — gate 6, gate 7, every `hq exec` — runs inside it and never asks again
//! (SPEC 4.2).

use std::sync::Arc;

use crate::engine::Engine;
use crate::gate;
use crate::harness::{Role, RunState};
use crate::mission::dir::Paths;
use crate::mission::flow::{Event, Handover, Stage, Work};
use crate::project::Project;
use crate::state::{MissionState, Store, lock::SlotLock};

/// One thing `verify` did, in the order it did it. The caller prints them;
/// the state file is what actually carries the mission forward.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// The gates were played, and what they said.
    Gates {
        role: Role,
        report: Box<gate::Report>,
    },
    /// The flow moved.
    Moved { to: Stage },
    /// A role is owed a run, and `hq` cannot launch it yet.
    NeedsRun { role: Role, why: String },
    /// Every declared stage is green; the human validates and `hq push`
    /// pushes.
    Verified,
    /// The flow stopped and the human decides (SPEC 4.5).
    AwaitingHuman(Handover),
    /// The security agent came back with findings: the HQ iterates or the
    /// human lifts them.
    Findings { report: String },
}

#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    #[error("mission {0} has not started — `hq mission start {0} --slot <slot>` does that")]
    NotStarted(String),
    #[error(
        "a run is still going in slot {slot}: verifying now would judge a tree the \
         agent is still writing — `hq mission stop {mission}` ends its turn first"
    )]
    RunInProgress { mission: String, slot: String },
    #[error(transparent)]
    Gate(#[from] gate::GateError),
    #[error(transparent)]
    State(#[from] crate::state::StateError),
    #[error(transparent)]
    Lock(#[from] crate::state::LockError),
    #[error(transparent)]
    Slot(#[from] crate::slot::SlotError),
}

/// Play the phase as far as it goes, and say what happened.
pub fn verify(
    project: &Project,
    id: &str,
    engine: Arc<dyn Engine>,
) -> Result<Vec<Step>, VerifyError> {
    let store = Store::open(&project.hq_root)?;
    let mut state = store
        .load(id)
        .map_err(|_| VerifyError::NotStarted(id.to_string()))?;

    // Once, at the top. Everything underneath runs under it (SPEC 4.2).
    let _lock = SlotLock::acquire(&project.hq_root.join("locks"), &state.slot, "verify")?;

    // Gates read a tree. A tree an agent is still writing is not a tree to
    // judge — this is the interlock `hq exec --tree` deliberately does not
    // have, and this is where it belongs.
    refuse_while_running(project, id, &state)?;

    let slot = crate::slot::find(project, &state.slot)?;
    let paths = Paths::of(&project.hq_root, id);
    let stack = project
        .config
        .stacks
        .first()
        .cloned()
        .unwrap_or_else(|| "rust".to_string());

    let mut steps = Vec::new();
    loop {
        // The frozen header, never the file: from the moment a mission
        // starts, `hq` reads its own copy (SPEC 4.1).
        let header = state.flow.header().clone();
        let subject = gate::Subject {
            role: role_of(state.flow.stage()),
            tree: &slot.tree,
            journal: &paths.journal,
            pr: &paths.pr,
            verdict: &paths.verdict,
            mission_dir: &paths.dir,
            header: &header,
            protected_branches: &project.config.protected_branches,
            protected_paths: &project.config.protected_paths,
        };
        let verification = gate::Verification {
            project,
            slot: &slot,
            engine: engine.clone(),
            stack: &stack,
        };

        match state.flow.stage().clone() {
            // A lot is still owed a run. Gates 1 to 4 are played anyway,
            // because a forbidden commit found at the end of the first lot
            // costs one run and found at the end costs the mission.
            Stage::Coding { work, .. } => {
                let report = gate::after_run(&subject)?;
                let failure = report.failure();
                steps.push(Step::Gates {
                    role: subject.role,
                    report: Box::new(report),
                });
                let owed = match failure {
                    Some(reason) => {
                        store.apply(
                            &mut state,
                            Event::GatesFailed {
                                reason: reason.clone(),
                            },
                        )?;
                        steps.push(Step::Moved {
                            to: state.flow.stage().clone(),
                        });
                        format!("the gates were red, and fixing them is a run: {reason}")
                    }
                    None => what_is_owed(&work, &header),
                };
                // Either way the coder is owed a run, and `verify` launches
                // none: looping here would spend every attempt the lot has
                // against a tree nobody has touched in between.
                if !matches!(state.flow.stage(), Stage::Coding { .. }) {
                    continue;
                }
                steps.push(Step::NeedsRun {
                    role: Role::Coder,
                    why: owed,
                });
                return Ok(steps);
            }

            // The final verification: every gate, including the deliverable,
            // the battery and the mutation campaign.
            Stage::Gates => {
                let report = gate::at_verification(&subject, &verification)?;
                let failure = report.failure();
                steps.push(Step::Gates {
                    role: subject.role,
                    report: Box::new(report),
                });
                let event = match failure {
                    Some(reason) => Event::GatesFailed { reason },
                    None => Event::GatesPassed,
                };
                store.apply(&mut state, event)?;
                steps.push(Step::Moved {
                    to: state.flow.stage().clone(),
                });
            }

            Stage::Integration { .. } => {
                steps.push(Step::NeedsRun {
                    role: Role::Integrator,
                    why: "the integration mission has not run: `hq` lifts the system \
                          profile and starts the application for it, and that verb does \
                          not exist yet"
                        .into(),
                });
                return Ok(steps);
            }
            Stage::SecurityAgent { .. } => {
                steps.push(Step::NeedsRun {
                    role: Role::Security,
                    why: "the security mission has not run: it needs the livrable \
                          started in its own profile, and that verb does not exist yet"
                        .into(),
                });
                return Ok(steps);
            }
            Stage::Findings { report } => {
                steps.push(Step::Findings { report });
                return Ok(steps);
            }
            Stage::AwaitingHuman(handover) => {
                steps.push(Step::AwaitingHuman(handover));
                return Ok(steps);
            }
            Stage::Verified => {
                // Persisted before anything is printed: a session that dies
                // after the print must not lose the verdict.
                steps.push(Step::Verified);
                return Ok(steps);
            }
        }
    }
}

/// Which role the current stage belongs to, for the gates that role owes.
fn role_of(stage: &Stage) -> Role {
    match stage {
        Stage::Integration { .. } => Role::Integrator,
        Stage::SecurityAgent { .. } => Role::Security,
        _ => Role::Coder,
    }
}

fn what_is_owed(work: &Work, header: &crate::mission::Header) -> String {
    match work {
        Work::Lot(i) => match header.lots.get(*i) {
            Some(lot) => format!("lot {} — {} is owed a run", lot.id, lot.title),
            None => "a lot is owed a run".to_string(),
        },
        Work::Volet { n, cause } => format!("volet {n} is owed a run: {cause}"),
    }
}

/// Refuse while the agent is still writing. A gate that reads a tree
/// mid-run reads a tree that is not finished, and its verdict would be about
/// a moment nobody chose.
fn refuse_while_running(
    project: &Project,
    id: &str,
    state: &MissionState,
) -> Result<(), VerifyError> {
    let Some(handle) = &state.run else {
        return Ok(());
    };
    let harness = crate::harness::claude_code::ClaudeCode::new(
        Default::default(),
        Box::new(harness_spawner(project, &state.slot, &handle.session.0)),
    );
    // A run `hq` cannot reach is not a run in progress: it says so elsewhere
    // (`hq mission status`), and refusing to verify because the engine is
    // down would be the same lie in another place.
    if let Ok(RunState::Running(_)) = crate::harness::Harness::state(&harness, handle) {
        return Err(VerifyError::RunInProgress {
            mission: id.to_string(),
            slot: state.slot.clone(),
        });
    }
    Ok(())
}

fn harness_spawner(
    project: &Project,
    slot: &str,
    session: &str,
) -> crate::engine::spawn::ContainerSpawner {
    let engine: Arc<dyn Engine> = Arc::new(crate::engine::docker::Docker::real());
    let file = crate::run::profile_path(project, slot);
    let compose_project =
        crate::compose::project_name(slot).unwrap_or_else(|_| format!("hq-{slot}"));
    crate::engine::spawn::ContainerSpawner::new(
        engine,
        file,
        &compose_project,
        crate::compose::AGENT_SERVICE,
    )
    .identified_by(session)
}
