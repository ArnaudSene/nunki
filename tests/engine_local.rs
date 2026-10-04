//! What the fake engine cannot see, played against real state.
//!
//! The suite is five hundred tests against [`FakeEngine`], which answers
//! every `exec` with the next canned output whatever the argv. So it has no
//! directory a `git reset --hard` could destroy, no process a `stop` could
//! kill, and no exit status anything could come from — and six orchestration
//! defects went through it untouched, every one of them an interaction
//! between a real command and real filesystem state.
//!
//! `LocalEngine` runs what it is given, here, with container paths rewritten
//! to temporary directories. It is not the `#[ignore]`d live tests and does
//! not replace them: those exercise a real container and are the only proof
//! that counts. This one runs **in CI**, on both runners, where those never do.

use std::path::{Path, PathBuf};
use std::process::Command;

use nunki::engine::Engine;
use nunki::engine::local::LocalEngine;
use nunki::exec::{self, On};
use nunki::project::{Config, Project};
use nunki::slot::Slot;

fn git(at: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(at)
        .args(["-c", "user.name=Local Test", "-c", "user.email=l@test"])
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

fn project(root: &Path) -> Project {
    Project::at(
        root.join("repo"),
        Config {
            root: None,
            harness: "claude-code".into(),
            forge: vec![],
            stacks: vec!["rust".into()],
            protected_branches: vec![],
            protected_paths: Default::default(),
            account: None,
            model: None,
            bounds: Default::default(),
            credentials: None,
            run: None,
            services_file: None,
            permission_mode: "auto".to_string(),
            rigor: None,
            mutation_threshold: 80,
            forge_protection: Default::default(),
        },
        root.join("nunki"),
    )
}

/// A slot, its clean copy, and an engine that really runs in them.
struct World {
    _dir: tempfile::TempDir,
    project: Project,
    slot: Slot,
    proof: PathBuf,
    engine: std::sync::Arc<LocalEngine>,
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let tree = dir.path().join("tree");
    std::fs::create_dir_all(tree.join("src")).unwrap();
    git(&tree, &["init", "-q", "-b", "dev"]);
    std::fs::write(
        tree.join("src/lib.rs"),
        "pub fn keep(n: u8) -> bool {\n    n > 3\n}\n",
    )
    .unwrap();
    git(&tree, &["add", "-A"]);
    git(&tree, &["commit", "-q", "-m", "the code as it stands"]);

    let proof = dir.path().join("proof");
    std::fs::create_dir_all(&proof).unwrap();
    let run = dir.path().join("run");
    std::fs::create_dir_all(&run).unwrap();

    let project = project(dir.path());
    let profile = nunki::run::profile_path(&project, "one");
    std::fs::create_dir_all(profile.parent().unwrap()).unwrap();
    std::fs::write(&profile, "services: {}\n").unwrap();

    let engine = std::sync::Arc::new(LocalEngine::new(&[
        (nunki::exec::PROOF_AT, proof.as_path()),
        (nunki::run::TREE_AT, tree.as_path()),
        (nunki::engine::spawn::RUN_DIR, run.as_path()),
    ]));
    engine.up(Path::new("profile"), "local").unwrap();

    World {
        project,
        slot: Slot {
            name: "one".into(),
            tree,
        },
        proof,
        engine,
        _dir: dir,
    }
}

/// The clean copy really is a copy, and a proof really is replayed on it.
///
/// The claim `nunki exec` rests on (SPEC 4.2): what it runs cannot see what
/// an agent left uncommitted. Against the fake, every test of this proved
/// that the argv contained `cd /work/proof` — not that the copy was clean,
/// nor that anything reset it.
#[test]
fn a_proof_is_replayed_on_a_copy_the_agent_never_touched() {
    let w = world();
    // What an agent left in its working tree, uncommitted.
    std::fs::write(
        w.slot.tree.join("src/lib.rs"),
        "pub fn keep(n: u8) -> bool {\n    true\n}\n",
    )
    .unwrap();

    let out = exec::run(
        &w.project,
        &w.slot,
        w.engine.clone(),
        &["sh".into(), "-c".into(), "cat src/lib.rs".into()],
        On::Proof,
    )
    .unwrap();

    assert!(out.ok(), "{out:?}");
    assert!(
        out.stdout.contains("n > 3"),
        "the proof saw the working tree: {}",
        out.stdout
    );
    assert!(!out.stdout.contains("true"), "{}", out.stdout);
}

/// And the refresh really destroys what is in the copy — which is why a
/// campaign rewriting it in place cannot survive one.
///
/// This is the half no fake can hold: `git reset --hard` and `git clean`
/// against a directory that exists. Without it, the refusal proved in the
/// test below guards nothing that was ever shown to need guarding.
#[test]
fn a_refresh_really_resets_the_copy() {
    let w = world();
    exec::refresh(&w.project, &w.slot, w.engine.clone()).unwrap();
    // A mutant, as `cargo mutants --in-place` leaves one.
    let mutated = w.proof.join("src/lib.rs");
    std::fs::write(&mutated, "pub fn keep(_n: u8) -> bool {\n    true\n}\n").unwrap();
    std::fs::write(w.proof.join("leftover.txt"), "and something untracked\n").unwrap();

    exec::refresh(&w.project, &w.slot, w.engine.clone()).unwrap();

    assert!(
        std::fs::read_to_string(&mutated).unwrap().contains("n > 3"),
        "the mutant survived a refresh"
    );
    assert!(
        !w.proof.join("leftover.txt").exists(),
        "`git clean` left an untracked file behind"
    );
}

/// So a campaign in flight is refused the copy, and the mutant it is working
/// on is still there afterwards.
///
/// Both halves matter and only one of them was ever tested: that the refusal
/// happens, and that what it protects would really have been destroyed. The
/// second is what this engine exists for.
#[test]
fn a_campaign_in_flight_keeps_the_copy_it_is_rewriting() {
    let w = world();
    exec::refresh(&w.project, &w.slot, w.engine.clone()).unwrap();
    let mutated = w.proof.join("src/lib.rs");
    std::fs::write(&mutated, "pub fn keep(_n: u8) -> bool {\n    true\n}\n").unwrap();

    nunki::mutants::write_running(
        &w.project.hq_root,
        &w.slot.name,
        &nunki::mutants::Running {
            fingerprint: "abc1234".into(),
            head: git(&w.slot.tree, &["rev-parse", "HEAD"]),
            started_at: "2026-09-19T12:00:00Z".into(),
            container: nunki::engine::local::CONTAINER.into(),
            pid: Some(std::process::id()),
            log: w._dir.path().join("mutants.log"),
            deadline_minutes: u32::MAX,
        },
    )
    .unwrap();

    let answer = exec::run(
        &w.project,
        &w.slot,
        w.engine.clone(),
        &["sh".into(), "-c".into(), "cat src/lib.rs".into()],
        On::Proof,
    );

    // What it protects, asserted first: with the refusal taken out this is
    // the line that goes red, and it says why the refusal is there at all.
    assert!(
        std::fs::read_to_string(&mutated).unwrap().contains("true"),
        "the campaign lost the tree it was mutating: {answer:?}"
    );
    let refused = answer.expect_err("it was let in");
    assert!(
        refused.to_string().contains("2026-09-19T12:00:00Z"),
        "{refused}"
    );
}

/// A `stop` really ends what the engine started, which is why a launch kills
/// a campaign and why that had to be said rather than left silent.
#[test]
fn a_stop_really_ends_a_detached_process() {
    let w = world();
    let spec = w.engine.detached_command(
        Path::new("profile"),
        "local",
        "agent",
        &["-w".into(), nunki::exec::PROOF_AT.into()],
        &["sh".into(), "-c".into(), "sleep 120".into()],
    );
    let log = w._dir.path().join("detached.log");
    std::fs::create_dir_all(&w.proof).unwrap();
    let pid = nunki::engine::local::detach(&w.engine, &spec, &log).unwrap();

    assert_eq!(w.engine.running(), vec![pid], "it never started");

    w.engine
        .stop(Path::new("profile"), "local", &["agent"])
        .unwrap();

    // Signalled, then reaped: the kill is asynchronous, so the check waits
    // for the state it asserts rather than for a fixed time.
    for _ in 0..100 {
        if w.engine.running().is_empty() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("the process outlived the container it was in");
}

/// A project carrying `rust` at the root and `next` in `frontend/`, each with
/// its fragment's scripts where the image mounts them, in a world whose tree
/// holds both (SPEC 4.2, "plusieurs stacks").
struct Several {
    w: World,
    project: Project,
    engine: std::sync::Arc<LocalEngine>,
}

fn several(scripts: &[(&str, &str, &str)]) -> Several {
    let w = world();
    std::fs::create_dir_all(w.slot.tree.join("frontend/app")).unwrap();
    std::fs::write(w.slot.tree.join("frontend/package.json"), "{}\n").unwrap();
    std::fs::write(w.slot.tree.join("frontend/app/page.ts"), "export {}\n").unwrap();
    git(&w.slot.tree, &["add", "-A"]);
    git(&w.slot.tree, &["commit", "-q", "-m", "a frontend"]);

    let mut project = w.project.clone();
    project.config.stacks = vec![
        "rust".into(),
        nunki::project::Stack::new("next", "frontend").unwrap(),
    ];
    for (stack, name, body) in scripts {
        let dir = project.fragment(stack);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(name);
        std::fs::write(&file, body).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let run = w.proof.with_file_name("run");
    let engine = std::sync::Arc::new(LocalEngine::new(&[
        (nunki::exec::PROOF_AT, w.proof.as_path()),
        (nunki::run::TREE_AT, w.slot.tree.as_path()),
        (nunki::engine::spawn::RUN_DIR, run.as_path()),
        (nunki::run::STACK_AT, project.fragment("rust").as_path()),
        ("/work/stack-next", project.fragment("next").as_path()),
    ]));
    engine.up(Path::new("profile"), "local").unwrap();
    Several { w, project, engine }
}

fn battery_of(s: &Several) -> nunki::gate::Decision {
    decision_of(s, nunki::gate::Gate::Battery)
}

fn decision_of(s: &Several, gate: nunki::gate::Gate) -> nunki::gate::Decision {
    let header = nunki::mission::Header {
        branch: "dev".to_string(),
        base: "dev".to_string(),
        lots: vec![],
        integration: nunki::mission::Integration::None {
            reason: "none".to_string(),
        },
        security: nunki::mission::Security::Gates,
        rigor: Default::default(),
        arbiter: None,
        run: None,
        account: None,
        model: None,
        bounds: Default::default(),
    };
    let dir = s.w.proof.with_file_name("mission");
    std::fs::create_dir_all(&dir).unwrap();
    // What gates 3 and 5 read. Their verdicts do not matter here; that they
    // can be read does, or the report is never made.
    std::fs::write(dir.join("JOURNAL.md"), "# Journal\n").unwrap();
    std::fs::write(dir.join("PR.md"), "# PR\n").unwrap();
    let report = nunki::gate::at_verification(
        &nunki::gate::Subject {
            role: nunki::harness::Role::Coder,
            tree: &s.w.slot.tree,
            journal: &dir.join("JOURNAL.md"),
            pr: &dir.join("PR.md"),
            verdict: &dir.join("VERDICT.json"),
            mission_dir: &dir,
            header: &header,
            protected_branches: &[],
            protected_paths: &Default::default(),
            coder_head: None,
        },
        &nunki::gate::Verification {
            project: &s.project,
            slot: &s.w.slot,
            engine: s.engine.clone(),
            stack: "rust",
        },
    )
    .unwrap();
    report
        .outcomes
        .into_iter()
        .find(|o| o.gate == gate)
        .expect("the gate is reported")
        .decision
}

/// Gate 6 plays every stack's battery, each in its own directory of the
/// copy, and says which stack said what. The next battery fails with 9 if it
/// is not run from `frontend/` — so a wrong directory reads as 9, not as the
/// 3 it owes.
#[test]
fn every_stack_plays_its_battery_in_its_own_directory() {
    let s = several(&[
        (
            "rust",
            "prepush.sh",
            "#!/bin/sh\n[ -f src/lib.rs ] || exit 9\nexit 0\n",
        ),
        (
            "next",
            "prepush.sh",
            "#!/bin/sh\n[ -f package.json ] || exit 9\necho 'a spec is red' >&2\nexit 3\n",
        ),
    ]);
    match battery_of(&s) {
        nunki::gate::Decision::Failed(why) => {
            assert!(why.contains("[next in frontend/]"), "{why}");
            assert!(why.contains("came back 3"), "{why}");
            assert!(why.contains("a spec is red"), "{why}");
            assert!(!why.contains("[rust]"), "the rust battery passed: {why}");
        }
        other => panic!("{other:?}"),
    }

    // Green on both is green.
    std::fs::write(
        s.project.fragment("next").join("prepush.sh"),
        "#!/bin/sh\n[ -f package.json ] || exit 9\nexit 0\n",
    )
    .unwrap();
    assert_eq!(battery_of(&s), nunki::gate::Decision::Passed);
}

/// A stack whose directory the copy does not hold is red, and says so,
/// rather than running its battery from wherever it happened to be.
#[test]
fn a_stack_whose_directory_is_gone_is_red() {
    let s = several(&[
        ("rust", "prepush.sh", "#!/bin/sh\nexit 0\n"),
        ("next", "prepush.sh", "#!/bin/sh\nexit 0\n"),
    ]);
    git(&s.w.slot.tree, &["rm", "-rq", "frontend"]);
    git(&s.w.slot.tree, &["commit", "-q", "-m", "no frontend"]);
    match battery_of(&s) {
        nunki::gate::Decision::Failed(why) => {
            assert!(why.contains("frontend/ is not in the clean copy"), "{why}")
        }
        other => panic!("{other:?}"),
    }
}

/// The campaign over two stacks speaks for the project: each stack's
/// `mutation.sh` runs in its directory on its own touched paths, relative to
/// it; ids carry the stack's name and files the stack's directory; and the
/// last line says the campaign finished only when both did.
#[test]
fn a_campaign_over_several_stacks_speaks_for_the_project() {
    let rust = "#!/bin/sh\n[ -f src/lib.rs ] || exit 9\n\
                [ \"$2\" = src/lib.rs ] || exit 8\n\
                echo 'progress, not a result'\n\
                printf '%s\\n' '{\"id\":\"7\",\"file\":\"src/lib.rs\",\"line\":2,\"description\":\"replace > with >=\"}'\n\
                printf '%s\\n' '{\"campaign\":\"done\"}'\n";
    let next_done = "#!/bin/sh\n[ -f package.json ] || exit 9\n\
                     [ \"$2\" = app/page.ts ] || exit 8\n\
                     printf '%s\\n' '{\"id\":\"7\",\"file\":\"app/page.ts\",\"line\":1,\"description\":\"remove export\"}'\n\
                     printf '%s\\n' '{\"campaign\":\"done\"}'\n";
    let s = several(&[
        ("rust", "mutation.sh", rust),
        ("next", "mutation.sh", next_done),
    ]);
    let judged = nunki::run::judged(&s.project, "rust");
    let touched = vec!["src/lib.rs".to_string(), "frontend/app/page.ts".to_string()];
    let play = |s: &Several| {
        let script = nunki::mutants::several_campaigns(&judged, "abc1234", &touched);
        exec::run(
            &s.project,
            &s.w.slot,
            s.engine.clone(),
            &["sh".into(), "-c".into(), script],
            On::Proof,
        )
        .unwrap()
    };

    let out = play(&s);
    let survivors = nunki::mutants::parse(&out.stdout);
    let ids: Vec<(&str, &str)> = survivors
        .iter()
        .map(|m| (m.id.as_str(), m.file.as_str()))
        .collect();
    assert_eq!(
        ids,
        [("rust:7", "src/lib.rs"), ("next:7", "frontend/app/page.ts")],
        "{}",
        out.stdout
    );
    assert_eq!(
        out.stdout
            .lines()
            .filter(|l| l.contains("\"campaign\""))
            .count(),
        1,
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.trim_end().ends_with("{\"campaign\":\"done\"}"),
        "{}",
        out.stdout
    );

    // One stack that did not finish, and the campaign measured nothing.
    std::fs::write(
        s.project.fragment("next").join("mutation.sh"),
        "#!/bin/sh\nexit 1\n",
    )
    .unwrap();
    let out = play(&s);
    assert!(!out.stdout.contains("\"campaign\""), "{}", out.stdout);
}

/// Gate 8 runs every stack's `security.sh` in its directory, handed its own
/// advisory database: each script here answers 69, "I could not look",
/// unless it is where it should be and reads what it should.
#[test]
fn every_stack_audits_itself_against_its_own_database() {
    let s = several(&[
        (
            "rust",
            "security.sh",
            "#!/bin/sh\n[ -f src/lib.rs ] && [ \"$2\" = /nunki/advisories ] || exit 69\nexit 0\n",
        ),
        (
            "next",
            "security.sh",
            "#!/bin/sh\n[ -f package.json ] && [ \"$2\" = /nunki/advisories-next ] || exit 69\n\
             exit 0\n",
        ),
    ]);
    assert_eq!(
        decision_of(&s, nunki::gate::Gate::MechanicalSecurity),
        nunki::gate::Decision::Passed
    );
}
