//! `nunki exec` (SPEC 4.2): a command in the slot's container, on a clean copy
//! of `HEAD`.
//!
//! The live test is the only one that means anything here: what is being
//! claimed is that a proof replayed by `nunki` cannot see what an agent left
//! uncommitted, and only a real container with a real git can show that.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use nunki::exec::{self, On, PROOF_AT};
use nunki::project::{Config, Project};
use nunki::slot::Slot;

#[test]
fn a_slot_names_its_own_copy_and_mounts_it_where_the_spec_says() {
    assert_eq!(exec::proof_volume("one"), "nunki-one-proof");
    let volume = exec::volume("one");
    assert_eq!(volume.name, "nunki-one-proof");
    assert_eq!(volume.at, PathBuf::from(PROOF_AT));
    // Not the working tree, and that is the whole point of the verb.
    assert_ne!(PROOF_AT, nunki::run::TREE_AT);
}

#[test]
fn a_slot_with_no_profile_up_says_what_to_do_about_it() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let slot = Slot {
        name: "gone".into(),
        tree: dir.path().join("tree"),
    };
    let engine: Arc<dyn nunki::engine::Engine> =
        Arc::new(nunki::engine::fake::FakeEngine::default());
    let err = exec::run(&project, &slot, engine, &["true".into()], On::Proof).unwrap_err();
    let said = err.to_string();
    assert!(said.contains("nunki mission start"), "{said}");
    assert!(said.contains("nunki slot rebuild"), "{said}");
}

/// A campaign in flight owns the clean copy of `HEAD`, and this is the door
/// everything else reaches it through.
///
/// `mutation.sh` runs `cargo mutants --in-place` there for up to an hour,
/// while `exec::run(On::Proof)` opens with `git reset --hard` and `git clean`.
/// Whoever gets there second wrecks the other: the caller reads a mutant back
/// as the project's code, and the campaign loses the tree under it. Measured
/// on `notes-4`, 2026-09-18 — the battery came back 101 on a function whose
/// body cargo-mutants had replaced, and the flow sent the coder back for it
/// four times.
///
/// Gates 6 and 8 stand down on their own, one layer up. This is for the four
/// callers that do not: `nunki exec`, `nunki mission gates`, and whatever asks
/// next.
#[test]
fn nothing_touches_the_copy_a_campaign_is_rewriting() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let slot = Slot {
        name: "one".into(),
        tree: dir.path().join("tree"),
    };
    let profile = nunki::run::profile_path(&project, &slot.name);
    std::fs::create_dir_all(profile.parent().unwrap()).unwrap();
    std::fs::write(&profile, "services: {}\n").unwrap();
    nunki::mutants::write_running(
        &project.hq_root,
        &slot.name,
        &nunki::mutants::Running {
            fingerprint: "abc1234".into(),
            head: "def5678".into(),
            started_at: "2026-09-18T22:52:37Z".into(),
            container: "cafe1234".into(),
            pid: Some(41),
            log: dir.path().join("mutants.log"),
            deadline_minutes: 45,
        },
    )
    .unwrap();
    let engine = || -> Arc<dyn nunki::engine::Engine> {
        Arc::new(nunki::engine::fake::FakeEngine::default())
    };

    let err = exec::run(&project, &slot, engine(), &["true".into()], On::Proof).unwrap_err();
    let said = err.to_string();
    assert!(said.contains("2026-09-18T22:52:37Z"), "{said}");
    assert!(said.contains("reset --hard"), "{said}");

    // The refresh on its own too: a campaign launcher calls it directly.
    let err = exec::refresh(&project, &slot, engine()).unwrap_err();
    assert!(err.to_string().contains("2026-09-18T22:52:37Z"), "{err}");

    // The working tree is the agent's, and no campaign touches it.
    assert!(
        exec::run(&project, &slot, engine(), &["true".into()], On::Tree).is_ok(),
        "the tree was refused for a campaign that is not in it"
    );

    // And once the campaign is gone, the door opens again — it fails further
    // in, on a tree this fixture never made, which is the point: the refusal
    // is no longer the answer.
    nunki::mutants::forget_running(&project.hq_root, &slot.name).unwrap();
    let after = exec::refresh(&project, &slot, engine())
        .unwrap_err()
        .to_string();
    assert!(!after.contains("2026-09-18T22:52:37Z"), "{after}");
}

