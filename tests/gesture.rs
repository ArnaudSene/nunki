//! The human's three gestures on a run (SPEC 4.3): they apply to the
//! container, and they always pass.

use std::path::PathBuf;
use std::sync::Arc;

use hq::engine::Engine;
use hq::engine::fake::{Call, FakeEngine};
use hq::gesture::{self, GestureError};
use hq::harness::{RunHandle, SessionId};
use hq::mission::flow::Flow;
use hq::mission::{Bounds, Header, Integration, Lot, Security};
use hq::project::{Config, Project, ProtectedPaths};
use hq::state::{MissionState, Store};

fn header() -> Header {
    Header {
        branch: "mission/x".into(),
        base: "dev".into(),
        lots: vec![Lot {
            id: "L1".into(),
            title: "one".into(),
        }],
        integration: Integration::None {
            reason: "none".into(),
        },
        security: Security::Gates,
        arbiter: None,
        run: None,
        account: None,
        model: None,
        bounds: Bounds::default(),
    }
}

struct World {
    _dir: tempfile::TempDir,
    project: Project,
}

impl World {
    fn new(with_run: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let hq_root = dir.path().join("hq");
        for d in ["locks", "missions", "state/missions", "profiles"] {
            std::fs::create_dir_all(hq_root.join(d)).unwrap();
        }
        std::fs::write(hq_root.join("profiles/one.yml"), "services: {}\n").unwrap();
        let project = Project::at(
            dir.path().join("repo"),
            Config {
                harness: "claude-code".into(),
                forge: vec![],
                stacks: vec!["rust".into()],
                protected_branches: vec!["main".into()],
                protected_paths: ProtectedPaths::default(),
                account: None,
                model: None,
                bounds: Default::default(),
                credentials: None,
                run: None,
                services_file: None,
                forge_protection: Default::default(),
            },
            hq_root.clone(),
        );
        hq::mission::dir::create(&hq_root, "m1", &header(), "do it").unwrap();
        Store::open(&hq_root)
            .unwrap()
            .save(&MissionState {
                id: "m1".into(),
                slot: "one".into(),
                flow: Flow::new(header()).unwrap(),
                run: with_run.then(|| RunHandle {
                    session: SessionId("s1".into()),
                    container: "cafe1234".into(),
                    pid: Some(41),
                    log: PathBuf::from("/dev/null"),
                }),
                app: None,
                verdicts: Vec::new(),
                accepted: Vec::new(),
                stopped: None,
                harness_down: None,
                spent: Default::default(),
                spared: None,
                coder_session: None,
                updated_at: String::new(),
            })
            .unwrap();
        Self { _dir: dir, project }
    }

    fn followup(&self) -> String {
        std::fs::read_to_string(self.project.hq_root.join("missions/m1/FOLLOWUP_HQ.md")).unwrap()
    }
}

/// A fake engine, kept typed so the test can read what was asked of it, and
/// handed to `hq` as the trait object it takes.
fn engine() -> (Arc<dyn Engine>, Arc<FakeEngine>) {
    let fake = Arc::new(FakeEngine::default());
    (fake.clone(), fake)
}

/// The gesture is on the **container**, and it takes the sidecar with it: a
/// frozen agent behind a live firewall is half a pair, and the firewall
/// exists to fence the agent.
#[test]
fn pausing_freezes_the_agent_and_its_sidecar_and_nothing_else() {
    let world = World::new(true);
    let (engine, fake) = engine();
    gesture::pause(&world.project, "m1", engine).unwrap();

    let services: Vec<String> = hq::run::SERVICES.iter().map(|s| s.to_string()).collect();
    assert_eq!(fake.calls(), vec![Call::Pause("hq-one".into(), services)]);
}

/// The emergency brake is `kill` and not a stop with no patience: nothing is
/// asked and nothing is waited for, and the project's services are not
/// touched.
#[test]
fn killing_is_a_kill_and_leaves_the_projects_services_alone() {
    let world = World::new(true);
    let (engine, fake) = engine();
    gesture::kill(&world.project, "m1", engine).unwrap();

    let services: Vec<String> = hq::run::SERVICES.iter().map(|s| s.to_string()).collect();
    assert_eq!(fake.calls(), vec![Call::Kill("hq-one".into(), services)]);
}

