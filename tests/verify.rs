//! `hq verify` (SPEC 4.2, 4.4, 4.5): the verification phase as one resumable
//! state machine.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use hq::gate::Gate;
use hq::harness::Role;
use hq::mission::flow::{Flow, Stage, Work};
use hq::mission::{Bounds, Header, Integration, Lot, Security};
use hq::project::{Config, Project, ProtectedPaths};
use hq::state::{MissionState, Store};
use hq::verify::{self, Step, VerifyError};

fn git(at: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(at)
        .args(["-c", "user.name=Verify Test", "-c", "user.email=v@test"])
        .args(args)
        .output()
        .expect("git is on the path");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write(at: &Path, path: &str, body: &str) {
    let full = at.join(path);
    if let Some(dir) = full.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(full, body).unwrap();
}

fn header_of(lots: usize, integration: Integration) -> Header {
    Header {
        branch: "mission/x".into(),
        base: "dev".into(),
        lots: (1..=lots)
            .map(|n| Lot {
                id: format!("L{n}"),
                title: format!("lot {n}"),
            })
            .collect(),
        integration,
        security: Security::Gates,
        arbiter: None,
        run: None,
        account: None,
        bounds: Bounds {
            attempts_per_lot: 3,
            ..Default::default()
        },
    }
}

struct World {
    _dir: tempfile::TempDir,
    project: Project,
    tree: PathBuf,
}

impl World {
    fn new(lots: usize) -> Self {
        Self::shaped(
            lots,
            Integration::None {
                reason: "no external service is involved".into(),
            },
        )
    }

    fn shaped(lots: usize, integration: Integration) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let hq_root = dir.path().join("hq");
        std::fs::create_dir_all(hq_root.join("locks")).unwrap();
        std::fs::create_dir_all(hq_root.join("missions")).unwrap();
        std::fs::create_dir_all(hq_root.join("state/missions")).unwrap();

        // A slot, where `hq slot find` looks: beside the repository root.
        let slots = dir.path().join("repo-slots");
        std::fs::create_dir_all(&slots).unwrap();
        let tree = slots.join("one");
        std::fs::create_dir_all(&tree).unwrap();
        git(&tree, &["init", "-q", "-b", "dev"]);
        write(&tree, "src/lib.rs", "pub fn one() -> u8 { 1 }\n");
        write(&tree, "AGENTS.md", "the rules\n");
        git(&tree, &["add", "-A"]);
        git(&tree, &["commit", "-q", "-m", "base"]);
        git(&tree, &["checkout", "-q", "-b", "mission/x"]);

        let project = Project::at(
            dir.path().join("repo"),
            Config {
                harness: "claude-code".into(),
                forge: vec!["github.com".into()],
                stacks: vec!["rust".into()],
                protected_branches: vec!["main".into(), "dev".into()],
                protected_paths: ProtectedPaths {
                    refuse: vec!["AGENTS.md".into(), ".hq/**".into()],
                    refuse_if_exists: vec![],
                },
                account: None,
                bounds: Default::default(),
                credentials: None,
                run: None,
                services_file: None,
            },
            hq_root,
        );
        let world = Self {
            _dir: dir,
            project,
            tree,
        };
        hq::mission::dir::create(
            &world.project.hq_root,
            "m1",
            &header_of(lots, integration.clone()),
            "do it",
        )
        .unwrap();
        let store = Store::open(&world.project.hq_root).unwrap();
        store
            .save(&MissionState {
                id: "m1".into(),
                slot: "one".into(),
                flow: Flow::new(header_of(lots, integration)).unwrap(),
                run: None,
                app: None,
                updated_at: String::new(),
            })
            .unwrap();
        world
    }

    fn mission(&self) -> PathBuf {
        self.project.hq_root.join("missions/m1")
    }

    fn journal_names_head(&self) {
        std::fs::write(
            self.mission().join("JOURNAL.md"),
            format!(
                "# Journal\n\n## ÉTAT DE REPRISE\n\nHEAD is `{}`.\n",
                git(&self.tree, &["rev-parse", "HEAD"])
            ),
        )
        .unwrap();
    }

    fn commit(&self, path: &str, body: &str, message: &str) {
        write(&self.tree, path, body);
        git(&self.tree, &["add", "-A"]);
        git(&self.tree, &["commit", "-q", "-m", message]);
    }

    fn state(&self) -> MissionState {
        Store::open(&self.project.hq_root)
            .unwrap()
            .load("m1")
            .unwrap()
    }

    /// What a finished coder run would tell the flow.
    fn coder_finished_the_lot(&self) {
        let store = Store::open(&self.project.hq_root).unwrap();
        let mut state = store.load("m1").unwrap();
        store
            .apply(
                &mut state,
                hq::mission::flow::Event::RunEnded {
                    outcome: hq::harness::Outcome::Finished(Default::default()),
                    lot_done: true,
                },
            )
            .unwrap();
    }

    fn verify(&self) -> Result<Vec<Step>, VerifyError> {
        let engine: Arc<dyn hq::engine::Engine> = Arc::new(hq::engine::fake::FakeEngine::default());
        verify::verify(&self.project, "m1", engine, "docker")
    }
}

