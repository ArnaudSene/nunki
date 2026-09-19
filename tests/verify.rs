//! `nunki verify` (SPEC 4.2, 4.4, 4.5): the verification phase as one resumable
//! state machine.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use nunki::gate::Gate;
use nunki::harness::Role;
use nunki::mission::flow::{Flow, Stage, Work};
use nunki::mission::{Bounds, Header, Integration, Lot, Security};
use nunki::project::{Config, Project, ProtectedPaths};
use nunki::state::{MissionState, Store};
use nunki::verify::{self, Step, VerifyError};

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
        model: None,
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
        let hq_root = dir.path().join("nunki").join(nunki::project::HQ_DIR);
        std::fs::create_dir_all(hq_root.join("locks")).unwrap();
        std::fs::create_dir_all(hq_root.join("missions")).unwrap();
        std::fs::create_dir_all(hq_root.join("state/missions")).unwrap();

        // A slot, where `nunki slot find` looks: beside the repository root.
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
                root: None,
                harness: "claude-code".into(),
                forge: vec!["github.com".into()],
                stacks: vec!["rust".into()],
                protected_branches: vec!["main".into(), "dev".into()],
                protected_paths: ProtectedPaths {
                    refuse: vec!["AGENTS.md".into(), ".nunki/**".into()],
                    refuse_if_exists: vec![],
                },
                account: None,
                model: None,
                bounds: Default::default(),
                credentials: None,
                run: None,
                services_file: None,
                permission_mode: "auto".to_string(),
                forge_protection: Default::default(),
            },
            hq_root.parent().unwrap().to_path_buf(),
        );
        // A profile on disk, because gate 8 runs in the container at the end
        // of **every** run (SPEC 4.4) and `nunki exec` refuses a slot that has
        // none. A real run has one: the agent has just worked in it. The fake
        // engine answers with an empty contract, which is what a scan finding
        // nothing looks like.
        let profile = nunki::run::profile_path(&project, "one");
        std::fs::create_dir_all(profile.parent().unwrap()).unwrap();
        std::fs::write(&profile, "services: {}\n").unwrap();

        let world = Self {
            _dir: dir,
            project,
            tree,
        };
        nunki::mission::dir::create(
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
        world
    }

    fn mission(&self) -> PathBuf {
        self.project.hq_root.join("missions/m1")
    }

    /// Take the slot's profile away, so anything that needs the container
    /// cannot reach it — what a slot whose profile was never lifted looks
    /// like.
    fn without_profile(&self) {
        let _ = std::fs::remove_file(nunki::run::profile_path(&self.project, "one"));
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
                nunki::mission::flow::Event::RunEnded {
                    outcome: nunki::harness::Outcome::Finished(Default::default()),
                    lot_done: true,
                },
            )
            .unwrap();
    }

    /// What `nunki mission stop` writes: the mission is held.
    fn hold(&self) {
        let store = Store::open(&self.project.hq_root).unwrap();
        let mut state = store.load("m1").unwrap();
        state.hold("Arnaud", false);
        store.save(&state).unwrap();
    }

    fn verify(&self) -> Result<Vec<Step>, VerifyError> {
        let engine: Arc<dyn nunki::engine::Engine> =
            Arc::new(nunki::engine::fake::FakeEngine::default());
        verify::verify(&self.project, "m1", engine, "docker")
    }
}