/// A gesture on a mission with no run says so rather than acting on whatever
/// container happens to answer to the slot's name.
#[test]
fn a_gesture_with_no_run_in_progress_is_named() {
    let world = World::new(false);
    let (engine, fake) = engine();
    let err = gesture::pause(&world.project, "m1", engine.clone()).unwrap_err();
    assert!(matches!(err, GestureError::NoRun(_)), "{err}");
    let err = gesture::kill(&world.project, "m1", engine).unwrap_err();
    assert!(matches!(err, GestureError::NoRun(_)), "{err}");
    assert!(fake.calls().is_empty(), "{:?}", fake.calls());
}

/// And on a mission that never started, the message is about the mission.
#[test]
fn a_gesture_on_a_mission_that_never_started_is_named() {
    let world = World::new(false);
    let (engine, _) = engine();
    let err = gesture::kill(&world.project, "nope", engine).unwrap_err();
    assert!(matches!(err, GestureError::NotStarted(_)), "{err}");
}

/// `say` is not a channel: there is none during a run. It writes where the
/// next run will read it, and it never touches the engine.
#[test]
fn saying_leaves_an_instruction_for_the_next_run() {
    let world = World::new(true);
    gesture::say(&world.project, "m1", "  prefer the smaller refactor  ").unwrap();
    let text = world.followup();
    assert!(text.contains("prefer the smaller refactor"), "{text}");
    assert!(text.contains("next run"), "{text}");
    // Trimmed, so a stray newline does not become a blank paragraph in a
    // file whose headings have to be findable.
    assert!(!text.contains("  prefer"), "{text}");
}

/// An instruction left before the first run is read by the first run, which
/// is exactly what it is for — so `say` works on a mission that has been
/// framed and never started, and has no state file at all.
#[test]
fn saying_works_before_a_mission_has_started() {
    let world = World::new(false);
    hq::mission::dir::create(&world.project.hq_root, "m2", &header(), "not started").unwrap();
    assert!(
        Store::open(&world.project.hq_root)
            .unwrap()
            .load("m2")
            .is_err(),
        "m2 is framed and not started"
    );

    gesture::say(&world.project, "m2", "start with the parser").unwrap();
    let text =
        std::fs::read_to_string(world.project.hq_root.join("missions/m2/FOLLOWUP_HQ.md")).unwrap();
    assert!(text.contains("start with the parser"), "{text}");
}

#[test]
fn an_empty_instruction_is_not_one() {
    let world = World::new(true);
    let err = gesture::say(&world.project, "m1", "  \n\t ").unwrap_err();
    assert!(matches!(err, GestureError::NothingSaid), "{err}");
}

/// SPEC 4.5 reads as two clauses, and the first is unconditional: `stop`
/// means "hq launches no further run", and `--now` only chooses what happens
/// to the run already going. So a bare `stop` writes the hold and signals
/// nothing — the old `STOP` file was a marker, not a gesture.
#[test]
fn stopping_holds_the_mission_and_signals_nothing() {
    let world = World::new(true);
    let harness = hq::harness::fake::FakeHarness::new();
    let held = gesture::stop(&world.project, "m1", &harness, false).unwrap();

    assert!(!held.interrupted, "a bare stop interrupts nothing");
    assert!(harness.stopped().is_empty(), "{:?}", harness.stopped());
    let state = Store::open(&world.project.hq_root)
        .unwrap()
        .load("m1")
        .unwrap();
    assert!(state.held(), "the hold is what stop is for");
    assert!(state.run.is_some(), "the run in progress finishes its lot");
}

/// The other clause: `--now` ends the turn as well, and the record keeps the
/// two apart — what `hq` will not do next, and what was done to the run that
/// was going.
#[test]
fn stopping_now_also_ends_the_turn() {
    let world = World::new(true);
    let harness = hq::harness::fake::FakeHarness::new();
    let held = gesture::stop(&world.project, "m1", &harness, true).unwrap();

    assert!(held.interrupted);
    assert_eq!(harness.stopped(), vec![SessionId("s1".into())]);
    assert!(
        Store::open(&world.project.hq_root)
            .unwrap()
            .load("m1")
            .unwrap()
            .held()
    );
}

/// A hold is a decision about the future, so it needs no run: the coder's
/// run ended, `verify` has not launched the integrator, and the human wants
/// the mission held. That is the case a signal cannot express.
#[test]
fn a_mission_between_two_runs_can_still_be_held() {
    let world = World::new(false);
    let harness = hq::harness::fake::FakeHarness::new();
    gesture::stop(&world.project, "m1", &harness, false).unwrap();
    assert!(
        Store::open(&world.project.hq_root)
            .unwrap()
            .load("m1")
            .unwrap()
            .held()
    );
}