fn gates_of(steps: &[Step]) -> &hq::gate::Report {
    steps
        .iter()
        .find_map(|s| match s {
            Step::Gates { report, .. } => Some(&**report),
            _ => None,
        })
        .expect("the gates were played")
}

/// Gates 1 to 4 are played while the coder still has lots, not only at the
/// end. A forbidden commit found after the first lot costs one run; found at
/// the final verification it costs the mission (SPEC 4.4, decided
/// 2026-09-09).
#[test]
fn a_forbidden_commit_costs_a_run_now_and_not_the_mission_later() {
    let world = World::new(2);
    world.commit("AGENTS.md", "rewritten by the agent\n", "loosen the rules");
    world.journal_names_head();

    let steps = world.verify().unwrap();
    let report = gates_of(&steps);
    assert!(matches!(
        report
            .outcomes
            .iter()
            .find(|o| o.gate == Gate::Perimeter)
            .map(|o| &o.decision),
        Some(hq::gate::Decision::Failed(_))
    ));
    // One more run on the same lot, not a volet and not the human.
    match world.state().flow.stage() {
        Stage::Coding { work, attempt } => {
            assert_eq!(*work, Work::Lot(0));
            assert_eq!(*attempt, 2, "the red gate cost one attempt");
        }
        other => panic!("a red gate during coding is one more run: {other:?}"),
    }
}

/// Bounded like any other attempt: a lot whose gates keep failing goes to the
/// human rather than round for ever.
#[test]
fn a_gate_that_keeps_failing_reaches_the_human() {
    let world = World::new(1);
    world.commit("AGENTS.md", "rewritten\n", "loosen");
    world.journal_names_head();
    for _ in 0..3 {
        let _ = world.verify();
    }
    match world.state().flow.stage() {
        Stage::AwaitingHuman(handover) => {
            let said = format!("{handover:?}");
            assert!(said.contains("L1"), "{said}");
        }
        other => panic!("three attempts spent: {other:?}"),
    }
}

/// The claim SPEC makes about this verb: *"Reprenable : son état est persisté
/// à chaque transition, et le relancer reprend au même point."*
#[test]
fn running_it_again_picks_up_where_it_stopped() {
    let world = World::new(2);
    world.commit("src/new.rs", "pub fn two() -> u8 { 2 }\n", "L1");
    world.journal_names_head();

    // Green gates, first lot still owed a run.
    let first = world.verify().unwrap();
    assert!(
        gates_of(&first).passed(),
        "{:?}",
        gates_of(&first).failure()
    );
    assert!(matches!(first.last(), Some(Step::NeedsRun { .. })));
    let after_first = world.state();

    // The run happens; the flow moves to the second lot.
    world.coder_finished_the_lot();
    assert!(matches!(
        world.state().flow.stage(),
        Stage::Coding {
            work: Work::Lot(1),
            attempt: 1
        }
    ));

    // Running verify again does not replay the first lot, and does not spend
    // an attempt on the second.
    let again = world.verify().unwrap();
    assert!(gates_of(&again).passed());
    match world.state().flow.stage() {
        Stage::Coding { work, attempt } => {
            assert_eq!(*work, Work::Lot(1), "it did not go back a lot");
            assert_eq!(*attempt, 1, "a green gate spends nothing");
        }
        other => panic!("{other:?}"),
    }
    assert_ne!(after_first.updated_at, world.state().updated_at);
}