fn gates_of(steps: &[Step]) -> &nunki::gate::Report {
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
    // Held, so this world — no account, no images — launches nothing and
    // `verify` answers with its steps.
    world.hold();

    let steps = world.verify().unwrap();
    let report = gates_of(&steps);
    assert!(matches!(
        report
            .outcomes
            .iter()
            .find(|o| o.gate == Gate::Perimeter)
            .map(|o| &o.decision),
        Some(nunki::gate::Decision::Failed(_))
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
    world.hold();

    // Green gates, first lot still owed a run — and held, so none launched.
    let first = world.verify().unwrap();
    assert!(
        gates_of(&first).passed(),
        "{:?}",
        gates_of(&first).failure()
    );
    assert!(matches!(first.last(), Some(Step::Held { .. })));
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
    let engine: Arc<dyn nunki::engine::Engine> =
        Arc::new(nunki::engine::fake::FakeEngine::default());
    let err = verify::verify(&world.project, "nope", engine, "docker").unwrap_err();
    assert!(matches!(err, VerifyError::NotStarted(_)), "{err}");
    assert!(err.to_string().contains("nunki mission start"), "{err}");
}

/// A tree the agent is still writing is not a tree to judge. This is the
/// interlock `nunki exec --tree` deliberately does not carry, and this is where
/// it belongs (SPEC 4.2).
#[test]
fn a_mission_whose_run_is_still_going_is_not_verified_under_it() {
    let world = World::new(1);
    world.commit("src/new.rs", "pub fn two() -> u8 { 2 }\n", "L1");
    world.journal_names_head();

    // A handle whose log holds no result and whose container is unknown to
    // the engine reads as *not* running — nunki does not know, and refusing to
    // verify because the engine is down would be a lie in another place.
    let store = Store::open(&world.project.hq_root).unwrap();
    let mut state = store.load("m1").unwrap();
    state.run = Some(nunki::harness::RunHandle {
        session: nunki::harness::SessionId("s".into()),
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
/// (SPEC 4.2). A slot already driven by another `nunki` is not driven twice.
#[test]
fn a_slot_already_held_is_not_verified_under_the_other_hq() {
    let world = World::new(1);
    world.journal_names_head();
    let held = nunki::state::SlotLock::acquire(
        &world.project.hq_root.join("locks"),
        "one",
        "something else",
    )
    .unwrap();
    assert!(matches!(world.verify(), Err(VerifyError::Lock(_))));
    drop(held);
    // Held, so the coder's run is refused by name rather than launched in a
    // world with no account.
    world.hold();
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
    use nunki::mutants::Triage;
    use std::collections::BTreeMap;

    let world = World::new(1);
    // The battery and the campaign the stack declares, in the project's home
    // where `nunki init` puts them, and mounted read-only below: they judge
    // the mission, so they are never the mission's to commit.
    let stack = world.project.fragment("rust");
    std::fs::create_dir_all(&stack).unwrap();
    let executable = |name: &str, body: &str| {
        let path = stack.join(name);
        std::fs::write(&path, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    };
    executable(
        nunki::gate::BATTERY,
        "#!/bin/sh\nset -eu\ntest -f src/new.rs\n",
    );
    executable(
        nunki::mutants::SCRIPT,
        "#!/bin/sh\n\
         echo '{\"id\":\"src/new.rs:1\",\"file\":\"src/new.rs\",\"line\":1,\
         \"description\":\"replace two with 0\"}'\n",
    );

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
    let slot = nunki::slot::Slot {
        name: "one".into(),
        tree: world.tree.clone(),
    };
    let volume = nunki::exec::proof_volume(&slot.name);
    let file = nunki::run::profile_path(&world.project, &slot.name);
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
             \x20     - {stack}/{battery}:{stack_at}/{battery}:ro\n\
             \x20     - {stack}/{campaign}:{stack_at}/{campaign}:ro\n\
             \x20   tmpfs:\n\
             \x20     - /run/nunki\n\
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
            tree_at = nunki::run::TREE_AT,
            proof = nunki::exec::PROOF_AT,
            stack = stack.display(),
            stack_at = nunki::run::STACK_AT,
            battery = nunki::gate::BATTERY,
            campaign = nunki::mutants::SCRIPT,
        ),
    )
    .unwrap();

    let engine: Arc<dyn nunki::engine::Engine> = Arc::new(nunki::engine::docker::Docker::real());
    let compose_project =
        nunki::compose::project_name(&world.project.session(), &slot.name).unwrap();
    let _ = engine.down(&file, &compose_project, true);
    engine.up(&file, &compose_project).unwrap();
    let go = || verify::verify(&world.project, "m1", engine.clone(), "docker").unwrap();

    // 1. The coder still owes its lot a run; gates 1 to 4 are green. Held,
    //    because this world has no account to launch it with.
    world.hold();
    let steps = go();
    assert!(
        gates_of(&steps).passed(),
        "{:?}",
        gates_of(&steps).failure()
    );
    assert!(matches!(steps.last(), Some(Step::Held { role, .. }) if *role == Role::Coder));

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
        Some(&nunki::gate::Decision::Passed),
        "{report:?}"
    );
    match report
        .outcomes
        .iter()
        .find(|o| o.gate == Gate::Mutation)
        .map(|o| &o.decision)
    {
        Some(nunki::gate::Decision::Unplayed(why)) => {
            assert!(why.contains("nunki mission mutants"))
        }
        other => panic!("no campaign has run: {other:?}"),
    }
    // That gate is unplayable, not red, and the difference decides who is
    // owed the next move. The flow stays where it is — no volet, no attempt,
    // no run — and names what it is waiting for. Here the battery is green in
    // a real container, so gate 7 stands alone and the obstacle is one nunki
    // clears itself: the step is a campaign owed, not a wall.
    assert!(matches!(world.state().flow.stage(), Stage::Gates));
    assert!(
        steps.iter().any(|s| matches!(s, Step::CampaignOwed { .. })),
        "{steps:?}"
    );

    // 4. The campaign runs — through the flow's own entry point, the one the
    // monitor calls on that step, so this proves it against a real engine
    // where a fake can only prove the decision to call it.
    loop {
        match nunki::verify::campaign(&world.project, "m1", engine.clone()).unwrap() {
            nunki::mutants::Progress::Running { .. } => {
                std::thread::sleep(std::time::Duration::from_millis(300))
            }
            nunki::mutants::Progress::Started { .. } => {}
            other => {
                println!("campaign: {other:?}");
                break;
            }
        }
    }
    // 4 bis. The survivor has no outcome, so gate 7 is **red** — the agent's
    // to answer, unlike the unplayable gate above — and a red gate at the
    // final verification opens a volet: the coder is sent back for it.
    //
    // This is what the whole chain exists to do, and it was not checked here
    // until 2026-09-16: the test wrote the triage itself before ever playing
    // the gates again, so the volet never opened, and step 5's coder run was
    // applied to a flow still sitting at the gates. It has been red on `dev`
    // ever since — measured by running it there, unchanged — and nothing
    // caught it because a live test is `#[ignore]`d and CI never plays it.
    let steps = go();
    assert!(
        matches!(gates_of(&steps).verdict(), nunki::gate::Verdict::Red(_)),
        "a survivor with no outcome is red: {steps:?}"
    );
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

    // What the coder writes on that volet: the survivor, killed by a test it
    // names and commits.
    let mut triage = BTreeMap::new();
    triage.insert(
        "src/new.rs:1".to_string(),
        Triage::Killed {
            test: "two_is_two".into(),
        },
    );
    nunki::mutants::write_triage(&world.mission(), &triage).unwrap();

    // 4 ter. A run in the slot stops verification dead: gates read a tree,
    // and a tree the agent is still writing is not a tree to judge.
    {
        use nunki::harness::spawn::{CommandSpec, Spawner};
        let session = "77777777-8888-4999-8aaa-bbbbbbbbbbbb";
        let spawner = nunki::engine::spawn::ContainerSpawner::new(
            engine.clone(),
            file.clone(),
            &compose_project,
            nunki::compose::AGENT_SERVICE,
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
        state.run = Some(nunki::harness::RunHandle {
            session: nunki::harness::SessionId(session.to_string()),
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
            .signal(&spawned, nunki::harness::spawn::Signal::Terminate)
            .unwrap();
        let mut state = store.load("m1").unwrap();
        state.run = None;
        store.save(&state).unwrap();
    }

    // 5. The volet's run ends, and this time everything is green.
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
        services: vec![nunki::mission::Service {
            name: "db".into(),
            reach: vec!["db".into()],
            shared: false,
        }],
        wiring: vec!["compose.yaml".into()],
    }
}

impl World {
    /// Drive the flow to the stage where the integrator is due, the way a
    /// finished coder and a green verification do.
    ///
    /// And leave a world where the integrator owes its gates nothing: the
    /// coder's commit, recorded as green the way `verify` records it, one
    /// wiring commit after it, the journal naming `HEAD`, the integrator's
    /// section of `PR.md`, and a profile up for the system battery to run in.
    /// A test that wants a gate red breaks exactly one of these.
    fn at_integration(&self) {
        self.commit("src/new.rs", "pub fn two() -> u8 { 2 }\n", "L1");
        let coder = self.head();
        self.commit("compose.yaml", "services: {}\n", "wire it");
        self.journal_names_head();
        std::fs::write(
            self.mission().join("PR.md"),
            "# L1\n\nThe lot.\n\n## Integration\n\nWired to its services.\n",
        )
        .unwrap();
        let profile = nunki::run::profile_path(&self.project, "one");
        std::fs::create_dir_all(profile.parent().unwrap()).unwrap();
        std::fs::write(&profile, "services: {}\n").unwrap();

        let store = Store::open(&self.project.hq_root).unwrap();
        let mut state = store.load("m1").unwrap();
        state.conclude(Role::Coder, None, &coder);
        store
            .apply(
                &mut state,
                nunki::mission::flow::Event::RunEnded {
                    outcome: nunki::harness::Outcome::Finished(Default::default()),
                    lot_done: true,
                },
            )
            .unwrap();
        store
            .apply(&mut state, nunki::mission::flow::Event::GatesPassed)
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
        state.run = Some(nunki::harness::RunHandle {
            session: nunki::harness::SessionId("s-integration".into()),
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
    // On its gates, not on its word: the system battery was played.
    let report = gates_of(&steps);
    assert_eq!(report.role, Role::Integrator);
    assert!(
        report
            .outcomes
            .iter()
            .any(|o| o.gate == nunki::gate::Gate::Battery
                && o.decision == nunki::gate::Decision::Passed),
        "{report:?}"
    );
    // The run is forgotten with the transition: left recorded, the next
    // `verify` would read the same run back a second time.
    assert!(world.state().run.is_none(), "{:?}", world.state().run);
}

/// An `INTEGRATED` is the integrator's word, and its gates are what the word
/// is worth: an agent's report is never the truth (SPEC 2).
///
/// Found on 2026-09-15, before the first integration mission was launched:
/// the flow read the verdict and moved on, so an `INTEGRATED` with no system
/// test run and no section of `PR.md` reached `Verified`.
#[test]
fn an_integrated_verdict_is_worth_its_gates_and_no_more() {
    let world = World::shaped(1, integration());
    world.at_integration();
    // The coder's pull request, never completed.
    std::fs::write(world.mission().join("PR.md"), "# L1\n\nThe lot.\n").unwrap();
    world.run_recorded(Some(41), FINISHED);
    world.verdict("Integrator", "INTEGRATED", &world.head(), "wired");

    // The relaunch that follows needs images and an account this world has
    // neither of; what is asserted is the transition, written before.
    let _ = world.verify();
    let state = world.state();
    assert!(
        matches!(state.flow.stage(), Stage::Integration { attempt: 2 }),
        "the integrator fixes its own work, run after run (SPEC 4.5): {:?}",
        state.flow.stage()
    );
    assert_eq!(state.flow.volets(), 0, "and the coder is not sent a volet");
    assert!(
        state.concluded(Role::Integrator).is_none(),
        "a verdict its gates refused is not recorded"
    );
    let told = world.followup();
    assert!(
        told.contains("integration, attempt 1") && told.contains("Integration"),
        "the next attempt is told which gate and why: {told}"
    );
}

/// A `BROKEN` sends the coder a volet — once the integrator's own run has
/// honoured what every run owes. A commit outside its wiring is its own to
/// answer for, whatever it concluded.
#[test]
fn a_broken_verdict_still_owes_the_gates_every_run_owes() {
    let world = World::shaped(1, integration());
    world.at_integration();
    world.commit(
        "src/new.rs",
        "pub fn two() -> u8 { 3 }\n",
        "repair it myself",
    );
    world.journal_names_head();
    world.run_recorded(Some(41), FINISHED);
    world.verdict(
        "Integrator",
        "BROKEN",
        &world.head(),
        "the adapter cannot be wired as it stands",
    );
    world.hold();

    let _ = world.verify();
    let state = world.state();
    assert!(
        matches!(state.flow.stage(), Stage::Integration { attempt: 2 }),
        "{:?}",
        state.flow.stage()
    );
    assert_eq!(state.flow.volets(), 0);
}

/// A gate about the machine stops the flow instead of spending an attempt:
/// here, no profile is up for the system battery to run in, and no
/// integrator run would change that.
#[test]
fn an_integrators_gate_nobody_could_play_stops_and_spends_nothing() {
    let world = World::shaped(1, integration());
    world.at_integration();
    std::fs::remove_file(nunki::run::profile_path(&world.project, "one")).unwrap();
    world.run_recorded(Some(41), FINISHED);
    world.verdict("Integrator", "INTEGRATED", &world.head(), "wired");

    let steps = world.verify().unwrap();
    assert!(
        matches!(steps.last(), Some(Step::GateUnplayable { role, .. }) if *role == Role::Integrator),
        "{steps:?}"
    );
    let state = world.state();
    assert!(
        matches!(state.flow.stage(), Stage::Integration { attempt: 1 }),
        "{:?}",
        state.flow.stage()
    );
    assert!(
        state.run.is_some(),
        "the run stays recorded, to be read back and judged again"
    );
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
    // No VERDICT.json at all: `nunki mission dir` creates an empty one.

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

/// A run `nunki` cannot ask about decides nothing: a machine that slept must not
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
    // Held: the read-back still happens, and the coder's launch is refused
    // by name rather than failing in a world with no account.
    world.hold();

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
        matches!(steps.last(), Some(Step::Held { role, .. }) if *role == Role::Coder),
        "the coder is owed the volet: {steps:?}"
    );
}

// --- the security mission, read back (SPEC 4.4, 4.5) -----------------------

fn with_security_agent(lots: usize) -> World {
    let world = World::shaped(lots, integration());
    // `verify` reads the **frozen** header out of the state and never the
    // file (SPEC 4.1), so declaring the security agent means rebuilding the
    // flow, not rewriting `MISSION.md`.
    let mut header = header_of(lots, integration());
    header.security = Security::Agent;
    let store = Store::open(&world.project.hq_root).unwrap();
    store
        .save(&MissionState {
            id: "m1".into(),
            slot: "one".into(),
            flow: Flow::new(header).unwrap(),
            run: None,
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
    world
}

impl World {
    /// Drive the flow to the stage where the security agent is due, and leave
    /// a world where the security agent owes its gates nothing: a journal
    /// naming `HEAD`, and a report in the verdict each test writes. A test
    /// that wants a gate red breaks one of the two.
    fn at_security(&self) {
        self.journal_names_head();
        let store = Store::open(&self.project.hq_root).unwrap();
        let mut state = store.load("m1").unwrap();
        for event in [
            nunki::mission::flow::Event::RunEnded {
                outcome: nunki::harness::Outcome::Finished(Default::default()),
                lot_done: true,
            },
            nunki::mission::flow::Event::GatesPassed,
            nunki::mission::flow::Event::Verdict {
                verdict: nunki::mission::Verdict::Integrated,
                report: "wired".into(),
            },
        ] {
            store.apply(&mut state, event).unwrap();
        }
        assert!(
            matches!(state.flow.stage(), Stage::SecurityAgent { .. }),
            "{:?}",
            state.flow.stage()
        );
    }
}

/// `CLEAR` on this `HEAD` is the last thing the mission was waiting for.
#[test]
fn a_clear_security_run_ends_the_mission() {
    let world = with_security_agent(1);
    world.at_security();
    world.run_recorded(Some(41), FINISHED);
    world.verdict("Security", "CLEAR", &world.head(), "nothing found");

    let steps = world.verify().unwrap();
    assert!(matches!(steps.last(), Some(Step::Verified)), "{steps:?}");
    assert!(world.state().run.is_none());
}

/// `FINDINGS` parks the mission where a human decides: the HQ iterates, or
/// the findings are lifted. Both events exist in the flow and no verb applies
/// either yet — so this asserts where it stops, not that it moves on.
#[test]
fn findings_park_the_mission_and_carry_the_report() {
    let world = with_security_agent(1);
    world.at_security();
    world.run_recorded(Some(41), FINISHED);
    world.verdict("Security", "FINDINGS", &world.head(), "an open redirect");

    let steps = world.verify().unwrap();
    assert!(
        matches!(steps.last(), Some(Step::Findings { report, .. }) if report.contains("open redirect")),
        "{steps:?}"
    );
    assert!(matches!(world.state().flow.stage(), Stage::Findings { .. }));
}

/// A verdict is the security agent's word, and its gates are what the word is
/// worth (SPEC 2: an agent's report is never the truth). It commits nothing,
/// so what it owes is a resume block naming the commit it attacked and a
/// report — and `CLEAR` with an empty report is a verdict about nothing.
///
/// Found on 2026-09-15, before the first security mission was launched: the
/// flow read the verdict and moved on, so both were taken on trust.
#[test]
fn a_security_verdict_is_worth_its_gates_and_no_more() {
    // An empty report, which is the deliverable this role owes.
    let world = with_security_agent(1);
    world.at_security();
    world.run_recorded(Some(41), FINISHED);
    world.verdict("Security", "CLEAR", &world.head(), "");

    let _ = world.verify();
    let state = world.state();
    assert!(
        matches!(state.flow.stage(), Stage::SecurityAgent { attempt: 2 }),
        "{:?}",
        state.flow.stage()
    );
    assert!(
        state.concluded(Role::Security).is_none(),
        "a verdict its gates refused is not recorded"
    );
    let told = world.followup();
    assert!(
        told.contains("security, attempt 1"),
        "the next attempt is told why: {told}"
    );

    // And a journal that names another commit: the block is about work
    // nobody attacked.
    let other = with_security_agent(1);
    other.at_security();
    std::fs::write(
        other.mission().join("JOURNAL.md"),
        "# Journal\n\n## ÉTAT DE REPRISE\n\nHEAD is `0000000000000000000000000000000000000000`.\n",
    )
    .unwrap();
    other.run_recorded(Some(41), FINISHED);
    other.verdict("Security", "CLEAR", &other.head(), "nothing found");

    let _ = other.verify();
    assert!(
        matches!(
            other.state().flow.stage(),
            Stage::SecurityAgent { attempt: 2 }
        ),
        "{:?}",
        other.state().flow.stage()
    );
}

/// The security agent's verdict is signed by the security agent. An
/// integrator's `INTEGRATED` left in the same file is not a security answer.
#[test]
fn an_integrators_verdict_does_not_clear_the_security_run() {
    let world = with_security_agent(1);
    world.at_security();
    world.run_recorded(Some(41), FINISHED);
    world.verdict("Integrator", "INTEGRATED", &world.head(), "wired");

    let _ = world.verify();
    assert!(
        matches!(
            world.state().flow.stage(),
            Stage::SecurityAgent { attempt: 2 }
        ),
        "{:?}",
        world.state().flow.stage()
    );
}

// --- lifting a security verdict (SPEC 4.5) ---------------------------------

impl World {
    /// Drive a mission to `FINDINGS`, and say who is running `nunki`, so the
    /// acceptance records a name rather than whatever `$USER` happens to be.
    fn at_findings(&self) {
        std::fs::write(
            self.project.nunki_home().join("me.yaml"),
            "name: Arnaud\nemail: a@example.com\n",
        )
        .unwrap();
        self.at_security();
        self.run_recorded(Some(41), FINISHED);
        self.verdict(
            "Security",
            "FINDINGS",
            &self.head(),
            "an open redirect in /auth/callback",
        );
        let steps = self.verify().unwrap();
        assert!(
            matches!(steps.last(), Some(Step::Findings { .. })),
            "{steps:?}"
        );
    }

    fn followup(&self) -> String {
        std::fs::read_to_string(self.mission().join("FOLLOWUP_HQ.md")).unwrap()
    }
}

/// The constat lives in the journal of the role that made it, and the HQ
/// carries it to the coder — dated, in `FOLLOWUP_HQ.md`. An agent never
/// speaks to another agent.
#[test]
fn a_red_verdict_is_carried_to_the_coder_and_a_green_one_is_not() {
    let world = with_security_agent(1);
    world.at_findings();
    let carried = world.followup();
    assert!(carried.contains("security"), "{carried}");
    assert!(carried.contains("open redirect"), "{carried}");

    // A green verdict is not something the coder has to act on, and a
    // follow-up file that fills with green is one nobody reads.
    let other = World::shaped(1, integration());
    other.at_integration();
    other.run_recorded(Some(41), FINISHED);
    other.verdict("Integrator", "INTEGRATED", &other.head(), "wired");
    other.verify().unwrap();
    assert!(
        !other.followup().contains("concluded Integrated"),
        "{}",
        other.followup()
    );
}

/// Lifting the verdict as a whole is what concludes the mission — and it
/// never touches `VERDICT.json`: the verdict says what the agent found, the
/// state says what the human decided.
#[test]
fn lifting_the_verdict_concludes_the_mission_and_leaves_the_verdict_red() {
    let world = with_security_agent(1);
    world.at_findings();

    let state = nunki::findings::accept(
        &world.project,
        "m1",
        nunki::findings::Lift::Verdict,
        "the callback is behind the VPN and the host allowlist is closed",
    )
    .unwrap();
    assert!(
        matches!(state.flow.stage(), Stage::Verified),
        "{:?}",
        state.flow.stage()
    );
    assert!(state.verdict_lifted_on(&world.head()));

    let verdict = std::fs::read_to_string(world.mission().join("VERDICT.json")).unwrap();
    assert!(verdict.contains("FINDINGS"), "{verdict}");
    let carried = world.followup();
    assert!(carried.contains("Arnaud"), "{carried}");
    assert!(carried.contains("behind the VPN"), "{carried}");
}

/// Naming one finding records it and does **not** conclude: itemising is not
/// concluding, and a session that treated the first acceptance as the last
/// would push on a report nobody finished reading.
#[test]
fn accepting_one_finding_records_it_without_concluding() {
    let world = with_security_agent(1);
    world.at_findings();

    let state = nunki::findings::accept(
        &world.project,
        "m1",
        nunki::findings::Lift::Finding("open redirect in /auth/callback".into()),
        "unreachable from outside the VPN",
    )
    .unwrap();
    assert!(matches!(state.flow.stage(), Stage::Findings { .. }));
    assert!(!state.verdict_lifted_on(&world.head()));
    assert_eq!(state.accepted_on(&world.head()).len(), 1);

    // And `verify` shows it back, so the human sees what they have lifted.
    let steps = world.verify().unwrap();
    assert!(
        matches!(steps.last(), Some(Step::Findings { lifted, .. })
            if lifted.iter().any(|l| l.contains("unreachable from outside"))),
        "{steps:?}"
    );
}

/// A verdict, and its lift, are worth one commit and no other. A new commit
/// makes an acceptance stale rather than silently carrying it forward.
#[test]
fn an_acceptance_given_on_another_commit_does_not_hold() {
    let world = with_security_agent(1);
    world.at_findings();
    nunki::findings::accept(
        &world.project,
        "m1",
        nunki::findings::Lift::Verdict,
        "accepted on the commit that was judged",
    )
    .unwrap();
    let judged = world.head();
    assert!(world.state().verdict_lifted_on(&judged));

    world.commit(
        "src/new.rs",
        "pub fn two() -> u8 { 2 }\n",
        "one more change",
    );
    assert!(
        !world.state().verdict_lifted_on(&world.head()),
        "the lift was given on {judged}, and the slot has moved"
    );
}

/// A risk accepted without a reason is not accepted, it is forgotten.
#[test]
fn a_lift_without_a_reason_is_refused() {
    let world = with_security_agent(1);
    world.at_findings();
    let err = nunki::findings::accept(
        &world.project,
        "m1",
        nunki::findings::Lift::Verdict,
        "   \n\t ",
    )
    .unwrap_err();
    assert!(
        matches!(err, nunki::findings::FindingsError::NoReason),
        "{err}"
    );
    assert!(matches!(world.state().flow.stage(), Stage::Findings { .. }));
}

/// Iterating is a gesture, not a default: a `verify` that sent the mission
/// back by itself would spend a volet the human might have wanted to spend on
/// an acceptance instead.
#[test]
fn iterating_sends_the_mission_back_to_the_coder_as_a_volet() {
    let world = with_security_agent(1);
    world.at_findings();

    // Running `verify` again changes nothing: it stops at the same place.
    let again = world.verify().unwrap();
    assert!(
        matches!(again.last(), Some(Step::Findings { .. })),
        "{again:?}"
    );

    let state = nunki::findings::iterate(&world.project, "m1").unwrap();
    assert!(
        matches!(
            state.flow.stage(),
            Stage::Coding {
                work: Work::Volet { .. },
                ..
            }
        ),
        "{:?}",
        state.flow.stage()
    );
}

/// Neither verb applies anywhere but on a security verdict, and says where
/// the mission actually is rather than failing obscurely.
#[test]
fn neither_verb_applies_before_there_is_a_verdict_to_lift() {
    let world = with_security_agent(1);
    for err in [
        nunki::findings::accept(&world.project, "m1", nunki::findings::Lift::Verdict, "why")
            .unwrap_err(),
        nunki::findings::iterate(&world.project, "m1").unwrap_err(),
    ] {
        assert!(
            matches!(err, nunki::findings::FindingsError::NotOnFindings { .. }),
            "{err}"
        );
        assert!(err.to_string().contains("Coding"), "{err}");
    }
}

/// The whole point of `nunki mission stop` (SPEC 4.5): "`nunki` ne relancera aucun
/// run". This world has neither images nor an account, so an unheld `verify`
/// at this stage **fails trying to launch** — which is what makes the held
/// case worth asserting: it returns, having launched nothing.
#[test]
fn a_held_mission_launches_no_integration_run() {
    let world = World::shaped(1, integration());
    world.at_integration();
    assert!(
        world.verify().is_err(),
        "the probe is worth nothing unless the unheld path really launches"
    );

    world.hold();
    let steps = world.verify().unwrap();
    assert!(
        matches!(steps.last(), Some(Step::Held { role, .. }) if *role == nunki::harness::Role::Integrator),
        "{steps:?}"
    );
}

/// Holding a mission stops `nunki` from starting work, not from reading what is
/// already there: the gates still run, and the coder's run is refused by a
/// step that names who held it.
#[test]
fn a_held_mission_still_plays_its_gates_and_says_who_held_it() {
    let world = World::shaped(1, integration());
    world.hold();
    let steps = world.verify().unwrap();

    assert!(
        steps.iter().any(|s| matches!(s, Step::Gates { .. })),
        "{steps:?}"
    );
    match steps.last() {
        Some(Step::Held { role, who, .. }) => {
            assert_eq!(*role, nunki::harness::Role::Coder);
            assert!(!who.is_empty(), "it names who held it");
        }
        other => panic!("{other:?}"),
    }
}

/// The hold is lifted by `resume`, and the flow is exactly where it was: a
/// hold is not a stage, so nothing about the mission's progress moved while
/// it was held.
#[test]
fn lifting_the_hold_leaves_the_flow_where_it_was() {
    let world = World::shaped(1, integration());
    world.at_integration();
    let before = world.state().flow.stage().clone();
    world.hold();
    world.verify().unwrap();

    let engine: Arc<dyn nunki::engine::Engine> =
        Arc::new(nunki::engine::fake::FakeEngine::default());
    nunki::gesture::resume(&world.project, "m1", engine).unwrap();
    assert_eq!(&before, world.state().flow.stage());
    assert!(!world.state().held());
}

/// The hold has to mean the same thing at every launch site, and the
/// security one is a second site: a mutation that removed only its guard
/// survived the whole battery until this test existed.
#[test]
fn a_held_mission_launches_no_security_run() {
    let world = with_security_agent(1);
    world.at_security();
    assert!(
        world.verify().is_err(),
        "the probe is worth nothing unless the unheld path really launches"
    );

    world.hold();
    let steps = world.verify().unwrap();
    assert!(
        matches!(steps.last(), Some(Step::Held { role, .. }) if *role == nunki::harness::Role::Security),
        "{steps:?}"
    );
}

// --- waiting out the harness (SPEC 4.3) -------------------------------------

/// A run that fell for a quota, as Claude Code writes it.
const QUOTA: &str = "{\"type\":\"result\",\"subtype\":\"error\",\"is_error\":true,\
                     \"result\":\"API Error: 429 rate limit reached\",\"usage\":{}}\n";
/// A run whose harness could not authenticate; the text says nothing, the
/// status says it all.
const UNAUTHORIZED: &str = "{\"type\":\"result\",\"subtype\":\"error\",\"is_error\":true,\
                            \"result\":\"boom\",\"api_error_status\":401}\n";

impl World {
    fn harness_down(&self, record: Option<nunki::backoff::HarnessDown>) {
        let store = Store::open(&self.project.hq_root).unwrap();
        let mut state = store.load("m1").unwrap();
        state.harness_down = record;
        store.save(&state).unwrap();
    }
}

/// A quota is replayed, not at once: the run is forgotten, no attempt is
/// spent, and the launch site answers with the wait. Asked again at once,
/// it still waits — and this world cannot launch (no images, no account),
/// so a guard that let the launch through would fail the call instead.
#[test]
fn a_harness_failure_is_waited_out_rather_than_relaunched() {
    let world = World::shaped(1, integration());
    world.at_integration();
    world.run_recorded(Some(41), QUOTA);

    let steps = world.verify().unwrap();
    match steps.last() {
        Some(Step::Waiting {
            role,
            failures,
            last,
            ..
        }) => {
            assert_eq!(*role, Role::Integrator);
            assert_eq!(*failures, 1);
            assert!(last.contains("rate limit"), "{last}");
        }
        other => panic!("{other:?}"),
    }
    let state = world.state();
    assert!(state.run.is_none(), "{:?}", state.run);
    assert_eq!(state.flow.stage(), &Stage::Integration { attempt: 1 });
    let down = state.harness_down.expect("the failure is counted");
    assert!(down.not_before > nunki::state::now_secs(), "{down:?}");

    let steps = world.verify().unwrap();
    assert!(
        matches!(steps.last(), Some(Step::Waiting { failures: 1, .. })),
        "{steps:?}"
    );
}

/// The wait has to hold at every launch site, as the hold does: the
/// security one is the second.
#[test]
fn a_harness_failure_of_the_security_run_is_waited_out_too() {
    let world = with_security_agent(1);
    world.at_security();
    world.run_recorded(Some(41), QUOTA);

    let steps = world.verify().unwrap();
    assert!(
        matches!(steps.last(), Some(Step::Waiting { role, .. }) if *role == Role::Security),
        "{steps:?}"
    );
    let steps = world.verify().unwrap();
    assert!(
        matches!(steps.last(), Some(Step::Waiting { failures: 1, .. })),
        "{steps:?}"
    );
}

/// No wait mends a revoked token: the first 401 holds the mission, and the
/// hold says why and that `nunki` put it there.
#[test]
fn an_authentication_failure_holds_the_mission_at_once() {
    let world = World::shaped(1, integration());
    world.at_integration();
    world.run_recorded(Some(41), UNAUTHORIZED);

    let steps = world.verify().unwrap();
    match steps.last() {
        Some(Step::Held { who, reason, .. }) => {
            assert_eq!(who, "nunki");
            let reason = reason.as_deref().unwrap_or_default();
            assert!(reason.contains("could not authenticate"), "{reason}");
        }
        other => panic!("{other:?}"),
    }
    assert!(world.state().held());
}

/// Past the ceiling, `nunki` stops waiting and holds the mission: a harness that
/// has been failing for six hours is not one more wait away from working.
#[test]
fn a_harness_down_past_the_ceiling_holds_the_mission() {
    let world = World::shaped(1, integration());
    world.at_integration();
    let now = nunki::state::now_secs();
    world.harness_down(Some(nunki::backoff::HarnessDown {
        failures: 6,
        since: now - 6 * 3600 + 60,
        not_before: now - 1,
        last: "429".into(),
    }));
    world.run_recorded(Some(41), QUOTA);

    let steps = world.verify().unwrap();
    match steps.last() {
        Some(Step::Held { reason, .. }) => {
            let reason = reason.as_deref().unwrap_or_default();
            assert!(reason.contains("6-hour ceiling"), "{reason}");
        }
        other => panic!("{other:?}"),
    }
}

/// A run the harness carried through, whatever it concluded, means the
/// harness works again: the count starts over.
#[test]
fn a_run_the_harness_carried_forgets_its_failures() {
    let world = World::shaped(1, integration());
    world.at_integration();
    world.harness_down(Some(nunki::backoff::HarnessDown {
        failures: 3,
        since: 0,
        not_before: 0,
        last: "429".into(),
    }));
    world.run_recorded(Some(41), FINISHED);
    world.verdict("Integrator", "INTEGRATED", &world.head(), "wired");

    let steps = world.verify().unwrap();
    assert!(matches!(steps.last(), Some(Step::Verified)), "{steps:?}");
    assert!(world.state().harness_down.is_none());
}

/// Lifting `nunki`'s hold forgets the failures with it: a count carried over
/// would send the very next quota straight back to the human.
#[test]
fn resuming_forgets_the_harness_failures() {
    let world = World::shaped(1, integration());
    world.at_integration();
    world.run_recorded(Some(41), UNAUTHORIZED);
    world.verify().unwrap();
    assert!(world.state().harness_down.is_some());

    let engine: Arc<dyn nunki::engine::Engine> =
        Arc::new(nunki::engine::fake::FakeEngine::default());
    nunki::gesture::resume(&world.project, "m1", engine).unwrap();
    let state = world.state();
    assert!(!state.held());
    assert!(state.harness_down.is_none(), "{:?}", state.harness_down);
}

// --- what a mission spends, and its caps (SPEC 7) ---------------------------

/// A finished run as Claude Code writes it, with what it spent in each kind.
const SPENDING: &str = "{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\
                        \"usage\":{\"input_tokens\":10,\"output_tokens\":40,\
                        \"cache_creation_input_tokens\":20,\"cache_read_input_tokens\":30}}\n";
/// A run that fell for a quota, having spent something first.
const QUOTA_SPENT: &str = "{\"type\":\"result\",\"subtype\":\"error\",\"is_error\":true,\
                           \"result\":\"API Error: 429 rate limit reached\",\
                           \"usage\":{\"input_tokens\":5,\"output_tokens\":1}}\n";

impl World {
    /// Re-freeze the header with these caps, the way `nunki mission reframe`
    /// would.
    fn capped(&self, max_runs: Option<u32>, max_tokens: Option<u64>) {
        let store = Store::open(&self.project.hq_root).unwrap();
        let mut state = store.load("m1").unwrap();
        let mut header = state.flow.header().clone();
        header.bounds.max_runs = max_runs;
        header.bounds.max_tokens = max_tokens;
        state.flow.reframe(header).unwrap();
        store.save(&state).unwrap();
    }

    fn spent(&self, runs: u32, input_tokens: u64) {
        let store = Store::open(&self.project.hq_root).unwrap();
        let mut state = store.load("m1").unwrap();
        state.spent = nunki::state::Spent {
            runs,
            usage: nunki::harness::Usage {
                input_tokens,
                ..Default::default()
            },
        };
        store.save(&state).unwrap();
    }
}

#[test]
fn a_run_read_back_is_counted_with_the_four_kinds_it_spent() {
    let world = World::shaped(1, integration());
    world.at_integration();
    world.run_recorded(Some(41), SPENDING);
    world.verdict("Integrator", "INTEGRATED", &world.head(), "wired");

    let steps = world.verify().unwrap();
    assert!(matches!(steps.last(), Some(Step::Verified)), "{steps:?}");
    let spent = world.state().spent;
    assert_eq!(spent.runs, 1);
    assert_eq!(
        spent.usage,
        nunki::harness::Usage {
            input_tokens: 10,
            output_tokens: 40,
            cache_creation_input_tokens: 20,
            cache_read_input_tokens: 30,
        }
    );
}

#[test]
fn a_run_that_fell_for_a_quota_is_counted_too() {
    let world = World::shaped(1, integration());
    world.at_integration();
    world.run_recorded(Some(41), QUOTA_SPENT);

    world.verify().unwrap();
    let spent = world.state().spent;
    assert_eq!(spent.runs, 1);
    assert_eq!(spent.usage.total(), 6);
}

/// At its cap, the mission is held instead of launched, and the hold says
/// which cap and how to raise it. One run short of it, the launch is really
/// attempted — this world cannot launch, so that is an error — which is what
/// makes the held case worth asserting.
#[test]
fn a_mission_at_its_run_cap_is_held_rather_than_launched() {
    let world = World::shaped(1, integration());
    world.at_integration();
    world.capped(Some(2), None);
    world.spent(1, 0);
    assert!(
        world.verify().is_err(),
        "below the cap, a launch is attempted"
    );

    world.spent(2, 0);
    let steps = world.verify().unwrap();
    match steps.last() {
        Some(Step::Held { who, reason, .. }) => {
            assert_eq!(who, "nunki");
            let reason = reason.as_deref().unwrap_or_default();
            assert!(reason.contains("max_runs"), "{reason}");
            assert!(reason.contains("nunki mission reframe m1"), "{reason}");
        }
        other => panic!("{other:?}"),
    }
    assert!(world.state().held());
}

#[test]
fn a_mission_at_its_token_cap_is_held_rather_than_launched() {
    let world = World::shaped(1, integration());
    world.at_integration();
    world.capped(None, Some(100));
    world.spent(0, 99);
    assert!(
        world.verify().is_err(),
        "below the cap, a launch is attempted"
    );

    world.spent(0, 100);
    let steps = world.verify().unwrap();
    match steps.last() {
        Some(Step::Held { reason, .. }) => {
            let reason = reason.as_deref().unwrap_or_default();
            assert!(reason.contains("max_tokens"), "{reason}");
        }
        other => panic!("{other:?}"),
    }
}

/// The cap holds at every launch site, as the hold and the wait do.
#[test]
fn the_cap_holds_at_the_security_launch_site_too() {
    let world = with_security_agent(1);
    world.at_security();
    world.capped(Some(1), None);
    world.spent(1, 0);

    let steps = world.verify().unwrap();
    assert!(
        matches!(steps.last(), Some(Step::Held { role, .. }) if *role == Role::Security),
        "{steps:?}"
    );
}

/// Raised in the header and resumed, the mission launches again: the cap is
/// a question put to the human, not an end.
#[test]
fn raising_the_cap_and_resuming_lets_the_mission_launch_again() {
    let world = World::shaped(1, integration());
    world.at_integration();
    world.capped(Some(1), None);
    world.spent(1, 0);
    world.verify().unwrap();
    assert!(world.state().held());

    world.capped(Some(5), None);
    let engine: Arc<dyn nunki::engine::Engine> =
        Arc::new(nunki::engine::fake::FakeEngine::default());
    nunki::gesture::resume(&world.project, "m1", engine).unwrap();
    assert!(world.verify().is_err(), "the launch is attempted again");
    assert!(!world.state().held());
}

/// No cap by default, and none written into a header that did not set one:
/// `mission new` copies the bounds into every header it writes.
#[test]
fn caps_are_absent_unless_set() {
    let bounds = Bounds::default();
    assert_eq!((bounds.max_runs, bounds.max_tokens), (None, None));
    let yaml = serde_yaml_ng::to_string(&bounds).unwrap();
    assert!(
        !yaml.contains("max_runs") && !yaml.contains("max_tokens"),
        "{yaml}"
    );
}

/// Counted at every place `nunki` reads a run back: the security run is the
/// second, and a mutation that dropped only its count survived until this
/// test existed.
#[test]
fn a_security_run_read_back_is_counted_too() {
    let world = with_security_agent(1);
    world.at_security();
    world.run_recorded(Some(41), SPENDING);
    world.verdict("Security", "CLEAR", &world.head(), "nothing found");

    let steps = world.verify().unwrap();
    assert!(matches!(steps.last(), Some(Step::Verified)), "{steps:?}");
    let spent = world.state().spent;
    assert_eq!((spent.runs, spent.usage.total()), (1, 100));
}

// --- the subscription's windows (SPEC 4.3) ----------------------------------

/// A finished run whose stream said how far the windows were used.
const MEASURED: &str = "{\"type\":\"rate_limit_event\",\"rate_limit_info\":{\"unifiedWindows\":\
                        {\"five_hour\":{\"utilization\":0.37,\"resetsAt\":1789003800},\
                        \"seven_day\":{\"utilization\":0.41,\"resetsAt\":1789351200}}}}\n\
                        {\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"usage\":{}}\n";

impl World {
    /// One account, so the mission spends it without naming it. Its token
    /// is absent: a launch attempted here fails, which is what makes a
    /// launch that was *not* attempted worth asserting.
    fn with_account(&self) {
        std::fs::write(
            self.project.nunki_home().join("accounts.yaml"),
            "accounts:\n  main:\n    harness: claude-code\n    token_file: accounts/main.token\n",
        )
        .unwrap();
    }

    fn five_hours_at(&self, per_mille: u32, resets_at: u64) {
        nunki::consumption::record(
            &self.project.nunki_home(),
            "main",
            &nunki::consumption::Measure {
                windows: nunki::consumption::Windows {
                    five_hour: Some(nunki::consumption::Window {
                        per_mille,
                        resets_at,
                    }),
                    weekly: None,
                },
                measured_at: nunki::state::now_secs(),
                harness: "claude-code".into(),
            },
        )
        .unwrap();
    }
}

/// Past 90 % of five hours, the launch waits for the reset — a wait, not a
/// hold: nothing for a human to lift. Below it, the launch is attempted.
#[test]
fn a_window_past_its_threshold_launches_nothing_until_it_resets() {
    let world = World::shaped(1, integration());
    world.at_integration();
    world.with_account();
    let now = nunki::state::now_secs();
    world.five_hours_at(100, now + 3_600);
    assert!(
        world.verify().is_err(),
        "below the threshold, a launch is attempted"
    );

    world.five_hours_at(950, now + 3_600);
    let steps = world.verify().unwrap();
    match steps.last() {
        Some(Step::Saving {
            role,
            account,
            per_mille,
            stop_at_percent,
            ..
        }) => {
            assert_eq!(*role, Role::Integrator);
            assert_eq!(account, "main");
            assert_eq!((*per_mille, *stop_at_percent), (950, 90));
        }
        other => panic!("{other:?}"),
    }
    let state = world.state();
    assert!(state.run.is_none() && !state.held(), "{state:?}");
}

/// Once the window has reset, the same measure forbids nothing: `nunki` goes on
/// by itself.
#[test]
fn a_window_that_has_reset_lets_the_launch_through() {
    let world = World::shaped(1, integration());
    world.at_integration();
    world.with_account();
    world.five_hours_at(990, nunki::state::now_secs() - 1);
    assert!(world.verify().is_err(), "the launch is attempted");
}

/// The wait holds at every launch site.
#[test]
fn the_window_holds_at_the_security_launch_site_too() {
    let world = with_security_agent(1);
    world.at_security();
    world.with_account();
    world.five_hours_at(950, nunki::state::now_secs() + 3_600);
    let steps = world.verify().unwrap();
    assert!(
        matches!(steps.last(), Some(Step::Saving { role, .. }) if *role == Role::Security),
        "{steps:?}"
    );
}

/// A run read back leaves its measure in the account's file, where the
/// supervisor reads it.
#[test]
fn a_run_read_back_leaves_its_measure_for_the_account() {
    let world = World::shaped(1, integration());
    world.at_integration();
    world.with_account();
    world.run_recorded(Some(41), MEASURED);
    world.verdict("Integrator", "INTEGRATED", &world.head(), "wired");

    world.verify().unwrap();
    let kept = nunki::consumption::read(&world.project.nunki_home(), "main")
        .unwrap()
        .expect("measured");
    assert_eq!(kept.windows.five_hour.map(|w| w.per_mille), Some(370));
    assert_eq!(kept.windows.weekly.map(|w| w.per_mille), Some(410));
    assert_eq!(kept.harness, "claude-code");
}

/// Recorded at every place `nunki` reads a run back: the security run is the
/// second, and a mutation that dropped only its record survived until this
/// test existed.
#[test]
fn a_security_run_read_back_leaves_its_measure_too() {
    let world = with_security_agent(1);
    world.at_security();
    world.with_account();
    world.run_recorded(Some(41), MEASURED);
    world.verdict("Security", "CLEAR", &world.head(), "nothing found");

    world.verify().unwrap();
    let kept = nunki::consumption::read(&world.project.nunki_home(), "main")
        .unwrap()
        .expect("measured");
    assert_eq!(kept.windows.five_hour.map(|w| w.per_mille), Some(370));
}

// --- a run nunki stopped for the window (SPEC 4.3) -----------------------------

impl World {
    /// What `nunki` writes when it tells the run to end its turn.
    fn spared(&self) {
        let store = Store::open(&self.project.hq_root).unwrap();
        let mut state = store.load("m1").unwrap();
        state.spared = Some(nunki::state::Spared {
            account: "main".into(),
            window: "five-hour window".into(),
            per_mille: 950,
            until: nunki::state::now_secs() + 3_600,
            date: "2026-09-11T00:00:00Z".into(),
        });
        store.save(&state).unwrap();
    }
}

/// Unmarked, a run that finished without a verdict is a failed attempt
/// (`a_run_that_left_no_verdict_did_not_conclude`). Marked, it is a turn `nunki`
/// ended: the same attempt is replayed, and the mark is spent.
#[test]
fn a_run_hq_spared_costs_no_attempt() {
    let world = World::shaped(1, integration());
    world.at_integration();
    world.run_recorded(Some(41), FINISHED);
    world.spared();

    // This world cannot launch the replay; what matters is written before.
    let _ = world.verify();
    let state = world.state();
    assert_eq!(state.flow.stage(), &Stage::Integration { attempt: 1 });
    assert!(state.spared.is_none(), "the mark is spent by its read-back");
    assert_eq!(state.spent.runs, 1, "it spent, and it is counted");
}

/// An nunki-initiated stop is not a harness failure: two of them in a row must
/// not walk the harness wait towards its ceiling.
#[test]
fn a_run_hq_spared_is_not_a_harness_failure() {
    let world = World::shaped(1, integration());
    world.at_integration();
    world.run_recorded(Some(41), QUOTA);
    world.spared();

    let _ = world.verify();
    let state = world.state();
    assert!(state.harness_down.is_none(), "{:?}", state.harness_down);
    assert_eq!(state.flow.stage(), &Stage::Integration { attempt: 1 });
}

/// A verdict written before the turn ended is a conclusion; the window has
/// nothing to say about it.
#[test]
fn a_verdict_written_before_the_turn_ended_still_stands() {
    let world = World::shaped(1, integration());
    world.at_integration();
    world.run_recorded(Some(41), FINISHED);
    world.verdict("Integrator", "INTEGRATED", &world.head(), "wired");
    world.spared();

    let steps = world.verify().unwrap();
    assert!(matches!(steps.last(), Some(Step::Verified)), "{steps:?}");
}

/// The mark is read at every read-back site: the security one is the second.
#[test]
fn a_security_run_hq_spared_costs_no_attempt_too() {
    let world = with_security_agent(1);
    world.at_security();
    world.run_recorded(Some(41), FINISHED);
    world.spared();

    let _ = world.verify();
    assert_eq!(
        world.state().flow.stage(),
        &Stage::SecurityAgent { attempt: 1 }
    );
}

/// `verify` finding a run still going is the moment to judge the window
/// while it runs: past the threshold the run is told to end its turn, and
/// the caller hears why instead of "a run is still going". Below it, the
/// run goes on and `verify` refuses as before.
///
/// The fake engine answers the liveness probe with the marker the probe
/// prints for a live process (`nunki-run-running`, in `engine::spawn`).
#[test]
fn a_run_still_going_past_the_threshold_is_told_to_end_its_turn() {
    use nunki::engine::{ExecOutput, Liveness, fake::FakeEngine};
    let still_going = || -> Arc<dyn nunki::engine::Engine> {
        Arc::new(
            FakeEngine::default()
                .with_liveness("cafe1234", Liveness::Running)
                .with_exec(ExecOutput {
                    status: 0,
                    stdout: "nunki-run-running\n".into(),
                    stderr: String::new(),
                }),
        )
    };
    let world = World::shaped(1, integration());
    world.at_integration();
    world.with_account();
    world.run_recorded(Some(41), "");
    let now = nunki::state::now_secs();

    world.five_hours_at(100, now + 3_600);
    match verify::verify(&world.project, "m1", still_going(), "docker") {
        Err(VerifyError::RunInProgress { .. }) => {}
        other => panic!("below the threshold the run goes on: {other:?}"),
    }
    assert!(world.state().spared.is_none());

    world.five_hours_at(950, now + 3_600);
    match verify::verify(&world.project, "m1", still_going(), "docker") {
        Err(VerifyError::Spared { account, .. }) => assert_eq!(account, "main"),
        other => panic!("past the threshold the turn is ended: {other:?}"),
    }
    assert!(world.state().spared.is_some());
}

// --- the coder's runs, read back and relaunched (SPEC 4.3) -------------------

impl World {
    /// The journal's resume block, naming HEAD, with the run's `Lot:` line.
    fn journal_says(&self, line: &str) {
        std::fs::write(
            self.mission().join("JOURNAL.md"),
            format!(
                "# Journal\n\n## ÉTAT DE REPRISE\n\nHEAD is `{}`.\n{line}\n",
                self.head()
            ),
        )
        .unwrap();
    }

    /// A coder session, as `nunki mission start` records one.
    fn coder_session(&self, id: &str) {
        let store = Store::open(&self.project.hq_root).unwrap();
        let mut state = store.load("m1").unwrap();
        state.coder_session = Some(nunki::harness::SessionId(id.into()));
        store.save(&state).unwrap();
    }

    /// A coder run that finished, having left `line` in its resume block.
    /// Held, so the relaunch is refused by name instead of failing in a
    /// world with no account.
    fn coder_ran(&self, line: &str) -> Vec<Step> {
        self.journal_says(line);
        self.coder_session("s-coder");
        self.run_recorded(Some(41), FINISHED);
        self.hold();
        self.verify().unwrap()
    }
}

fn coding(state: &MissionState) -> (Work, u32) {
    match state.flow.stage() {
        Stage::Coding { work, attempt } => (work.clone(), *attempt),
        other => panic!("{other:?}"),
    }
}

/// The run says its lot is done, the gates agree: the next lot, first
/// attempt, in the same session — and the run is counted and forgotten.
#[test]
fn a_coder_run_that_says_its_lot_is_done_moves_to_the_next_lot() {
    let world = World::new(2);
    world.commit("src/new.rs", "pub fn two() -> u8 { 2 }\n", "L1");
    let steps = world.coder_ran("Lot: L1 — done");

    assert!(
        gates_of(&steps).passed(),
        "{:?}",
        gates_of(&steps).failure()
    );
    let state = world.state();
    assert_eq!(coding(&state), (Work::Lot(1), 1));
    assert!(state.run.is_none(), "{:?}", state.run);
    assert_eq!(state.spent.runs, 1, "counted when read back");
    assert_eq!(
        state.coder_session,
        Some(nunki::harness::SessionId("s-coder".into())),
        "the next lot resumes the session"
    );
    assert!(
        matches!(steps.last(), Some(Step::Held { role, .. }) if *role == Role::Coder),
        "{steps:?}"
    );
}

/// A lot the run says it failed is one more attempt, in a fresh session, and
/// the next attempt reads why in the file every run reads first.
#[test]
fn a_run_that_says_its_lot_failed_costs_an_attempt_and_its_session() {
    let world = World::new(2);
    world.commit("src/new.rs", "pub fn two() -> u8 { 2 }\n", "L1");
    world.coder_ran("Lot: L1 — failed: the parser rejects empty input");

    let state = world.state();
    assert_eq!(coding(&state), (Work::Lot(0), 2));
    assert!(state.coder_session.is_none(), "{:?}", state.coder_session);
    let followup = world.followup();
    assert!(followup.contains("L1, attempt 1"), "{followup}");
    assert!(
        followup.contains("the parser rejects empty input"),
        "{followup}"
    );
}

/// No `Lot:` line is no word that the lot is done: a failed attempt.
#[test]
fn a_run_that_does_not_say_its_lot_is_done_has_not_finished_it() {
    let world = World::new(2);
    world.commit("src/new.rs", "pub fn two() -> u8 { 2 }\n", "L1");
    world.coder_ran("Nothing said about the lot.");

    let state = world.state();
    assert_eq!(coding(&state), (Work::Lot(0), 2));
    assert!(state.coder_session.is_none());
    assert!(
        world.followup().contains("no `Lot: L1 — done` line"),
        "{}",
        world.followup()
    );
}

/// A `done` for another lot is not this lot's.
#[test]
fn a_done_line_for_another_lot_is_not_this_lots() {
    let world = World::new(2);
    world.commit("src/new.rs", "pub fn two() -> u8 { 2 }\n", "L1");
    world.coder_ran("Lot: L2 — done");

    assert_eq!(coding(&world.state()), (Work::Lot(0), 2));
    assert!(
        world.followup().contains("reports lot L2"),
        "{}",
        world.followup()
    );
}

/// Red gates after a run cost that run's attempt, once: the launch that
/// follows is not preceded by a second judgement of the same tree.
#[test]
fn red_gates_after_a_coder_run_spend_one_attempt_not_two() {
    let world = World::new(2);
    world.commit("AGENTS.md", "rewritten by the agent\n", "loosen the rules");
    let steps = world.coder_ran("Lot: L1 — done");

    assert_eq!(coding(&world.state()), (Work::Lot(0), 2));
    assert_eq!(
        steps
            .iter()
            .filter(|s| matches!(s, Step::Gates { .. }))
            .count(),
        1,
        "{steps:?}"
    );
    assert!(world.state().coder_session.is_none());
    assert!(
        world.followup().contains("the gates were red"),
        "{}",
        world.followup()
    );
}

/// A turn `nunki` ended is replayed in the same session, and its tree is not
/// judged: a resume block one commit behind would turn the replay into a
/// spent attempt.
#[test]
fn a_coder_turn_hq_spared_is_replayed_without_judging_the_tree() {
    let world = World::new(2);
    world.journal_names_head();
    world.commit("src/new.rs", "pub fn two() -> u8 { 2 }\n", "L1, half done");
    world.coder_session("s-coder");
    world.run_recorded(Some(41), FINISHED);
    world.spared();
    world.hold();

    let steps = world.verify().unwrap();
    let state = world.state();
    assert_eq!(coding(&state), (Work::Lot(0), 1));
    assert!(state.spared.is_none(), "the mark is spent");
    assert_eq!(
        state.coder_session,
        Some(nunki::harness::SessionId("s-coder".into()))
    );
    assert!(
        !steps.iter().any(|s| matches!(s, Step::Gates { .. })),
        "{steps:?}"
    );
}

/// A harness that failed the coder's run is waited out, like any other role's,
/// and the session is kept for the replay.
#[test]
fn a_coder_harness_failure_is_waited_out_and_keeps_its_session() {
    let world = World::new(2);
    world.journal_names_head();
    world.coder_session("s-coder");
    world.run_recorded(Some(41), QUOTA);

    let steps = world.verify().unwrap();
    assert!(
        matches!(steps.last(), Some(Step::Waiting { role, .. }) if *role == Role::Coder),
        "{steps:?}"
    );
    let state = world.state();
    assert_eq!(coding(&state), (Work::Lot(0), 1));
    assert!(state.harness_down.is_some());
    assert_eq!(
        state.coder_session,
        Some(nunki::harness::SessionId("s-coder".into()))
    );
}

/// The coder's launch site answers to the account's window like the others.
#[test]
fn the_window_holds_at_the_coder_launch_site_too() {
    let world = World::new(2);
    world.journal_names_head();
    world.with_account();
    world.five_hours_at(950, nunki::state::now_secs() + 3_600);

    let steps = world.verify().unwrap();
    assert!(
        matches!(steps.last(), Some(Step::Saving { role, .. }) if *role == Role::Coder),
        "{steps:?}"
    );
}

/// And to the mission's caps: below, the launch is attempted (and fails in
/// this world); at the cap, the mission is held.
#[test]
fn the_cap_holds_at_the_coder_launch_site_too() {
    let world = World::new(2);
    world.journal_names_head();
    world.capped(Some(1), None);
    assert!(
        world.verify().is_err(),
        "below the cap, a launch is attempted"
    );

    world.spent(1, 0);
    let steps = world.verify().unwrap();
    match steps.last() {
        Some(Step::Held {
            role, who, reason, ..
        }) => {
            assert_eq!((*role, who.as_str()), (Role::Coder, "nunki"));
            assert!(reason.as_deref().unwrap_or_default().contains("max_runs"));
        }
        other => panic!("{other:?}"),
    }
}

/// Red final gates open a volet, and the coder is told why in the file every
/// run reads first — the one place a volet's reason reaches the agent.
#[test]
fn red_final_gates_tell_the_coder_why() {
    let world = World::new(1);
    world.commit("src/new.rs", "pub fn two() -> u8 { 2 }\n", "L1");
    world.journal_names_head();
    world.coder_finished_the_lot();
    world.hold();

    let steps = world.verify();
    let stage = world.state().flow.stage().clone();
    assert!(
        matches!(
            stage,
            Stage::Coding {
                work: Work::Volet { .. },
                ..
            }
        ),
        "{stage:?} after {steps:?}"
    );
    assert!(
        world.followup().contains("the final gates were red"),
        "{}",
        world.followup()
    );
}

/// The last lot done is not a stopping point: the same `verify` goes on to
/// the final gates, which this world has red — so the mission is already on
/// its first volet when it returns.
#[test]
fn the_last_lot_done_goes_on_to_the_final_gates_at_once() {
    let world = World::new(1);
    world.commit("src/new.rs", "pub fn two() -> u8 { 2 }\n", "L1");
    let steps = world.coder_ran("Lot: L1 — done");

    assert!(
        steps
            .iter()
            .any(|s| matches!(s, Step::Moved { to: Stage::Gates })),
        "{steps:?}"
    );
    assert_ne!(world.state().flow.stage(), &Stage::Gates, "{steps:?}");
}

/// A gate nobody could play stops the verification where it stands, and the
/// coder is not sent back at it.
///
/// Measured on 2026-09-13, on the first mission nunki drove from end to end:
/// gate 7 said `Unplayed` because no campaign had run, and starting one is
/// `nunki mission mutants` — an HQ verb no container holds. nunki opened a volet
/// anyway, three runs in a row, each reading the same instruction, saying it
/// had no way to carry it out, and stopping. The last cost 34 000 output
/// tokens to say so.
///
/// This world owes nothing: the journal names `HEAD` and `PR.md` is written,
/// so no gate is red. What is left is a battery with no profile up and a
/// campaign nobody has started — two gates about the machine, which is the
/// one thing an agent cannot be sent back to fix.
#[test]
fn a_gate_nobody_could_play_stops_instead_of_opening_a_volet() {
    let world = World::new(1);
    // Take the profile away: this test is about a gate that needs the machine
    // and cannot reach it, and the battery is the one it was written around.
    world.without_profile();
    world.commit("src/new.rs", "pub fn two() -> u8 { 2 }\n", "L1");
    world.journal_names_head();
    std::fs::write(
        world.mission().join("PR.md"),
        "# What this changes\n\nL1, and nothing else.\n",
    )
    .unwrap();
    world.coder_finished_the_lot();

    let steps = world.verify().unwrap();
    let report = gates_of(&steps);
    assert!(
        !matches!(report.verdict(), nunki::gate::Verdict::Red(_)),
        "nothing is the agent's to fix here: {report:?}"
    );
    assert!(
        matches!(steps.last(), Some(Step::GateUnplayable { .. })),
        "{steps:?}"
    );

    // The flow has not moved, which is the whole point: no volet, no
    // attempt, no run spent against a wall the agent cannot move. A later
    // `nunki verify`, once a human has done what the gate's sentence asks,
    // plays it again and goes on.
    assert_eq!(world.state().flow.stage(), &Stage::Gates, "{steps:?}");
    assert_eq!(world.state().flow.volets(), 0);

    // And nothing is written to the file the next run reads first, unlike a
    // red gate: there is no next run to read it.
    assert!(
        !world.followup().contains("could not be played"),
        "{}",
        world.followup()
    );
}

/// A volet is called `volet-<n>`: the name the coder is given, the one its
/// `Lot:` line must repeat, and the one a handover to the human names.
#[test]
fn a_volet_is_named_by_its_number() {
    let world = World::new(1);
    let work = Work::Volet {
        n: 2,
        cause: "gate: red".into(),
    };
    assert_eq!(world.state().flow.label(&work), "volet-2");
}

// --- a real provider, locked for the other missions (SPEC 7) -----------------

fn shared_integration() -> Integration {
    Integration::Services {
        services: vec![nunki::mission::Service {
            name: "stripe".into(),
            reach: vec!["api.stripe.com".into()],
            shared: true,
        }],
        wiring: vec!["compose.yaml".into()],
    }
}

impl World {
    /// Another mission of the project on the same framing, at its
    /// integration stage, with its run recorded or read back.
    fn another_mission_integrating(&self, with_run: bool) {
        let store = Store::open(&self.project.hq_root).unwrap();
        let mut other = self.state();
        other.id = "m2".into();
        other.slot = "two".into();
        other.run = with_run.then(|| nunki::harness::RunHandle {
            session: nunki::harness::SessionId("s-m2".into()),
            container: "beef5678".into(),
            pid: Some(42),
            log: PathBuf::from("/dev/null"),
        });
        store.save(&other).unwrap();
    }
}

/// A provider another mission's integration run holds is waited for: no
/// run, no attempt spent. Once that run is read back, the launch goes.
#[test]
fn a_provider_another_mission_holds_is_waited_for() {
    let world = World::shaped(1, shared_integration());
    world.at_integration();
    world.another_mission_integrating(true);

    let steps = world.verify().unwrap();
    match steps.last() {
        Some(Step::Busy { role, provider, by }) => {
            assert_eq!(*role, Role::Integrator);
            assert_eq!(provider, "stripe");
            assert_eq!(by, "mission m2");
        }
        other => panic!("{other:?}"),
    }
    let state = world.state();
    assert!(state.run.is_none());
    assert_eq!(state.flow.stage(), &Stage::Integration { attempt: 1 });

    world.another_mission_integrating(false);
    assert!(
        world.verify().is_err(),
        "free again, the launch is attempted"
    );
}

/// A service not declared shared is not a real provider: two missions may
/// use it at once.
#[test]
fn a_service_not_declared_shared_is_not_locked() {
    let world = World::shaped(1, integration());
    world.at_integration();
    world.another_mission_integrating(true);
    assert!(world.verify().is_err(), "the launch is attempted");
}

/// A battery that came back red while a campaign was rewriting the copy of
/// `HEAD` opens no volet, because it was judging a mutant.
///
/// Measured on `notes-4`, 2026-09-18, the first three-agent mission driven end
/// to end. `verify` started the campaign — `cargo mutants --in-place`, in the
/// clean copy of `HEAD` — and then, called again, played the whole gate set
/// against that same copy. The battery came back 101 on `warning: unused
/// variable: value` at `src/api.rs:160`, a function whose body cargo-mutants
/// had replaced and which uses its argument in the coder's own code. Every
/// play also reset the copy under the running campaign, `exec::run(On::Proof)`
/// refreshing it first.
///
/// The flow read gate 6 red and opened a volet, four times, until the volets
/// were spent: six runs, 59M tokens, and neither the integrator nor the
/// security agent ever ran. Nothing any agent did was wrong.
#[test]
fn a_battery_red_under_a_running_campaign_opens_no_volet() {
    use nunki::engine::{ExecOutput, fake::FakeEngine};

    let world = World::new(1);
    world.commit("src/new.rs", "pub fn two() -> u8 { 2 }\n", "L1");
    world.journal_names_head();
    std::fs::write(
        world.mission().join("PR.md"),
        "# What this changes\n\nL1, and nothing else.\n",
    )
    .unwrap();
    world.coder_finished_the_lot();

    let file = nunki::run::profile_path(&world.project, "one");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "services:\n  agent:\n    image: alpine:3.20\n").unwrap();
    // The container answers the way it did: the battery failed, on a mutant.
    //
    // A queue and not one answer, because `exec::run(On::Proof)` refreshes the
    // copy first and `battery` reports an exec that **errored** as unplayed
    // too. With a single 101 the gate came back unplayed for the wrong reason,
    // and this test passed with the guard taken out.
    let ok = ExecOutput {
        status: 0,
        stdout: String::new(),
        stderr: String::new(),
    };
    let engine: Arc<dyn nunki::engine::Engine> = Arc::new(FakeEngine::default().with_execs(vec![
        ok.clone(),
        ExecOutput {
            status: 101,
            stdout: String::new(),
            stderr: "error: test failed, to rerun pass `--lib`".into(),
        },
        ok,
    ]));

    nunki::mutants::write_running(
        &world.project.hq_root,
        "one",
        &nunki::mutants::Running {
            fingerprint: "a62d271".into(),
            head: git(&world.tree, &["rev-parse", "HEAD"]),
            started_at: "2026-09-18T20:56:38Z".into(),
            container: "64e0796ca1de".into(),
            pid: Some(2736),
            log: world.mission().join("mutants.log"),
            deadline_minutes: 45,
        },
    )
    .unwrap();

    let steps = verify::verify(&world.project, "m1", engine, "docker").unwrap();

    let report = gates_of(&steps);
    assert!(
        !matches!(report.verdict(), nunki::gate::Verdict::Red(_)),
        "the coder was sent back for a mutant it did not write: {report:?}"
    );
    // And unplayed for the campaign, not for something else that went wrong.
    let battery = report
        .outcomes
        .iter()
        .find(|o| o.gate == Gate::Battery)
        .expect("gate 6 is played");
    let nunki::gate::Decision::Unplayed(why) = &battery.decision else {
        panic!("the battery judged a copy a campaign is rewriting: {battery:?}");
    };
    assert!(why.contains("2026-09-18T20:56:38Z"), "{why}");
    // No volet, no attempt, no run spent. A later `verify`, once the campaign
    // has been read back, plays the battery on the copy it owns again.
    assert_eq!(world.state().flow.stage(), &Stage::Gates, "{steps:?}");
    assert_eq!(world.state().flow.volets(), 0);

    // And the turn ends by going to read the campaign, not by calling it a
    // wall. A wall stops `verify` before the campaign is read back, and the
    // gates that stood down for it then stand down for ever: measured on
    // `notes-4` on 2026-09-18, 146 turns of `verify`, every one of them
    // reporting a campaign in flight and none of them reading it.
    match steps.last() {
        Some(Step::CampaignOwed { role, .. }) => assert_eq!(*role, Role::Coder),
        other => panic!("waiting on the campaign became a wall: {other:?}"),
    }
}

/// Gate 7 with no campaign, and everything else played: the flow does not
/// stop for a verb a human would have typed. It says a campaign is owed, and
/// the monitor runs one.
///
/// A profile on disk and an engine that answers, so gate 6 is green and gate
/// 7 stands alone — the shape a real mission has, and the one a world with no
/// profile can never show: there, gate 6 is unplayable too and the report is
/// a wall, which is what `a_gate_nobody_could_play_stops_instead_of_opening_a_volet`
/// covers.
#[test]
fn a_gate_seven_with_no_campaign_asks_for_one_rather_than_stopping() {
    use nunki::engine::{ExecOutput, fake::FakeEngine};

    let world = World::new(1);
    world.commit("src/new.rs", "pub fn two() -> u8 { 2 }\n", "L1");
    world.journal_names_head();
    std::fs::write(
        world.mission().join("PR.md"),
        "# What this changes\n\nL1, and nothing else.\n",
    )
    .unwrap();
    world.coder_finished_the_lot();

    let file = nunki::run::profile_path(&world.project, "one");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "services:\n  agent:\n    image: alpine:3.20\n").unwrap();
    let engine: Arc<dyn nunki::engine::Engine> =
        Arc::new(FakeEngine::default().with_exec(ExecOutput {
            status: 0,
            stdout: String::new(),
            stderr: String::new(),
        }));

    let steps = verify::verify(&world.project, "m1", engine, "docker").unwrap();
    let report = gates_of(&steps);
    assert!(
        !matches!(report.verdict(), nunki::gate::Verdict::Red(_)),
        "nothing is the agent's to fix here: {report:?}"
    );
    assert_eq!(
        report
            .outcomes
            .iter()
            .find(|o| o.gate == Gate::Battery)
            .map(|o| &o.decision),
        Some(&nunki::gate::Decision::Passed),
        "the battery has to be green for gate 7 to stand alone: {report:?}"
    );
    match steps.last() {
        Some(Step::CampaignOwed { role, why }) => {
            assert_eq!(*role, Role::Coder);
            assert!(why.contains("no mutation campaign has run"), "{why}");
        }
        other => panic!("a campaign is owed, not a wall: {other:?}"),
    }

    // The flow has not moved, exactly as it does not for a wall: no volet,
    // no attempt, no run. What changed is who is expected to act next.
    assert_eq!(world.state().flow.stage(), &Stage::Gates, "{steps:?}");
    assert_eq!(world.state().flow.volets(), 0);
}
