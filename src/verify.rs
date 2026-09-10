//! `hq verify <mission>` (SPEC 4.2, 4.4, 4.5).
//!
//! One verb for a whole phase, because it is one state machine: the coder's
//! gates, then the integration mission, then the security mission, each with
//! its own gates, iteration rules applied at the first red, and the hand
//! back to the human for the push. It is **resumable** — the state is
//! persisted at every transition, and running it again picks up where it
//! stopped.
//!
//! What it launches, and what it does not. It lifts a role's profile, starts
//! the application in it and launches that role's run, then returns: a run
//! takes hours and `verify` never waits on one. The next `verify` reads that
//! run back — its verdict, or why there is none — and moves. What it still
//! does not do is lift a `FINDINGS` verdict: the flow has both the events for
//! it, and no verb applies either yet.
//!
//! The slot's lock is taken **once**, here, at the top: everything under it
//! — gate 6, gate 7, every `hq exec` — runs inside it and never asks again
//! (SPEC 4.2).

use std::sync::Arc;

use crate::engine::Engine;
use crate::gate;
use crate::harness::{Outcome, Role, RunHandle, RunState};
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
    /// A run was launched for this role, and `verify` returned: a run takes
    /// hours, and nothing here waits on one.
    Launched {
        role: Role,
        /// How the application was started for it, or why it was not.
        application: String,
    },
    /// A run was recorded and the engine could not be asked about it.
    /// Nothing is decided on a silence (SPEC 4.2, "la reprise re-dérive
    /// avant de décider").
    Unreachable { role: Role, why: String },
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
    #[error(transparent)]
    Run(#[from] crate::run::RunError),
    #[error(transparent)]
    Git(#[from] crate::git::GitError),
}

/// Play the phase as far as it goes, and say what happened.
pub fn verify(
    project: &Project,
    id: &str,
    engine: Arc<dyn Engine>,
    engine_bin: &str,
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
    refuse_while_running(project, engine.clone(), id, &state)?;

    let slot = crate::slot::find(project, &state.slot)?;
    let paths = Paths::of(&project.hq_root, id);
    let stack = crate::run::stack_of(project);

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

            // The integration mission. Two branches, and which one applies
            // is decided by whether a run was already launched for this
            // stage — never by a timer, and never by launching a second one
            // to see.
            Stage::Integration { attempt } => {
                if let Some(handle) = state.run.clone() {
                    match read_back(project, engine.clone(), &state, &handle) {
                        Ended::Unreachable(why) => {
                            steps.push(Step::Unreachable {
                                role: Role::Integrator,
                                why,
                            });
                            return Ok(steps);
                        }
                        Ended::With(outcome) => {
                            let event = concluded(
                                Role::Integrator,
                                outcome,
                                paths.verdict.as_path(),
                                &crate::git::head(&slot.tree)?,
                            );
                            // Forgotten before the transition is written: an
                            // attempt that stays recorded is a run the next
                            // `verify` would read back a second time.
                            state.run = None;
                            state.app = None;
                            store.apply(&mut state, event)?;
                            steps.push(Step::Moved {
                                to: state.flow.stage().clone(),
                            });
                            continue;
                        }
                    }
                }

                let launched = crate::run::launch(&crate::run::Launching {
                    project,
                    slot: &slot,
                    engine: engine.clone(),
                    engine_bin,
                    paths: &paths,
                    header: &header,
                    role: Role::Integrator,
                    lot: "integration".to_string(),
                    attempt,
                })?;
                let application = describe(&launched);
                state.run = Some(launched.run);
                state.app = launched.app;
                store.save(&state)?;
                steps.push(Step::Launched {
                    role: Role::Integrator,
                    application,
                });
                return Ok(steps);
            }
            // The security mission, on the same two branches as the
            // integration one — the profile differs, the reading does not.
            Stage::SecurityAgent { attempt } => {
                if let Some(handle) = state.run.clone() {
                    match read_back(project, engine.clone(), &state, &handle) {
                        Ended::Unreachable(why) => {
                            steps.push(Step::Unreachable {
                                role: Role::Security,
                                why,
                            });
                            return Ok(steps);
                        }
                        Ended::With(outcome) => {
                            let event = concluded(
                                Role::Security,
                                outcome,
                                paths.verdict.as_path(),
                                &crate::git::head(&slot.tree)?,
                            );
                            state.run = None;
                            state.app = None;
                            store.apply(&mut state, event)?;
                            steps.push(Step::Moved {
                                to: state.flow.stage().clone(),
                            });
                            continue;
                        }
                    }
                }

                let launched = crate::run::launch(&crate::run::Launching {
                    project,
                    slot: &slot,
                    engine: engine.clone(),
                    engine_bin,
                    paths: &paths,
                    header: &header,
                    role: Role::Security,
                    lot: "security".to_string(),
                    attempt,
                })?;
                let application = describe(&launched);
                state.run = Some(launched.run);
                state.app = launched.app;
                store.save(&state)?;
                steps.push(Step::Launched {
                    role: Role::Security,
                    application,
                });
                return Ok(steps);
            }
            // Terminal for now, and said as such: `Event::Iterate` and
            // `Event::HumanAccepted` exist in the flow and no verb applies
            // either, so a `FINDINGS` verdict parks the mission until
            // `hq mission accept` and the iteration decision are written.
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

/// What a recorded run came to, as the engine answers it.
enum Ended {
    With(Outcome),
    /// The question could not be put. A run `hq` cannot reach is not a run
    /// that failed, and turning one into the other would consume an attempt
    /// on a machine that was asleep.
    Unreachable(String),
}

fn read_back(
    project: &Project,
    engine: Arc<dyn Engine>,
    state: &MissionState,
    handle: &RunHandle,
) -> Ended {
    let harness = crate::harness::claude_code::ClaudeCode::new(
        Default::default(),
        Box::new(harness_spawner(
            project,
            engine,
            &state.slot,
            &handle.session.0,
        )),
    );
    match crate::harness::Harness::state(&harness, handle) {
        Ok(RunState::Finished(outcome)) => Ended::With(outcome),
        // `refuse_while_running` already returned for a running run, so this
        // is a run that started running between the two questions.
        Ok(RunState::Running(_)) => Ended::Unreachable(
            "the run started again between two questions; ask once more".to_string(),
        ),
        Err(e) => Ended::Unreachable(e.to_string()),
    }
}

/// The event a finished role run yields.
///
/// A run that finished on its own has to have left a verdict: SPEC 4.4 makes
/// the verdict the role's conclusion and pins it to a `HEAD`. Finished
/// without one, or with one that names another commit or another role, is a
/// run that did not honour the contract — a mission failure, which costs an
/// attempt. A harness failure costs none, and that distinction is the whole
/// of SPEC 4.3 on this point.
fn concluded(role: Role, outcome: Outcome, verdict: &std::path::Path, head: &str) -> Event {
    let Outcome::Finished(_) = &outcome else {
        return Event::RunEnded {
            outcome,
            lot_done: false,
        };
    };
    match read_verdict(verdict, role, head) {
        Ok(file) => Event::Verdict {
            verdict: file.verdict,
            report: file.report,
        },
        Err(why) => Event::RunEnded {
            outcome: Outcome::MissionFailure(why),
            lot_done: false,
        },
    }
}

fn read_verdict(
    path: &std::path::Path,
    role: Role,
    head: &str,
) -> Result<crate::mission::VerdictFile, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("{} could not be read: {e}", path.display()))?;
    if text.trim().is_empty() {
        return Err(format!(
            "the run finished and {} is empty: a role concludes with its verdict",
            path.display()
        ));
    }
    let file: crate::mission::VerdictFile = serde_json::from_str(&text)
        .map_err(|e| format!("{} is not a verdict `hq` can read: {e}", path.display()))?;
    if file.role != role {
        return Err(format!(
            "{} carries {:?}'s verdict, and this run was {role:?}'s",
            path.display(),
            file.role
        ));
    }
    // "Un verdict vaut pour un HEAD" (SPEC 4.4): a verdict on another commit
    // is a verdict on other work, and accepting it would carry a green over
    // a change nobody judged.
    if file.head != head {
        return Err(format!(
            "{} concludes on {}, and the slot is on {head} — a verdict is worth one \
             commit and no other",
            path.display(),
            file.head
        ));
    }
    Ok(file)
}