#[test]
fn a_mission_that_never_started_is_named_rather_than_invented() {
    let world = World::new(1);
    let engine: Arc<dyn hq::engine::Engine> = Arc::new(hq::engine::fake::FakeEngine::default());
    let err = verify::verify(&world.project, "nope", engine, "docker").unwrap_err();
    assert!(matches!(err, VerifyError::NotStarted(_)), "{err}");
    assert!(err.to_string().contains("hq mission start"), "{err}");
}

/// A tree the agent is still writing is not a tree to judge. This is the
/// interlock `hq exec --tree` deliberately does not carry, and this is where
/// it belongs (SPEC 4.2).
#[test]
fn a_mission_whose_run_is_still_going_is_not_verified_under_it() {
    let world = World::new(1);
    world.commit("src/new.rs", "pub fn two() -> u8 { 2 }\n", "L1");
    world.journal_names_head();

    // A handle whose log holds no result and whose container is unknown to
    // the engine reads as *not* running — hq does not know, and refusing to
    // verify because the engine is down would be a lie in another place.
    let store = Store::open(&world.project.hq_root).unwrap();
    let mut state = store.load("m1").unwrap();
    state.run = Some(hq::harness::RunHandle {
        session: hq::harness::SessionId("s".into()),
        container: "no-such-container".into(),
        pid: Some(4242),
        log: world.mission().join("runs/s.jsonl"),
    });
    store.save(&state).unwrap();
    assert!(
        world.verify().is_ok(),
        "an unreachable run is not a run in progress"
    );
}

/// The lock is taken once, at the top, and everything under it runs inside it
/// (SPEC 4.2). A slot already driven by another `hq` is not driven twice.
#[test]
fn a_slot_already_held_is_not_verified_under_the_other_hq() {
    let world = World::new(1);
    world.journal_names_head();
    let held = hq::state::SlotLock::acquire(
        &world.project.hq_root.join("locks"),
        "one",
        "something else",
    )
    .unwrap();
    assert!(matches!(world.verify(), Err(VerifyError::Lock(_))));
    drop(held);
    assert!(world.verify().is_ok());
}

