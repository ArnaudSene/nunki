//! The human's three gestures on a run (SPEC 4.3): they apply to the
//! container, and they always pass.

use std::path::PathBuf;
use std::sync::Arc;

use hq::engine::Engine;
use hq::engine::fake::{Call, FakeEngine};
use hq::gesture::{self, Freeze, GestureError};
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
                bounds: Default::default(),
                credentials: None,
                run: None,
                services_file: None,
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
    gesture::freeze(&world.project, "m1", engine.clone(), Freeze::On).unwrap();
    gesture::freeze(&world.project, "m1", engine, Freeze::Off).unwrap();

    let services: Vec<String> = hq::run::SERVICES.iter().map(|s| s.to_string()).collect();
    assert_eq!(
        fake.calls(),
        vec![
            Call::Pause("hq-one".into(), services.clone()),
            Call::Unpause("hq-one".into(), services),
        ]
    );
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
    let err = gesture::freeze(&world.project, "m1", engine.clone(), Freeze::On).unwrap_err();
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