/// What to tell the human about the application `hq` started.
fn describe(launched: &crate::run::Launched) -> String {
    match (&launched.launch, &launched.app) {
        (Some(crate::launch::Launch::Script { path, declared }), Some(_)) => {
            format!(
                "the application was started from {path} ({})",
                declared.where_from()
            )
        }
        (Some(crate::launch::Launch::Nothing { declared }), _) => {
            format!("nothing was started: {} says `none`", declared.where_from())
        }
        _ => "nothing was started".to_string(),
    }
}

/// Refuse while the agent is still writing. A gate that reads a tree
/// mid-run reads a tree that is not finished, and its verdict would be about
/// a moment nobody chose.
fn refuse_while_running(
    project: &Project,
    engine: Arc<dyn Engine>,
    id: &str,
    state: &MissionState,
) -> Result<(), VerifyError> {
    let Some(handle) = &state.run else {
        return Ok(());
    };
    let harness = crate::harness::claude_code::ClaudeCode::new(
        Default::default(),
        Box::new(harness_spawner(
            project,
            engine,
            &state.slot,
            &handle.session.0,
        )),
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

/// The spawner that can ask about a run: the engine `verify` was given, not
/// one of its own. A second engine here would answer about another machine's
/// containers on any caller that passed a fake or a Podman adapter — and
/// nothing in the type would have said so.
fn harness_spawner(
    project: &Project,
    engine: Arc<dyn Engine>,
    slot: &str,
    session: &str,
) -> crate::engine::spawn::ContainerSpawner {
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