/// The whole phase, end to end, in a real container: gates 1 to 4 while the
/// coder works, then the final seven, a red gate 7 sending it back, a
/// campaign, a triage, and `VERIFIED`.
///
/// This is the test the verb exists for. Everything else here checks one
/// transition; this one checks that the transitions compose.
///
/// ```text
/// cargo test --test verify -- --ignored --nocapture
/// ```
#[test]
#[ignore = "lifts real containers; run by hand"]
fn live_a_mission_is_driven_from_its_first_lot_to_verified() {
    use hq::mutants::Triage;
    use std::collections::BTreeMap;

    let world = World::new(1);
    // The battery and the campaign the stack declares, committed: gate 6
    // judges what is on the commit, never what is in the tree.
    let executable = |path: &str, body: &str| {
        write(&world.tree, path, body);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                world.tree.join(path),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }
    };
    // On the base, where `hq init` puts them: `.hq/**` is a protected path,
    // and a mission that committed its own battery would fail gate 4 — which
    // is exactly what it is there for.
    git(&world.tree, &["checkout", "-q", "dev"]);
    executable(
        ".hq/stacks/rust/prepush.sh",
        "#!/bin/sh\nset -eu\ntest -f src/new.rs\n",
    );
    executable(
        ".hq/stacks/rust/mutation.sh",
        "#!/bin/sh\n\
         echo '{\"id\":\"src/new.rs:1\",\"file\":\"src/new.rs\",\"line\":1,\
         \"description\":\"replace two with 0\"}'\n",
    );
    git(&world.tree, &["add", "-A"]);
    git(&world.tree, &["commit", "-q", "-m", "the stack fragments"]);
    git(&world.tree, &["checkout", "-q", "-B", "mission/x", "dev"]);

    write(&world.tree, "src/new.rs", "pub fn two() -> u8 { 2 }\n");
    write(
        &world.tree,
        "tests/thing.rs",
        "#[test]\nfn two_is_two() { assert_eq!(2, 2); }\n",
    );
    git(&world.tree, &["add", "-A"]);
    git(&world.tree, &["commit", "-q", "-m", "L1"]);
    world.journal_names_head();
    std::fs::write(
        world.mission().join("PR.md"),
        "# What this changes\n\nA lot.\n",
    )
    .unwrap();

    // A profile, as a slot would have one up.
    let slot = hq::slot::Slot {
        name: "one".into(),
        tree: world.tree.clone(),
    };
    let volume = hq::exec::proof_volume(&slot.name);
    let file = hq::run::profile_path(&world.project, &slot.name);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(
        &file,
        format!(
            "services:\n\
             \x20 agent:\n\
             \x20   image: alpine:3.20\n\
             \x20   volumes:\n\
             \x20     - {tree}:{tree_at}\n\
             \x20     - {volume}:{proof}\n\
             \x20   tmpfs:\n\
             \x20     - /run/hq\n\
             \x20   command: [\"sh\", \"-c\", \"apk add --no-cache git > /dev/null && \
             sleep 600\"]\n\
             \x20   healthcheck:\n\
             \x20     test: [\"CMD-SHELL\", \"command -v git > /dev/null\"]\n\
             \x20     interval: 1s\n\
             \x20     timeout: 2s\n\
             \x20     retries: 60\n\
             \x20     start_period: 1s\n\
             volumes:\n\
             \x20 {volume}:\n",
            tree = world.tree.display(),
            tree_at = hq::run::TREE_AT,
            proof = hq::exec::PROOF_AT,
        ),
    )
    .unwrap();

    let engine: Arc<dyn hq::engine::Engine> = Arc::new(hq::engine::docker::Docker::real());
    let compose_project = hq::compose::project_name(&slot.name).unwrap();
    let _ = engine.down(&file, &compose_project, true);
    engine.up(&file, &compose_project).unwrap();
    let go = || verify::verify(&world.project, "m1", engine.clone(), "docker").unwrap();

    // 1. The coder still owes its lot a run; gates 1 to 4 are green.
    let steps = go();
    assert!(
        gates_of(&steps).passed(),
        "{:?}",
        gates_of(&steps).failure()
    );
    assert!(matches!(steps.last(), Some(Step::NeedsRun { role, .. }) if *role == Role::Coder));

    // 2. The run happens, the flow reaches the final gates.
    world.coder_finished_the_lot();
    assert!(matches!(world.state().flow.stage(), Stage::Gates));

    // 3. All seven: the battery is green, and gate 7 has nothing to read.
    let steps = go();
    let report = gates_of(&steps);
    assert_eq!(
        report
            .outcomes
            .iter()
            .find(|o| o.gate == Gate::Battery)
            .map(|o| &o.decision),
        Some(&hq::gate::Decision::Passed),
        "{report:?}"
    );
    match report
        .outcomes
        .iter()
        .find(|o| o.gate == Gate::Mutation)
        .map(|o| &o.decision)
    {
        Some(hq::gate::Decision::Unplayed(why)) => assert!(why.contains("hq mission mutants")),
        other => panic!("no campaign has run: {other:?}"),
    }
    // A red final gate sends the coder back, and does not consume a volet.
    assert!(matches!(world.state().flow.stage(), Stage::Coding { .. }));

    // 4. The campaign runs, and its one survivor is answered by the coder.
    let touched = hq::gate::touched_paths(&world.tree, "dev").unwrap();
    loop {
        match hq::mutants::campaign(
            &world.project,
            &slot,
            engine.clone(),
            &world.mission(),
            "rust",
            &touched,
            45,
        )
        .unwrap()
        {
            hq::mutants::Progress::Running { .. } => {
                std::thread::sleep(std::time::Duration::from_millis(300))
            }
            hq::mutants::Progress::Started { .. } => {}
            other => {
                println!("campaign: {other:?}");
                break;
            }
        }
    }
    let mut triage = BTreeMap::new();
    triage.insert(
        "src/new.rs:1".to_string(),
        Triage::Killed {
            test: "two_is_two".into(),
        },
    );
    hq::mutants::write_triage(&world.mission(), &triage).unwrap();

    // 4 bis. A run in the slot stops verification dead: gates read a tree,
    // and a tree the agent is still writing is not a tree to judge.
    {
        use hq::harness::spawn::{CommandSpec, Spawner};
        let session = "77777777-8888-4999-8aaa-bbbbbbbbbbbb";
        let spawner = hq::engine::spawn::ContainerSpawner::new(
            engine.clone(),
            file.clone(),
            &compose_project,
            hq::compose::AGENT_SERVICE,
        )
        .identified_by(session);
        let log = world
            .mission()
            .join("runs")
            .join(format!("{session}.jsonl"));
        let spawned = spawner
            .spawn(
                &CommandSpec {
                    program: "sh".to_string(),
                    args: vec![
                        "-c".to_string(),
                        "while :; do sleep 1; done".to_string(),
                        session.to_string(),
                    ],
                    cwd: std::path::PathBuf::from("/"),
                    env: Default::default(),
                },
                &log,
            )
            .unwrap();
        let store = Store::open(&world.project.hq_root).unwrap();
        let mut state = store.load("m1").unwrap();
        state.run = Some(hq::harness::RunHandle {
            session: hq::harness::SessionId(session.to_string()),
            container: spawned.container.clone(),
            pid: spawned.pid,
            log,
        });
        store.save(&state).unwrap();
        match verify::verify(&world.project, "m1", engine.clone(), "docker") {
            Err(VerifyError::RunInProgress { slot, .. }) => assert_eq!(slot, "one"),
            other => panic!("a live run must stop verification: {other:?}"),
        }
        spawner
            .signal(&spawned, hq::harness::spawn::Signal::Terminate)
            .unwrap();
        let mut state = store.load("m1").unwrap();
        state.run = None;
        store.save(&state).unwrap();
    }

    // 5. The coder's run happens again, and this time everything is green.
    world.coder_finished_the_lot();
    world.journal_names_head();
    let steps = go();
    let report = gates_of(&steps);
    assert!(report.passed(), "{:?}", report.failure());
    assert!(matches!(steps.last(), Some(Step::Verified)));

    // And it is persisted before anything was printed: a session that dies
    // here must not lose the verdict.
    assert!(matches!(world.state().flow.stage(), Stage::Verified));

    engine.down(&file, &compose_project, true).unwrap();
}