fn project(root: &Path) -> Project {
    Project::at(
        root.join("repo"),
        Config {
            root: None,
            harness: "claude-code".into(),
            forge: vec!["github.com".into()],
            stacks: vec!["rust".into()],
            protected_branches: vec!["main".into(), "dev".into()],
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

fn git(at: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(at)
        .args(["-c", "user.name=Exec Test", "-c", "user.email=exec@test"])
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

/// What `nunki exec` is for, proved the only way it can be.
///
/// A tree with one commit, an **uncommitted** file beside it, and an ignored
/// directory standing in for a build cache. The copy must hold the commit,
/// must not hold the uncommitted file — that is the defect this verb exists
/// to prevent, a battery replayed in a tree an agent has been living in —
/// and must keep the ignored directory across a refresh, because SPEC 4.4
/// wants the cache warmed once per slot and kept.
///
/// ```text
/// cargo test --test exec -- --ignored --nocapture
/// ```
#[test]
#[ignore = "lifts real containers; run by hand"]
fn live_a_proof_runs_on_the_commit_and_never_on_what_the_agent_left_behind() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let tree = dir.path().join("tree");
    std::fs::create_dir_all(&tree).unwrap();
    git(&tree, &["init", "-q", "-b", "work"]);
    std::fs::write(tree.join(".gitignore"), "cache/\n").unwrap();
    std::fs::write(tree.join("hello.txt"), "from the commit\n").unwrap();
    git(&tree, &["add", "-A"]);
    git(&tree, &["commit", "-q", "-m", "the lot"]);
    let head = git(&tree, &["rev-parse", "HEAD"]);
    // What an agent leaves in its tree and what a proof must never see.
    std::fs::write(tree.join("stray.txt"), "an uncommitted Makefile, morally\n").unwrap();

    let slot = Slot {
        name: "execlive".into(),
        tree: tree.clone(),
    };
    let file = nunki::run::profile_path(&project, &slot.name);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    // Alpine plus git: this test is about what the copy contains, not about
    // ownership — that one is settled by the mount point nunki's own image
    // layer creates, and measured there.
    //
    // Which is why `safe.directory` is set below. A real profile runs as the
    // host's own uid (`compose::generate` writes `user:`), so git in the
    // container and the owner of the mounted tree are the same person and it
    // never asks. This fixture runs as root, because it installs git at
    // start-up and `apk` needs to. On Linux git then refuses the tree —
    // "detected dubious ownership", measured on the ubuntu runner — while on
    // macOS the engine's file sharing hides the mismatch and it passes. The
    // fixture says out loud what the real profile gets by construction.
    let volume = exec::proof_volume(&slot.name);
    std::fs::write(
        &file,
        format!(
            "services:\n\
             \x20 agent:\n\
             \x20   image: alpine:3.20\n\
             \x20   volumes:\n\
             \x20     - {tree}:{tree_at}\n\
             \x20     - {volume}:{PROOF_AT}\n\
             \x20   tmpfs:\n\
             \x20     - /run/nunki\n\
             \x20   command: [\"sh\", \"-c\", \"apk add --no-cache git > /dev/null && \
             git config --global --add safe.directory '*' && sleep 600\"]\n\
             \x20   healthcheck:\n\
             \x20     test: [\"CMD-SHELL\", \"command -v git > /dev/null\"]\n\
             \x20     interval: 1s\n\
             \x20     timeout: 2s\n\
             \x20     retries: 60\n\
             \x20     start_period: 1s\n\
             volumes:\n\
             \x20 {volume}:\n",
            tree = tree.display(),
            tree_at = nunki::run::TREE_AT,
        ),
    )
    .unwrap();

    let engine: Arc<dyn nunki::engine::Engine> = Arc::new(nunki::engine::docker::Docker::real());
    let compose_project = nunki::compose::project_name(&project.session(), &slot.name).unwrap();
    let _ = engine.down(&file, &compose_project, true);
    engine.up(&file, &compose_project).unwrap();

    let say = |argv: &[&str], on: On| {
        exec::run(
            &project,
            &slot,
            engine.clone(),
            &argv.iter().map(|a| a.to_string()).collect::<Vec<_>>(),
            on,
        )
        .unwrap()
    };

    // The copy holds the commit.
    let out = say(&["cat", "hello.txt"], On::Proof);
    assert!(out.ok(), "{out:?}");
    assert_eq!(out.stdout.trim(), "from the commit");
    let at = say(&["git", "rev-parse", "HEAD"], On::Proof);
    assert_eq!(at.stdout.trim(), head, "the copy is at the slot's HEAD");

    // And not what the agent left uncommitted. This is the defect the verb
    // exists to prevent.
    let stray = say(&["ls", "stray.txt"], On::Proof);
    assert!(
        !stray.ok(),
        "an uncommitted file must not reach the copy: {stray:?}"
    );

    // The build cache survives a refresh; a stray left in the copy does not.
    let seeded = say(
        &[
            "sh",
            "-c",
            "mkdir -p cache && date > cache/warm && touch leftover",
        ],
        On::Proof,
    );
    assert!(seeded.ok(), "{seeded:?}");
    let kept = say(&["cat", "cache/warm"], On::Proof);
    assert!(
        kept.ok() && !kept.stdout.trim().is_empty(),
        "the ignored cache is warmed once per slot and kept (SPEC 4.4): {kept:?}"
    );
    let leftover = say(&["ls", "leftover"], On::Proof);
    assert!(
        !leftover.ok(),
        "an untracked leftover from a previous exec is cleaned: {leftover:?}"
    );

    // `--tree` really is the other tree, which is why it is not the default.
    let in_tree = say(&["ls", "stray.txt"], On::Tree);
    assert!(in_tree.ok(), "{in_tree:?}");

    // A command's own status comes back, so a red battery is red.
    let failed = say(&["sh", "-c", "exit 3"], On::Proof);
    assert_eq!(failed.status, 3);

    engine.down(&file, &compose_project, true).unwrap();
}