/// `--now` is only ever the second clause. Typed a second after the run
/// ended, it still holds the mission — refusing the hold because there was
/// nothing to interrupt would make the unconditional clause conditional on
/// the optional one. The record says the interruption did not happen.
#[test]
fn stopping_now_after_the_run_ended_still_holds_the_mission() {
    let world = World::new(false);
    let harness = hq::harness::fake::FakeHarness::new();
    let held = gesture::stop(&world.project, "m1", &harness, true).unwrap();

    assert!(!held.interrupted, "nothing was there to interrupt");
    assert!(harness.stopped().is_empty());
    assert!(
        Store::open(&world.project.hq_root)
            .unwrap()
            .load("m1")
            .unwrap()
            .held(),
        "the hold is not conditional on there being a run"
    );
}

/// `resume` lifts the hold and unfreezes in one gesture: to the human who
/// types it, the mission was held and is held no longer.
#[test]
fn resuming_lifts_the_hold_and_unfreezes() {
    let world = World::new(true);
    let harness = hq::harness::fake::FakeHarness::new();
    gesture::stop(&world.project, "m1", &harness, false).unwrap();

    let fake = Arc::new(
        hq::engine::fake::FakeEngine::default()
            .with_liveness("cafe1234", hq::engine::Liveness::Paused),
    );
    let engine: Arc<dyn Engine> = fake.clone();
    let lifted = gesture::resume(&world.project, "m1", engine).unwrap();
    assert!(lifted.is_some(), "it says what it lifted");

    let services: Vec<String> = hq::run::SERVICES.iter().map(|s| s.to_string()).collect();
    assert_eq!(
        fake.calls(),
        vec![
            Call::Liveness("cafe1234".into()),
            Call::Unpause("hq-one".into(), services)
        ]
    );
    assert!(
        !Store::open(&world.project.hq_root)
            .unwrap()
            .load("m1")
            .unwrap()
            .held()
    );
}

/// A mission held between two runs has no container to unfreeze, and that is
/// exactly a mission worth resuming: `resume` lifts the hold and asks the
/// engine nothing.
#[test]
fn resuming_a_mission_with_no_run_lifts_the_hold_all_the_same() {
    let world = World::new(false);
    let harness = hq::harness::fake::FakeHarness::new();
    gesture::stop(&world.project, "m1", &harness, false).unwrap();

    let (engine, fake) = engine();
    assert!(
        gesture::resume(&world.project, "m1", engine)
            .unwrap()
            .is_some()
    );
    assert!(fake.calls().is_empty(), "{:?}", fake.calls());
}

/// It reports what it did, not what it meant to do: resuming a mission
/// nobody held lifts nothing, and says so.
#[test]
fn resuming_a_mission_nobody_held_lifts_nothing() {
    let world = World::new(true);
    let (engine, _) = engine();
    assert!(
        gesture::resume(&world.project, "m1", engine)
            .unwrap()
            .is_none()
    );
}

/// The common path, and the one a fake engine that always says yes hides:
/// after a bare `stop` the run **keeps going**, so at `resume` its container
/// is running and not frozen — and `unpause` on a running container is
/// refused by the engine (measured on Docker 28: "is not paused", exit 1).
/// `resume` reads the liveness and unfreezes only what is frozen.
#[test]
fn resuming_a_run_that_was_never_frozen_asks_for_no_unpause() {
    let world = World::new(true);
    let harness = hq::harness::fake::FakeHarness::new();
    gesture::stop(&world.project, "m1", &harness, false).unwrap();

    let fake = Arc::new(
        hq::engine::fake::FakeEngine::default()
            .with_liveness("cafe1234", hq::engine::Liveness::Running),
    );
    let engine: Arc<dyn Engine> = fake.clone();
    assert!(
        gesture::resume(&world.project, "m1", engine)
            .unwrap()
            .is_some()
    );
    assert_eq!(
        fake.calls(),
        vec![Call::Liveness("cafe1234".into())],
        "it asked, and it did not unpause"
    );
}

// --- sparing the subscription (SPEC 4.3) ------------------------------------