// --- the integration run, read back (SPEC 4.2, 4.4, 4.5) -------------------

fn integration() -> Integration {
    Integration::Services {
        services: vec![hq::mission::Service {
            name: "db".into(),
            reach: vec!["db".into()],
        }],
        wiring: vec!["compose.yaml".into()],
    }
}

impl World {
    /// Drive the flow to the stage where the integrator is due, the way a
    /// finished coder and a green verification do.
    fn at_integration(&self) {
        let store = Store::open(&self.project.hq_root).unwrap();
        let mut state = store.load("m1").unwrap();
        store
            .apply(
                &mut state,
                hq::mission::flow::Event::RunEnded {
                    outcome: hq::harness::Outcome::Finished(Default::default()),
                    lot_done: true,
                },
            )
            .unwrap();
        store
            .apply(&mut state, hq::mission::flow::Event::GatesPassed)
            .unwrap();
        assert!(matches!(state.flow.stage(), Stage::Integration { .. }));
    }

    /// Record a run for the current stage, with a harness log that says how
    /// it ended. `pid` absent is a run nothing can be asked about.
    fn run_recorded(&self, pid: Option<u32>, log_body: &str) {
        let runs = self.mission().join("runs");
        std::fs::create_dir_all(&runs).unwrap();
        let log = runs.join("integration.log");
        std::fs::write(&log, log_body).unwrap();
        let store = Store::open(&self.project.hq_root).unwrap();
        let mut state = store.load("m1").unwrap();
        state.run = Some(hq::harness::RunHandle {
            session: hq::harness::SessionId("s-integration".into()),
            container: "cafe1234".into(),
            pid,
            log,
        });
        store.save(&state).unwrap();
    }

    fn verdict(&self, role: &str, verdict: &str, head: &str, report: &str) {
        std::fs::write(
            self.mission().join("VERDICT.json"),
            format!(
                "{{\"role\":\"{role}\",\"verdict\":\"{verdict}\",\"head\":\"{head}\",\
                 \"date\":\"2026-09-10T00:00:00Z\",\"report\":\"{report}\"}}"
            ),
        )
        .unwrap();
    }

    fn head(&self) -> String {
        git(&self.tree, &["rev-parse", "HEAD"])
    }
}

/// A harness log whose run concluded normally.
const FINISHED: &str =
    "{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"usage\":{}}\n";

