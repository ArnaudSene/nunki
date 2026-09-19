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