impl World {
    /// One account, whose five-hour window was last measured at `per_mille`.
    fn measured(&self, per_mille: u32) {
        let hq_home = self.project.hq_home();
        std::fs::write(
            hq_home.join("accounts.yaml"),
            "accounts:\n  main:\n    harness: claude-code\n    token_file: accounts/main.token\n",
        )
        .unwrap();
        let now = hq::state::now_secs();
        hq::consumption::record(
            &hq_home,
            "main",
            &hq::consumption::Measure {
                windows: hq::consumption::Windows {
                    five_hour: Some(hq::consumption::Window {
                        per_mille,
                        resets_at: now + 3_600,
                    }),
                    weekly: None,
                },
                measured_at: now,
                harness: "claude-code".into(),
            },
        )
        .unwrap();
    }

    fn state(&self) -> MissionState {
        Store::open(&self.project.hq_root)
            .unwrap()
            .load("m1")
            .unwrap()
    }
}

fn harness_saying(state: hq::harness::RunState) -> hq::harness::fake::FakeHarness {
    let harness = hq::harness::fake::FakeHarness::new();
    harness.script(SessionId("s1".into()), vec![state]);
    harness
}

fn running() -> hq::harness::fake::FakeHarness {
    harness_saying(hq::harness::RunState::Running(Default::default()))
}

/// Past the threshold, the run is told to end its turn — once — and the
/// mission is marked, not held: nothing for a human to lift.
#[test]
fn a_run_past_the_threshold_is_told_to_end_its_turn_once() {
    let world = World::new(true);
    world.measured(950);
    let harness = running();

    let spared = gesture::spare(
        &world.project,
        "m1",
        &harness,
        hq::state::now_secs(),
        "mission monitor",
    )
    .unwrap()
    .expect("past 90 % of five hours");
    assert_eq!((spared.account.as_str(), spared.per_mille), ("main", 950));
    assert_eq!(harness.stopped(), vec![SessionId("s1".into())]);
    let state = world.state();
    assert_eq!(state.spared.as_ref(), Some(&spared));
    assert!(!state.held(), "a wait, not a hold");

    // A second SIGINT would interrupt the resume block the first asked for.
    gesture::spare(
        &world.project,
        "m1",
        &harness,
        hq::state::now_secs(),
        "mission monitor",
    )
    .unwrap();
    assert_eq!(harness.stopped().len(), 1);
}

#[test]
fn below_the_threshold_the_run_goes_on() {
    let world = World::new(true);
    world.measured(100);
    let harness = running();
    assert!(
        gesture::spare(
            &world.project,
            "m1",
            &harness,
            hq::state::now_secs(),
            "mission monitor"
        )
        .unwrap()
        .is_none()
    );
    assert!(harness.stopped().is_empty());
    assert!(world.state().spared.is_none());
}

/// A run that has already ended is not signalled: there is no turn to end.
#[test]
fn a_run_that_is_not_running_is_not_signalled() {
    let world = World::new(true);
    world.measured(950);
    let harness = harness_saying(hq::harness::RunState::Finished(
        hq::harness::Outcome::Finished(Default::default()),
    ));
    assert!(
        gesture::spare(
            &world.project,
            "m1",
            &harness,
            hq::state::now_secs(),
            "mission monitor"
        )
        .unwrap()
        .is_none()
    );
    assert!(harness.stopped().is_empty());
}

/// Nothing measured is not "nothing spent": it stops nothing, and says
/// nothing it does not know.
#[test]
fn nothing_measured_stops_nothing() {
    let world = World::new(true);
    let harness = running();
    assert!(
        gesture::spare(
            &world.project,
            "m1",
            &harness,
            hq::state::now_secs(),
            "mission monitor"
        )
        .unwrap()
        .is_none()
    );
    assert!(harness.stopped().is_empty());
}

/// A slot someone else drives — a human's `verify` — is theirs this time: a
/// mark written behind their back would be lost when they save, and the run
/// would then cost an attempt. Their own `verify` spares the run itself.
#[test]
fn a_slot_driven_by_another_verb_is_left_to_it() {
    let world = World::new(true);
    world.measured(950);
    let harness = running();
    let spare = || {
        gesture::spare(
            &world.project,
            "m1",
            &harness,
            hq::state::now_secs(),
            "mission monitor",
        )
        .unwrap()
    };

    let lock = hq::state::SlotLock::acquire(&world.project.hq_root.join("locks"), "one", "verify")
        .unwrap();
    assert!(spare().is_none());
    assert!(harness.stopped().is_empty());
    assert!(world.state().spared.is_none());

    drop(lock);
    assert!(spare().is_some(), "free again, the run is spared");
}