/// The chain SPEC 4.5 describes: the integrator concludes `INTEGRATED`, and
/// the mission moves on rather than waiting for a human to relay the verdict.
#[test]
fn an_integration_run_that_concluded_moves_the_mission_on() {
    let world = World::shaped(1, integration());
    world.at_integration();
    world.run_recorded(Some(41), FINISHED);
    world.verdict("Integrator", "INTEGRATED", &world.head(), "wired");

    let steps = world.verify().unwrap();
    assert!(
        matches!(steps.last(), Some(Step::Verified)),
        "the shape declares no security agent, so INTEGRATED ends it: {steps:?}"
    );
    // The run is forgotten with the transition: left recorded, the next
    // `verify` would read the same run back a second time.
    assert!(world.state().run.is_none(), "{:?}", world.state().run);
}

/// "Un verdict vaut pour un `HEAD`" (SPEC 4.4). A verdict concluding on
/// another commit is a verdict on other work, and accepting it would carry a
/// green over a change nobody judged.
#[test]
fn a_verdict_on_another_commit_is_not_a_verdict_on_this_one() {
    let world = World::shaped(1, integration());
    world.at_integration();
    world.run_recorded(Some(41), FINISHED);
    world.verdict(
        "Integrator",
        "INTEGRATED",
        "0000000000000000000000000000000000000000",
        "wired",
    );

    // The relaunch that follows needs images and an account this world has
    // neither of; what is asserted is the transition, which is written
    // before anything is lifted.
    let _ = world.verify();
    assert!(
        matches!(
            world.state().flow.stage(),
            Stage::Integration { attempt: 2 }
        ),
        "it costs an attempt and the role runs again: {:?}",
        world.state().flow.stage()
    );
}

#[test]
fn a_run_that_left_no_verdict_did_not_conclude() {
    let world = World::shaped(1, integration());
    world.at_integration();
    world.run_recorded(Some(41), FINISHED);
    // No VERDICT.json at all: `hq mission dir` creates an empty one.

    let _ = world.verify();
    assert!(
        matches!(
            world.state().flow.stage(),
            Stage::Integration { attempt: 2 }
        ),
        "{:?}",
        world.state().flow.stage()
    );
}

#[test]
fn a_verdict_signed_by_another_role_is_not_this_roles() {
    let world = World::shaped(1, integration());
    world.at_integration();
    world.run_recorded(Some(41), FINISHED);
    world.verdict("Coder", "INTEGRATED", &world.head(), "wired");

    let _ = world.verify();
    assert!(
        matches!(
            world.state().flow.stage(),
            Stage::Integration { attempt: 2 }
        ),
        "{:?}",
        world.state().flow.stage()
    );
}

/// A run `hq` cannot ask about decides nothing: a machine that slept must not
/// cost the mission an attempt (SPEC 4.2, "la reprise re-dérive avant de
/// décider"; AGENTS.md §4, "a check that cannot say 'I do not know' will
/// lie").
#[test]
fn a_run_that_cannot_be_asked_about_decides_nothing() {
    let world = World::shaped(1, integration());
    world.at_integration();
    // No pid, and a log that says nothing: there is no answer to be had.
    world.run_recorded(None, "");
    world.verdict("Integrator", "INTEGRATED", &world.head(), "wired");

    let steps = world.verify().unwrap();
    assert!(
        matches!(steps.last(), Some(Step::Unreachable { role, .. }) if *role == Role::Integrator),
        "{steps:?}"
    );
    assert!(
        matches!(
            world.state().flow.stage(),
            Stage::Integration { attempt: 1 }
        ),
        "no attempt was spent: {:?}",
        world.state().flow.stage()
    );
    assert!(world.state().run.is_some(), "and the run is still recorded");
}

/// A `BROKEN` integrator sends the mission back to the coder as a volet,
/// bounded — it does not stop the mission and does not repair it itself
/// (SPEC 4.5).
#[test]
fn a_broken_integration_goes_back_to_the_coder_as_a_volet() {
    let world = World::shaped(1, integration());
    world.journal_names_head();
    world.at_integration();
    world.run_recorded(Some(41), FINISHED);
    world.verdict(
        "Integrator",
        "BROKEN",
        &world.head(),
        "the adapter cannot be wired as it stands",
    );

    let steps = world.verify().unwrap();
    assert!(
        matches!(
            world.state().flow.stage(),
            Stage::Coding {
                work: Work::Volet { .. },
                ..
            }
        ),
        "{:?}",
        world.state().flow.stage()
    );
    assert!(
        steps
            .iter()
            .any(|s| matches!(s, Step::NeedsRun { role, .. } if *role == Role::Coder)),
        "{steps:?}"
    );
}
