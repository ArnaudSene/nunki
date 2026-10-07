//! The mutation campaign and gate 7 (SPEC 4.4).
//!
//! The campaign is deliberately not run against `cargo-mutants` here: what
//! has to be right is the fingerprint, the three outcomes and the staleness
//! rule, and a stub campaign that prints the tool's line shape exercises all
//! of them. Which tool the stack declares is the stack's business.

use std::path::{Path, PathBuf};
use std::process::Command;

use nunki::mutants::{self, Campaign, Survivor, Triage};

fn git(at: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(at)
        .args(["-c", "user.name=Mutants Test", "-c", "user.email=m@test"])
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

fn repo(dir: &Path) -> PathBuf {
    let tree = dir.join("tree");
    std::fs::create_dir_all(&tree).unwrap();
    git(&tree, &["init", "-q", "-b", "dev"]);
    write(&tree, "src/lib.rs", "pub fn one() -> u8 { 1 }\n");
    git(&tree, &["add", "-A"]);
    git(&tree, &["commit", "-q", "-m", "base"]);
    git(&tree, &["checkout", "-q", "-b", "mission/x"]);
    tree
}

/// The fingerprint is over **content**, not over `HEAD`. A commit that
/// rewrites history without changing a byte of the touched files must not
/// cost an hour of campaign — SPEC section 7 counts that hour.
#[test]
fn the_fingerprint_follows_the_content_and_not_the_commit() {
    let dir = tempfile::tempdir().unwrap();
    let tree = repo(dir.path());
    write(&tree, "src/lib.rs", "pub fn one() -> u8 { 2 }\n");
    git(&tree, &["add", "-A"]);
    git(&tree, &["commit", "-q", "-m", "L1"]);
    let touched = vec!["src/lib.rs".to_string()];
    let before = mutants::fingerprint(&tree, &touched).unwrap();

    // A new commit, the same bytes in the touched file.
    write(&tree, "README.md", "unrelated\n");
    git(&tree, &["add", "-A"]);
    git(&tree, &["commit", "-q", "-m", "something else"]);
    assert_ne!(git(&tree, &["rev-parse", "HEAD"]), "");
    assert_eq!(
        mutants::fingerprint(&tree, &touched).unwrap(),
        before,
        "HEAD moved and the touched file did not: the campaign must not replay"
    );

    // The bytes change: so does the fingerprint.
    write(&tree, "src/lib.rs", "pub fn one() -> u8 { 3 }\n");
    git(&tree, &["add", "-A"]);
    git(&tree, &["commit", "-q", "-m", "L2"]);
    assert_ne!(mutants::fingerprint(&tree, &touched).unwrap(), before);
}

#[test]
fn the_order_the_paths_arrive_in_does_not_change_the_fingerprint() {
    let dir = tempfile::tempdir().unwrap();
    let tree = repo(dir.path());
    write(&tree, "src/a.rs", "pub fn a() {}\n");
    write(&tree, "src/b.rs", "pub fn b() {}\n");
    git(&tree, &["add", "-A"]);
    git(&tree, &["commit", "-q", "-m", "two files"]);
    let one = mutants::fingerprint(&tree, &["src/a.rs".into(), "src/b.rs".into()]).unwrap();
    let other = mutants::fingerprint(&tree, &["src/b.rs".into(), "src/a.rs".into()]).unwrap();
    assert_eq!(one, other);
    // And a path repeated is a path, not two.
    let twice = mutants::fingerprint(
        &tree,
        &["src/a.rs".into(), "src/b.rs".into(), "src/a.rs".into()],
    )
    .unwrap();
    assert_eq!(one, twice);
}

/// A campaign's stdout carries the tool's own chatter as well as its
/// survivors. One malformed line is not a lost campaign.
#[test]
fn the_survivors_are_read_out_of_whatever_else_the_tool_printed() {
    let text = "Found 12 mutants to test\n\
        {\"id\":\"a\",\"file\":\"src/lib.rs\",\"line\":3,\"description\":\"replace one with 0\"}\n\
        ok  src/lib.rs:9 caught\n\
        {\"id\":\"b\",\"file\":\"src/gate.rs\",\"line\":7}\n\
        \n";
    let survivors = mutants::parse(text);
    assert_eq!(survivors.len(), 2);
    assert_eq!(survivors[0].file, "src/lib.rs");
    assert_eq!(survivors[1].line, 7);
    // Nobody has answered for either of them yet.
    assert!(survivors.iter().all(|s| s.outcome.is_none()));
}

/// A campaign that stopped is read back as lost, not as a green gate.
///
/// The half that matters: `mutants::campaign` finding the process gone. It
/// used to parse whatever the log held and write a `Campaign` from it, so a
/// killed campaign became "no survivor" and gate 7 passed. Now nothing is
/// written, the in-flight record is cleared so no gate stands down for ever,
/// and the next `verify` runs a real one.
#[test]
fn a_campaign_found_stopped_writes_no_result() {
    use nunki::engine::{ExecOutput, fake::FakeEngine};

    let dir = tempfile::tempdir().unwrap();
    let (project, slot) = context(dir.path());
    let tree = slot.tree.clone();
    let profile = nunki::run::profile_path(&project, &slot.name);
    std::fs::create_dir_all(profile.parent().unwrap()).unwrap();
    std::fs::write(&profile, "services: {}\n").unwrap();

    let mission = dir.path().join("mission");
    std::fs::create_dir_all(&mission).unwrap();
    let log = mission.join("mutants.log");
    // What it printed before it died: a survivor, and no line saying it
    // reached the end.
    std::fs::write(
        &log,
        "{\"id\":\"a\",\"file\":\"src/lib.rs\",\"line\":3,\"description\":\"replace one\"}\n",
    )
    .unwrap();
    mutants::write_running(
        &project.hq_root,
        &slot.name,
        &mutants::Running {
            chain: Default::default(),
            fingerprint: "abc1234".into(),
            head: git(&tree, &["rev-parse", "HEAD"]),
            started_at: "2026-09-19T02:00:00Z".into(),
            container: "cafe1234".into(),
            pid: Some(41),
            log: log.clone(),
            deadline_minutes: 45,
        },
    )
    .unwrap();

    // The container is up and the process is not — the shape a kill leaves.
    let engine: std::sync::Arc<dyn nunki::engine::Engine> = std::sync::Arc::new(
        FakeEngine::default()
            .with_liveness("cafe1234", nunki::engine::Liveness::Running)
            .with_exec(ExecOutput {
                status: 0,
                stdout: "nunki-run-ended\n".into(),
                stderr: String::new(),
            }),
    );

    let progress = mutants::campaign(
        &project,
        &slot,
        engine,
        &mission,
        "rust",
        &mutants::Asked {
            base: "dev",
            deadline_minutes: 45,
            replay: mutants::Replay::WhenChanged,
            rigor: nunki::mission::Rigor::Critical,
            threshold: 80,
        },
    )
    .unwrap();

    let mutants::Progress::Lost(why) = progress else {
        panic!("a campaign that never said it finished was read as one: {progress:?}");
    };
    assert!(why.contains("without saying it had finished"), "{why}");
    // Nothing recorded, so gate 7 keeps asking rather than passing.
    assert_eq!(mutants::read(&mission).unwrap(), None);
    // And the in-flight record is gone, so no gate stands down for it.
    assert!(
        mutants::read_running(&project.hq_root, &slot.name)
            .unwrap()
            .is_none()
    );
}

/// A campaign that stopped **and said why** on stderr is the branch's defect,
/// not a machine's, and it is told apart from one that vanished.
///
/// When a guard fails inside `mutants/` and the stderr names it, a monitor
/// that stops on "lost" leaves the volet gate 7 would send waiting for a
/// human to type `nunki verify`.
#[test]
fn a_campaign_that_stopped_and_said_why_goes_back_to_the_branch() {
    use nunki::engine::{ExecOutput, fake::FakeEngine};

    let dir = tempfile::tempdir().unwrap();
    let (project, slot) = context(dir.path());
    let tree = slot.tree.clone();
    let profile = nunki::run::profile_path(&project, &slot.name);
    std::fs::create_dir_all(profile.parent().unwrap()).unwrap();
    std::fs::write(&profile, "services: {}\n").unwrap();

    let mission = dir.path().join("mission");
    std::fs::create_dir_all(&mission).unwrap();
    let log = mission.join("mutants.log");
    // What it printed before it died: a survivor, and no line saying it
    // reached the end.
    std::fs::write(
        &log,
        "{\"id\":\"a\",\"file\":\"src/lib.rs\",\"line\":3,\"description\":\"replace one\"}\n",
    )
    .unwrap();
    // And why, on stderr: what a guard failing inside `mutants/` leaves.
    std::fs::write(
        log.with_extension("err"),
        "FAILED tests/test_register_delivery_boundaries.py::test_br16\n\
         nunki: the campaign could not run, so no mutant was tested\n",
    )
    .unwrap();
    mutants::write_running(
        &project.hq_root,
        &slot.name,
        &mutants::Running {
            chain: Default::default(),
            fingerprint: "abc1234".into(),
            head: git(&tree, &["rev-parse", "HEAD"]),
            started_at: "2026-09-19T02:00:00Z".into(),
            container: "cafe1234".into(),
            pid: Some(41),
            log: log.clone(),
            deadline_minutes: 45,
        },
    )
    .unwrap();

    // The container is up and the process is not — the shape a kill leaves.
    let engine: std::sync::Arc<dyn nunki::engine::Engine> = std::sync::Arc::new(
        FakeEngine::default()
            .with_liveness("cafe1234", nunki::engine::Liveness::Running)
            .with_exec(ExecOutput {
                status: 0,
                stdout: "nunki-run-ended\n".into(),
                stderr: String::new(),
            }),
    );

    let progress = mutants::campaign(
        &project,
        &slot,
        engine,
        &mission,
        "rust",
        &mutants::Asked {
            base: "dev",
            deadline_minutes: 45,
            replay: mutants::Replay::WhenChanged,
            rigor: nunki::mission::Rigor::Critical,
            threshold: 80,
        },
    )
    .unwrap();

    // Not lost: it said why, gate 7 will read that as red, and the monitor
    // goes on to the volet rather than stopping for a human.
    let mutants::Progress::CouldNotRun(why) = progress else {
        panic!("a campaign that stopped and said why was read as {progress:?}");
    };
    assert!(why.contains("without saying it had finished"), "{why}");
    // Nothing recorded, so gate 7 keeps asking rather than passing.
    assert_eq!(mutants::read(&mission).unwrap(), None);
    // And the in-flight record is gone, so no gate stands down for it.
    assert!(
        mutants::read_running(&project.hq_root, &slot.name)
            .unwrap()
            .is_none()
    );
}

/// A project and a slot a campaign can be launched in, on a real repository.
fn context(dir: &Path) -> (nunki::project::Project, nunki::slot::Slot) {
    let tree = repo(dir);
    let project = nunki::project::Project::at(
        tree.clone(),
        nunki::project::Config {
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
        dir.join("nunki"),
    );
    let slot = nunki::slot::Slot {
        name: "one".into(),
        tree,
    };
    (project, slot)
}

/// What a launch does with the campaign it is about to kill.
///
/// `run::launch` recreates the agent's container, and a campaign is a
/// detached process inside it: it dies either way. Three states, three
/// answers, and only one of them is a killing.
#[test]
fn a_launch_ends_the_campaign_it_would_have_killed_anyway() {
    use nunki::engine::{ExecOutput, Liveness, fake::FakeEngine};

    let dir = tempfile::tempdir().unwrap();
    let (project, slot) = context(dir.path());
    let profile = nunki::run::profile_path(&project, &slot.name);
    std::fs::create_dir_all(profile.parent().unwrap()).unwrap();
    std::fs::write(&profile, "services: {}\n").unwrap();
    let mission = dir.path().join("mission");
    std::fs::create_dir_all(&mission).unwrap();
    let log = mission.join("mutants.log");
    std::fs::write(&log, "").unwrap();

    let file = |what: &str| -> std::sync::Arc<dyn nunki::engine::Engine> {
        mutants::write_running(
            &project.hq_root,
            &slot.name,
            &mutants::Running {
                chain: Default::default(),
                fingerprint: "abc1234".into(),
                head: "def5678".into(),
                started_at: "2026-09-18T22:52:37Z".into(),
                container: "cafe1234".into(),
                pid: Some(41),
                log: log.clone(),
                // Never overrun, so that "still running" stays the answer
                // however long after `started_at` this test is played: the
                // deadline has its own arm, and it is not what this is about.
                deadline_minutes: u32::MAX,
            },
        )
        .unwrap();
        std::sync::Arc::new(
            FakeEngine::default()
                .with_liveness("cafe1234", Liveness::Running)
                .with_exec(ExecOutput {
                    status: 0,
                    stdout: format!("{what}\n"),
                    stderr: String::new(),
                }),
        )
    };

    // Nothing filed: nothing to end, and nothing said.
    let engine: std::sync::Arc<dyn nunki::engine::Engine> =
        std::sync::Arc::new(FakeEngine::default());
    assert_eq!(
        nunki::run::end_campaign_before_switching(&project, &slot, engine, &mission).unwrap(),
        None
    );

    // Filed, and already over: read back, cleared, and nothing to end. This
    // is what the check mostly does — a record outlives its campaign whenever
    // something stopped before reading it.
    assert_eq!(
        nunki::run::end_campaign_before_switching(
            &project,
            &slot,
            file("nunki-run-ended"),
            &mission
        )
        .unwrap(),
        None
    );
    assert!(
        mutants::read_running(&project.hq_root, &slot.name)
            .unwrap()
            .is_none()
    );

    // Genuinely running: ended, and said since when.
    assert_eq!(
        nunki::run::end_campaign_before_switching(
            &project,
            &slot,
            file("nunki-run-running"),
            &mission
        )
        .unwrap(),
        Some("2026-09-18T22:52:37Z".to_string())
    );
    assert!(
        mutants::read_running(&project.hq_root, &slot.name)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        mutants::read(&mission).unwrap(),
        None,
        "a campaign cut short was recorded as a measurement"
    );
}

/// `end` stops the campaign and records nothing.
///
/// What `run::launch` does when it finds one genuinely running: the switch it
/// performs recreates the container the campaign lives in, so the campaign
/// dies either way — and a campaign cut short measured nothing, so gate 7
/// must ask for another rather than read this one.
#[test]
fn ending_a_campaign_records_no_result() {
    use nunki::engine::fake::FakeEngine;

    let dir = tempfile::tempdir().unwrap();
    let (project, slot) = context(dir.path());
    let profile = nunki::run::profile_path(&project, &slot.name);
    std::fs::create_dir_all(profile.parent().unwrap()).unwrap();
    std::fs::write(&profile, "services: {}\n").unwrap();
    let mission = dir.path().join("mission");
    std::fs::create_dir_all(&mission).unwrap();
    let log = mission.join("mutants.log");
    // It had found something, and said nothing about being finished.
    std::fs::write(
        &log,
        "{\"id\":\"a\",\"file\":\"src/lib.rs\",\"line\":3,\"description\":\"replace one\"}\n",
    )
    .unwrap();
    mutants::write_running(
        &project.hq_root,
        &slot.name,
        &mutants::Running {
            chain: Default::default(),
            fingerprint: "abc1234".into(),
            head: "def5678".into(),
            started_at: "2026-09-18T22:52:37Z".into(),
            container: "cafe1234".into(),
            pid: Some(41),
            log,
            deadline_minutes: 45,
        },
    )
    .unwrap();
    let engine: std::sync::Arc<dyn nunki::engine::Engine> =
        std::sync::Arc::new(FakeEngine::default());

    mutants::end(&project, &slot, engine).unwrap();

    assert!(
        mutants::read_running(&project.hq_root, &slot.name)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        mutants::read(&mission).unwrap(),
        None,
        "a campaign cut short was recorded as a measurement"
    );
}

/// A campaign is only a measurement once it has said it finished.
///
/// Three failures leave the same evidence — a log that parses to zero
/// survivors: a campaign killed halfway, one whose container went away, and
/// one that never compiled. `nunki` read all three as "nothing survived" and
/// gate 7 went green on a measurement nobody made, which is the one thing
/// SPEC 4.4 forbids it: "une porte 7 qui ne peut pas dire « je n'ai pas pu
/// mesurer » ment".
///
/// The exit status cannot carry this. The spawner runs `echo $$ > pid; exec
/// "$@"` so that the pid it publishes is the campaign's own, and a shell that
/// has been replaced cannot write `$?`; dropping the `exec` would take the
/// identity check for every harness run with it.
#[test]
fn a_log_without_the_last_line_is_not_a_finished_campaign() {
    // What a campaign that ran to the end prints, survivors or not.
    assert!(mutants::completed("{\"campaign\":\"done\"}\n"));
    assert!(mutants::completed(
        "Found 12 mutants to test\n\
         {\"id\":\"a\",\"file\":\"src/lib.rs\",\"line\":3,\"description\":\"replace one\"}\n\
         {\"campaign\":\"done\"}\n"
    ));

    // And the three that stopped. Each parses to a campaign with no survivor,
    // which is exactly what a green gate 7 would have been read from.
    for stopped in [
        "",
        "Found 12 mutants to test\n",
        "{\"id\":\"a\",\"file\":\"src/lib.rs\",\"line\":3,\"description\":\"replace one\"}\n",
        "{\"campaign\":\"started\"}\n",
    ] {
        assert!(
            !mutants::completed(stopped),
            "a campaign that never said it finished read as one: {stopped:?}"
        );
    }
}

#[test]
fn a_campaign_round_trips_through_the_mission_folder() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(mutants::read(dir.path()).unwrap(), None);
    let campaign = Campaign {
        chain: Default::default(),
        fingerprint: "abc1234".into(),
        head: "def5678".into(),
        tried: None,
        date: "2026-09-10T12:00:00Z".into(),
        survivors: vec![Survivor {
            found_on: None,
            id: "src/lib.rs:3".into(),
            file: "src/lib.rs".into(),
            line: 3,
            end_line: None,
            description: "replace one with 0".into(),
            outcome: Some(Triage::Equivalent {
                why: "the branch is unreachable from any caller".into(),
                carried_from: None,
            }),
            refused: None,
        }],
    };
    mutants::write(dir.path(), &campaign).unwrap();
    assert_eq!(mutants::read(dir.path()).unwrap(), Some(campaign));
}

#[test]
fn only_two_of_the_three_outcomes_rest_on_a_test() {
    assert_eq!(
        Triage::Killed {
            test: "a_reverted_commit_is_still_caught".into()
        }
        .test(),
        Some("a_reverted_commit_is_still_caught")
    );
    assert_eq!(
        Triage::Bug {
            test: "the_known_hole".into()
        }
        .test(),
        Some("the_known_hole")
    );
    // The one no gate can check, and SPEC gives its counter-check to the HQ.
    assert_eq!(
        Triage::Equivalent {
            why: "x".into(),
            carried_from: None,
        }
        .test(),
        None
    );
}

/// A campaign, launched detached in a real container and watched to its end,
/// then read back — which is the whole shape SPEC 4.4 gives gate 7.
///
/// The campaign here is a stub that prints the tool's line shape. What has to
/// be right is the launching, the watching, the file it produces and the
/// staleness rule; which tool a stack declares is the stack's business, and
/// `cargo-mutants` is not in this image.
///
/// ```text
/// cargo test --test mutants -- --ignored --nocapture
/// ```
#[test]
#[ignore = "lifts real containers; run by hand"]
fn live_a_campaign_is_launched_watched_and_read_back() {
    use nunki::mutants::Progress;

    let dir = tempfile::tempdir().unwrap();
    let tree = repo(dir.path());
    // In the project's home, where the stack lives, and mounted read-only
    // into the container below: never in the tree the campaign runs on.
    let script = dir.path().join("nunki/stacks/rust").join(mutants::SCRIPT);
    std::fs::create_dir_all(script.parent().unwrap()).unwrap();
    std::fs::write(
        &script,
        "#!/bin/sh\n\
         # A stand-in campaign: the shape nunki reads, without the tool.\n\
         echo \"campaign $1 on $# path(s)\" >&2\n\
         sleep 2\n\
         echo \"{\\\"id\\\":\\\"src/lib.rs:1\\\",\\\"file\\\":\\\"src/lib.rs\\\",\\\"line\\\":1,\
         \\\"description\\\":\\\"replace one with 0 since $NUNKI_BASE\\\"}\"\n\
         echo 'not a survivor, just chatter'\n\
         echo '{\"id\":\"src/lib.rs:1b\",\"file\":\"src/lib.rs\",\"line\":1,\
         \"description\":\"replace one with 255\"}'\n\
         echo '{\"campaign\":\"done\"}'\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    // Changed on the branch, so it is a touched file and the campaign has
    // something to run on.
    write(&tree, "src/lib.rs", "pub fn one() -> u8 { 2 }\n");
    git(&tree, &["add", "-A"]);
    git(&tree, &["commit", "-q", "-m", "L1"]);

    let project = nunki::project::Project::at(
        dir.path().join("repo"),
        nunki::project::Config {
            root: None,
            harness: "claude-code".into(),
            forge: vec![],
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
            rigor: None,
            mutation_threshold: 80,
            forge_protection: Default::default(),
        },
        dir.path().join("nunki"),
    );
    let slot = nunki::slot::Slot {
        name: "mutlive".into(),
        tree: tree.clone(),
    };
    let volume = nunki::exec::proof_volume(&slot.name);
    let file = nunki::run::profile_path(&project, &slot.name);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let script_at = format!("{}/{}", nunki::run::STACK_AT, mutants::SCRIPT);
    // `safe.directory` for the reason `tests/exec.rs` gives: the fixture
    // runs as root so `apk` can install git, and on Linux git then refuses
    // the mounted tree as "dubious ownership" (measured on the ubuntu
    // runner); a real profile runs as the host's uid and is never asked.
    std::fs::write(
        &file,
        format!(
            "services:\n\
             \x20 agent:\n\
             \x20   image: alpine:3.20\n\
             \x20   volumes:\n\
             \x20     - {tree}:{tree_at}\n\
             \x20     - {volume}:{proof}\n\
             \x20     - {script}:{script_at}:ro\n\
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
            proof = nunki::exec::PROOF_AT,
            script = script.display(),
        ),
    )
    .unwrap();

    let engine: std::sync::Arc<dyn nunki::engine::Engine> =
        std::sync::Arc::new(nunki::engine::docker::Docker::real());
    let compose_project = nunki::compose::project_name(&project.session(), &slot.name).unwrap();
    let _ = engine.down(&file, &compose_project, true);
    engine.up(&file, &compose_project).unwrap();

    let mission = dir.path().join("mission");
    std::fs::create_dir_all(&mission).unwrap();
    // The base by name, the way `campaign` reads it now: it works the paths
    // out itself, so a launcher cannot mean something else by them than the
    // gate that judges what it produced.
    let touched = nunki::gate::touched_since_base(&tree, "dev").unwrap();
    assert!(touched.contains(&"src/lib.rs".to_string()), "{touched:?}");

    let go = || {
        mutants::campaign(
            &project,
            &slot,
            engine.clone(),
            &mission,
            "rust",
            &mutants::Asked {
                base: "dev",
                deadline_minutes: 45,
                replay: mutants::Replay::WhenChanged,
                rigor: nunki::mission::Rigor::Critical,
                threshold: 80,
            },
        )
        .unwrap()
    };

    match go() {
        Progress::Started { fingerprint, .. } => println!("started {fingerprint}"),
        other => panic!("expected a launch, got {other:?}"),
    }
    // While it runs, the record is beside the campaign — never in the mission
    // state, which holds the agent's run and only that.
    assert!(
        mutants::read_running(&project.hq_root, &slot.name)
            .unwrap()
            .is_some()
    );

    let mut finished = None;
    for _ in 0..60 {
        match go() {
            Progress::Running { lines, .. } => {
                println!("running, {lines} line(s)");
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            other => {
                finished = Some(other);
                break;
            }
        }
    }
    match finished.expect("the campaign ended") {
        Progress::Finished { survivors, .. } => assert_eq!(survivors, 2),
        other => panic!("expected a finished campaign, got {other:?}"),
    }
    assert!(
        mutants::read_running(&project.hq_root, &slot.name)
            .unwrap()
            .is_none(),
        "the record is cleared when the campaign is on file"
    );

    let campaign = mutants::read(&mission).unwrap().expect("it is on file");
    assert_eq!(campaign.survivors.len(), 2, "the chatter is not a survivor");
    assert_eq!(
        campaign.fingerprint,
        mutants::fingerprint(&tree, &touched).unwrap()
    );
    assert!(campaign.survivors.iter().all(|s| s.outcome.is_none()));
    // The fork point reached the script inside the container, through the
    // engine's own `exec -e`, and not only the command nunki built.
    let fork = nunki::gate::fork_point(&tree, "dev").unwrap();
    assert!(
        campaign
            .survivors
            .iter()
            .any(|s| s.description == format!("replace one with 0 since {fork}")),
        "{:?}",
        campaign.survivors
    );

    // Asked again on the same content, it does not spend another campaign.
    match go() {
        Progress::Fresh { survivors } => assert_eq!(survivors, 2),
        other => panic!("it replays only when the touched files change: {other:?}"),
    }

    // And forced to run again on that same content — what a human does when
    // a campaign's answer looked wrong — it reports what **this** run found,
    // not that plus what the one before it found.
    //
    // Several campaigns on one fingerprint write into one log, because the
    // spawner appends and the name carries the fingerprint. If the runs find
    // 5, then 1, then none, `MUTANTS.json` must not say six, every one of them
    // dead — or gate 7 sends a coder back for mutants that no longer exist.
    std::fs::remove_file(mission.join(mutants::FILE)).unwrap();
    let again = loop {
        match go() {
            Progress::Running { .. } | Progress::Started { .. } => {
                std::thread::sleep(std::time::Duration::from_millis(500))
            }
            other => break other,
        }
    };
    match again {
        Progress::Finished { survivors, .. } => assert_eq!(
            survivors, 2,
            "the same two, and not four: the log is this campaign's alone"
        ),
        other => panic!("expected a finished campaign, got {other:?}"),
    }
    assert_eq!(mutants::read(&mission).unwrap().unwrap().survivors.len(), 2);

    engine.down(&file, &compose_project, true).unwrap();
}

/// The HQ's ruling is a verb, not an invitation to hand-edit JSON: the one
/// outcome nobody can check should be given on purpose.
#[test]
fn an_equivalence_is_ruled_by_a_verb_and_lands_in_the_hqs_own_file() {
    let dir = tempfile::tempdir().unwrap();
    // Nothing to rule on yet, and it says so rather than inventing a campaign.
    let err = mutants::rule_equivalent(dir.path(), "src/lib.rs:3", "unreachable").unwrap_err();
    assert!(err.to_string().contains("no campaign"), "{err}");

    mutants::write(
        dir.path(),
        &Campaign {
            chain: Default::default(),
            fingerprint: "abc1234".into(),
            head: "def5678".into(),
            tried: None,
            date: "2026-09-10T12:00:00Z".into(),
            survivors: vec![Survivor {
                found_on: None,
                id: "src/lib.rs:3".into(),
                file: "src/lib.rs".into(),
                line: 3,
                end_line: None,
                description: "replace one with 0".into(),
                outcome: None,
                refused: None,
            }],
        },
    )
    .unwrap();

    let err = mutants::rule_equivalent(dir.path(), "src/lib.rs:9", "unreachable").unwrap_err();
    assert!(err.to_string().contains("no survivor is called"), "{err}");

    mutants::rule_equivalent(dir.path(), "src/lib.rs:3", "no caller reaches it").unwrap();
    let campaign = mutants::read(dir.path()).unwrap().unwrap();
    assert_eq!(
        campaign.survivors[0].outcome,
        Some(Triage::Equivalent {
            why: "no caller reaches it".into(),
            carried_from: None,
        })
    );
    // And it landed in the HQ's file, not in the agent's.
    assert!(mutants::read_triage(dir.path()).unwrap().is_empty());
}

fn one(id: &str, line: u32, description: &str) -> Survivor {
    Survivor {
        found_on: None,
        id: id.into(),
        file: "src/cells.py".into(),
        line,
        end_line: None,
        description: description.into(),
        outcome: None,
        refused: None,
    }
}

fn ruled(mut survivor: Survivor, why: &str) -> Survivor {
    survivor.outcome = Some(Triage::Equivalent {
        why: why.into(),
        carried_from: None,
    });
    survivor
}

fn ruled_before(dir: &std::path::Path, survivors: Vec<Survivor>) {
    mutants::write(
        dir,
        &Campaign {
            chain: Default::default(),
            fingerprint: "old0000".into(),
            head: "def5678".into(),
            date: "2026-09-10T12:00:00Z".into(),
            survivors,
            tried: None,
        },
    )
    .unwrap();
}

fn log_of(survivors: &[Survivor]) -> String {
    let mut text: String = survivors
        .iter()
        .map(|s| serde_json::to_string(s).unwrap() + "\n")
        .collect();
    text.push_str("{\"campaign\":\"done\"}\n");
    text
}

const F_TO_UPPER: &str = "survived: number = format(cell, \"f\") -> number = format(cell, \"F\")";

/// A commit above a ruled survivor moves its line and nothing else. The line
/// is what a commit moves; the mutation is what was ruled on — and the next
/// campaign proposes the ruling, never gives it (HQ review 4).
#[test]
fn a_ruling_follows_its_mutant_to_the_next_campaign_when_only_the_line_moved() {
    let dir = tempfile::tempdir().unwrap();
    ruled_before(
        dir.path(),
        vec![ruled(
            one("cells.x_json__mutmut_7", 112, F_TO_UPPER),
            "'f' and 'F' agree on finite decimals",
        )],
    );
    let count = mutants::record_finished(
        dir.path(),
        "new1111",
        "abc9999",
        &log_of(&[one("cells.x_json__mutmut_7", 123, F_TO_UPPER)]),
    )
    .unwrap();
    assert_eq!(count, 1);
    let campaign = mutants::read(dir.path()).unwrap().unwrap();
    assert_eq!(campaign.fingerprint, "new1111");
    assert_eq!(campaign.survivors[0].line, 123);
    assert_eq!(
        campaign.survivors[0].outcome,
        Some(Triage::ProposedByNunki {
            why: "'f' and 'F' agree on finite decimals".into(),
            from: mutants::ProposedFrom::Carried {
                commit: "def5678".into()
            },
        })
    );
}

/// A tool that renumbers its mutants must not lose the ruling either, as long
/// as the mutation is told apart: under another id the HQ is proposed its own
/// ruling, and confirms it with one verb.
#[test]
fn a_renamed_mutant_whose_mutation_is_unique_is_proposed_the_ruling() {
    let dir = tempfile::tempdir().unwrap();
    ruled_before(
        dir.path(),
        vec![ruled(
            one("cells.x_json__mutmut_7", 112, F_TO_UPPER),
            "same output",
        )],
    );
    mutants::record_finished(
        dir.path(),
        "new1111",
        "abc9999",
        &log_of(&[one("cells.x_json__mutmut_9", 118, F_TO_UPPER)]),
    )
    .unwrap();
    let campaign = mutants::read(dir.path()).unwrap().unwrap();
    // Under another id, the match is a heuristic: it proposes, for the HQ to
    // ratify, and never rules (HQ review 2, B).
    assert_eq!(
        campaign.survivors[0].outcome,
        Some(Triage::ProposedByNunki {
            why: "same output".into(),
            from: mutants::ProposedFrom::Carried {
                commit: "def5678".into()
            },
        })
    );
    assert_eq!(mutants::awaiting_ruling(dir.path()).unwrap().len(), 1);
}

/// A different mutation is a different question, and that includes a status
/// that moved: `no tests` is code nothing reaches, which a ruling about a
/// survivor never spoke for.
#[test]
fn a_ruling_does_not_follow_a_mutation_that_changed() {
    let dir = tempfile::tempdir().unwrap();
    ruled_before(
        dir.path(),
        vec![ruled(
            one("cells.x_json__mutmut_7", 112, F_TO_UPPER),
            "same output",
        )],
    );
    let moved = F_TO_UPPER.replacen("survived", "no tests", 1);
    mutants::record_finished(
        dir.path(),
        "new1111",
        "abc9999",
        &log_of(&[one("cells.x_json__mutmut_7", 112, &moved)]),
    )
    .unwrap();
    let campaign = mutants::read(dir.path()).unwrap().unwrap();
    assert_eq!(campaign.survivors[0].outcome, None);
}

/// The same `x = False -> x = None` twice in one file is two mutants the
/// ruling may not speak for alike. Ruled again rather than guessed.
#[test]
fn a_ruling_does_not_follow_a_mutation_it_cannot_tell_apart() {
    let dir = tempfile::tempdir().unwrap();
    let flag = "survived: quarantined = False -> quarantined = None";
    ruled_before(
        dir.path(),
        vec![ruled(
            one("run.x_once__mutmut_64", 795, flag),
            "only truth is read",
        )],
    );
    mutants::record_finished(
        dir.path(),
        "new1111",
        "abc9999",
        &log_of(&[
            one("run.x_once__mutmut_70", 801, flag),
            one("run.x_other__mutmut_3", 40, flag),
        ]),
    )
    .unwrap();
    let campaign = mutants::read(dir.path()).unwrap().unwrap();
    assert!(
        campaign.survivors.iter().all(|s| s.outcome.is_none()),
        "{campaign:?}"
    );

    // The same id still carries, ambiguity or not: it is the same mutant.
    ruled_before(
        dir.path(),
        vec![ruled(
            one("run.x_once__mutmut_64", 795, flag),
            "only truth is read",
        )],
    );
    mutants::record_finished(
        dir.path(),
        "new2222",
        "abc9999",
        &log_of(&[
            one("run.x_once__mutmut_64", 801, flag),
            one("run.x_other__mutmut_3", 40, flag),
        ]),
    )
    .unwrap();
    let campaign = mutants::read(dir.path()).unwrap().unwrap();
    assert!(campaign.survivors[0].outcome.is_some());
    assert_eq!(campaign.survivors[1].outcome, None);
}

/// A ruling proposed twice still names the campaign it was first given on.
#[test]
fn a_ruling_carried_again_keeps_the_campaign_it_was_given_on() {
    let dir = tempfile::tempdir().unwrap();
    ruled_before(
        dir.path(),
        vec![ruled(
            one("cells.x_json__mutmut_7", 112, F_TO_UPPER),
            "same output",
        )],
    );
    mutants::record_finished(
        dir.path(),
        "new1111",
        "abc9999",
        &log_of(&[one("cells.x_json__mutmut_7", 123, F_TO_UPPER)]),
    )
    .unwrap();
    mutants::record_finished(
        dir.path(),
        "new2222",
        "fff0000",
        &log_of(&[one("cells.x_json__mutmut_7", 130, F_TO_UPPER)]),
    )
    .unwrap();
    let campaign = mutants::read(dir.path()).unwrap().unwrap();
    assert!(matches!(
        &campaign.survivors[0].outcome,
        Some(Triage::ProposedByNunki {
            from: mutants::ProposedFrom::Carried { commit },
            ..
        }) if commit == "def5678"
    ));
}

/// The log is printed inside the agent's container. An `equivalent` in it
/// would reach the HQ's file with no human having given it.
#[test]
fn an_outcome_written_in_the_campaigns_log_is_never_taken() {
    let injected = ruled(one("cells.x_json__mutmut_7", 112, F_TO_UPPER), "trust me");
    assert_eq!(
        mutants::parse(&log_of(std::slice::from_ref(&injected)))[0].outcome,
        None
    );

    let dir = tempfile::tempdir().unwrap();
    mutants::record_finished(dir.path(), "new1111", "abc9999", &log_of(&[injected])).unwrap();
    let campaign = mutants::read(dir.path()).unwrap().unwrap();
    assert_eq!(campaign.survivors[0].outcome, None);
}

/// A carried ruling is written with where it came from, and a campaign file
/// written before the field existed still reads.
#[test]
fn where_a_ruling_came_from_round_trips_and_older_files_still_read() {
    let carried = Triage::Equivalent {
        why: "same output".into(),
        carried_from: Some("def5678".into()),
    };
    let text = serde_json::to_string(&carried).unwrap();
    assert_eq!(serde_json::from_str::<Triage>(&text).unwrap(), carried);

    let older = r#"{"kind":"equivalent","why":"same output"}"#;
    assert_eq!(
        serde_json::from_str::<Triage>(older).unwrap(),
        Triage::Equivalent {
            why: "same output".into(),
            carried_from: None,
        }
    );
    // And a ruling given on this campaign writes no empty field.
    let fresh = serde_json::to_string(&Triage::Equivalent {
        why: "same output".into(),
        carried_from: None,
    })
    .unwrap();
    assert!(!fresh.contains("carried_from"), "{fresh}");
}

/// Carried rulings outlive every replay, so a wrong one needs a way back.
#[test]
fn a_ruling_can_be_lifted_and_only_a_ruling() {
    let dir = tempfile::tempdir().unwrap();
    let err = mutants::lift_equivalent(dir.path(), "cells.x_json__mutmut_7").unwrap_err();
    assert!(err.to_string().contains("no campaign"), "{err}");

    ruled_before(
        dir.path(),
        vec![
            ruled(
                one("cells.x_json__mutmut_7", 112, F_TO_UPPER),
                "same output",
            ),
            one("cells.x_json__mutmut_8", 113, "survived: a -> b"),
        ],
    );
    let err = mutants::lift_equivalent(dir.path(), "cells.x_json__mutmut_8").unwrap_err();
    assert!(err.to_string().contains("no `equivalent` ruling"), "{err}");
    let err = mutants::lift_equivalent(dir.path(), "nobody").unwrap_err();
    assert!(err.to_string().contains("no survivor is called"), "{err}");

    mutants::lift_equivalent(dir.path(), "cells.x_json__mutmut_7").unwrap();
    let campaign = mutants::read(dir.path()).unwrap().unwrap();
    assert_eq!(campaign.survivors[0].outcome, None);

    // And a lifted ruling is not carried by the next campaign.
    mutants::record_finished(
        dir.path(),
        "new1111",
        "abc9999",
        &log_of(&[one("cells.x_json__mutmut_7", 123, F_TO_UPPER)]),
    )
    .unwrap();
    assert_eq!(
        mutants::read(dir.path()).unwrap().unwrap().survivors[0].outcome,
        None
    );
}

#[test]
fn only_the_two_outcomes_that_rest_on_a_test_are_the_coders_to_give() {
    assert!(Triage::Killed { test: "x".into() }.is_the_coders_to_give());
    assert!(Triage::Bug { test: "x".into() }.is_the_coders_to_give());
    // The judgement nobody can check.
    assert!(
        !Triage::Equivalent {
            why: "x".into(),
            carried_from: None,
        }
        .is_the_coders_to_give()
    );
    assert_eq!(
        Triage::Equivalent {
            why: "x".into(),
            carried_from: None,
        }
        .kind(),
        "equivalent"
    );
}

#[test]
fn a_triage_file_the_coder_wrote_and_hq_cannot_read_is_said_not_ignored() {
    let dir = tempfile::tempdir().unwrap();
    assert!(mutants::read_triage(dir.path()).unwrap().is_empty());
    // Created empty by `nunki mission new`, and empty is not an error.
    std::fs::write(dir.path().join("MUTANTS.triage.json"), "").unwrap();
    assert!(mutants::read_triage(dir.path()).unwrap().is_empty());
    // Written and unreadable is another matter: silently treating it as no
    // triage would lose every answer the coder wrote.
    std::fs::write(dir.path().join("MUTANTS.triage.json"), "{not json").unwrap();
    assert!(mutants::read_triage(dir.path()).is_err());
}

/// The mutation script `nunki init` ships, run against the real cargo-mutants
/// (SPEC 4.4, gate 7).
///
/// A test of gate 7 that uses a stub echoing a JSON line proves nothing about
/// the shipped script, which can be wrong in two ways that only running it
/// shows — measured against cargo-mutants 27.1.0:
///
/// 1. `--output DIR` writes into `DIR/mutants.out/`, not into `DIR`. The
///    script read `DIR/missed.txt`, found nothing, and exited 1: every
///    campaign would have failed with "the campaign left no …".
/// 2. Several mutants share one **position** — `> ==`, `> <` and `> >=` are
///    all at `src/lib.rs:2:7` — so neither `file:line` nor `file:line:col`
///    tells them apart, and the coder would have been handed survivors it
///    cannot answer one by one in a file whose whole purpose is that. The
///    identifier is the whole line, which is the tool's own name for a
///    mutant.
/// 3. `--output` creates its own directory and not the path above it, so a
///    clean copy of `HEAD` that has never been built — which is exactly what
///    gate 7 runs in — failed with "create output parent directory".
#[test]
#[ignore = "runs a real mutation campaign; needs cargo-mutants; run by hand"]
fn live_the_shipped_mutation_script_reads_a_real_campaign() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("crate");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"tiny\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    // `keep` has no test at all, so its mutants survive; `double` has one, so
    // some of its are caught. A campaign where everything survives would not
    // prove the script reads `missed.txt` rather than every mutant.
    std::fs::write(
        root.join("src/lib.rs"),
        "pub fn keep(n: u8) -> bool {\n    n > 3\n}\n\n\
         pub fn double(n: u8) -> u8 {\n    n * 2\n}\n\n\
         #[cfg(test)]\nmod tests {\n    #[test]\n    fn double_works() {\n        \
         assert_eq!(super::double(2), 4);\n    }\n}\n",
    )
    .unwrap();

    // The script exactly as `nunki init` deposits it, not a copy of it.
    nunki::init::init(&root, &dir.path().join("nunki"), &["rust".to_string()]).unwrap();
    let script = dir
        .path()
        .join("nunki/stacks/rust")
        .join(nunki::mutants::SCRIPT);
    assert!(script.is_file());

    let out = std::process::Command::new(&script)
        .arg("campaign-1")
        .arg("src/lib.rs")
        .current_dir(&root)
        .output()
        .expect("the script runs; cargo-mutants must be installed");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // What `nunki` reads is what the script printed, through the very parser
    // gate 7 uses.
    let survivors = nunki::mutants::parse(&stdout);
    assert!(
        survivors.len() >= 4,
        "a crate with an untested function has survivors: {stdout}"
    );
    assert!(
        survivors.iter().all(|s| s.file == "src/lib.rs"),
        "{survivors:?}"
    );
    assert!(
        survivors.iter().any(|s| s.description.contains("replace")),
        "{survivors:?}"
    );

    // The identifiers are distinct, which is the second defect: three mutants
    // of one line differ only by their column.
    let mut ids: Vec<&str> = survivors.iter().map(|s| s.id.as_str()).collect();
    let total = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), total, "two survivors share an id: {survivors:?}");
    let same_line = survivors.iter().filter(|s| s.line == 2).collect::<Vec<_>>();
    assert!(
        same_line.len() >= 2,
        "line 2 carries several mutants: {survivors:?}"
    );

    // And what `double` proves: the script reads the missed list, not every
    // mutant the campaign tried.
    assert!(
        !survivors
            .iter()
            .any(|s| s.description.contains("replace * with /")),
        "that one is caught by the test, and a caught mutant is not a survivor: {survivors:?}"
    );

    // Every survivor carries the span of the code it replaces, from the
    // tool's own listing: one line for an operator, the whole body for a
    // body replaced whole (HQ review, the class rule).
    for s in &survivors {
        assert!(s.span().is_some(), "a span the listing named once: {s:?}");
    }
    let operator = survivors
        .iter()
        .find(|s| s.description.starts_with("replace > with"))
        .expect("an operator survives in `keep`");
    assert_eq!(operator.span(), Some((2, 2)), "{operator:?}");
}

/// The Rust stack's script carries each survivor's span from cargo-mutants'
/// own listing, `mutants.json`: its `name` is the line `missed.txt` holds,
/// its `span` the code replaced. Run as `nunki init` deposits it, with a
/// `cargo` that writes what cargo-mutants 27.1.0 was measured to write — a
/// body replaced whole spanning lines 6 to 7 — and a name the listing gives
/// twice, whose span is then said to be unknown.
#[cfg(unix)]
#[test]
fn the_rust_campaign_carries_each_survivors_span_from_the_tools_listing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("crate");
    std::fs::create_dir_all(root.join("src")).unwrap();
    nunki::init::init(&root, &dir.path().join("nunki"), &["rust".to_string()]).unwrap();
    let script = dir
        .path()
        .join("nunki/stacks/rust")
        .join(nunki::mutants::SCRIPT);

    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let listing = r#"[
 {"name":"src/lib.rs:6:5: replace body -> u8 with 0","span":{"start":{"line":6,"column":5},"end":{"line":7,"column":10}}},
 {"name":"src/lib.rs:2:7: replace > with < in keep","span":{"start":{"line":2,"column":7},"end":{"line":2,"column":8}}},
 {"name":"src/lib.rs:9:1: twice","span":{"start":{"line":9,"column":1},"end":{"line":9,"column":2}}},
 {"name":"src/lib.rs:9:1: twice","span":{"start":{"line":9,"column":1},"end":{"line":12,"column":2}}}
]"#;
    let missed = "src/lib.rs:6:5: replace body -> u8 with 0\n\
                  src/lib.rs:2:7: replace > with < in keep\n\
                  src/lib.rs:9:1: twice\n";
    std::fs::write(dir.path().join("listing.json"), listing).unwrap();
    std::fs::write(dir.path().join("missed.txt"), missed).unwrap();
    std::fs::write(
        bin.join("cargo"),
        format!(
            "#!/bin/sh\n\
             out=''\n\
             while [ $# -gt 0 ]; do [ \"$1\" = --output ] && out=$2; shift; done\n\
             mkdir -p \"$out/mutants.out\"\n\
             cp {listing} \"$out/mutants.out/mutants.json\"\n\
             cp {missed} \"$out/mutants.out/missed.txt\"\n\
             : > \"$out/mutants.out/caught.txt\"\n\
             exit 2\n",
            listing = dir.path().join("listing.json").display(),
            missed = dir.path().join("missed.txt").display(),
        ),
    )
    .unwrap();
    std::fs::set_permissions(
        bin.join("cargo"),
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .unwrap();

    let out = std::process::Command::new("sh")
        .arg(&script)
        .arg("campaign-1")
        .arg("src/lib.rs")
        .current_dir(&root)
        .env(
            "PATH",
            format!(
                "{}:{}",
                bin.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env_remove(nunki::mutants::BASE_ENV)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && mutants::completed(&stdout),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let survivors = mutants::parse(&stdout);
    let span = |id: &str| {
        survivors
            .iter()
            .find(|s| s.id == id)
            .unwrap_or_else(|| panic!("{id} in {stdout}"))
            .span()
    };
    assert_eq!(
        span("src/lib.rs:6:5: replace body -> u8 with 0"),
        Some((6, 7))
    );
    assert_eq!(
        span("src/lib.rs:2:7: replace > with < in keep"),
        Some((2, 2))
    );
    assert_eq!(span("src/lib.rs:9:1: twice"), None, "named twice: unknown");
}

/// A campaign that killed everything has nobody to triage, and says so.
///
/// A sentence that asks for "one of the three outcomes" whatever the
/// campaign found makes a clean campaign demand outcomes for nobody. A line that reads the same
/// whatever happened is a line that stops being read.
#[test]
fn a_campaign_with_no_survivor_asks_for_no_outcome() {
    let clean = mutants::ended(0);
    assert!(
        !clean.contains("outcome"),
        "there is nobody to give one: {clean}"
    );
    assert!(clean.contains("no survivor"), "{clean}");

    let three = mutants::ended(3);
    assert!(three.contains("3 survivor(s)"), "{three}");
    assert!(
        three.contains("each needs one of the three outcomes"),
        "{three}"
    );
    // Both name the file the answers are read from, so a human knows where
    // to look either way.
    for said in [&clean, &three] {
        assert!(said.contains(mutants::FILE), "{said}");
    }
}

/// Every sentence this module hands a human is one line, with no run of
/// spaces in the middle of it.
///
/// Without this, `nunki verify` can print
///
/// ```text
/// started; it runs detached, and `nunki verify`                    again reads it back
/// ```
///
/// A multi-line string literal in `main.rs` had been joined by `cargo fmt`,
/// which kept the continuation's indentation **inside** the string. The five
/// gates were green — formatting a literal is not a warning, and no test read
/// the sentence. This one does.
#[test]
fn what_a_campaign_says_is_a_sentence_and_not_a_layout() {
    for said in [mutants::started("m1"), mutants::ended(0), mutants::ended(3)] {
        assert!(!said.contains("  "), "two spaces in a row: {said:?}");
        assert!(!said.contains('\n'), "a line break: {said:?}");
        assert_eq!(said.trim(), said, "space at either end: {said:?}");
    }
    // And it names the mission, because the verb it suggests takes one.
    assert!(mutants::started("m1").contains("nunki verify m1"));
}

fn on_file(dir: &std::path::Path, fingerprint: &str, survivors: usize) {
    let survivors = (0..survivors)
        .map(|i| mutants::Survivor {
            found_on: None,
            id: format!("src/lib.rs:{i}:1: replace a with b"),
            file: "src/lib.rs".into(),
            line: i as u32 + 1,
            end_line: None,
            description: "replace a with b".into(),
            outcome: None,
            refused: None,
        })
        .collect();
    mutants::write(
        dir,
        &mutants::Campaign {
            chain: Default::default(),
            fingerprint: fingerprint.into(),
            head: "0".repeat(40),
            date: "2026-09-17T00:00:00Z".into(),
            survivors,
            tried: None,
        },
    )
    .unwrap();
}

/// A campaign costs an hour (SPEC § 7), so one on the same content is not run
/// again. That is the default and it stays the default.
#[test]
fn a_campaign_on_the_same_content_is_not_run_again() {
    let dir = tempfile::tempdir().unwrap();
    on_file(dir.path(), "abc", 2);

    let answer =
        mutants::already_answered(dir.path(), "abc", mutants::Replay::WhenChanged).unwrap();

    assert_eq!(answer, Some(mutants::Progress::Fresh { survivors: 2 }));
}

/// The fingerprint is over the **touched files**. It cannot see a change to
/// the stack's `mutation.sh`, to the tool's version, or to an exclusion added
/// since — and every one of those changes the answer.
///
/// Without it the only way past is deleting `MUTANTS.json` by hand, which is
/// easily done with `MUTANTS.triage.json` alongside: the engine then replaces
/// the missing file with a directory and the mission comes down on
/// `Is a directory (os error 21)`. A verb is cheaper than the workaround it
/// replaces.
#[test]
fn a_campaign_is_run_again_when_the_human_says_something_changed() {
    let dir = tempfile::tempdir().unwrap();
    on_file(dir.path(), "abc", 2);

    let answer = mutants::already_answered(dir.path(), "abc", mutants::Replay::Now).unwrap();

    assert_eq!(answer, None, "the campaign on file was taken anyway");
    // And the file is left where it is: asking again is not deleting.
    assert!(mutants::read(dir.path()).unwrap().is_some());
}

/// Content that moved is run again whoever asks, which is the rule that was
/// already there.
#[test]
fn a_campaign_on_content_that_moved_is_run_again() {
    let dir = tempfile::tempdir().unwrap();
    on_file(dir.path(), "abc", 2);

    let answer =
        mutants::already_answered(dir.path(), "def", mutants::Replay::WhenChanged).unwrap();

    assert_eq!(answer, None);
}

/// Nothing on file is nothing to reuse.
#[test]
fn a_campaign_that_never_ran_is_run() {
    let dir = tempfile::tempdir().unwrap();

    let answer =
        mutants::already_answered(dir.path(), "abc", mutants::Replay::WhenChanged).unwrap();

    assert_eq!(answer, None);
}

/// The campaign is told the commit its branch forked from, so a stack can
/// answer for the lines the branch changed rather than for whole files
/// (SPEC 4.4).
///
/// The fork point and not the base's tip: the base moves on after the
/// branch leaves it, and diffing against the tip would hand the campaign
/// every line merged there since.
#[test]
fn a_campaign_is_told_the_commit_its_branch_forked_from() {
    let dir = tempfile::tempdir().unwrap();
    let (project, slot) = context(dir.path());
    let tree = &slot.tree;
    let forked = git(tree, &["rev-parse", "dev"]);
    write(tree, "src/lib.rs", "pub fn one() -> u8 { 2 }\n");
    git(tree, &["commit", "-q", "-am", "L1"]);
    // The base moves on after the branch left it.
    git(tree, &["checkout", "-q", "dev"]);
    write(tree, "src/other.rs", "pub fn other() {}\n");
    git(tree, &["add", "-A"]);
    git(tree, &["commit", "-q", "-m", "merged meanwhile"]);
    git(tree, &["checkout", "-q", "mission/x"]);

    let fork = nunki::gate::fork_point(tree, "dev").unwrap();
    assert_eq!(fork, forked, "the fork point, not the base's tip");

    let touched = nunki::gate::touched_since_base(tree, "dev").unwrap();
    let judged = nunki::run::judged(&project, "rust");
    let one = mutants::command(&judged, "abc123", &touched, &fork);
    assert_eq!(
        one.env.get(mutants::BASE_ENV).map(String::as_str),
        Some(forked.as_str()),
        "{one:?}"
    );
    assert_eq!(
        one.args,
        vec!["abc123".to_string(), "src/lib.rs".to_string()]
    );

    // Several stacks run through one shell, whose children inherit the same
    // environment: the variable is set on that shell.
    let mut two = judged.clone();
    two.push(nunki::run::Judged {
        stack: nunki::project::Stack::new("next", "frontend").unwrap(),
        scripts_at: "/work/stack-next".to_string(),
        advisories_at: "/nunki/advisories-next".to_string(),
    });
    let several = mutants::command(&two, "abc123", &touched, &fork);
    assert_eq!(several.program, "sh");
    assert_eq!(
        several.env.get(mutants::BASE_ENV).map(String::as_str),
        Some(forked.as_str())
    );
}

/// With the fork point, the shipped script answers for the lines the branch
/// changed, against the real tool (SPEC 4.4).
///
/// `keep` has no test and is not touched: its mutants would all survive a
/// whole-file campaign, and must not appear here. `double` is changed on the
/// branch: its mutants are the campaign's, and one of them survives.
///
/// ```text
/// cargo test --test mutants live_the_shipped_mutation_script_answers -- --ignored
/// ```
#[test]
#[ignore = "runs a real mutation campaign; needs cargo-mutants; run by hand"]
fn live_the_shipped_mutation_script_answers_for_the_changed_lines() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("crate");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"tiny\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    let lib = |double: &str| {
        format!(
            "pub fn keep(n: u8) -> bool {{\n    n > 3\n}}\n\n\
             pub fn double(n: u8) -> u8 {{\n    {double}\n}}\n\n\
             #[cfg(test)]\nmod tests {{\n    #[test]\n    fn double_works() {{\n        \
             assert!(super::double(2) > 0);\n    }}\n}}\n"
        )
    };
    std::fs::write(root.join("src/lib.rs"), lib("n * 2")).unwrap();
    std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
    git(&root, &["init", "-q", "-b", "dev"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "base"]);
    let base = git(&root, &["rev-parse", "HEAD"]);
    std::fs::write(root.join("src/lib.rs"), lib("n + n")).unwrap();
    git(&root, &["commit", "-q", "-am", "L1"]);

    nunki::init::init(&root, &dir.path().join("nunki"), &["rust".to_string()]).unwrap();
    let script = dir
        .path()
        .join("nunki/stacks/rust")
        .join(nunki::mutants::SCRIPT);

    let out = std::process::Command::new(&script)
        .arg("campaign-1")
        .arg("src/lib.rs")
        .env(nunki::mutants::BASE_ENV, &base)
        .current_dir(&root)
        .output()
        .expect("the script runs; cargo-mutants must be installed");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        nunki::mutants::completed(&stdout),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let survivors = nunki::mutants::parse(&stdout);
    assert!(
        survivors.iter().all(|s| s.description.contains("double")),
        "a mutant outside the changed function: {survivors:?}"
    );
    assert!(
        !survivors.is_empty(),
        "`double` is only checked for being positive: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// How many mutants a campaign tried, for a `standard` mission's gate 7.
// ---------------------------------------------------------------------------

/// The count is the script's own word on its terminal line, and nothing
/// else: a line without it, a log without a terminal line, or a count on a
/// line that is not the terminal one all say "I do not know".
#[test]
fn the_tried_count_is_read_from_the_terminal_line_only() {
    let log = "Found 12 mutants to test\n\
               {\"id\":\"a\",\"file\":\"src/lib.rs\",\"line\":3,\"description\":\"x\"}\n\
               {\"campaign\":\"done\",\"tried\":12}\n";
    assert!(mutants::completed(log), "a count does not hide the end");
    assert_eq!(mutants::tried(log), Some(12));
    assert_eq!(mutants::parse(log).len(), 1);

    assert_eq!(mutants::tried("{\"campaign\":\"done\"}\n"), None);
    assert_eq!(
        mutants::tried("{\"campaign\":\"started\",\"tried\":4}\n"),
        None
    );
    assert_eq!(mutants::tried(""), None);
    assert_eq!(
        mutants::tried("{\"campaign\":\"done\",\"tried\":0}\n"),
        Some(0)
    );
}

/// A finished campaign keeps its count in `MUTANTS.json`; one without a
/// count writes no `tried` at all, so a campaign file is byte for byte what
/// it was before the count existed, and an older file reads as `None`.
#[test]
fn a_campaign_records_how_many_mutants_it_tried_and_an_older_file_still_reads() {
    let dir = tempfile::tempdir().unwrap();
    mutants::record_finished(
        dir.path(),
        "abc1234",
        "def5678",
        "{\"id\":\"a\",\"file\":\"src/lib.rs\",\"line\":3,\"description\":\"x\"}\n\
         {\"campaign\":\"done\",\"tried\":7}\n",
    )
    .unwrap();
    let campaign = mutants::read(dir.path()).unwrap().unwrap();
    assert_eq!(campaign.tried, Some(7));
    assert_eq!(campaign.survivors.len(), 1);

    let older = tempfile::tempdir().unwrap();
    mutants::record_finished(
        older.path(),
        "abc1234",
        "def5678",
        "{\"campaign\":\"done\"}\n",
    )
    .unwrap();
    let text = std::fs::read_to_string(older.path().join(mutants::FILE)).unwrap();
    assert!(!text.contains("tried"), "{text}");
    assert_eq!(mutants::read(older.path()).unwrap().unwrap().tried, None);

    std::fs::write(
        older.path().join(mutants::FILE),
        "{\"fingerprint\":\"f\",\"head\":\"h\",\"date\":\"d\",\"survivors\":[]}\n",
    )
    .unwrap();
    assert_eq!(mutants::read(older.path()).unwrap().unwrap().tried, None);
}

/// Two stacks, each with a `mutation.sh` of its own, run by the script
/// `nunki` hands a project that carries both.
struct TwoStacks {
    _dir: tempfile::TempDir,
    tree: PathBuf,
    judged: Vec<nunki::run::Judged>,
}

impl TwoStacks {
    fn new(root_script: &str, frontend_script: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let tree = dir.path().join("tree");
        std::fs::create_dir_all(tree.join("frontend/app")).unwrap();
        let mut judged = Vec::new();
        for (name, sub, body) in [
            ("rust", "", root_script),
            ("next", "frontend", frontend_script),
        ] {
            let at = dir.path().join(format!("stack-{name}"));
            std::fs::create_dir_all(&at).unwrap();
            let script = at.join(mutants::SCRIPT);
            std::fs::write(&script, body).unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
            judged.push(nunki::run::Judged {
                stack: nunki::project::Stack::new(name, sub).unwrap(),
                scripts_at: at.display().to_string(),
                advisories_at: String::new(),
            });
        }
        Self {
            _dir: dir,
            tree,
            judged,
        }
    }

    fn run(&self) -> String {
        let touched = vec!["src/lib.rs".to_string(), "frontend/app/page.ts".to_string()];
        let script = mutants::several_campaigns(&self.judged, "abc1234", &touched);
        let out = Command::new("sh")
            .args(["-c", &script])
            .current_dir(&self.tree)
            .output()
            .expect("sh runs the campaign");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }
}

fn says(lines: &[&str]) -> String {
    let mut script = String::from("#!/bin/sh\n");
    for line in lines {
        script.push_str(&format!("printf '%s\\n' '{line}'\n"));
    }
    script
}

/// Over several stacks the count is the sum of theirs, and only when every
/// stack that ran gave one: a sum missing one stack's mutants is a share of
/// the wrong whole.
#[cfg(unix)]
#[test]
fn a_campaign_over_several_stacks_adds_up_what_each_tried() {
    let rust = says(&[
        r#"{"id":"1","file":"src/lib.rs","line":2,"description":"x"}"#,
        r#"{"campaign":"done","tried":3}"#,
    ]);
    let next = says(&[r#"{"campaign":"done","tried":4}"#]);
    let out = TwoStacks::new(&rust, &next).run();
    assert!(mutants::completed(&out), "{out}");
    assert_eq!(mutants::tried(&out), Some(7), "{out}");
    assert_eq!(mutants::parse(&out).len(), 1, "{out}");

    // One stack whose script predates the count: the campaign finished, and
    // says it does not know how many were tried.
    let older = says(&[r#"{"campaign":"done"}"#]);
    let out = TwoStacks::new(&rust, &older).run();
    assert!(mutants::completed(&out), "{out}");
    assert_eq!(mutants::tried(&out), None, "{out}");
}

/// A stack that printed nothing did not finish, whatever jq's exit status
/// says: jq 1.6 — Debian bookworm's — exits 0 under `-e` on an empty input,
/// and the check that relied on it counted a silent stack as done.
#[cfg(unix)]
#[test]
fn a_stack_that_printed_nothing_has_not_finished_the_campaign() {
    let rust = says(&[r#"{"campaign":"done","tried":3}"#]);
    let out = TwoStacks::new(&rust, "#!/bin/sh\nexit 0\n").run();
    assert!(!mutants::completed(&out), "{out}");
    let out = TwoStacks::new(&rust, "#!/bin/sh\necho progress\nexit 1\n").run();
    assert!(!mutants::completed(&out), "{out}");
}

// ---------------------------------------------------------------------------
// A campaign that found mutants and tried none did not measure (security
// round 1 on the rigor mission, HQ ruling).
// ---------------------------------------------------------------------------

/// Found some, tried none: not measured, whatever the terminal line says.
/// Found none, or a line that does not say, is not this rule's to judge.
#[test]
fn a_campaign_that_found_mutants_and_tried_none_did_not_measure() {
    let log = "{\"campaign\":\"done\",\"tried\":0,\"found\":78}\n";
    assert!(mutants::completed(log));
    assert_eq!(mutants::found(log), Some(78));
    let why = mutants::unmeasured(log).expect("78 found and none tried");
    assert!(
        why.contains("found 78") && why.contains("measured nothing"),
        "{why}"
    );

    // One mutant found and none tried is as unmeasured as 78.
    assert!(mutants::unmeasured("{\"campaign\":\"done\",\"tried\":0,\"found\":1}\n").is_some());
    // A count of found without one of tried is no count at all, which gate 7
    // judges as `critical` does, and not a campaign that said it tried
    // nothing (HQ review).
    assert_eq!(
        mutants::unmeasured("{\"campaign\":\"done\",\"found\":3}\n"),
        None
    );
    for measured in [
        "{\"campaign\":\"done\",\"tried\":0,\"found\":0}\n",
        "{\"campaign\":\"done\",\"tried\":1,\"found\":78}\n",
        "{\"campaign\":\"done\",\"tried\":5}\n",
        "{\"campaign\":\"done\"}\n",
        "",
    ] {
        assert_eq!(mutants::unmeasured(measured), None, "{measured:?}");
    }
}

/// Settling one appends the reason to the campaign's stderr, where gate 7
/// reads why a campaign could not run, and leaves a measured one alone.
#[test]
fn an_unmeasured_campaign_leaves_its_reason_where_gate_seven_reads_it() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("mutants-abc1234.log");
    let err = log.with_extension("err");
    std::fs::write(&err, "Found 78 mutants to test\n").unwrap();

    let why =
        mutants::settle_unmeasured(&log, "{\"campaign\":\"done\",\"tried\":0,\"found\":78}\n")
            .unwrap()
            .expect("not measured");
    assert!(why.contains("measured nothing"), "{why}");
    let said = std::fs::read_to_string(&err).unwrap();
    assert!(
        said.starts_with("Found 78 mutants to test\n"),
        "appended: {said}"
    );
    assert!(said.contains("nunki: the campaign found 78"), "{said}");

    let measured = dir.path().join("mutants-def5678.log");
    assert_eq!(
        mutants::settle_unmeasured(
            &measured,
            "{\"campaign\":\"done\",\"tried\":3,\"found\":3}\n"
        )
        .unwrap(),
        None
    );
    assert!(!measured.with_extension("err").exists());
}

/// Over several stacks `found` is summed like `tried`, and given only when
/// every stack gave one.
#[cfg(unix)]
#[test]
fn a_campaign_over_several_stacks_adds_up_what_each_found() {
    let rust = says(&[r#"{"campaign":"done","tried":3,"found":5}"#]);
    let next = says(&[r#"{"campaign":"done","tried":4,"found":4}"#]);
    let out = TwoStacks::new(&rust, &next).run();
    assert_eq!(mutants::tried(&out), Some(7), "{out}");
    assert_eq!(mutants::found(&out), Some(9), "{out}");

    let older = says(&[r#"{"campaign":"done","tried":4}"#]);
    let out = TwoStacks::new(&rust, &older).run();
    assert_eq!(mutants::tried(&out), Some(7), "{out}");
    assert_eq!(mutants::found(&out), None, "{out}");

    // A stack that found mutants and tried none leaves the whole campaign
    // unfinished, even beside a stack that tried nothing either.
    let none = says(&[r#"{"campaign":"done","tried":0,"found":12}"#]);
    let empty = says(&[r#"{"campaign":"done","tried":0,"found":0}"#]);
    let out = TwoStacks::new(&none, &empty).run();
    assert!(!mutants::completed(&out), "{out}");
}

/// One stack that found mutants and tried none measured nothing, and the
/// sum must not hide it behind another stack's count.
#[cfg(unix)]
#[test]
fn a_stack_that_found_mutants_and_tried_none_leaves_the_campaign_unfinished() {
    let none = says(&[r#"{"campaign":"done","tried":0,"found":12}"#]);
    let some = says(&[r#"{"campaign":"done","tried":5,"found":5}"#]);
    let out = TwoStacks::new(&none, &some).run();
    assert!(!mutants::completed(&out), "{out}");
    let out = TwoStacks::new(&some, &none).run();
    assert!(!mutants::completed(&out), "{out}");
}

/// Read back, a campaign that said it finished having found mutants and
/// tried none is not recorded: it goes back to the branch as one that could
/// not run, with its reason left where gate 7 reads it.
#[test]
fn a_finished_campaign_that_tried_nothing_of_what_it_found_is_not_recorded() {
    use nunki::engine::{ExecOutput, fake::FakeEngine};

    let dir = tempfile::tempdir().unwrap();
    let (project, slot) = context(dir.path());
    let tree = slot.tree.clone();
    let profile = nunki::run::profile_path(&project, &slot.name);
    std::fs::create_dir_all(profile.parent().unwrap()).unwrap();
    std::fs::write(&profile, "services: {}\n").unwrap();

    let mission = dir.path().join("mission");
    std::fs::create_dir_all(&mission).unwrap();
    let log = mission.join("mutants.log");
    std::fs::write(&log, "{\"campaign\":\"done\",\"tried\":0,\"found\":12}\n").unwrap();
    mutants::write_running(
        &project.hq_root,
        &slot.name,
        &mutants::Running {
            chain: Default::default(),
            fingerprint: "abc1234".into(),
            head: git(&tree, &["rev-parse", "HEAD"]),
            started_at: "2026-09-19T02:00:00Z".into(),
            container: "cafe1234".into(),
            pid: Some(41),
            log: log.clone(),
            deadline_minutes: 45,
        },
    )
    .unwrap();
    let engine: std::sync::Arc<dyn nunki::engine::Engine> = std::sync::Arc::new(
        FakeEngine::default()
            .with_liveness("cafe1234", nunki::engine::Liveness::Running)
            .with_exec(ExecOutput {
                status: 0,
                stdout: "nunki-run-ended\n".into(),
                stderr: String::new(),
            }),
    );

    let progress = mutants::campaign(
        &project,
        &slot,
        engine,
        &mission,
        "rust",
        &mutants::Asked {
            base: "dev",
            deadline_minutes: 45,
            replay: mutants::Replay::WhenChanged,
            rigor: nunki::mission::Rigor::Critical,
            threshold: 80,
        },
    )
    .unwrap();

    let mutants::Progress::CouldNotRun(why) = progress else {
        panic!("a campaign that tried none of 12 was read as {progress:?}");
    };
    assert!(why.contains("measured nothing"), "{why}");
    assert_eq!(mutants::read(&mission).unwrap(), None, "nothing recorded");
    let said = std::fs::read_to_string(log.with_extension("err")).unwrap();
    assert!(said.contains("found 12"), "{said}");
    assert!(
        mutants::read_running(&project.hq_root, &slot.name)
            .unwrap()
            .is_none()
    );
}

// ---------------------------------------------------------------------------
// A campaign says it finished once (HQ review of the rigor mission).
// ---------------------------------------------------------------------------

/// A log with two done lines is not a finished campaign, whichever of them
/// carries the bigger count: the first and the last are both a line
/// something else could have written, so neither is read.
#[test]
fn a_log_that_says_it_finished_twice_is_not_a_finished_campaign() {
    let survivor = "{\"id\":\"a\",\"file\":\"src/lib.rs\",\"line\":3,\"description\":\"x\"}\n";
    for log in [
        format!(
            "{{\"campaign\":\"done\",\"tried\":1000}}\n{survivor}{{\"campaign\":\"done\",\"tried\":3}}\n"
        ),
        format!(
            "{survivor}{{\"campaign\":\"done\",\"tried\":3}}\n{{\"campaign\":\"done\",\"tried\":1000}}\n"
        ),
    ] {
        assert!(!mutants::completed(&log), "{log}");
        assert_eq!(mutants::tried(&log), None, "{log}");
        let why = mutants::repeated(&log).expect("said twice");
        assert!(why.contains("finished 2 times"), "{why}");
    }
    let once = format!("{survivor}{{\"campaign\":\"done\",\"tried\":3}}\n");
    assert!(mutants::completed(&once));
    assert_eq!(mutants::tried(&once), Some(3));
    assert_eq!(mutants::repeated(&once), None);
    assert_eq!(mutants::repeated(""), None);
}

/// Read back by the monitor, such a log is a campaign that could not run,
/// with its reason where gate 7 reads it — not a lost campaign, and nothing
/// is recorded.
#[test]
fn a_campaign_that_said_it_finished_twice_leaves_its_reason_and_no_result() {
    use nunki::engine::{ExecOutput, fake::FakeEngine};

    let dir = tempfile::tempdir().unwrap();
    let (project, slot) = context(dir.path());
    let tree = slot.tree.clone();
    let profile = nunki::run::profile_path(&project, &slot.name);
    std::fs::create_dir_all(profile.parent().unwrap()).unwrap();
    std::fs::write(&profile, "services: {}\n").unwrap();

    let mission = dir.path().join("mission");
    std::fs::create_dir_all(&mission).unwrap();
    let log = mission.join("mutants.log");
    std::fs::write(
        &log,
        "{\"campaign\":\"done\",\"tried\":1000,\"found\":1000}\n\
         {\"id\":\"a\",\"file\":\"src/lib.rs\",\"line\":3,\"description\":\"replace one\"}\n\
         {\"campaign\":\"done\",\"tried\":1,\"found\":1}\n",
    )
    .unwrap();
    mutants::write_running(
        &project.hq_root,
        &slot.name,
        &mutants::Running {
            chain: Default::default(),
            fingerprint: "abc1234".into(),
            head: git(&tree, &["rev-parse", "HEAD"]),
            started_at: "2026-09-19T02:00:00Z".into(),
            container: "cafe1234".into(),
            pid: Some(41),
            log: log.clone(),
            deadline_minutes: 45,
        },
    )
    .unwrap();
    let engine: std::sync::Arc<dyn nunki::engine::Engine> = std::sync::Arc::new(
        FakeEngine::default()
            .with_liveness("cafe1234", nunki::engine::Liveness::Running)
            .with_exec(ExecOutput {
                status: 0,
                stdout: "nunki-run-ended\n".into(),
                stderr: String::new(),
            }),
    );

    let progress = mutants::campaign(
        &project,
        &slot,
        engine,
        &mission,
        "rust",
        &mutants::Asked {
            base: "dev",
            deadline_minutes: 45,
            replay: mutants::Replay::WhenChanged,
            rigor: nunki::mission::Rigor::Critical,
            threshold: 80,
        },
    )
    .unwrap();

    let mutants::Progress::CouldNotRun(why) = progress else {
        panic!("a log that said it finished twice was read as {progress:?}");
    };
    assert!(why.contains("finished 2 times"), "{why}");
    let said = std::fs::read_to_string(log.with_extension("err")).unwrap();
    assert!(
        said.contains("nunki: the campaign said it had finished 2 times"),
        "{said}"
    );
    assert_eq!(mutants::read(&mission).unwrap(), None);
    assert!(
        mutants::read_running(&project.hq_root, &slot.name)
            .unwrap()
            .is_none()
    );
}

/// Over several stacks, a stack that said it finished twice did not finish,
/// by the same rule — and a stack that said it once still counts.
#[cfg(unix)]
#[test]
fn a_stack_that_said_it_finished_twice_has_not_finished_the_campaign() {
    let rust = says(&[r#"{"campaign":"done","tried":3,"found":3}"#]);
    let twice = says(&[
        r#"{"campaign":"done","tried":1000,"found":1000}"#,
        r#"{"campaign":"done","tried":4,"found":4}"#,
    ]);
    let out = TwoStacks::new(&rust, &twice).run();
    assert!(!mutants::completed(&out), "{out}");
    assert_eq!(
        mutants::repeated(&out),
        None,
        "no done line is left at all: {out}"
    );

    let once = says(&[r#"{"campaign":"done","tried":4,"found":4}"#]);
    let out = TwoStacks::new(&rust, &once).run();
    assert!(mutants::completed(&out), "{out}");
    assert_eq!(mutants::tried(&out), Some(7), "{out}");
}

// ---------------------------------------------------------------------------
// The coder's proposal of an equivalence, and the HQ's ruling on it.
// ---------------------------------------------------------------------------

fn proposes(dir: &Path, answers: &[(&str, &str)]) {
    let map = answers
        .iter()
        .map(|(id, why)| {
            (
                (*id).to_string(),
                Triage::EquivalentProposed {
                    why: (*why).to_string(),
                },
            )
        })
        .collect();
    mutants::write_triage(dir, &map).unwrap();
}

/// The coder's file takes the proposal in the shape the prompt gives, and
/// `why` is required: a proposal with no reason field is not a file `nunki`
/// reads as one.
#[test]
fn a_proposal_reads_in_the_shape_the_prompt_gives_and_needs_its_reason() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(mutants::TRIAGE_FILE),
        r#"{"m1": {"kind": "equivalent_proposed", "why": "only a log line reads it"}}"#,
    )
    .unwrap();
    let triage = mutants::read_triage(dir.path()).unwrap();
    assert_eq!(
        triage.get("m1"),
        Some(&Triage::EquivalentProposed {
            why: "only a log line reads it".into()
        })
    );
    assert!(triage["m1"].is_the_coders_to_give());
    assert_eq!(triage["m1"].test(), None);
    assert_eq!(triage["m1"].kind(), "equivalent_proposed");

    std::fs::write(
        dir.path().join(mutants::TRIAGE_FILE),
        r#"{"m1": {"kind": "equivalent_proposed"}}"#,
    )
    .unwrap();
    // A proposal without its sentence is not read, and is named as one that
    // cannot be read (HQ review 3: an entry is refused, never the file).
    assert!(mutants::read_triage(dir.path()).unwrap().is_empty());
    let foreign = mutants::foreign(dir.path()).unwrap();
    assert_eq!(foreign.len(), 1);
    assert_eq!(foreign[0].kind, "equivalent_proposed");
    assert!(foreign[0].why.contains("why"), "{:?}", foreign[0]);
}

/// Old files keep reading: a campaign and a triage written before the
/// proposal existed read as they did, and a campaign with no refusal writes
/// no field for one.
#[test]
fn old_triage_and_campaign_files_still_read() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(mutants::FILE),
        r#"{"fingerprint":"f","head":"h","date":"d","survivors":[
            {"id":"m1","file":"src/lib.rs","line":3,"description":"replace one with 0",
             "outcome":{"kind":"equivalent","why":"same output"}},
            {"id":"m2","file":"src/lib.rs","line":4}]}"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join(mutants::TRIAGE_FILE),
        r#"{"m2": {"kind": "killed", "test": "one_is_one"}}"#,
    )
    .unwrap();
    let campaign = mutants::read(dir.path()).unwrap().unwrap();
    assert_eq!(campaign.survivors[0].refused, None);
    assert_eq!(campaign.survivors[1].refused, None);
    assert!(mutants::open(dir.path()).unwrap().is_empty());
    assert!(mutants::awaiting_ruling(dir.path()).unwrap().is_empty());

    mutants::write(dir.path(), &campaign).unwrap();
    let written = std::fs::read_to_string(dir.path().join(mutants::FILE)).unwrap();
    assert!(!written.contains("refused"), "{written}");
}

/// A survivor with a valid proposal is no longer open — it cannot be awaited
/// and proposed at once — and it awaits the HQ. A blank one leaves it open,
/// and awaits nothing.
#[test]
fn a_survivor_with_a_proposal_is_not_open_and_awaits_the_hq() {
    let dir = tempfile::tempdir().unwrap();
    ruled_before(
        dir.path(),
        vec![
            one("m1", 1, "a"),
            one("m2", 2, "b"),
            one("m3", 3, "c"),
            ruled(one("m4", 4, "d"), "the HQ's"),
        ],
    );
    proposes(
        dir.path(),
        &[
            ("m1", "only a log line reads it"),
            ("m2", "  "),
            ("m4", "the HQ's already"),
        ],
    );
    assert_eq!(mutants::open(dir.path()).unwrap(), vec!["m2", "m3"]);
    let waiting = mutants::awaiting_ruling(dir.path()).unwrap();
    assert_eq!(
        waiting,
        vec![mutants::Proposal {
            id: "m1".into(),
            file: "src/cells.py".into(),
            line: 1,
            why: "only a log line reads it".into(),
            from: None,
        }]
    );
    assert_eq!(
        mutants::proposals_await(&waiting).as_deref(),
        Some(
            "1 equivalence proposal(s) await the HQ's ruling (1 from the coder), and `nunki \
             push` refuses until each is ratified or refused"
        )
    );
    assert_eq!(mutants::proposals_await(&[]), None);
}

/// `--ratify` writes exactly what `--equivalent` writes: the same campaign
/// file, byte for byte, when the reason is the same — and the proposal is
/// gone from the coder's file, because it is a ruling now.
#[test]
fn ratifying_writes_the_same_equivalence_equivalent_writes() {
    let by_hand = tempfile::tempdir().unwrap();
    let ratified = tempfile::tempdir().unwrap();
    for dir in [by_hand.path(), ratified.path()] {
        ruled_before(dir, vec![one("m1", 1, "a"), one("m2", 2, "b")]);
    }
    mutants::rule_equivalent(by_hand.path(), "m1", "only a log line reads it").unwrap();
    proposes(
        ratified.path(),
        &[("m1", "only a log line reads it"), ("m2", "kept")],
    );
    let why = mutants::ratify(ratified.path(), "m1", None).unwrap();
    assert_eq!(why, "only a log line reads it");
    assert_eq!(
        std::fs::read_to_string(by_hand.path().join(mutants::FILE)).unwrap(),
        std::fs::read_to_string(ratified.path().join(mutants::FILE)).unwrap()
    );
    let left = mutants::read_triage(ratified.path()).unwrap();
    assert!(!left.contains_key("m1"), "{left:?}");
    assert!(left.contains_key("m2"), "only the ratified one is taken");

    // `--because` replaces the coder's sentence.
    let why = mutants::ratify(ratified.path(), "m2", Some("the HQ's own words")).unwrap();
    assert_eq!(why, "the HQ's own words");
    let campaign = mutants::read(ratified.path()).unwrap().unwrap();
    assert_eq!(
        campaign.survivors[1].outcome,
        Some(Triage::Equivalent {
            why: "the HQ's own words".into(),
            carried_from: None,
        })
    );
    assert!(
        mutants::awaiting_ruling(ratified.path())
            .unwrap()
            .is_empty()
    );
}

/// Only a proposal is ratified, and one with a reason: anything else is said.
#[test]
fn ratifying_needs_a_proposal_with_a_reason() {
    let dir = tempfile::tempdir().unwrap();
    ruled_before(dir.path(), vec![one("m1", 1, "a"), one("m2", 2, "b")]);
    proposes(dir.path(), &[("m2", " ")]);

    let err = mutants::ratify(dir.path(), "m1", None).unwrap_err();
    assert!(err.to_string().contains("proposes no equivalence"), "{err}");
    let err = mutants::ratify(dir.path(), "m2", None).unwrap_err();
    assert!(err.to_string().contains("gives no reason"), "{err}");
    proposes(dir.path(), &[("m2", "only a log line reads it")]);
    let err = mutants::ratify(dir.path(), "m2", Some(" \u{200b}")).unwrap_err();
    assert!(err.to_string().contains("blank"), "{err}");
    // Nothing was written by the refusals.
    let campaign = mutants::read(dir.path()).unwrap().unwrap();
    assert!(campaign.survivors.iter().all(|s| s.outcome.is_none()));
}

/// A ratified proposal is proposed again to the next campaign, from the
/// HQ's ruling (HQ review 4: carry never rules); a coder's proposal is
/// never carried — nothing it says lands in the HQ's file.
#[test]
fn a_ratified_proposal_is_proposed_again_and_a_coders_proposal_never_carried() {
    let dir = tempfile::tempdir().unwrap();
    let before = vec![one("m1", 1, "a"), one("m2", 2, "b")];
    ruled_before(dir.path(), before.clone());
    proposes(
        dir.path(),
        &[("m1", "only a log line reads it"), ("m2", "same constant")],
    );
    mutants::ratify(dir.path(), "m2", None).unwrap();

    // A commit above both moves their lines; the next campaign finds them.
    let moved: Vec<Survivor> = before
        .iter()
        .map(|s| Survivor {
            found_on: None,
            line: s.line + 5,
            end_line: None,
            ..s.clone()
        })
        .collect();
    mutants::record_finished(dir.path(), "new0000", "abc9999", &log_of(&moved)).unwrap();
    let campaign = mutants::read(dir.path()).unwrap().unwrap();
    assert_eq!(
        campaign.survivors[0].outcome, None,
        "a proposal is never carried"
    );
    assert_eq!(
        campaign.survivors[1].outcome,
        Some(Triage::ProposedByNunki {
            why: "same constant".into(),
            from: mutants::ProposedFrom::Carried {
                commit: "def5678".into()
            },
        })
    );
    // The coder's proposal is still in the coder's file: both await.
    let waiting = mutants::awaiting_ruling(dir.path()).unwrap();
    assert_eq!(
        waiting
            .iter()
            .map(|p| (p.id.as_str(), p.from.is_some()))
            .collect::<Vec<_>>(),
        vec![("m1", false), ("m2", true)]
    );
}

/// `--refuse` takes the proposal out of the coder's file, records the
/// refusal on the survivor, reopens it — and the next run reads why in
/// `FOLLOWUP_HQ.md`. A proposal written again on it is no outcome.
#[test]
fn refusing_reopens_the_survivor_and_the_next_run_reads_why() {
    let hq = tempfile::tempdir().unwrap();
    let paths = nunki::mission::dir::Paths::of(hq.path(), "m1");
    std::fs::create_dir_all(&paths.dir).unwrap();
    ruled_before(&paths.dir, vec![one("m1", 1, "a"), one("m2", 2, "b")]);
    proposes(
        &paths.dir,
        &[("m1", "only a log line reads it"), ("m2", "kept")],
    );
    assert_eq!(mutants::open(&paths.dir).unwrap(), Vec::<String>::new());

    mutants::refuse_and_say(&paths, "Arnaud", "m1", "the CLI prints that line").unwrap();

    assert_eq!(mutants::open(&paths.dir).unwrap(), vec!["m1"]);
    let triage = mutants::read_triage(&paths.dir).unwrap();
    assert!(!triage.contains_key("m1"), "{triage:?}");
    assert!(triage.contains_key("m2"), "only the refused one is taken");
    let campaign = mutants::read(&paths.dir).unwrap().unwrap();
    assert_eq!(
        campaign.survivors[0].refused,
        Some(mutants::Refusal {
            proposed: "only a log line reads it".into(),
            because: "the CLI prints that line".into(),
        })
    );
    assert_eq!(campaign.survivors[0].outcome, None);
    let followup = std::fs::read_to_string(&paths.followup).unwrap();
    assert!(
        followup.contains("Arnaud refused an equivalence"),
        "{followup}"
    );
    assert!(followup.contains("`m1`"), "{followup}");
    assert!(followup.contains("only a log line reads it"), "{followup}");
    assert!(followup.contains("the CLI prints that line"), "{followup}");

    // Proposed again: still open, and not awaiting the HQ.
    proposes(
        &paths.dir,
        &[("m1", "only a log line reads it, truly"), ("m2", "kept")],
    );
    assert_eq!(mutants::open(&paths.dir).unwrap(), vec!["m1"]);
    let waiting = mutants::awaiting_ruling(&paths.dir).unwrap();
    assert_eq!(
        waiting.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        vec!["m2"]
    );
    let survivor = &campaign.survivors[0];
    let coders = mutants::read_triage(&paths.dir).unwrap();
    assert_eq!(mutants::answer(survivor, &coders), None);
    assert!(
        mutants::not_an_outcome(survivor, &coders)
            .unwrap()
            .contains("the CLI prints that line")
    );

    // And a later ruling of the HQ's supersedes its refusal.
    mutants::ratify(&paths.dir, "m1", None).unwrap();
    let campaign = mutants::read(&paths.dir).unwrap().unwrap();
    assert_eq!(campaign.survivors[0].refused, None);
    assert!(mutants::open(&paths.dir).unwrap().is_empty());
}

/// A refusal says why, and is of a proposal: anything else is refused, and
/// nothing is written.
#[test]
fn refusing_needs_a_reason_and_a_proposal() {
    let hq = tempfile::tempdir().unwrap();
    let paths = nunki::mission::dir::Paths::of(hq.path(), "m1");
    std::fs::create_dir_all(&paths.dir).unwrap();
    ruled_before(&paths.dir, vec![one("m1", 1, "a"), one("m2", 2, "b")]);
    proposes(&paths.dir, &[("m1", "only a log line reads it")]);

    let err = mutants::refuse_and_say(&paths, "Arnaud", "m1", " \n").unwrap_err();
    assert!(err.to_string().contains("--because"), "{err}");
    let err = mutants::refuse_and_say(&paths, "Arnaud", "m2", "no").unwrap_err();
    assert!(err.to_string().contains("proposes no equivalence"), "{err}");
    let err = mutants::refuse_and_say(&paths, "Arnaud", "m9", "no").unwrap_err();
    assert!(err.to_string().contains("no survivor is called"), "{err}");
    assert!(
        !paths.followup.exists(),
        "nothing said for a refusal refused"
    );
    assert_eq!(mutants::awaiting_ruling(&paths.dir).unwrap().len(), 1);

    // A proposal on a survivor the campaign does not hold is said too.
    proposes(&paths.dir, &[("gone", "only a log line reads it")]);
    let err = mutants::refuse_and_say(&paths, "Arnaud", "gone", "no").unwrap_err();
    assert!(err.to_string().contains("no survivor is called"), "{err}");
}

/// A refusal is the HQ's, and is never read from a campaign's log: the log
/// is written inside the agent's container.
#[test]
fn a_refusal_written_in_the_campaigns_log_is_never_taken() {
    let line = r#"{"id":"m1","file":"a","line":1,"refused":{"proposed":"x","because":"y"}}"#;
    let parsed = mutants::parse(line);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].refused, None);
}

// ---------------------------------------------------------------------------
// Security round 1: what push re-checks, and survivors sharing an id.
// ---------------------------------------------------------------------------

fn campaign_of(survivors: Vec<Survivor>, tried: Option<u32>) -> Campaign {
    Campaign {
        chain: Default::default(),
        fingerprint: "f".into(),
        head: "h".into(),
        date: "d".into(),
        survivors,
        tried,
    }
}

/// Gate 7's rule, as push asks it again: nothing at prototype; at critical
/// every survivor answered, a pending proposal included; at standard the
/// share, judged as critical when the count is missing or cannot be true.
#[test]
fn what_gate_seven_still_owes_follows_its_rule_at_every_rigor() {
    use nunki::mission::Rigor;
    let three = || vec![one("m1", 1, "a"), one("m2", 2, "b"), one("m3", 3, "c")];
    let mut coders = std::collections::BTreeMap::new();
    coders.insert(
        "m1".to_string(),
        Triage::EquivalentProposed {
            why: "only a log line reads it".into(),
        },
    );
    coders.insert(
        "m2".to_string(),
        Triage::Killed {
            test: "two_holds".into(),
        },
    );

    // Critical: m3 is open, and only m3 is named.
    let owed = mutants::owed(
        &campaign_of(three(), Some(10)),
        &coders,
        Rigor::Critical,
        80,
    )
    .expect("m3 has no outcome");
    assert!(owed.contains("1 survivor(s) have no outcome"), "{owed}");
    assert!(owed.contains("src/cells.py:3 m3"), "{owed}");
    assert!(!owed.contains("m1") && !owed.contains("m2"), "{owed}");

    // Prototype owes nothing, whatever is open.
    assert_eq!(
        mutants::owed(
            &campaign_of(three(), Some(10)),
            &coders,
            Rigor::Prototype,
            80
        ),
        None
    );

    // Standard: 9 of 10 clears 80%, 9 of 10 does not clear 95%.
    assert_eq!(
        mutants::owed(
            &campaign_of(three(), Some(10)),
            &coders,
            Rigor::Standard,
            80
        ),
        None
    );
    assert_eq!(
        mutants::owed(
            &campaign_of(three(), Some(10)),
            &coders,
            Rigor::Standard,
            90
        ),
        None,
        "exactly at the threshold passes"
    );
    let owed = mutants::owed(
        &campaign_of(three(), Some(10)),
        &coders,
        Rigor::Standard,
        95,
    )
    .expect("90% is below 95%");
    assert!(owed.contains("9 of 10 tried mutant(s) killed"), "{owed}");
    assert!(owed.contains("threshold of 95%"), "{owed}");
    assert!(owed.contains("src/cells.py:3 m3"), "{owed}");

    // Standard with no count, or a count below the survivors: as critical.
    for tried in [None, Some(2)] {
        let owed = mutants::owed(&campaign_of(three(), tried), &coders, Rigor::Standard, 0)
            .unwrap_or_else(|| panic!("{tried:?} is judged as critical"));
        assert!(owed.contains("1 survivor(s) have no outcome"), "{owed}");
    }
    // Exactly as many tried as survivors is a count, and nothing killed of
    // three is 0%.
    let owed = mutants::owed(
        &campaign_of(three(), Some(3)),
        &std::collections::BTreeMap::new(),
        Rigor::Standard,
        1,
    )
    .expect("0 of 3 is below 1%");
    assert!(owed.contains("0 of 3"), "{owed}");
    // A campaign that tried nothing owes nothing at standard.
    assert_eq!(
        mutants::owed(&campaign_of(vec![], Some(0)), &coders, Rigor::Standard, 80),
        None
    );

    // Everything answered: nothing owed at critical.
    coders.insert(
        "m3".to_string(),
        Triage::Bug {
            test: "three_is_wrong".into(),
        },
    );
    assert_eq!(
        mutants::owed(&campaign_of(three(), None), &coders, Rigor::Critical, 80),
        None
    );
}

/// On the mission folder: no campaign owes nothing, and one with an open
/// survivor says so.
#[test]
fn what_is_owed_is_read_from_the_mission_folder() {
    use nunki::mission::Rigor;
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        mutants::owed_on_file(dir.path(), Rigor::Critical, 80).unwrap(),
        None
    );
    ruled_before(dir.path(), vec![one("m1", 1, "a")]);
    assert!(
        mutants::owed_on_file(dir.path(), Rigor::Critical, 80)
            .unwrap()
            .unwrap()
            .contains("m1")
    );
    proposes(dir.path(), &[("m1", "only a log line reads it")]);
    assert_eq!(
        mutants::owed_on_file(dir.path(), Rigor::Critical, 80).unwrap(),
        None
    );
}

/// The share is compared in whole numbers.
#[test]
fn the_share_is_reached_only_at_or_above_the_threshold() {
    assert!(mutants::share_reached(8, 10, 80));
    assert!(!mutants::share_reached(7, 9, 78), "77.7% is not 78%");
    assert!(mutants::share_reached(0, 0, 80));
}

/// One proposal answers every survivor carrying its id, so the HQ's ruling
/// and its refusal land on every one of them — and lifting a ruling lifts
/// it from all. With the first only, the twin stayed answered by a proposal
/// nobody ruled on, and push let it through.
#[test]
fn ratify_refuse_and_lift_act_on_every_survivor_sharing_the_id() {
    let twins = || vec![one("X", 1, "a"), one("X", 2, "b"), one("Y", 3, "c")];

    let dir = tempfile::tempdir().unwrap();
    ruled_before(dir.path(), twins());
    proposes(dir.path(), &[("X", "only a log line reads it")]);
    mutants::ratify(dir.path(), "X", None).unwrap();
    let campaign = mutants::read(dir.path()).unwrap().unwrap();
    for s in &campaign.survivors[..2] {
        assert_eq!(
            s.outcome,
            Some(Triage::Equivalent {
                why: "only a log line reads it".into(),
                carried_from: None,
            }),
            "{s:?}"
        );
    }
    assert_eq!(campaign.survivors[2].outcome, None, "Y is not X");
    assert!(mutants::awaiting_ruling(dir.path()).unwrap().is_empty());
    assert_eq!(mutants::open(dir.path()).unwrap(), vec!["Y"]);

    mutants::lift_equivalent(dir.path(), "X").unwrap();
    let campaign = mutants::read(dir.path()).unwrap().unwrap();
    assert!(campaign.survivors.iter().all(|s| s.outcome.is_none()));

    let dir = tempfile::tempdir().unwrap();
    ruled_before(dir.path(), twins());
    proposes(dir.path(), &[("X", "only a log line reads it")]);
    mutants::refuse(dir.path(), "X", "the CLI prints it").unwrap();
    let campaign = mutants::read(dir.path()).unwrap().unwrap();
    for s in &campaign.survivors[..2] {
        assert!(s.refused.is_some(), "{s:?}");
    }
    assert_eq!(campaign.survivors[2].refused, None);
    assert_eq!(mutants::open(dir.path()).unwrap(), vec!["X", "X", "Y"]);

    // A ruling on an id nobody carries is still refused.
    let err = mutants::rule_equivalent(dir.path(), "Z", "no").unwrap_err();
    assert!(err.to_string().contains("no survivor is called"), "{err}");
    let err = mutants::lift_equivalent(dir.path(), "Z").unwrap_err();
    assert!(err.to_string().contains("no survivor is called"), "{err}");
}

// ---------------------------------------------------------------------------
// HQ review of the pull request.
// ---------------------------------------------------------------------------

fn refused(mut survivor: Survivor, because: &str) -> Survivor {
    survivor.refused = Some(mutants::Refusal {
        proposed: "only a log line reads it".into(),
        because: because.into(),
    });
    survivor
}

/// A refusal survives a new campaign on the same two tiers as a ruling: the
/// same proposal written again on the same mutation after a replay is still
/// no outcome. Where the twin cannot be told apart, nothing is carried.
#[test]
fn a_refusal_follows_its_mutant_to_the_next_campaign_like_a_ruling() {
    let dir = tempfile::tempdir().unwrap();
    ruled_before(
        dir.path(),
        vec![
            refused(one("m1", 1, "a"), "the CLI prints it"),
            refused(one("old-name", 2, "b"), "a test can see it"),
            refused(one("d1", 3, "dup"), "twice in one file"),
            one("m4", 4, "c"),
        ],
    );
    // A commit above everything; `old-name` is renamed; `dup` now names two
    // survivors, and the new one, `d2`, has no twin of its own id.
    let now = vec![
        one("m1", 11, "a"),
        one("new-name", 12, "b"),
        one("d1", 13, "dup"),
        one("d2", 14, "dup"),
        one("m4", 15, "c"),
    ];
    mutants::record_finished(dir.path(), "new0000", "abc9999", &log_of(&now)).unwrap();
    let campaign = mutants::read(dir.path()).unwrap().unwrap();
    let because = |i: usize| {
        campaign.survivors[i]
            .refused
            .as_ref()
            .map(|r| r.because.as_str())
    };
    assert_eq!(
        because(0),
        Some("the CLI prints it"),
        "same id, file and mutation"
    );
    assert_eq!(
        because(1),
        Some("a test can see it"),
        "one twin by file and mutation"
    );
    assert_eq!(
        because(2),
        Some("twice in one file"),
        "the same id is exact"
    );
    // A refusal is always carried (HQ review 2, C): carrying one too many
    // reopens a survivor; carrying one too few lets a refused proposal count
    // as an outcome again.
    assert_eq!(
        because(3),
        Some("twice in one file"),
        "a refusal on the same file and mutation reaches a new id, ambiguous or not"
    );
    assert_eq!(because(4), None, "never refused, nothing to carry");
    assert!(campaign.survivors.iter().all(|s| s.outcome.is_none()));

    // The same proposal written again: still no outcome, still open.
    proposes(
        dir.path(),
        &[("m1", "only a log line reads it"), ("new-name", "same")],
    );
    assert!(mutants::awaiting_ruling(dir.path()).unwrap().is_empty());
    assert_eq!(
        mutants::open(dir.path()).unwrap(),
        vec!["m1", "new-name", "d1", "d2", "m4"]
    );
}

/// A proposal and a refusal never both land: a survivor whose twin was
/// ruled is proposed the ruling, and the refusal list is not read for it.
#[test]
fn a_carried_ruling_is_not_also_carried_a_refusal() {
    let dir = tempfile::tempdir().unwrap();
    ruled_before(
        dir.path(),
        vec![
            ruled(one("m1", 1, "a"), "same output"),
            refused(one("m2", 2, "a"), "the CLI prints it"),
        ],
    );
    let now = vec![one("m1", 5, "a")];
    mutants::record_finished(dir.path(), "new0000", "abc9999", &log_of(&now)).unwrap();
    let campaign = mutants::read(dir.path()).unwrap().unwrap();
    assert!(
        matches!(
            campaign.survivors[0].outcome,
            Some(Triage::ProposedByNunki { .. })
        ),
        "{:?}",
        campaign.survivors[0]
    );
    assert_eq!(campaign.survivors[0].refused, None);
}

/// A refusal cannot undo the HQ's own ruling: refusing a survivor that holds
/// one is an error that points at `--lift`, and nothing is written.
#[test]
fn refusing_a_survivor_the_hq_already_ruled_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    ruled_before(
        dir.path(),
        vec![ruled(one("X", 1, "a"), "same output"), one("X", 2, "b")],
    );
    proposes(dir.path(), &[("X", "only a log line reads it")]);
    let before = std::fs::read_to_string(dir.path().join(mutants::FILE)).unwrap();
    let err = mutants::refuse(dir.path(), "X", "the CLI prints it").unwrap_err();
    assert!(err.to_string().contains("--lift X"), "{err}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join(mutants::FILE)).unwrap(),
        before
    );
    assert!(mutants::read_triage(dir.path()).unwrap().contains_key("X"));
}

// The project's registry of equivalences: a ruling given on one mission is
// applied to every later campaign while the line it sat on is unchanged.

use nunki::equivalences::{self, Registered};

const LIFTED: &str = "replace one -> u8 with 0";

/// A project whose repository holds `src/lib.rs` as `body`, committed; the
/// mission folders live under its `hq/`.
fn registry_project(dir: &Path, body: &str) -> nunki::project::Project {
    let (project, _) = context(dir);
    write(&project.root, "src/lib.rs", body);
    git(
        &project.root,
        &["commit", "-q", "-am", "the code under test"],
    );
    project
}

/// Commit `body` as `src/lib.rs` and say which commit holds it.
fn commit_lib(project: &nunki::project::Project, body: &str) -> String {
    write(&project.root, "src/lib.rs", body);
    git(&project.root, &["commit", "-q", "-am", "change"]);
    git(&project.root, &["rev-parse", "HEAD"])
}

fn mission_dir(project: &nunki::project::Project, id: &str) -> PathBuf {
    let dir = nunki::mission::dir::Paths::of(&project.hq_root, id).dir;
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The survivor a campaign names on `line` of `src/lib.rs`.
fn on_lib(line: u32) -> Survivor {
    Survivor {
        found_on: None,
        id: format!("src/lib.rs:{line}:5: {LIFTED}"),
        file: "src/lib.rs".into(),
        line,
        end_line: None,
        description: LIFTED.into(),
        outcome: None,
        refused: None,
    }
}

/// A campaign of mission `id` on the repository's `HEAD`, read back the way
/// a finished campaign is.
fn campaign_on(
    project: &nunki::project::Project,
    id: &str,
    survivors: &[Survivor],
) -> (Campaign, mutants::Recorded) {
    let dir = mission_dir(project, id);
    let head = git(&project.root, &["rev-parse", "HEAD"]);
    let recorded = mutants::record_finished_with_registry(
        &dir,
        &project.hq_root,
        &project.root,
        &format!("fp-{head}"),
        &head,
        &log_of(survivors),
        &mutants::Chain::default(),
    )
    .unwrap();
    (mutants::read(&dir).unwrap().unwrap(), recorded)
}

const BODY: &str = "pub fn one() -> u8 {\n    1\n}\n";
const MOVED: &str = "pub fn zero() -> u8 {\n    0\n}\n\npub fn one() -> u8 {\n    1\n}\n";

/// Mission A's campaign, and the HQ's ruling on its one survivor.
fn ruled_on_a(project: &nunki::project::Project) -> String {
    let (campaign, _) = campaign_on(project, "a", &[on_lib(2)]);
    let id = campaign.survivors[0].id.clone();
    let registered =
        nunki::findings::rule_equivalent(project, "a", &id, "nothing reads the value").unwrap();
    assert_eq!(registered, Registered::Done(1));
    id
}

#[test]
fn a_mutant_ruled_on_one_mission_is_not_asked_again_on_the_next() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    let head_a = git(&project.root, &["rev-parse", "HEAD"]);
    ruled_on_a(&project);

    // The entry: the mutation, the line's digest, the ruling and its origin.
    let registry = equivalences::read(&project.hq_root).unwrap();
    assert_eq!(registry.entries.len(), 1);
    let entry = &registry.entries[0];
    assert_eq!(entry.file, "src/lib.rs");
    assert_eq!(entry.description, LIFTED);
    assert_eq!(entry.line, equivalences::line_digest("1"));
    assert_eq!(entry.why, "nothing reads the value");
    assert_eq!(entry.mission, "a");
    assert_eq!(entry.commit, head_a);
    assert!(!entry.by.is_empty() && !entry.date.is_empty());

    // Mission B: another branch, the same line, the same mutation.
    git(&project.root, &["checkout", "-q", "-b", "mission/b"]);
    commit_lib(&project, &format!("{BODY}// unrelated\n"));
    let (campaign, recorded) = campaign_on(&project, "b", &[on_lib(2)]);
    assert_eq!(recorded.from_registry, 1);
    assert_eq!(recorded.registry_unread, None);
    // A proposal of the HQ's own ruling, never the ruling (HQ review 2).
    assert_eq!(
        campaign.survivors[0].outcome,
        Some(Triage::ProposedByNunki {
            why: "nothing reads the value".into(),
            from: mutants::ProposedFrom::Registry {
                mission: "a".into(),
                commit: head_a.clone(),
                date: entry.date.clone(),
            },
        })
    );
    let b = mission_dir(&project, "b");
    assert!(
        mutants::open(&b).unwrap().is_empty(),
        "B's coder is not asked again"
    );
    let waiting = mutants::awaiting_ruling(&b).unwrap();
    assert_eq!(waiting.len(), 1, "the HQ is, in one word");
    let status = mutants::proposals_await(&waiting).unwrap();
    assert!(status.contains("1 from the registry"), "{status}");
    assert!(waiting[0].source().contains("ruled on mission a"));
}

#[test]
fn a_line_that_only_moved_still_matches() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    ruled_on_a(&project);
    // Code added above it: the line is now the sixth.
    commit_lib(&project, MOVED);
    let (campaign, recorded) = campaign_on(&project, "b", &[on_lib(6)]);
    assert_eq!(recorded.from_registry, 1);
    assert!(matches!(
        campaign.survivors[0].outcome,
        Some(Triage::ProposedByNunki {
            from: mutants::ProposedFrom::Registry { .. },
            ..
        })
    ));
}

#[test]
fn after_its_line_changes_the_same_mutation_is_open_again() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    ruled_on_a(&project);
    // The same mutation, on a line whose content changed.
    commit_lib(&project, "pub fn one() -> u8 {\n    1 + 0\n}\n");
    let (campaign, recorded) = campaign_on(&project, "b", &[on_lib(2)]);
    assert_eq!(recorded.from_registry, 0);
    assert_eq!(campaign.survivors[0].outcome, None);
    assert_eq!(
        mutants::open(&mission_dir(&project, "b")).unwrap().len(),
        1,
        "the ruling was about other code"
    );
    assert!(
        mutants::awaiting_ruling(&mission_dir(&project, "b"))
            .unwrap()
            .is_empty()
    );

    // Whitespace at both ends is not content: a re-indented line still
    // matches.
    commit_lib(&project, "pub fn one() -> u8 {\n\t\t1  \n}\n");
    let (_, recorded) = campaign_on(&project, "c", &[on_lib(2)]);
    assert_eq!(recorded.from_registry, 1);
}

#[test]
fn a_line_that_cannot_be_read_matches_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    ruled_on_a(&project);
    let head = git(&project.root, &["rev-parse", "HEAD"]);
    // Out of range, and line zero.
    assert_eq!(
        equivalences::digest_at(&project.root, &head, "src/lib.rs", 99),
        None
    );
    assert_eq!(
        equivalences::digest_at(&project.root, &head, "src/lib.rs", 0),
        None
    );
    let (campaign, _) = campaign_on(&project, "b", &[on_lib(99)]);
    assert_eq!(campaign.survivors[0].outcome, None);

    // The file gone.
    git(&project.root, &["rm", "-q", "src/lib.rs"]);
    git(&project.root, &["commit", "-q", "-m", "gone"]);
    let head = git(&project.root, &["rev-parse", "HEAD"]);
    assert_eq!(
        equivalences::digest_at(&project.root, &head, "src/lib.rs", 2),
        None
    );
    let (campaign, _) = campaign_on(&project, "c", &[on_lib(2)]);
    assert_eq!(campaign.survivors[0].outcome, None);
    // And a commit the repository does not hold.
    assert_eq!(
        equivalences::digest_at(&project.root, "0123456789abcdef", "src/lib.rs", 2),
        None
    );
}

#[test]
fn two_same_mutations_in_the_file_apply_nothing_on_either_side() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    ruled_on_a(&project);

    // The campaign's side: the same mutation twice in the file.
    commit_lib(
        &project,
        "pub fn one() -> u8 {\n    1\n}\npub fn uno() -> u8 {\n    1\n}\n",
    );
    let (campaign, recorded) = campaign_on(&project, "b", &[on_lib(2), on_lib(5)]);
    assert_eq!(recorded.from_registry, 0);
    assert!(campaign.survivors.iter().all(|s| s.outcome.is_none()));

    // The registry's side: two entries on the same file and description.
    let mut registry = equivalences::read(&project.hq_root).unwrap();
    let mut twin = registry.entries[0].clone();
    twin.line = equivalences::line_digest("2");
    twin.mission = "z".into();
    registry.entries.push(twin);
    equivalences::write(&project.hq_root, &registry).unwrap();
    commit_lib(&project, BODY);
    let (campaign, recorded) = campaign_on(&project, "c", &[on_lib(2)]);
    assert_eq!(recorded.from_registry, 0);
    assert_eq!(campaign.survivors[0].outcome, None);

    // And with one of them gone, the other applies again: it was the
    // ambiguity, not the entry, that held it back.
    registry.entries.pop();
    equivalences::write(&project.hq_root, &registry).unwrap();
    let (_, recorded) = campaign_on(&project, "d", &[on_lib(2)]);
    assert_eq!(recorded.from_registry, 1);
}

#[test]
fn lifting_on_a_later_mission_takes_the_ruling_out_and_the_next_is_asked_again() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    ruled_on_a(&project);
    let (campaign, _) = campaign_on(&project, "b", &[on_lib(2)]);
    let id = campaign.survivors[0].id.clone();

    let registered = nunki::findings::lift_equivalent(&project, "b", &id).unwrap();
    assert_eq!(registered, Registered::Done(1));
    assert!(
        equivalences::read(&project.hq_root)
            .unwrap()
            .entries
            .is_empty()
    );
    let b = mission_dir(&project, "b");
    assert_eq!(
        mutants::open(&b).unwrap(),
        vec![id.clone()],
        "B needs an outcome"
    );

    let (campaign, recorded) = campaign_on(&project, "c", &[on_lib(2)]);
    assert_eq!(recorded.from_registry, 0);
    assert_eq!(campaign.survivors[0].outcome, None, "C is asked again");
}

#[test]
fn lifting_on_the_mission_that_ruled_takes_the_ruling_out_too() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    let id = ruled_on_a(&project);
    // Carried once inside mission A onto a line whose content has changed
    // since: the entry is found by where the ruling was given, not by the
    // line as it stands.
    commit_lib(&project, "pub fn one() -> u8 {\n    1 + 0\n}\n");
    let (campaign, _) = campaign_on(&project, "a", &[on_lib(2)]);
    // Its code changed under the same id: proposed, not carried (HQ review
    // 3, item 4).
    assert!(matches!(
        campaign.survivors[0].outcome,
        Some(Triage::ProposedByNunki {
            from: mutants::ProposedFrom::Carried { .. },
            ..
        })
    ));

    let registered = nunki::findings::lift_equivalent(&project, "a", &id).unwrap();
    assert_eq!(registered, Registered::Done(1));
    assert!(
        equivalences::read(&project.hq_root)
            .unwrap()
            .entries
            .is_empty()
    );
}

/// A ruling lifted is a ruling on the mutation of that code: an entry on
/// the same line, whoever entered it since, goes with it.
#[test]
fn lifting_takes_out_an_entry_on_the_same_line_given_elsewhere() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    let id = ruled_on_a(&project);
    let (campaign, _) = campaign_on(&project, "b", &[on_lib(2)]);
    let id_b = campaign.survivors[0].id.clone();
    nunki::findings::rule_equivalent(&project, "b", &id_b, "ruled again on b").unwrap();
    assert_eq!(
        equivalences::read(&project.hq_root).unwrap().entries[0].mission,
        "b"
    );

    let registered = nunki::findings::lift_equivalent(&project, "a", &id).unwrap();
    assert_eq!(registered, Registered::Done(1));
    assert!(
        equivalences::read(&project.hq_root)
            .unwrap()
            .entries
            .is_empty()
    );
}

#[test]
fn a_ruling_from_the_registry_is_not_carried_and_is_asked_of_the_registry_again() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    ruled_on_a(&project);
    campaign_on(&project, "b", &[on_lib(2)]);
    // The registry no longer holds it — removed by hand, say: the next
    // campaign of B does not keep it on the strength of the last one.
    equivalences::write(&project.hq_root, &equivalences::Registry::default()).unwrap();
    commit_lib(&project, MOVED);
    let (campaign, _) = campaign_on(&project, "b", &[on_lib(6)]);
    assert_eq!(campaign.survivors[0].outcome, None);
}

#[test]
fn a_proposal_and_a_refusal_never_reach_the_registry() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    let (campaign, _) = campaign_on(&project, "a", &[on_lib(2)]);
    let id = campaign.survivors[0].id.clone();
    let a = mission_dir(&project, "a");

    proposes(&a, &[(&id, "nothing reads the value")]);
    assert!(!equivalences::path(&project.hq_root).exists());
    nunki::findings::refuse_proposal(&project, "a", &id, "the value is returned").unwrap();
    assert!(!equivalences::path(&project.hq_root).exists());

    // And a survivor refused on this mission is not answered by the
    // registry either: the HQ said no to it here.
    let (campaign, _) = campaign_on(&project, "b", &[on_lib(2)]);
    let id_b = campaign.survivors[0].id.clone();
    nunki::findings::rule_equivalent(&project, "b", &id_b, "nothing reads the value").unwrap();
    let (campaign, recorded) = campaign_on(&project, "a", &[on_lib(2)]);
    assert!(campaign.survivors[0].refused.is_some());
    assert_eq!(campaign.survivors[0].outcome, None);
    assert_eq!(recorded.from_registry, 0);

    // A ratified proposal is a ruling, and is entered.
    equivalences::write(&project.hq_root, &equivalences::Registry::default()).unwrap();
    let (campaign, _) = campaign_on(&project, "c", &[on_lib(2)]);
    let c = mission_dir(&project, "c");
    let id_c = campaign.survivors[0].id.clone();
    assert_eq!(campaign.survivors[0].outcome, None);
    proposes(&c, &[(&id_c, "nothing reads the value")]);
    let (_, registered) = nunki::findings::ratify_proposal(&project, "c", &id_c, None).unwrap();
    assert_eq!(registered, Registered::Done(1));
    assert_eq!(
        equivalences::read(&project.hq_root).unwrap().entries[0].mission,
        "c"
    );
}

#[test]
fn an_unreadable_registry_applies_nothing_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    ruled_on_a(&project);
    let file = equivalences::path(&project.hq_root);
    std::fs::write(&file, "{ this is not a registry").unwrap();

    let (campaign, recorded) = campaign_on(&project, "b", &[on_lib(2)]);
    assert_eq!(campaign.survivors[0].outcome, None);
    assert_eq!(recorded.from_registry, 0);
    let why = recorded.registry_unread.expect("it is reported");
    assert!(why.contains("equivalences.json"), "{why}");
    let followup =
        std::fs::read_to_string(nunki::mission::dir::Paths::of(&project.hq_root, "b").followup)
            .unwrap();
    assert!(
        followup.contains("the registry of equivalences could not be read"),
        "{followup}"
    );

    // A ruling given meanwhile stands on its mission, says the registry was
    // left alone, and does not write over the file it could not read.
    let id = campaign.survivors[0].id.clone();
    match nunki::findings::rule_equivalent(&project, "b", &id, "still").unwrap() {
        Registered::Not(why) => assert!(why.contains("equivalences.json"), "{why}"),
        other => panic!("an unreadable registry was written: {other:?}"),
    }
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "{ this is not a registry"
    );
    assert!(
        mutants::open(&mission_dir(&project, "b"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn with_no_registry_a_campaign_is_recorded_as_before() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    let (campaign, recorded) = campaign_on(&project, "a", &[on_lib(2)]);
    assert_eq!(
        recorded,
        mutants::Recorded {
            survivors: 1,
            from_registry: 0,
            registry_unread: None,
        }
    );
    assert_eq!(campaign.survivors[0].outcome, None);
    assert!(!equivalences::path(&project.hq_root).exists());
    assert!(
        !nunki::mission::dir::Paths::of(&project.hq_root, "a")
            .followup
            .exists(),
        "nothing to report"
    );
}

/// The digest is the one the campaign fingerprint uses — git's blob id —
/// of the line with its ends trimmed, so a human can check an entry with
/// one command.
#[test]
fn the_line_digest_is_gits_blob_id_of_the_trimmed_line() {
    let dir = tempfile::tempdir().unwrap();
    let tree = repo(dir.path());
    let by_git = |text: &str| {
        let mut child = Command::new("git")
            .arg("-C")
            .arg(&tree)
            .args(["hash-object", "--stdin"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
        String::from_utf8(child.wait_with_output().unwrap().stdout)
            .unwrap()
            .trim()
            .to_string()
    };
    assert_eq!(
        equivalences::line_digest("    return a > b;  "),
        by_git("return a > b;")
    );
    assert_eq!(equivalences::line_digest(""), by_git(""));
    assert_ne!(
        equivalences::line_digest("return a > b;"),
        equivalences::line_digest("return a >= b;")
    );
}

#[test]
fn the_registry_listing_says_whether_each_line_still_stands() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    ruled_on_a(&project);
    let registry = equivalences::read(&project.hq_root).unwrap();
    let listing = equivalences::listing(&registry, &project.root);
    assert!(listing.contains("src/lib.rs"), "{listing}");
    assert!(listing.contains(LIFTED), "{listing}");
    assert!(listing.contains("nothing reads the value"), "{listing}");
    assert!(listing.contains("mission a"), "{listing}");
    assert!(listing.contains("its code stands at HEAD"), "{listing}");

    commit_lib(&project, MOVED);
    assert_eq!(
        equivalences::standing(&project.root, &registry.entries[0]),
        equivalences::Standing::Stands,
        "a line that moved still stands"
    );
    commit_lib(&project, "pub fn one() -> u8 {\n    2\n}\n");
    assert!(
        equivalences::listing(&registry, &project.root).contains("its code has changed"),
        "the line changed"
    );
    git(&project.root, &["rm", "-q", "src/lib.rs"]);
    git(&project.root, &["commit", "-q", "-m", "gone"]);
    assert_eq!(
        equivalences::standing(&project.root, &registry.entries[0]),
        equivalences::Standing::Gone
    );
    assert!(
        equivalences::listing(&equivalences::Registry::default(), &project.root)
            .contains("holds no ruling")
    );
}

/// What the registry gives is a proposal, `nunki`'s and never the coder's to
/// write: the HQ refuses it like any other, and the survivor is open again
/// with the refusal on it (HQ review 2).
#[test]
fn a_proposal_from_the_registry_is_refused_like_any_proposal() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    ruled_on_a(&project);
    let (campaign, _) = campaign_on(&project, "b", &[on_lib(2)]);
    let id = campaign.survivors[0].id.clone();
    let outcome = campaign.survivors[0].outcome.clone().unwrap();
    assert!(!outcome.is_the_coders_to_give());
    assert!(!outcome.is_a_ruling());
    assert_eq!(outcome.kind(), "proposed_by_nunki");
    assert_eq!(outcome.test(), None);

    let b = mission_dir(&project, "b");
    let refusal = mutants::refuse(&b, &id, "it is printed").unwrap();
    assert_eq!(refusal.proposed, "nothing reads the value");
    let campaign = mutants::read(&b).unwrap().unwrap();
    assert_eq!(campaign.survivors[0].outcome, None);
    assert_eq!(mutants::open(&b).unwrap(), vec![id.clone()]);
    // And the next campaign of B neither proposes it again nor drops the
    // refusal.
    let (campaign, _) = campaign_on(&project, "b", &[on_lib(2)]);
    assert_eq!(campaign.survivors[0].outcome, None);
    assert!(campaign.survivors[0].refused.is_some());
}

/// Ruling again on the same mutation replaces the entry rather than adding
/// a second one, which would make every later match ambiguous.
#[test]
fn a_mutation_ruled_again_replaces_its_entry() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    ruled_on_a(&project);
    commit_lib(&project, "pub fn one() -> u8 {\n    1 + 0\n}\n");
    let (campaign, _) = campaign_on(&project, "b", &[on_lib(2)]);
    let id = campaign.survivors[0].id.clone();
    nunki::findings::rule_equivalent(&project, "b", &id, "still nothing reads it").unwrap();
    let registry = equivalences::read(&project.hq_root).unwrap();
    assert_eq!(registry.entries.len(), 1);
    assert_eq!(registry.entries[0].mission, "b");
    assert_eq!(registry.entries[0].line, equivalences::line_digest("1 + 0"));
}

/// Only a mission that has not started is ruled on without its slot's
/// lock. A state that cannot be read is not that: the lock it would name is
/// unknown, and a run or a read-back may be writing the same files, so every
/// ruling is refused and nothing is written.
#[test]
fn a_ruling_against_an_unreadable_state_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    let (campaign, _) = campaign_on(&project, "a", &[on_lib(2)]);
    let id = campaign.survivors[0].id.clone();
    let a = mission_dir(&project, "a");
    proposes(&a, &[(&id, "nothing reads the value")]);
    let before = std::fs::read_to_string(a.join(mutants::FILE)).unwrap();

    let states = project.hq_root.join("state").join("missions");
    std::fs::create_dir_all(&states).unwrap();
    std::fs::write(states.join("a.json"), "{ not a state").unwrap();

    let refused = |what: &str, err: nunki::findings::FindingsError| {
        let said = err.to_string();
        assert!(said.contains("a.json"), "{what}: {said}");
    };
    refused(
        "--equivalent",
        nunki::findings::rule_equivalent(&project, "a", &id, "nothing reads it").unwrap_err(),
    );
    refused(
        "--ratify",
        nunki::findings::ratify_proposal(&project, "a", &id, None).unwrap_err(),
    );
    refused(
        "--refuse",
        nunki::findings::refuse_proposal(&project, "a", &id, "it is returned").unwrap_err(),
    );
    refused(
        "--lift",
        nunki::findings::lift_equivalent(&project, "a", &id).unwrap_err(),
    );
    assert_eq!(
        std::fs::read_to_string(a.join(mutants::FILE)).unwrap(),
        before,
        "no ruling was written"
    );
    assert!(!equivalences::path(&project.hq_root).exists());
    assert_eq!(
        mutants::read_triage(&a).unwrap().len(),
        1,
        "the proposal stands"
    );

    // The same mission with no state at all has not started, and is ruled
    // on as before.
    std::fs::remove_file(states.join("a.json")).unwrap();
    nunki::findings::rule_equivalent(&project, "a", &id, "nothing reads it").unwrap();
}

/// m1 was ruled equivalent and m2's proposal refused, the same mutation in
/// the same file. The new campaign holds only m2: it is m2 by its id, so it
/// keeps its refusal — and the ruling given on m1 is never carried onto it,
/// though among the ruled survivors alone the pair looked unique.
#[test]
fn a_ruling_on_one_of_two_same_mutations_never_lands_on_the_refused_one() {
    let dir = tempfile::tempdir().unwrap();
    let refusal = mutants::Refusal {
        proposed: "the format is never read".into(),
        because: "it is printed".into(),
    };
    let mut m2 = one("m2", 12, F_TO_UPPER);
    m2.refused = Some(refusal.clone());
    ruled_before(
        dir.path(),
        vec![ruled(one("m1", 7, F_TO_UPPER), "the cell is empty"), m2],
    );
    mutants::record_finished(
        dir.path(),
        "new0000",
        "abc9999",
        &log_of(&[one("m2", 14, F_TO_UPPER)]),
    )
    .unwrap();
    let now = mutants::read(dir.path()).unwrap().unwrap();
    assert_eq!(now.survivors.len(), 1);
    assert_eq!(now.survivors[0].outcome, None, "m1's ruling is not m2's");
    assert_eq!(now.survivors[0].refused, Some(refusal));
}

/// The mirror: m2 was merely killed before — nothing on it in the HQ's
/// file. The new campaign holds only m2, and m1's ruling still does not
/// land on it: the pair named two survivors of the previous campaign.
#[test]
fn a_ruling_on_one_of_two_same_mutations_never_lands_on_the_killed_one() {
    let dir = tempfile::tempdir().unwrap();
    ruled_before(
        dir.path(),
        vec![
            ruled(one("m1", 7, F_TO_UPPER), "the cell is empty"),
            one("m2", 12, F_TO_UPPER),
        ],
    );
    // m2 under its own id, then under a new one: neither tier speaks for it.
    for id in ["m2", "m9"] {
        mutants::record_finished(
            dir.path(),
            "new0000",
            "abc9999",
            &log_of(&[one(id, 14, F_TO_UPPER)]),
        )
        .unwrap();
        let now = mutants::read(dir.path()).unwrap().unwrap();
        assert_eq!(now.survivors[0].outcome, None, "{id}");
        assert_eq!(now.survivors[0].refused, None, "{id}");
        // Back to the campaign the ruling was given on, for the next id.
        ruled_before(
            dir.path(),
            vec![
                ruled(one("m1", 7, F_TO_UPPER), "the cell is empty"),
                one("m2", 12, F_TO_UPPER),
            ],
        );
    }
}

/// The slot's repository is written by the coder's container. A replace
/// ref that hands back the ruled file for a changed one must change
/// nothing: the line reads as changed, the survivor stays open, and the
/// listing says the line has changed (security round 1, MEDIUM).
#[test]
fn a_replace_ref_cannot_make_a_changed_line_read_as_the_ruled_one() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    let ruled_blob = git(&project.root, &["rev-parse", "HEAD:src/lib.rs"]);
    ruled_on_a(&project);
    commit_lib(&project, "pub fn one() -> u8 {\n    1 + 0\n}\n");
    let changed_blob = git(&project.root, &["rev-parse", "HEAD:src/lib.rs"]);
    git(&project.root, &["replace", &changed_blob, &ruled_blob]);
    // The forgery holds for git as it is configured: it is what a reader
    // that trusts the repository would see.
    assert_eq!(
        git(&project.root, &["show", "HEAD:src/lib.rs"]),
        BODY.trim()
    );

    let (campaign, recorded) = campaign_on(&project, "b", &[on_lib(2)]);
    assert_eq!(recorded.from_registry, 0);
    assert_eq!(campaign.survivors[0].outcome, None);
    let entry = &equivalences::read(&project.hq_root).unwrap().entries[0];
    assert_eq!(
        equivalences::standing(&project.root, entry),
        equivalences::Standing::Changed
    );
    // A replaced commit reads as itself too.
    let head = git(&project.root, &["rev-parse", "HEAD"]);
    let first = git(&project.root, &["rev-parse", "HEAD~1"]);
    git(&project.root, &["replace", "-f", &head, &first]);
    assert_eq!(
        equivalences::digest_at(&project.root, &head, "src/lib.rs", 2),
        Some(equivalences::line_digest("1 + 0"))
    );
}

/// The objects git hands back are hashed and compared with the id they
/// were asked by: a loose object rewritten on disk to hold the ruled
/// content is not read as it, and matches nothing.
#[test]
fn an_object_rewritten_on_disk_is_not_read_as_what_it_was_made_to_say() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    let ruled_blob = git(&project.root, &["rev-parse", "HEAD:src/lib.rs"]);
    ruled_on_a(&project);
    commit_lib(&project, "pub fn one() -> u8 {\n    1 + 0\n}\n");
    let changed_blob = git(&project.root, &["rev-parse", "HEAD:src/lib.rs"]);
    let loose = |oid: &str| {
        project
            .root
            .join(".git/objects")
            .join(&oid[..2])
            .join(&oid[2..])
    };
    let forged = loose(&changed_blob);
    let mut perms = std::fs::metadata(&forged).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(&forged, perms).unwrap();
    std::fs::copy(loose(&ruled_blob), &forged).unwrap();
    // git itself hands back the forged content without a word.
    assert_eq!(
        git(&project.root, &["cat-file", "blob", &changed_blob]),
        BODY.trim()
    );

    let (campaign, recorded) = campaign_on(&project, "b", &[on_lib(2)]);
    assert_eq!(recorded.from_registry, 0);
    assert_eq!(campaign.survivors[0].outcome, None);
    let head = git(&project.root, &["rev-parse", "HEAD"]);
    assert_eq!(
        equivalences::source_at(&project.root, &head, "src/lib.rs"),
        None
    );
}

/// The line reader walks the trees itself: it finds a file among several
/// entries, at any depth, executable or not, in a SHA-1 or a SHA-256
/// repository — and reads nothing for a directory asked as a file, a file
/// asked as a directory, or a name that is not there.
#[test]
fn the_source_is_read_through_every_tree_on_its_path() {
    for format in ["sha1", "sha256"] {
        let dir = tempfile::tempdir().unwrap();
        let tree = dir.path().join("tree");
        std::fs::create_dir_all(&tree).unwrap();
        git(&tree, &["init", "-q", "--object-format", format]);
        write(&tree, "README.md", "read me\n");
        write(&tree, "src/a.rs", "pub fn a() {}\n");
        write(&tree, "src/lib.rs", "first\nsecond\n");
        write(&tree, "src/deep/z.rs", "deep\n");
        write(&tree, "tool.sh", "#!/bin/sh\necho tool\n");
        git(&tree, &["add", "-A"]);
        git(&tree, &["update-index", "--chmod=+x", "tool.sh"]);
        git(&tree, &["commit", "-q", "-m", "files"]);
        let head = git(&tree, &["rev-parse", "HEAD"]);
        let read = |file: &str| equivalences::source_at(&tree, &head, file);

        assert_eq!(read("README.md").as_deref(), Some("read me\n"), "{format}");
        assert_eq!(
            read("src/a.rs").as_deref(),
            Some("pub fn a() {}\n"),
            "{format}"
        );
        assert_eq!(
            read("src/lib.rs").as_deref(),
            Some("first\nsecond\n"),
            "{format}"
        );
        assert_eq!(read("src/deep/z.rs").as_deref(), Some("deep\n"), "{format}");
        assert_eq!(
            read("tool.sh").as_deref(),
            Some("#!/bin/sh\necho tool\n"),
            "{format}"
        );
        assert_eq!(read("src"), None, "a directory is not a file: {format}");
        assert_eq!(read("README.md/x"), None, "{format}");
        assert_eq!(read("src/b.rs"), None, "{format}");
        assert_eq!(read("src/lib.rs/"), None, "{format}");
        assert_eq!(
            equivalences::source_at(&tree, "HEAD", "src/lib.rs").as_deref(),
            Some("first\nsecond\n"),
            "HEAD resolves: {format}"
        );
        assert_eq!(
            equivalences::source_at(&tree, "--all", "src/lib.rs"),
            None,
            "{format}"
        );
        assert_eq!(
            equivalences::digest_at(&tree, &head, "src/lib.rs", 2),
            Some(equivalences::line_digest("second")),
            "{format}"
        );
    }
}

// The survivors of the 17:51Z campaign on 4023dc8, each answered by a test.

/// A registry that is there and cannot be read — here a directory where
/// the file should be — is an error, never an empty registry: only a file
/// that does not exist means the project never ruled.
#[test]
fn a_registry_that_cannot_be_opened_is_not_an_empty_one() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(equivalences::path(dir.path())).unwrap();
    let err = equivalences::read(dir.path()).unwrap_err();
    assert!(err.to_string().contains("equivalences.json"), "{err}");
    std::fs::remove_dir(equivalences::path(dir.path())).unwrap();
    assert_eq!(
        equivalences::read(dir.path()).unwrap(),
        equivalences::Registry::default()
    );
}

/// A ruling is about its own mutation: another mutation on the same line —
/// the same digest — gets nothing from it, and does not stop it applying to
/// its own.
#[test]
fn a_ruling_applies_only_to_its_own_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    ruled_on_a(&project);
    let mut other = on_lib(2);
    other.id = "src/lib.rs:2:5: replace one -> u8 with 1".into();
    other.description = "replace one -> u8 with 1".into();
    let mut elsewhere = on_lib(2);
    elsewhere.file = "src/other.rs".into();
    elsewhere.id = format!("src/other.rs:2:5: {LIFTED}");
    let (campaign, recorded) = campaign_on(&project, "b", &[other, on_lib(2), elsewhere]);
    assert_eq!(recorded.from_registry, 1);
    assert_eq!(campaign.survivors[0].outcome, None, "another mutation");
    assert!(matches!(
        campaign.survivors[1].outcome,
        Some(Triage::ProposedByNunki {
            from: mutants::ProposedFrom::Registry { .. },
            ..
        })
    ));
    assert_eq!(campaign.survivors[2].outcome, None, "another file");
}

/// What a verb says when the registry was left as it was — and says
/// nothing when it was changed.
#[test]
fn a_registry_left_as_it_was_says_why_and_a_changed_one_says_nothing() {
    assert_eq!(Registered::Done(0).warning(), None);
    assert_eq!(Registered::Done(2).warning(), None);
    assert_eq!(
        Registered::Not("the lock is held".into()).warning(),
        Some("the lock is held".to_string())
    );
}

/// Two mutations of one file ruled on one mission are two entries, and
/// lifting one leaves the other — whatever they share: the file, the
/// mission and commit, the line.
#[test]
fn two_mutations_of_one_file_are_entered_and_lifted_apart() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    let mut other = on_lib(2);
    other.id = "src/lib.rs:2:5: replace one -> u8 with 1".into();
    other.description = "replace one -> u8 with 1".into();
    let (campaign, _) = campaign_on(&project, "a", &[on_lib(2), other]);
    let (first, second) = (
        campaign.survivors[0].id.clone(),
        campaign.survivors[1].id.clone(),
    );
    for id in [&first, &second] {
        assert_eq!(
            nunki::findings::rule_equivalent(&project, "a", id, "nothing reads it").unwrap(),
            Registered::Done(1)
        );
    }
    assert_eq!(
        equivalences::read(&project.hq_root).unwrap().entries.len(),
        2
    );

    assert_eq!(
        nunki::findings::lift_equivalent(&project, "a", &first).unwrap(),
        Registered::Done(1)
    );
    let left = equivalences::read(&project.hq_root).unwrap().entries;
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].description, "replace one -> u8 with 1");
}

/// An entry on the same mutation from the same mission, but given on
/// another commit about another line, is not the ruling being lifted.
#[test]
fn a_lift_leaves_an_entry_given_on_another_commit_about_another_line() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    let id = ruled_on_a(&project);
    let mut registry = equivalences::read(&project.hq_root).unwrap();
    let mut other = registry.entries[0].clone();
    other.commit = "0123456789abcdef0123456789abcdef01234567".into();
    other.line = equivalences::line_digest("2");
    registry.entries.push(other.clone());
    equivalences::write(&project.hq_root, &registry).unwrap();

    assert_eq!(
        nunki::findings::lift_equivalent(&project, "a", &id).unwrap(),
        Registered::Done(1)
    );
    assert_eq!(
        equivalences::read(&project.hq_root).unwrap().entries,
        vec![other]
    );
}

/// The listing names the commit a ruling was given on by its first twelve
/// characters, and the entry names who ruled as the HQ's verbs know them.
#[test]
fn an_entry_names_who_ruled_and_the_listing_names_its_commit() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    let head = git(&project.root, &["rev-parse", "HEAD"]);
    ruled_on_a(&project);
    let registry = equivalences::read(&project.hq_root).unwrap();
    assert_eq!(
        registry.entries[0].by,
        nunki::human::me(&project.nunki_home(), Some(&project.root)).addressed()
    );
    let listing = equivalences::listing(&registry, &project.root);
    assert!(
        listing.contains(&format!("on mission a at {},", &head[..12])),
        "{listing}"
    );
}

/// Only the HQ's two rulings are rulings; the outcomes that rest on a test,
/// and the coder's proposal, are not.
#[test]
fn only_the_hqs_rulings_are_rulings() {
    for (outcome, ruling) in [
        (
            Triage::Equivalent {
                why: "w".into(),
                carried_from: None,
            },
            true,
        ),
        // What `nunki` proposes from a ruling it matched is not one (HQ
        // review 2), nor what an older file called a registry ruling.
        (
            Triage::ProposedByNunki {
                why: "w".into(),
                from: mutants::ProposedFrom::Carried { commit: "c".into() },
            },
            false,
        ),
        (
            Triage::EquivalentRegistered {
                why: "w".into(),
                mission: "a".into(),
                commit: "c".into(),
            },
            false,
        ),
        (Triage::Killed { test: "t".into() }, false),
        (Triage::Bug { test: "t".into() }, false),
        (Triage::EquivalentProposed { why: "w".into() }, false),
    ] {
        assert_eq!(outcome.is_a_ruling(), ruling, "{outcome:?}");
    }
}

// HQ review of the pull request: the registry never puts a ruling on a
// mutant the HQ did not rule on.

const TWICE: &str = "pub fn one() -> u8 {\n    1\n}\npub fn uno() -> u8 {\n    1\n}\n";

/// A survivor of `src/lib.rs` on lines `line` to `end`, mutated by `what`.
fn spanning(line: u32, end: u32, what: &str) -> Survivor {
    Survivor {
        found_on: None,
        id: format!("src/lib.rs:{line}:5: {what}"),
        file: "src/lib.rs".into(),
        line,
        end_line: Some(end),
        description: what.into(),
        outcome: None,
        refused: None,
    }
}

/// The same text on two lines, the same mutation on both, ruled on the one
/// the campaign kept: the registry cannot tell which was ruled, so nothing
/// is entered — and the other is answered neither by the same mission's
/// next campaign nor by another mission (class rule, b).
#[test]
fn the_same_text_on_two_lines_ruled_on_one_never_answers_the_other() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), TWICE);
    let (campaign, _) = campaign_on(&project, "a", &[on_lib(2)]);
    let ruled = campaign.survivors[0].id.clone();
    match nunki::findings::rule_equivalent(&project, "a", &ruled, "nothing reads it").unwrap() {
        Registered::Not(why) => {
            assert!(why.contains("occurs 2 times"), "{why}");
            assert!(why.contains("this mission only"), "{why}");
        }
        other => panic!("entered code that occurs twice: {other:?}"),
    }
    assert!(!equivalences::path(&project.hq_root).exists());

    // The same mission, next campaign: only the other line survives, under
    // its own id. The looser tier of carry does not hand it the ruling.
    let (campaign, _) = campaign_on(&project, "a", &[on_lib(5)]);
    assert_eq!(campaign.survivors[0].outcome, None, "the same mission");
    // Another mission.
    let (campaign, recorded) = campaign_on(&project, "b", &[on_lib(5)]);
    assert_eq!(recorded.from_registry, 0);
    assert_eq!(campaign.survivors[0].outcome, None, "another mission");
}

/// A registry entry on code that occurs once when it was ruled applies to
/// nothing in a campaign where that code occurs twice.
#[test]
fn an_entry_applies_to_nothing_where_its_code_occurs_twice() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    ruled_on_a(&project);
    commit_lib(&project, TWICE);
    let (campaign, recorded) = campaign_on(&project, "b", &[on_lib(2)]);
    assert_eq!(recorded.from_registry, 0);
    assert_eq!(campaign.survivors[0].outcome, None);
    let entry = &equivalences::read(&project.hq_root).unwrap().entries[0];
    assert_eq!(
        equivalences::standing(&project.root, entry),
        equivalences::Standing::Repeated(2)
    );
}

/// A ruling on a function body replaced whole is about every line of it:
/// one of its later lines changed, and the survivor is open again — the
/// first line alone would still have matched (class rule, a).
#[test]
fn a_body_replacement_whose_later_lines_change_is_open_again() {
    const LONG: &str = "pub fn body(x: u8) -> u8 {\n    let y = x + 1;\n    y * 2\n}\n";
    const WHOLE: &str = "replace body -> u8 with 0";
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), LONG);
    let (campaign, _) = campaign_on(&project, "a", &[spanning(2, 3, WHOLE)]);
    let id = campaign.survivors[0].id.clone();
    assert_eq!(
        nunki::findings::rule_equivalent(&project, "a", &id, "the value is discarded").unwrap(),
        Registered::Done(1)
    );
    let entry = &equivalences::read(&project.hq_root).unwrap().entries[0];
    assert_eq!(entry.lines, 2);
    assert_eq!(
        entry.line,
        equivalences::span_digest(&["let y = x + 1;", "y * 2"])
    );

    // Unchanged, moved: applied.
    commit_lib(&project, &format!("// a comment\n{LONG}"));
    let (campaign, _) = campaign_on(&project, "b", &[spanning(3, 4, WHOLE)]);
    assert!(matches!(
        campaign.survivors[0].outcome,
        Some(Triage::ProposedByNunki {
            from: mutants::ProposedFrom::Registry { .. },
            ..
        })
    ));
    // Its second line changed: open again.
    commit_lib(
        &project,
        "pub fn body(x: u8) -> u8 {\n    let y = x + 1;\n    y * 3\n}\n",
    );
    let (campaign, recorded) = campaign_on(&project, "c", &[spanning(2, 3, WHOLE)]);
    assert_eq!(recorded.from_registry, 0);
    assert_eq!(campaign.survivors[0].outcome, None);
    // And a span nobody knows is tied to nothing.
    let mut unknown = spanning(2, 3, WHOLE);
    unknown.end_line = Some(0);
    assert_eq!(unknown.span(), None);
    commit_lib(&project, LONG);
    let (campaign, _) = campaign_on(&project, "d", &[unknown]);
    assert_eq!(campaign.survivors[0].outcome, None);
}

/// A campaign that holds two survivors on one file and description cannot
/// say which one a ruling is about, even when their code differs: nothing
/// is entered (class rule, c).
#[test]
fn a_ruling_given_beside_a_twin_survivor_is_not_entered() {
    const TWO: &str = "pub fn one() -> u8 {\n    1\n}\npub fn two() -> u8 {\n    2\n}\n";
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), TWO);
    let (campaign, _) = campaign_on(&project, "a", &[on_lib(2), on_lib(5)]);
    let id = campaign.survivors[0].id.clone();
    match nunki::findings::rule_equivalent(&project, "a", &id, "nothing reads it").unwrap() {
        Registered::Not(why) => assert!(why.contains("2 survivors"), "{why}"),
        other => panic!("entered beside a twin: {other:?}"),
    }
    assert!(!equivalences::path(&project.hq_root).exists());
    // The ruling stands on the mission.
    assert!(matches!(
        mutants::read(&mission_dir(&project, "a"))
            .unwrap()
            .unwrap()
            .survivors[0]
            .outcome,
        Some(Triage::Equivalent { .. })
    ));
}

/// m1 ruled, m2's proposal refused, the same mutation in the same file; the
/// new campaign holds only m2, renamed. Neither tier, nor the registry, may
/// hand it m1's ruling (HQ review, item 6).
#[test]
fn m1_ruled_m2_refused_and_m2_renamed_gets_no_equivalence() {
    const TWO: &str = "pub fn one() -> u8 {\n    1\n}\npub fn two() -> u8 {\n    2\n}\n";
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), TWO);
    let mut m1 = on_lib(2);
    m1.id = "m1".into();
    let mut m2 = on_lib(5);
    m2.id = "m2".into();
    campaign_on(&project, "a", &[m1, m2]);
    let a = mission_dir(&project, "a");
    nunki::findings::rule_equivalent(&project, "a", "m1", "nothing reads it").unwrap();
    proposes(&a, &[("m2", "nothing reads it either")]);
    nunki::findings::refuse_proposal(&project, "a", "m2", "it is returned").unwrap();

    let mut renamed = on_lib(5);
    renamed.id = "m2-renamed".into();
    let (campaign, recorded) = campaign_on(&project, "a", &[renamed.clone()]);
    assert_eq!(recorded.from_registry, 0);
    assert_eq!(campaign.survivors[0].outcome, None, "no equivalence");
    let (campaign, _) = campaign_on(&project, "b", &[renamed]);
    assert_eq!(
        campaign.survivors[0].outcome, None,
        "nor on another mission"
    );
}

/// A tree object git itself would never write — an entry named twice, or
/// entries out of git's order — is read as nothing, at every tree on the
/// path. A forged commit can list `lib.rs` twice: `git show` reads the
/// first, a checkout writes the last (HQ review, item 2).
#[test]
fn a_tree_naming_an_entry_twice_or_out_of_order_is_read_as_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let tree = repo(dir.path());
    let hash = |kind: &str, body: &[u8]| {
        use std::io::Write;
        let mut child = Command::new("git")
            .arg("-C")
            .arg(&tree)
            .args(["hash-object", "-t", kind, "--literally", "-w", "--stdin"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(body).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    let raw = |oid: &str| -> Vec<u8> {
        (0..oid.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&oid[i..i + 2], 16).unwrap())
            .collect()
    };
    let tree_of = |entries: &[(&str, &str, &str)]| {
        let mut body = Vec::new();
        for (mode, name, oid) in entries {
            body.extend_from_slice(format!("{mode} {name}\0").as_bytes());
            body.extend_from_slice(&raw(oid));
        }
        hash("tree", &body)
    };
    let commit_of = |root: &str| git(&tree, &["commit-tree", root, "-m", "forged"]);
    let ruled = hash("blob", b"ruled\n");
    let other = hash("blob", b"other\n");
    let read = |root: &str, file: &str| equivalences::source_at(&tree, &commit_of(root), file);

    // Written as git writes them, they read.
    let src = tree_of(&[("100644", "a.rs", &other), ("100644", "lib.rs", &ruled)]);
    let root = tree_of(&[("40000", "src", &src)]);
    assert_eq!(read(&root, "src/lib.rs").as_deref(), Some("ruled\n"));

    // lib.rs twice, in the tree the file is in.
    let twice = tree_of(&[("100644", "lib.rs", &ruled), ("100644", "lib.rs", &other)]);
    assert_eq!(
        read(&tree_of(&[("40000", "src", &twice)]), "src/lib.rs"),
        None
    );
    // Out of order there.
    let unsorted = tree_of(&[("100644", "lib.rs", &ruled), ("100644", "a.rs", &other)]);
    assert_eq!(
        read(&tree_of(&[("40000", "src", &unsorted)]), "src/lib.rs"),
        None
    );
    // src twice, in the root above it.
    let doubled = tree_of(&[("40000", "src", &src), ("40000", "src", &src)]);
    assert_eq!(read(&doubled, "src/lib.rs"), None);
    // A file and a directory of one name: still one name twice.
    // In git's order (`src` before `src/`), so only the names give it away
    // — asked for as the file, or through the directory.
    let both = tree_of(&[("100644", "src", &other), ("40000", "src", &src)]);
    assert_eq!(read(&both, "src"), None);
    assert_eq!(read(&both, "src/lib.rs"), None);

    // A directory sorts as if its name ended in `/`: `x.rs` before `x/`, as
    // git writes it, reads.
    write(&tree, "x.rs", "file\n");
    write(&tree, "x/y.rs", "nested\n");
    git(&tree, &["add", "-A"]);
    git(&tree, &["commit", "-q", "-m", "x"]);
    let head = git(&tree, &["rev-parse", "HEAD"]);
    assert_eq!(
        equivalences::source_at(&tree, &head, "x/y.rs").as_deref(),
        Some("nested\n")
    );
    assert_eq!(
        equivalences::source_at(&tree, &head, "x.rs").as_deref(),
        Some("file\n")
    );
}

/// A span read out of a source is its lines start to end, all of them in
/// the file; anything else — reversed, out of range, from line 0 — is not
/// a span.
#[test]
fn a_span_is_read_only_when_every_line_of_it_is_there() {
    let source = "a\nb\nc\n";
    assert_eq!(
        equivalences::identified(source, 2, 3),
        Ok(equivalences::span_digest(&["b", "c"]))
    );
    assert_eq!(
        equivalences::identified(source, 3, 3),
        Ok(equivalences::line_digest("c"))
    );
    for (start, end) in [(3, 2), (0, 1), (3, 4), (4, 4)] {
        assert!(
            equivalences::identified(source, start, end).is_err(),
            "{start}..{end}"
        );
    }
}

/// A lock file held by a live process: this test's own.
fn hold_the_registry_lock(hq_root: &Path) {
    let locks = hq_root.join("locks");
    std::fs::create_dir_all(&locks).unwrap();
    std::fs::write(
        locks.join(".equivalences.lock"),
        format!(
            "{{\"slot\":\".equivalences\",\"verb\":\"a test\",\"pid\":{},\"since\":\"2026-10-06T00:00:00Z\"}}",
            std::process::id()
        ),
    )
    .unwrap();
}

/// `--lift` changes the registry first, and when it cannot — locked, or
/// unreadable — it fails and leaves the mission's file as it was: a ruling
/// lifted from the mission and left in the registry would answer the next
/// mission still (HQ review, item 4).
#[test]
fn a_lift_the_registry_cannot_take_fails_and_leaves_the_mission_alone() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    let id = ruled_on_a(&project);
    let a = mission_dir(&project, "a");
    let before = std::fs::read_to_string(a.join(mutants::FILE)).unwrap();

    hold_the_registry_lock(&project.hq_root);
    let err = nunki::findings::lift_equivalent(&project, "a", &id).unwrap_err();
    assert!(err.to_string().contains("Nothing was lifted"), "{err}");
    assert_eq!(
        std::fs::read_to_string(a.join(mutants::FILE)).unwrap(),
        before
    );
    std::fs::remove_file(project.hq_root.join("locks/.equivalences.lock")).unwrap();

    let file = equivalences::path(&project.hq_root);
    let registry = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&file, "{ not a registry").unwrap();
    let err = nunki::findings::lift_equivalent(&project, "a", &id).unwrap_err();
    assert!(err.to_string().contains("equivalences.json"), "{err}");
    assert_eq!(
        std::fs::read_to_string(a.join(mutants::FILE)).unwrap(),
        before
    );

    // Repaired, it lifts both.
    std::fs::write(&file, registry).unwrap();
    assert_eq!(
        nunki::findings::lift_equivalent(&project, "a", &id).unwrap(),
        Registered::Done(1)
    );
    assert!(mutants::open(&a).unwrap().contains(&id));
}

/// A registry entry outlives the ruling on its mission — lifted there by a
/// verb whose registry half failed — and `--lift` still takes it out. With
/// nothing on either side, it refuses.
#[test]
fn a_lift_cleans_a_registry_entry_the_mission_no_longer_holds() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    let id = ruled_on_a(&project);
    let a = mission_dir(&project, "a");
    mutants::lift_equivalent(&a, &id).unwrap();
    assert_eq!(
        equivalences::read(&project.hq_root).unwrap().entries.len(),
        1
    );

    assert_eq!(
        nunki::findings::lift_equivalent(&project, "a", &id).unwrap(),
        Registered::Done(1)
    );
    assert!(
        equivalences::read(&project.hq_root)
            .unwrap()
            .entries
            .is_empty()
    );
    let err = nunki::findings::lift_equivalent(&project, "a", &id).unwrap_err();
    assert!(err.to_string().contains("no `equivalent` ruling"), "{err}");
}

/// A zero-length registry is what a write cut short leaves: unreadable,
/// never an empty registry that would drop every ruling it held (HQ review,
/// item 5). And a write leaves no staged file behind.
#[test]
fn a_zero_length_registry_is_unreadable() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(equivalences::path(dir.path()), "").unwrap();
    let err = equivalences::read(dir.path()).unwrap_err();
    assert!(err.to_string().contains("equivalences.json"), "{err}");
    std::fs::write(equivalences::path(dir.path()), "  \n").unwrap();
    assert!(equivalences::read(dir.path()).is_err());

    equivalences::write(dir.path(), &equivalences::Registry::default()).unwrap();
    assert_eq!(
        equivalences::read(dir.path()).unwrap(),
        equivalences::Registry::default()
    );
    let left: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(left, vec![std::ffi::OsString::from(equivalences::FILE)]);
}

/// A campaign is recorded against the registry under the registry's lock:
/// held by a writer that does not let go, nothing is applied, and why is
/// said (HQ review, item 6).
#[test]
fn a_campaign_reads_the_registry_under_its_lock() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    ruled_on_a(&project);
    hold_the_registry_lock(&project.hq_root);
    let (campaign, recorded) = campaign_on(&project, "b", &[on_lib(2)]);
    assert_eq!(campaign.survivors[0].outcome, None);
    let why = recorded.registry_unread.expect("said");
    assert!(why.contains("a test"), "{why}");
}

// HQ review 2: the registry, and carry's looser tier, propose — they never
// rule. Each of the reviewer's three attacks ends with no equivalence
// written, and a proposal or a refusal in place.

/// The survivor `id` of `src/lib.rs` on `line`, mutated by `what`.
fn named(id: &str, line: u32, what: &str) -> Survivor {
    Survivor {
        found_on: None,
        id: id.into(),
        file: "src/lib.rs".into(),
        line,
        end_line: None,
        description: what.into(),
        outcome: None,
        refused: None,
    }
}

/// Whether any survivor of `campaign` holds an equivalence the HQ did not
/// give it on this campaign's own survivor: none may.
fn no_equivalence(campaign: &Campaign) -> bool {
    campaign
        .survivors
        .iter()
        .all(|s| !matches!(s.outcome, Some(Triage::Equivalent { .. })))
}

const KEEP: &str = "replace > with < in keep";

/// Attack 1: the same operator twice on one line, ruled on the one the
/// campaign kept. The other, surviving alone later — on another mission
/// through the registry, or on this one under its own id — is proposed the
/// ruling, never given it.
#[test]
fn the_same_operator_twice_on_a_line_ruled_on_one_only_proposes_on_the_other() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(
        dir.path(),
        "pub fn keep(a: u8, b: u8, c: u8, d: u8) -> bool {\n    a > b && c > d\n}\n",
    );
    let first = named("src/lib.rs:2:7: replace > with < in keep", 2, KEEP);
    let second = named("src/lib.rs:2:16: replace > with < in keep", 2, KEEP);
    campaign_on(&project, "a", std::slice::from_ref(&first));
    assert_eq!(
        nunki::findings::rule_equivalent(&project, "a", &first.id, "a is never above b").unwrap(),
        Registered::Done(1),
        "the line reads once, and the campaign held one survivor"
    );

    // Another mission: the registry proposes.
    let (campaign, recorded) = campaign_on(&project, "b", std::slice::from_ref(&second));
    assert_eq!(recorded.from_registry, 1);
    assert!(no_equivalence(&campaign));
    assert!(matches!(
        campaign.survivors[0].outcome,
        Some(Triage::ProposedByNunki {
            from: mutants::ProposedFrom::Registry { .. },
            ..
        })
    ));
    // The same mission, next campaign: carry's looser tier proposes.
    let (campaign, _) = campaign_on(&project, "a", &[second]);
    assert!(no_equivalence(&campaign));
    assert!(matches!(
        campaign.survivors[0].outcome,
        Some(Triage::ProposedByNunki {
            from: mutants::ProposedFrom::Carried { .. },
            ..
        })
    ));
    for mission in ["a", "b"] {
        assert_eq!(
            mutants::awaiting_ruling(&mission_dir(&project, mission))
                .unwrap()
                .len(),
            1,
            "{mission} waits on the HQ"
        );
    }
}

/// Attack 2: the ruled line deleted, leaving its twin — the same code, the
/// same mutation — now the only one of its kind under a new id. It is
/// proposed the ruling, for the HQ to confirm, never given it.
#[test]
fn a_ruled_line_deleted_leaves_its_twin_a_proposal_never_a_ruling() {
    const TWIN: &str =
        "pub fn keep(a: u8, b: u8) -> bool {\n    let x = a > b;\n    let x = a > b;\n    x\n}\n";
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), TWIN);
    // The campaign kept the first; the second was killed then.
    campaign_on(&project, "a", &[named("first", 2, KEEP)]);
    match nunki::findings::rule_equivalent(&project, "a", "first", "x is unused").unwrap() {
        Registered::Not(why) => assert!(why.contains("occurs 2 times"), "{why}"),
        other => panic!("{other:?}"),
    }
    // The ruled line goes; its twin, now line 2, survives under a new id.
    commit_lib(
        &project,
        "pub fn keep(a: u8, b: u8) -> bool {\n    let x = a > b;\n    x\n}\n",
    );
    let (campaign, _) = campaign_on(&project, "a", &[named("second", 2, KEEP)]);
    assert!(no_equivalence(&campaign));
    assert_eq!(
        campaign.survivors[0].outcome,
        Some(Triage::ProposedByNunki {
            why: "x is unused".into(),
            from: mutants::ProposedFrom::Carried {
                commit: git(&project.root, &["rev-parse", "HEAD~1"]),
            },
        })
    );
    // The HQ refuses it: the survivor is open, with the refusal on it.
    nunki::findings::refuse_proposal(&project, "a", "second", "the twin is read").unwrap();
    let a = mission_dir(&project, "a");
    assert_eq!(mutants::open(&a).unwrap(), vec!["second"]);
}

/// Attack 3: a refusal on text that occurs twice, and the id changes. The
/// refusal follows — whatever the uniqueness gate says — and the proposal
/// written again is still no outcome.
#[test]
fn a_refusal_on_repeated_text_follows_an_id_change() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), TWICE);
    campaign_on(&project, "a", &[named("before", 2, LIFTED)]);
    let a = mission_dir(&project, "a");
    proposes(&a, &[("before", "nothing reads it")]);
    nunki::findings::refuse_proposal(&project, "a", "before", "it is returned").unwrap();

    let (campaign, _) = campaign_on(&project, "a", &[named("after", 2, LIFTED)]);
    assert!(no_equivalence(&campaign));
    assert_eq!(
        campaign.survivors[0]
            .refused
            .as_ref()
            .map(|r| r.because.as_str()),
        Some("it is returned")
    );
    proposes(&a, &[("after", "nothing reads it")]);
    assert_eq!(mutants::open(&a).unwrap(), vec!["after"]);
    assert!(mutants::awaiting_ruling(&a).unwrap().is_empty());
}

/// The registry proposes nothing to two survivors sharing a file and
/// description, even when the code of each is unique and only one of them
/// matches an entry: which mutant the entry was about is a guess (HQ review
/// 2, E — the twin guard of `apply`).
#[test]
fn the_registry_proposes_nothing_to_twin_survivors() {
    const TWO: &str = "pub fn one() -> u8 {\n    1\n}\npub fn two() -> u8 {\n    2\n}\n";
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), TWO);
    campaign_on(&project, "a", &[on_lib(2)]);
    let id = mutants::read(&mission_dir(&project, "a"))
        .unwrap()
        .unwrap()
        .survivors[0]
        .id
        .clone();
    assert_eq!(
        nunki::findings::rule_equivalent(&project, "a", &id, "nothing reads it").unwrap(),
        Registered::Done(1)
    );
    let (campaign, recorded) = campaign_on(&project, "b", &[on_lib(2), on_lib(5)]);
    assert_eq!(recorded.from_registry, 0);
    assert!(campaign.survivors.iter().all(|s| s.outcome.is_none()));
}

/// `--lift` on a proposal carried from a ruling of this mission, on code
/// that has changed since, takes that ruling's registry entry out — found by
/// where it was given, the code no longer matching.
#[test]
fn lifting_a_carried_proposal_takes_its_ruling_out_of_the_registry() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    ruled_on_a(&project);
    commit_lib(&project, "pub fn one() -> u8 {\n    1 + 0\n}\n");
    let mut renamed = on_lib(2);
    renamed.id = "renamed".into();
    let (campaign, _) = campaign_on(&project, "a", &[renamed]);
    assert!(matches!(
        campaign.survivors[0].outcome,
        Some(Triage::ProposedByNunki {
            from: mutants::ProposedFrom::Carried { .. },
            ..
        })
    ));
    assert_eq!(
        nunki::findings::lift_equivalent(&project, "a", "renamed").unwrap(),
        Registered::Done(1)
    );
    assert!(
        equivalences::read(&project.hq_root)
            .unwrap()
            .entries
            .is_empty()
    );
    assert_eq!(
        mutants::open(&mission_dir(&project, "a")).unwrap(),
        vec!["renamed"]
    );
}

/// A file an older `nunki` wrote, with a ruling it applied from the
/// registry, reads as a proposal from the registry, awaiting the HQ.
#[test]
fn an_older_registry_ruling_reads_as_a_proposal() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(mutants::FILE),
        r#"{"fingerprint":"f","head":"h","date":"d","survivors":[{"id":"s1","file":"a","line":1,
           "outcome":{"kind":"equivalent_registered","why":"w","mission":"m","commit":"c"}}]}"#,
    )
    .unwrap();
    let campaign = mutants::read(dir.path()).unwrap().unwrap();
    assert_eq!(
        campaign.survivors[0].outcome,
        Some(Triage::ProposedByNunki {
            why: "w".into(),
            from: mutants::ProposedFrom::Registry {
                mission: "m".into(),
                commit: "c".into(),
                date: String::new(),
            },
        })
    );
    let waiting = mutants::awaiting_ruling(dir.path()).unwrap();
    assert_eq!(waiting.len(), 1);
    assert_eq!(
        waiting[0].source(),
        "nunki's, from the registry: ruled on mission m at c"
    );
}

/// A registry lock another verb lets go of within the wait is taken: the
/// ruling enters the registry rather than being refused because a
/// neighbour was quick (the 11e7d37 campaign's survivors on the retry).
#[test]
fn a_registry_lock_let_go_within_the_wait_is_taken() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    let (campaign, _) = campaign_on(&project, "a", &[on_lib(2)]);
    let id = campaign.survivors[0].id.clone();
    hold_the_registry_lock(&project.hq_root);
    let held = project.hq_root.join("locks/.equivalences.lock");
    let letting_go = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(400));
        std::fs::remove_file(held).unwrap();
    });
    assert_eq!(
        nunki::findings::rule_equivalent(&project, "a", &id, "nothing reads it").unwrap(),
        Registered::Done(1)
    );
    letting_go.join().unwrap();
}

/// An entry an older `nunki` wrote, before spans, says nothing of its
/// length: it is one line.
#[test]
fn an_entry_written_before_spans_is_one_line_long() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        equivalences::path(dir.path()),
        r#"{"entries":[{"file":"a","description":"d","line":"x","why":"w","by":"b",
            "mission":"m","commit":"c","date":"t"}]}"#,
    )
    .unwrap();
    assert_eq!(equivalences::read(dir.path()).unwrap().entries[0].lines, 1);
}

/// `--ratify --all` rules exactly what it printed: a proposal refused in
/// between fails the verb before anything is written.
#[test]
fn ratify_all_rules_nothing_when_what_it_listed_has_changed() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), TWICE);
    campaign_on(
        &project,
        "a",
        &[named("one", 2, "replace a"), named("two", 5, "replace b")],
    );
    let a = mission_dir(&project, "a");
    proposes(&a, &[("one", "nothing reads it"), ("two", "nor this")]);
    let listed = nunki::findings::pending(&project, "a").unwrap();
    assert_eq!(listed.len(), 2);
    nunki::findings::refuse_proposal(&project, "a", "two", "it is returned").unwrap();

    let err = nunki::findings::ratify_all(&project, "a", &listed).unwrap_err();
    assert!(
        err.to_string().contains("changed since it was listed"),
        "{err}"
    );
    let campaign = mutants::read(&a).unwrap().unwrap();
    assert!(no_equivalence(&campaign), "nothing was ratified");
    assert_eq!(mutants::awaiting_ruling(&a).unwrap().len(), 1);

    // Listed again, it rules what is pending.
    let listed = nunki::findings::pending(&project, "a").unwrap();
    let done = nunki::findings::ratify_all(&project, "a", &listed).unwrap();
    assert_eq!(done.len(), 1);
    assert!(matches!(
        mutants::read(&a).unwrap().unwrap().survivors[0].outcome,
        Some(Triage::Equivalent { .. })
    ));
}

// HQ review 3: from the coder's file only the outcomes it may give are
// read; --ratify --all rules what it printed; the exact tier checks the
// code; a ratified registry entry keeps its origin.

/// What the coder's file would hold to pass for `nunki`'s own proposal.
const FORGED_PROPOSAL: &str = r#"{"kind": "proposed_by_nunki", "why": "trust me",
    "from": {"source": "registry", "mission": "earlier", "commit": "c", "date": ""}}"#;

/// A triage entry forged as `nunki`'s proposal from the registry is listed
/// as nothing: no proposal awaits, the survivor is open, `--ratify` and
/// `--ratify --all` refuse it, and nothing reaches the registry.
#[test]
fn a_proposal_by_nunki_forged_in_the_coders_file_is_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    let (campaign, _) = campaign_on(&project, "a", &[on_lib(2)]);
    let id = campaign.survivors[0].id.clone();
    let a = mission_dir(&project, "a");
    std::fs::write(
        a.join(mutants::TRIAGE_FILE),
        format!("{{{:?}: {FORGED_PROPOSAL}}}", id),
    )
    .unwrap();

    assert!(mutants::awaiting_ruling(&a).unwrap().is_empty());
    assert!(nunki::findings::pending(&project, "a").unwrap().is_empty());
    assert_eq!(mutants::open(&a).unwrap(), vec![id.clone()]);
    let foreign = mutants::foreign(&a).unwrap();
    assert_eq!(foreign.len(), 1);
    assert_eq!(foreign[0].kind, "proposed_by_nunki");

    let err = nunki::findings::ratify_proposal(&project, "a", &id, None).unwrap_err();
    assert!(err.to_string().contains("proposes no equivalence"), "{err}");
    let listed = nunki::findings::pending(&project, "a").unwrap();
    assert!(
        nunki::findings::ratify_all(&project, "a", &listed)
            .unwrap()
            .is_empty()
    );
    assert!(no_equivalence(&mutants::read(&a).unwrap().unwrap()));
    assert!(!equivalences::path(&project.hq_root).exists());
}

/// A forged entry never shadows what the HQ's file holds: over the HQ's own
/// ruling, the coder's `equivalent` or `proposed_by_nunki` changes nothing
/// the survivor answers with.
#[test]
fn a_forged_entry_never_shadows_the_hqs_ruling() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    let id = ruled_on_a(&project);
    let a = mission_dir(&project, "a");
    let campaign = mutants::read(&a).unwrap().unwrap();
    for forged in [
        r#"{"kind": "equivalent", "why": "the coder's own"}"#.to_string(),
        FORGED_PROPOSAL.to_string(),
    ] {
        std::fs::write(
            a.join(mutants::TRIAGE_FILE),
            format!("{{{id:?}: {forged}}}"),
        )
        .unwrap();
        let coders = mutants::read_triage(&a).unwrap();
        match mutants::answer(&campaign.survivors[0], &coders) {
            Some(Triage::Equivalent { why, .. }) => assert_eq!(why, "nothing reads the value"),
            other => panic!("{forged} shadowed the ruling: {other:?}"),
        }
    }
}

/// `--ratify --all` rules each proposal with the sentence it printed: one
/// whose sentence changed since it was listed fails the verb, and nothing
/// is written.
#[test]
fn ratify_all_refuses_a_sentence_changed_since_it_was_listed() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    let (campaign, _) = campaign_on(&project, "a", &[on_lib(2)]);
    let id = campaign.survivors[0].id.clone();
    let a = mission_dir(&project, "a");
    proposes(&a, &[(&id, "as printed")]);
    let listed = nunki::findings::pending(&project, "a").unwrap();
    proposes(&a, &[(&id, "rewritten after the listing")]);
    let err = nunki::findings::ratify_all(&project, "a", &listed).unwrap_err();
    assert!(
        err.to_string().contains("changed since it was listed"),
        "{err}"
    );
    assert!(no_equivalence(&mutants::read(&a).unwrap().unwrap()));
}

/// When ratifying stops midway, it says truthfully what was written: the
/// survivors ruled before the one that failed, and nothing after it.
#[test]
fn ratify_all_stopping_midway_says_what_was_written() {
    let proposal = |id: &str| mutants::Proposal {
        id: id.into(),
        file: "src/lib.rs".into(),
        line: 1,
        why: "w".into(),
        from: None,
    };
    let listed = [proposal("one"), proposal("two"), proposal("three")];
    let mut tried = Vec::new();
    let err = nunki::findings::ratify_in_turn(
        &listed,
        |p| {
            tried.push(p.id.clone());
            if p.id == "two" {
                Err(nunki::findings::FindingsError::NoHuman)
            } else {
                Ok(Registered::Done(1))
            }
        },
        |_| false,
    )
    .unwrap_err();
    assert_eq!(tried, vec!["one", "two"], "nothing after it is touched");
    let said = err.to_string();
    assert!(said.contains("stopped at `two`"), "{said}");
    assert!(said.contains("written on `one` before it"), "{said}");
    assert!(!said.contains("on `two` itself"), "{said}");
    assert!(said.contains("nothing after it was touched"), "{said}");

    // The HQ's file already holds the ruling on the one it stopped at: what
    // failed came after it was written, and it says so (HQ review 4).
    let err = nunki::findings::ratify_in_turn(
        &listed,
        |p| {
            if p.id == "two" {
                Err(nunki::findings::FindingsError::NoHuman)
            } else {
                Ok(Registered::Done(1))
            }
        },
        |p| p.id == "two",
    )
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("and on `two` itself, before it failed"),
        "{err}"
    );

    let err = nunki::findings::ratify_in_turn(
        &listed,
        |_| Err(nunki::findings::FindingsError::NoHuman),
        |_| false,
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("written on no survivor before it"),
        "{err}"
    );

    // A second proposal under an id already ruled, with the same sentence,
    // is not ruled twice.
    let twice = [proposal("one"), proposal("one")];
    let done =
        nunki::findings::ratify_in_turn(&twice, |_| Ok(Registered::Done(1)), |_| false).unwrap();
    assert_eq!(done.len(), 1);

    // With different sentences, the pair is refused and nothing is ruled
    // (HQ review 4): one ruling answers every survivor of an id.
    let mut other = proposal("one");
    other.why = "another sentence".into();
    let pair = [proposal("one"), other];
    let mut touched = false;
    let err = nunki::findings::ratify_in_turn(
        &pair,
        |_| {
            touched = true;
            Ok(Registered::Done(1))
        },
        |_| false,
    )
    .unwrap_err();
    assert!(!touched, "nothing was ruled");
    assert!(err.to_string().contains("two proposals on `one`"), "{err}");
}

/// Carry never rules (HQ review 4): other code shifted onto the ruled
/// position — same id — gets a proposal, never the ruling.
#[test]
fn a_twin_shifted_onto_the_ruled_id_is_proposed_the_ruling() {
    const BEFORE: &str =
        "pub fn keep(a: u8, b: u8, c: u8, d: u8) {\n    let x = a > b;\n    let y = c > d;\n}\n";
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BEFORE);
    let at = named("src/lib.rs:2:15: replace > with < in keep", 2, KEEP);
    campaign_on(&project, "a", std::slice::from_ref(&at));
    nunki::findings::rule_equivalent(&project, "a", &at.id, "x is unused").unwrap();

    // The ruled line goes; the next one lands on its line and column.
    commit_lib(
        &project,
        "pub fn keep(a: u8, b: u8, c: u8, d: u8) {\n    let y = c > d;\n}\n",
    );
    let (campaign, _) = campaign_on(&project, "a", &[at]);
    assert!(no_equivalence(&campaign));
    assert!(matches!(
        campaign.survivors[0].outcome,
        Some(Triage::ProposedByNunki {
            from: mutants::ProposedFrom::Carried { .. },
            ..
        })
    ));
}

/// The reviewer's case (HQ review 4): `let x = a > b; log(x); let x = a > b;
/// x`, ruled on line 2; the first assignment and the log are deleted, so the
/// identical twin lands on the ruled id. Same id, same text: still a
/// proposal, never the ruling.
#[test]
fn an_identical_twin_shifted_onto_the_ruled_id_is_proposed_the_ruling() {
    const BEFORE: &str = "pub fn keep(a: u8, b: u8) -> bool {\n    let x = a > b;\n    log(x);\n    let x = a > b;\n    x\n}\n";
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BEFORE);
    let at = named("src/lib.rs:2:15: replace > with < in keep", 2, KEEP);
    let twin = named("src/lib.rs:4:15: replace > with < in keep", 4, KEEP);
    campaign_on(&project, "a", &[at.clone(), twin]);
    nunki::findings::rule_equivalent(&project, "a", &at.id, "x is unused").unwrap();
    commit_lib(
        &project,
        "pub fn keep(a: u8, b: u8) -> bool {\n    let x = a > b;\n    x\n}\n",
    );
    let (campaign, _) = campaign_on(&project, "a", &[at]);
    assert!(no_equivalence(&campaign));
    assert_eq!(
        campaign.survivors[0].outcome,
        Some(Triage::ProposedByNunki {
            why: "x is unused".into(),
            from: mutants::ProposedFrom::Carried {
                commit: git(&project.root, &["rev-parse", "HEAD~1"]),
            },
        })
    );
}

/// Even unchanged, the same id is given a carried proposal of the ruling —
/// with its sentence and the commit it was given on — never the ruling:
/// the only equivalences in a campaign are the HQ's own on it (HQ review 4).
/// Plain `record_finished`, with no source to read, proposes too.
#[test]
fn an_unchanged_survivor_with_the_same_id_is_proposed_the_ruling() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    let id = ruled_on_a(&project);
    let ruled_at = git(&project.root, &["rev-parse", "HEAD"]);
    let mut same = on_lib(2);
    same.id = id.clone();
    let (campaign, _) = campaign_on(&project, "a", std::slice::from_ref(&same));
    let carried = Some(Triage::ProposedByNunki {
        why: "nothing reads the value".into(),
        from: mutants::ProposedFrom::Carried { commit: ruled_at },
    });
    assert_eq!(campaign.survivors[0].outcome, carried);
    let a = mission_dir(&project, "a");
    assert_eq!(mutants::awaiting_ruling(&a).unwrap().len(), 1);

    // Ratified, it is the HQ's ruling on this campaign — and the next
    // campaign proposes it again.
    nunki::findings::ratify_proposal(&project, "a", &id, None).unwrap();
    assert!(!no_equivalence(&mutants::read(&a).unwrap().unwrap()));
    mutants::record_finished(&a, "fp", "h2", &log_of(&[same])).unwrap();
    let campaign = mutants::read(&a).unwrap().unwrap();
    assert!(no_equivalence(&campaign));
    assert!(matches!(
        campaign.survivors[0].outcome,
        Some(Triage::ProposedByNunki {
            from: mutants::ProposedFrom::Carried { .. },
            ..
        })
    ));
}

/// Ratifying a proposal from the registry keeps the entry's origin —
/// mission, commit, date, sentence — and records the ratification beside
/// it (HQ review 3, item 5).
#[test]
fn ratifying_a_registry_proposal_keeps_the_entrys_origin() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    ruled_on_a(&project);
    let before = equivalences::read(&project.hq_root).unwrap().entries[0].clone();
    let (campaign, _) = campaign_on(&project, "b", &[on_lib(2)]);
    let id = campaign.survivors[0].id.clone();
    let (_, registered) = nunki::findings::ratify_proposal(&project, "b", &id, None).unwrap();
    assert_eq!(registered, Registered::Done(1));

    let after = equivalences::read(&project.hq_root).unwrap().entries;
    assert_eq!(after.len(), 1);
    let entry = &after[0];
    assert_eq!(
        (
            &entry.mission,
            &entry.commit,
            &entry.date,
            &entry.why,
            &entry.by
        ),
        (
            &before.mission,
            &before.commit,
            &before.date,
            &before.why,
            &before.by
        )
    );
    assert_eq!(entry.ratified.len(), 1);
    assert_eq!(entry.ratified[0].mission, "b");
    assert_eq!(entry.ratified[0].commit, campaign.head);

    // A coder's proposal ratified is entered as this mission's ruling.
    equivalences::write(&project.hq_root, &equivalences::Registry::default()).unwrap();
    let (campaign, _) = campaign_on(&project, "c", &[on_lib(2)]);
    let id = campaign.survivors[0].id.clone();
    proposes(&mission_dir(&project, "c"), &[(&id, "nothing reads it")]);
    nunki::findings::ratify_proposal(&project, "c", &id, None).unwrap();
    let entries = equivalences::read(&project.hq_root).unwrap().entries;
    assert_eq!(entries[0].mission, "c");
    assert!(entries[0].ratified.is_empty());
}

/// The 05:47 campaign's survivors on `carried`: a proposal `carry` made is
/// carried again — under the same id, and under another — and one from the
/// registry never is.
#[test]
fn a_carried_proposal_is_carried_again_and_a_registry_one_never() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    let id = ruled_on_a(&project);
    commit_lib(&project, MOVED);
    let renamed = |name: &str| {
        let mut s = on_lib(6);
        s.id = name.into();
        s
    };
    let (campaign, _) = campaign_on(&project, "a", &[renamed("second")]);
    let proposed = campaign.survivors[0].outcome.clone();
    assert!(matches!(
        proposed,
        Some(Triage::ProposedByNunki {
            from: mutants::ProposedFrom::Carried { .. },
            ..
        })
    ));
    // The same id again: the same proposal.
    let (campaign, _) = campaign_on(&project, "a", &[renamed("second")]);
    assert_eq!(campaign.survivors[0].outcome, proposed);
    // Renamed again: the same proposal.
    let (campaign, _) = campaign_on(&project, "a", &[renamed("third")]);
    assert_eq!(campaign.survivors[0].outcome, proposed);
    let _ = id;

    // A registry proposal, under the same id, with the registry emptied:
    // nothing.
    let (campaign, _) = campaign_on(&project, "b", &[on_lib(6)]);
    assert!(matches!(
        campaign.survivors[0].outcome,
        Some(Triage::ProposedByNunki {
            from: mutants::ProposedFrom::Registry { .. },
            ..
        })
    ));
    equivalences::write(&project.hq_root, &equivalences::Registry::default()).unwrap();
    let (campaign, _) = campaign_on(&project, "b", &[on_lib(6)]);
    assert_eq!(campaign.survivors[0].outcome, None);
}

/// `--ratify --all` that fails after the HQ's file was written — the
/// coder's file could not be rewritten — says the ruling on that survivor
/// is written, rather than leaving the HQ to believe nothing was (HQ review
/// 4).
#[cfg(unix)]
#[test]
fn ratify_all_says_a_ruling_written_before_its_failure() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    let (campaign, _) = campaign_on(&project, "a", &[on_lib(2)]);
    let id = campaign.survivors[0].id.clone();
    let a = mission_dir(&project, "a");
    proposes(&a, &[(&id, "nothing reads it")]);
    let triage = a.join(mutants::TRIAGE_FILE);
    std::fs::set_permissions(&triage, std::fs::Permissions::from_mode(0o444)).unwrap();

    let listed = nunki::findings::pending(&project, "a").unwrap();
    let err = nunki::findings::ratify_all(&project, "a", &listed).unwrap_err();
    std::fs::set_permissions(&triage, std::fs::Permissions::from_mode(0o644)).unwrap();
    let said = err.to_string();
    assert!(said.contains(&format!("stopped at `{id}`")), "{said}");
    assert!(
        said.contains(&format!("on `{id}` itself, before it failed")),
        "{said}"
    );
    assert!(!no_equivalence(&mutants::read(&a).unwrap().unwrap()));
}

/// What a test name may be: letters, digits and underscores, three at least.
#[test]
fn a_test_name_is_an_identifier_of_three_characters_at_least() {
    for name in ["abc", "a_b", "one_is_two", "test_42", "___"] {
        assert!(mutants::is_test_name(name), "{name:?}");
    }
    for name in ["", " ", "ab", "u8", "fn", "a-b", "a b", "été", "two::is"] {
        assert!(!mutants::is_test_name(name), "{name:?}");
    }
}

// The 07:38 campaign's survivors on c48b3e6.

/// A coder's file that is there and cannot be read — here a directory — is
/// an error, never an empty triage, for every reader of it.
#[test]
fn a_coders_file_that_cannot_be_opened_is_not_an_empty_one() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(mutants::TRIAGE_FILE)).unwrap();
    let err = mutants::read_triage(dir.path()).unwrap_err();
    assert!(err.to_string().contains(mutants::TRIAGE_FILE), "{err}");
    assert!(mutants::foreign(dir.path()).is_err());
}

/// A ratification is recorded on its own entry only: not on one of the
/// same mutation given on another commit or another mission, nor on another
/// mutation or another file given where it was.
#[test]
fn a_ratification_is_recorded_on_its_own_entry_only() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    ruled_on_a(&project);
    let mut registry = equivalences::read(&project.hq_root).unwrap();
    let own = registry.entries[0].clone();
    let variant = |change: &dyn Fn(&mut equivalences::Entry)| {
        let mut e = own.clone();
        change(&mut e);
        e
    };
    registry.entries.extend([
        variant(&|e| e.commit = "0000000000000000000000000000000000000000".into()),
        variant(&|e| e.mission = "z".into()),
        variant(&|e| e.description = "another mutation".into()),
        variant(&|e| e.file = "src/other.rs".into()),
    ]);
    equivalences::write(&project.hq_root, &registry).unwrap();

    let ruling = equivalences::Ruling {
        hq_root: &project.hq_root,
        tree: &project.root,
        mission: "b",
        by: "the HQ",
    };
    let (campaign, _) = campaign_on(&project, "b", &[on_lib(2)]);
    let registered = equivalences::ratified(
        &ruling,
        &campaign,
        &campaign.survivors[0].id,
        (&own.mission, &own.commit),
    );
    assert_eq!(registered, Registered::Done(1));
    let after = equivalences::read(&project.hq_root).unwrap().entries;
    assert_eq!(after[0].ratified.len(), 1, "its own entry");
    assert!(
        after[1..].iter().all(|e| e.ratified.is_empty()),
        "{:?}",
        after
    );
}

/// `--lift` on a ruling an older `nunki` carried — kept with the commit it
/// was first given on — takes its registry entry out by that origin, though
/// the code has changed since.
#[test]
fn lifting_an_older_carried_ruling_takes_its_entry_out_by_origin() {
    let dir = tempfile::tempdir().unwrap();
    let project = registry_project(dir.path(), BODY);
    let id = ruled_on_a(&project);
    let ruled_at = git(&project.root, &["rev-parse", "HEAD"]);
    commit_lib(&project, "pub fn one() -> u8 {\n    1 + 0\n}\n");
    let head = git(&project.root, &["rev-parse", "HEAD"]);
    let a = mission_dir(&project, "a");
    let mut carried = on_lib(2);
    carried.id = id.clone();
    carried.outcome = Some(Triage::Equivalent {
        why: "nothing reads the value".into(),
        carried_from: Some(ruled_at),
    });
    mutants::write(
        &a,
        &Campaign {
            chain: Default::default(),
            fingerprint: "f".into(),
            head,
            date: "d".into(),
            survivors: vec![carried],
            tried: None,
        },
    )
    .unwrap();
    assert_eq!(
        nunki::findings::lift_equivalent(&project, "a", &id).unwrap(),
        Registered::Done(1)
    );
    assert!(
        equivalences::read(&project.hq_root)
            .unwrap()
            .entries
            .is_empty()
    );
}

// ---------------------------------------------------------------------------
// The chain of campaigns: at `standard`, a later campaign covers what changed
// since the previous one (SPEC 4.4, gate 7).
// ---------------------------------------------------------------------------

use mutants::{Chain, Changes, Link, Replay, Scope};
use nunki::mission::Rigor;

/// A campaign on file at `head`, full, with what it ran with.
fn on_file_at(head: &str, survivors: Vec<Survivor>, tried: Option<u32>) -> Campaign {
    Campaign {
        fingerprint: format!("fp-{head}"),
        head: head.into(),
        date: "2026-10-01T10:00:00Z".into(),
        survivors,
        tried,
        chain: Chain {
            scope: Scope::Full {
                why: "the first campaign of this mission".into(),
            },
            tooling: Some("tools-1".into()),
            earlier: vec![],
        },
    }
}

/// What [`mutants::scope`] answers at `standard` on a previous campaign
/// that passed, is an ancestor and ran with the same tooling — unless
/// `change` makes one condition false.
fn scope_with(
    change: impl FnOnce(
        &mut Rigor,
        &mut Replay,
        &mut Option<Campaign>,
        &mut Option<&str>,
        &mut bool,
        &mut bool,
    ),
) -> Scope {
    let (mut rigor, mut replay) = (Rigor::Standard, Replay::WhenChanged);
    let mut previous = Some(on_file_at("aaaaaaaaaaaaaaaa", vec![], Some(10)));
    let mut tooling = Some("tools-1");
    let (mut owes, mut ancestor) = (false, true);
    change(
        &mut rigor,
        &mut replay,
        &mut previous,
        &mut tooling,
        &mut owes,
        &mut ancestor,
    );
    mutants::scope(
        rigor,
        replay,
        previous.as_ref(),
        tooling,
        |_| owes.then(|| "3 of 10 tried mutant(s) killed".to_string()),
        |_| ancestor,
    )
}

/// At standard, after a passing campaign, the next one is partial and
/// continues from that campaign's `HEAD`.
#[test]
fn at_standard_a_campaign_after_a_passing_one_is_partial_from_its_head() {
    assert_eq!(
        scope_with(|_, _, _, _, _, _| {}),
        Scope::Partial {
            since: "aaaaaaaaaaaaaaaa".into()
        }
    );
}

/// Every condition the chain rests on, made false one at a time: each gives
/// a full campaign, and its record says why.
#[test]
fn each_refusal_gives_a_full_campaign_with_its_reason_recorded() {
    let full = |scope: Scope| match scope {
        Scope::Full { why } => why,
        partial => panic!("a refusal gave a partial campaign: {partial:?}"),
    };
    let cases: Vec<(&str, Scope)> = vec![
        (
            "`critical` mission",
            scope_with(|rigor, _, _, _, _, _| *rigor = Rigor::Critical),
        ),
        (
            "--again",
            scope_with(|_, replay, _, _, _, _| *replay = Replay::Now),
        ),
        (
            "the first campaign",
            scope_with(|_, _, previous, _, _, _| *previous = None),
        ),
        (
            "gate 7 did not pass on the previous campaign",
            scope_with(|_, _, _, _, owes, _| *owes = true),
        ),
        (
            "is not an ancestor of HEAD",
            scope_with(|_, _, _, _, _, ancestor| *ancestor = false),
        ),
        (
            "could not be read",
            scope_with(|_, _, _, tooling, _, _| *tooling = None),
        ),
        (
            "recorded nothing of what it ran with",
            scope_with(|_, _, previous, _, _, _| previous.as_mut().unwrap().chain.tooling = None),
        ),
        (
            "changed since the previous campaign",
            scope_with(|_, _, _, tooling, _, _| *tooling = Some("tools-2")),
        ),
    ];
    for (said, scope) in cases {
        let why = full(scope);
        assert!(why.contains(said), "{said:?} not in {why:?}");
    }
    // The reason of a failed previous campaign carries what it owes.
    let why = full(scope_with(|_, _, _, _, owes, _| *owes = true));
    assert!(why.contains("3 of 10 tried mutant(s) killed"), "{why}");
    assert!(why.contains("aaaaaaaaaaaa"), "{why}");
}

/// The previous campaign's failure is gate 7's own rule, asked through
/// [`mutants::owed`]: below the threshold, the next campaign is full.
#[test]
fn a_previous_campaign_below_the_threshold_gives_a_full_campaign() {
    let three_open = vec![one("m1", 1, "a"), one("m2", 2, "b"), one("m3", 3, "c")];
    let previous = on_file_at("aaaaaaaaaaaaaaaa", three_open, Some(10));
    let owed = |c: &Campaign| mutants::owed(c, &Default::default(), Rigor::Standard, 80);
    let scope = mutants::scope(
        Rigor::Standard,
        Replay::WhenChanged,
        Some(&previous),
        Some("tools-1"),
        owed,
        |_| true,
    );
    assert!(
        matches!(&scope, Scope::Full { why } if why.contains("7 of 10")),
        "{scope:?}"
    );
    // And with two of them answered it passed, so the next is partial.
    let answered: std::collections::BTreeMap<String, Triage> = ["m1", "m2"]
        .iter()
        .map(|id| {
            (
                id.to_string(),
                Triage::Killed {
                    test: "the_test".into(),
                },
            )
        })
        .collect();
    let scope = mutants::scope(
        Rigor::Standard,
        Replay::WhenChanged,
        Some(&previous),
        Some("tools-1"),
        |c: &Campaign| mutants::owed(c, &answered, Rigor::Standard, 80),
        |_| true,
    );
    assert!(matches!(scope, Scope::Partial { .. }), "{scope:?}");
}

/// A partial campaign is handed the previous campaign's `HEAD` as its base,
/// and only the touched paths that changed since; a full one the fork point
/// and every touched path.
#[test]
fn a_partial_campaign_is_given_the_previous_head_as_its_base() {
    let dir = tempfile::tempdir().unwrap();
    let (project, slot) = context(dir.path());
    let tree = &slot.tree;
    let fork = git(tree, &["rev-parse", "dev"]);
    write(tree, "src/lib.rs", "pub fn one() -> u8 { 2 }\n");
    write(tree, "src/other.rs", "pub fn other() {}\n");
    git(tree, &["add", "-A"]);
    git(tree, &["commit", "-q", "-m", "L1"]);
    let first = git(tree, &["rev-parse", "HEAD"]);
    write(tree, "src/lib.rs", "pub fn one() -> u8 { 3 }\n");
    git(tree, &["commit", "-q", "-am", "volet"]);

    let touched = nunki::gate::touched_since_base(tree, "dev").unwrap();
    assert_eq!(touched, ["src/lib.rs", "src/other.rs"]);
    let judged = nunki::run::judged(&project, "rust");

    let partial = Scope::Partial {
        since: first.clone(),
    };
    let cmd = mutants::launch_command(tree, &judged, "fp", &touched, &fork, &partial).unwrap();
    assert_eq!(
        cmd.env.get(mutants::BASE_ENV).map(String::as_str),
        Some(first.as_str())
    );
    assert_eq!(cmd.args, ["fp", "src/lib.rs"]);

    let full = Scope::Full { why: "x".into() };
    let cmd = mutants::launch_command(tree, &judged, "fp", &touched, &fork, &full).unwrap();
    assert_eq!(
        cmd.env.get(mutants::BASE_ENV).map(String::as_str),
        Some(fork.as_str())
    );
    assert_eq!(cmd.args, ["fp", "src/lib.rs", "src/other.rs"]);
}

/// The tool's version is read by the command the script names, and what a
/// campaign ran with is unknown when any stack's version is.
#[test]
fn what_a_campaign_runs_with_is_its_script_and_its_tools_version() {
    let script = "#!/bin/sh\n# comment\n# nunki-tool-version: cargo mutants --version\nexit 0\n";
    assert_eq!(
        mutants::tool_version_command(script),
        Some("cargo mutants --version")
    );
    assert_eq!(mutants::tool_version_command("#!/bin/sh\nexit 0\n"), None);
    assert_eq!(
        mutants::tool_version_command("# nunki-tool-version:   \n"),
        None
    );

    let entry = |script: &str, version: Option<&str>| {
        vec![(
            "rust".to_string(),
            script.to_string(),
            version.map(str::to_string),
        )]
    };
    let base = mutants::tooling_manifest(&entry(script, Some("cargo-mutants 27.1.0\n")));
    assert!(base.is_some());
    assert_eq!(mutants::tooling_manifest(&entry(script, None)), None);
    assert_eq!(mutants::tooling_manifest(&entry(script, Some(" \n"))), None);
    assert_ne!(
        mutants::tooling_manifest(&entry(script, Some("cargo-mutants 27.2.0"))),
        base,
        "a new tool version is a change"
    );
    assert_ne!(
        mutants::tooling_manifest(&entry(
            &format!("{script}# edited\n"),
            Some("cargo-mutants 27.1.0")
        )),
        base,
        "an edited script is a change"
    );
    // Every stack's version must be known.
    let mut two = entry(script, Some("cargo-mutants 27.1.0"));
    two.push(("next".into(), "#!/bin/sh\n".into(), None));
    assert_eq!(mutants::tooling_manifest(&two), None);
}

/// Every shipped `mutation.sh` names how to read its tool's version, so a
/// project on today's fragments can chain its campaigns.
#[test]
fn every_shipped_mutation_script_names_its_tools_version_command() {
    for stack in nunki::init::KNOWN_STACKS {
        let script = nunki::init::fragment(stack)
            .into_iter()
            .find(|(name, _, _)| *name == mutants::SCRIPT)
            .map(|(_, body, _)| body)
            .expect("each stack ships a mutation.sh");
        assert!(
            mutants::tool_version_command(&script).is_some(),
            "{stack}'s mutation.sh names no tool version command"
        );
    }
}

const BEFORE: &str =
    "pub fn a() -> u8 {\n    1\n}\npub fn b() -> u8 {\n    2\n}\npub fn c() -> u8 {\n    3\n}\n";
/// A line added on top, `b`'s body changed, a line inserted inside `c`.
const AFTER: &str = "// header\npub fn a() -> u8 {\n    1\n}\npub fn b() -> u8 {\n    20\n}\npub fn c() -> u8 {\n    // note\n    3\n}\n";

fn at(id: &str, file: &str, line: u32, end_line: Option<u32>) -> Survivor {
    Survivor {
        id: id.into(),
        file: file.into(),
        line,
        end_line,
        description: format!("mutation {id}"),
        outcome: None,
        refused: None,
        found_on: None,
    }
}

/// Two commits of a repository: `BEFORE`, then `AFTER`, with an untouched
/// file beside.
fn before_and_after(dir: &Path) -> (PathBuf, String, String) {
    let tree = repo(dir);
    write(&tree, "src/lib.rs", BEFORE);
    write(&tree, "src/other.rs", "pub fn other() -> u8 {\n    9\n}\n");
    git(&tree, &["add", "-A"]);
    git(&tree, &["commit", "-q", "-m", "before"]);
    let before = git(&tree, &["rev-parse", "HEAD"]);
    write(&tree, "src/lib.rs", AFTER);
    git(&tree, &["commit", "-q", "-am", "after"]);
    let after = git(&tree, &["rev-parse", "HEAD"]);
    (tree, before, after)
}

/// The survivors the change reaches are dropped — judged on the whole span,
/// a span nobody knows reached — and the others kept, at the lines they now
/// stand on, naming the campaign that found them.
#[test]
fn survivors_the_volet_reached_are_dropped_and_the_others_kept_where_they_stand() {
    let dir = tempfile::tempdir().unwrap();
    let (tree, before, after) = before_and_after(dir.path());
    let previous = on_file_at(
        &before,
        vec![
            Survivor {
                outcome: Some(Triage::Equivalent {
                    why: "ruled".into(),
                    carried_from: None,
                }),
                refused: Some(mutants::Refusal {
                    proposed: "p".into(),
                    because: "b".into(),
                }),
                ..at("in-a", "src/lib.rs", 2, None)
            },
            at("changed", "src/lib.rs", 5, None),
            at("body-of-b", "src/lib.rs", 4, Some(6)),
            at("in-c", "src/lib.rs", 8, Some(8)),
            at("body-of-c", "src/lib.rs", 7, Some(9)),
            at("unknown-span", "src/lib.rs", 8, Some(0)),
            at("elsewhere", "src/other.rs", 2, None),
        ],
        Some(30),
    );
    let changes = Changes::between(&tree, &before, &after).unwrap();
    let (earlier, kept) = mutants::continued(&previous, &changes);
    // What the HQ said comes back through `carry` alone, never kept as it was.
    assert!(
        kept.iter()
            .all(|s| s.outcome.is_none() && s.refused.is_none())
    );

    let kept: Vec<(&str, u32, Option<u32>, Option<&str>)> = kept
        .iter()
        .map(|s| (s.id.as_str(), s.line, s.end_line, s.found_on.as_deref()))
        .collect();
    assert_eq!(
        kept,
        [
            // One line added above: moved down by one.
            ("in-a", 3, None, Some(before.as_str())),
            // Two lines added above (the header, the note before it).
            ("in-c", 10, Some(10), Some(before.as_str())),
            // Another file, untouched: where it was.
            ("elsewhere", 2, None, Some(before.as_str())),
        ]
    );
    assert_eq!(
        earlier,
        [Link {
            head: before.clone(),
            date: previous.date.clone(),
            scope: previous.chain.scope.clone(),
            tried: Some(30),
        }]
    );
}

/// An insertion just above or just below a span leaves it standing; inside
/// it, between two of its lines, it reaches it.
#[test]
fn an_insertion_reaches_a_span_only_from_inside() {
    let diff = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -4,0 +5 @@\n+x\n";
    let changes = Changes::parse(diff);
    let kept = |line, end| changes.kept(&at("s", "f.rs", line, Some(end)));
    assert_eq!(kept(5, 6), Some((6, Some(7))), "inserted just above");
    assert_eq!(kept(2, 4), Some((2, Some(4))), "inserted just below");
    assert_eq!(kept(4, 5), None, "inserted between its lines");
    assert_eq!(
        kept(4, 4),
        Some((4, Some(4))),
        "a one-line span has no inside"
    );
    // A replacement reaches what it overlaps, and nothing next to it.
    let changes = Changes::parse("--- a/f.rs\n+++ b/f.rs\n@@ -4,2 +4,3 @@\n");
    let kept = |line, end| changes.kept(&at("s", "f.rs", line, Some(end)));
    assert_eq!(kept(5, 7), None);
    assert_eq!(kept(1, 4), None);
    assert_eq!(kept(1, 3), Some((1, Some(3))));
    assert_eq!(kept(6, 8), Some((7, Some(9))));
    // A file removed is reached whole; a file added holds no old survivor.
    let changes = Changes::parse("--- a/f.rs\n+++ /dev/null\n@@ -1,9 +0,0 @@\n");
    assert_eq!(changes.kept(&at("s", "f.rs", 3, None)), None);
}

/// A partial campaign recorded: the chain continued, the kept survivors
/// beside its own, and what the HQ said on them back only as carry gives it
/// — a ruling as a proposal marked carried, a refusal as a refusal.
#[test]
fn a_partial_campaign_keeps_what_the_hq_said_only_as_carry_gives_it() {
    let dir = tempfile::tempdir().unwrap();
    let (tree, before, after) = before_and_after(dir.path());
    let mission = dir.path().join("mission");
    std::fs::create_dir_all(&mission).unwrap();
    let hq = dir.path().join("hq");

    let mut ruled = at("in-a", "src/lib.rs", 2, None);
    ruled.outcome = Some(Triage::Equivalent {
        why: "nothing reads it".into(),
        carried_from: None,
    });
    let mut refused = at("elsewhere", "src/other.rs", 2, None);
    refused.refused = Some(mutants::Refusal {
        proposed: "same".into(),
        because: "a test can see it".into(),
    });
    mutants::write(
        &mission,
        &on_file_at(
            &before,
            vec![ruled, refused, at("changed", "src/lib.rs", 5, None)],
            Some(30),
        ),
    )
    .unwrap();

    let found = at("new-one", "src/lib.rs", 6, None);
    let log = format!(
        "{}\n{{\"campaign\":\"done\",\"tried\":4,\"found\":4}}\n",
        serde_json::to_string(&found).unwrap()
    );
    let chain = Chain {
        scope: Scope::Partial {
            since: before.clone(),
        },
        tooling: Some("tools-1".into()),
        earlier: vec![],
    };
    mutants::record_finished_with_registry(&mission, &hq, &tree, "fp-after", &after, &log, &chain)
        .unwrap();
    let campaign = mutants::read(&mission).unwrap().unwrap();

    assert_eq!(campaign.tried, Some(4));
    assert_eq!(campaign.chain.scope, chain.scope);
    assert_eq!(campaign.chain.earlier.len(), 1);
    assert_eq!(campaign.chain.earlier[0].head, before);
    assert_eq!(campaign.chain.earlier[0].tried, Some(30));
    let by_id = |id: &str| campaign.survivors.iter().find(|s| s.id == id);
    assert!(by_id("changed").is_none(), "reached by the volet: dropped");
    assert_eq!(by_id("new-one").unwrap().found_on, None);
    let kept = by_id("in-a").unwrap();
    assert_eq!(kept.found_on.as_deref(), Some(before.as_str()));
    assert_eq!(
        kept.outcome,
        Some(Triage::ProposedByNunki {
            why: "nothing reads it".into(),
            from: mutants::ProposedFrom::Carried {
                commit: before.clone()
            },
        }),
        "a ruling comes back as a carried proposal, never as a ruling"
    );
    let kept = by_id("elsewhere").unwrap();
    assert_eq!(kept.refused.as_ref().unwrap().because, "a test can see it");
    assert_eq!(kept.outcome, None);
    assert!(
        campaign
            .survivors
            .iter()
            .all(|s| !matches!(s.outcome, Some(Triage::Equivalent { .. }))),
        "nothing is ruled by the chain itself"
    );
}

/// A survivor the partial campaign found again is listed once, as its own.
#[test]
fn a_survivor_found_again_is_listed_once_as_the_new_campaigns() {
    let dir = tempfile::tempdir().unwrap();
    let (tree, before, after) = before_and_after(dir.path());
    let mission = dir.path().join("mission");
    std::fs::create_dir_all(&mission).unwrap();
    mutants::write(
        &mission,
        &on_file_at(
            &before,
            vec![at("elsewhere", "src/other.rs", 2, None)],
            Some(30),
        ),
    )
    .unwrap();
    let again = at("elsewhere", "src/other.rs", 2, None);
    let log = format!(
        "{}\n{{\"campaign\":\"done\",\"tried\":1,\"found\":1}}\n",
        serde_json::to_string(&again).unwrap()
    );
    let chain = Chain {
        scope: Scope::Partial { since: before },
        ..Chain::default()
    };
    mutants::record_finished_with_registry(
        &mission,
        &dir.path().join("hq"),
        &tree,
        "fp",
        &after,
        &log,
        &chain,
    )
    .unwrap();
    let campaign = mutants::read(&mission).unwrap().unwrap();
    assert_eq!(campaign.survivors.len(), 1);
    assert_eq!(campaign.survivors[0].found_on, None);
}

/// A partial campaign that does not continue the campaign on file answers
/// only for what changed since, and is not recorded alone.
#[test]
fn a_partial_campaign_whose_chain_broke_is_not_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let (tree, before, after) = before_and_after(dir.path());
    let mission = dir.path().join("mission");
    std::fs::create_dir_all(&mission).unwrap();
    let on_file = on_file_at(&after, vec![], Some(3));
    mutants::write(&mission, &on_file).unwrap();
    let chain = Chain {
        scope: Scope::Partial { since: before },
        ..Chain::default()
    };
    let err = mutants::record_finished_with_registry(
        &mission,
        &dir.path().join("hq"),
        &tree,
        "fp",
        &after,
        "{\"campaign\":\"done\",\"tried\":0,\"found\":0}\n",
        &chain,
    )
    .unwrap_err();
    assert!(matches!(err, mutants::MutantsError::Broken(_)), "{err}");
    assert_eq!(mutants::read(&mission).unwrap(), Some(on_file));
}

/// A chain of two: an earlier full campaign and the partial one on file,
/// with `open` survivors of the partial one left without an outcome.
fn chain_of(earlier_tried: u32, earlier_open: u32, tried: u32, open: u32) -> Campaign {
    let mut survivors: Vec<Survivor> = (0..earlier_open)
        .map(|n| {
            let mut s = at(&format!("old-{n}"), "src/lib.rs", n + 1, None);
            s.found_on = Some("aaaaaaaaaaaaaaaa".into());
            s
        })
        .collect();
    survivors.extend((0..open).map(|n| at(&format!("new-{n}"), "src/lib.rs", n + 100, None)));
    Campaign {
        fingerprint: "fp".into(),
        head: "bbbbbbbbbbbbbbbb".into(),
        date: "d".into(),
        survivors,
        tried: Some(tried),
        chain: Chain {
            scope: Scope::Partial {
                since: "aaaaaaaaaaaaaaaa".into(),
            },
            tooling: Some("tools-1".into()),
            earlier: vec![Link {
                head: "aaaaaaaaaaaaaaaa".into(),
                date: "d".into(),
                scope: Scope::Full { why: String::new() },
                tried: Some(earlier_tried),
            }],
        },
    }
}

/// Every campaign of the chain on its own mutants: a partial campaign below
/// the threshold owes, though the chain's total would pass; an earlier one
/// that owes is named; both passing, nothing is owed.
#[test]
fn what_a_chain_owes_is_judged_campaign_by_campaign() {
    let none = Default::default();
    // 7 of 10 on the partial one: red, though 197 of 200 overall would pass.
    let owed = mutants::owed(&chain_of(190, 0, 10, 3), &none, Rigor::Standard, 80)
        .expect("a partial campaign below the threshold owes");
    assert!(
        owed.contains("the partial campaign at bbbbbbbbbbbb"),
        "{owed}"
    );
    assert!(owed.contains("7 of 10"), "{owed}");
    // The earlier one below it, the later one perfect.
    let owed = mutants::owed(&chain_of(10, 3, 50, 0), &none, Rigor::Standard, 80)
        .expect("an earlier campaign below the threshold owes");
    assert!(owed.contains("the full campaign at aaaaaaaaaaaa"), "{owed}");
    assert!(!owed.contains("bbbbbbbbbbbb"), "{owed}");
    assert_eq!(
        mutants::owed(&chain_of(10, 2, 10, 2), &none, Rigor::Standard, 80),
        None
    );
}

/// Each campaign of the chain is said in a line: full or partial, from
/// which commit, what it tried, killed and left.
#[test]
fn each_campaign_of_the_chain_is_said_with_its_own_counts() {
    let said = mutants::chain_said(&chain_of(190, 1, 10, 3), &Default::default());
    assert_eq!(
        said,
        [
            "full at aaaaaaaaaaaa — tried 190, killed 189, 1 survivor(s), 1 without an outcome",
            "partial since aaaaaaaaaaaa at bbbbbbbbbbbb — tried 10, killed 7, 3 survivor(s), \
             3 without an outcome",
        ]
    );
}

/// A survivor naming a campaign the chain does not hold is listed, so it is
/// owed somewhere: with the campaign on file.
#[test]
fn a_survivor_of_no_known_campaign_is_judged_with_the_current_one() {
    let mut campaign = chain_of(10, 0, 10, 0);
    let mut stray = at("stray", "src/lib.rs", 1, None);
    stray.found_on = Some("cccccccccccc".into());
    campaign.survivors.push(stray);
    let parts = mutants::parts(&campaign);
    assert_eq!(parts.len(), 2);
    assert!(parts[0].campaign.survivors.is_empty());
    assert!(parts[1].current);
    assert_eq!(parts[1].campaign.survivors.len(), 1);
}

/// A campaign file written before the chain existed, and an in-flight
/// record likewise, read as a full campaign with nothing before it.
#[test]
fn an_old_campaign_file_reads_as_a_full_campaign_alone() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(mutants::FILE),
        "{\"fingerprint\":\"f\",\"head\":\"h\",\"date\":\"d\",\"tried\":3,\"survivors\":[\
         {\"id\":\"a\",\"file\":\"src/lib.rs\",\"line\":1}]}\n",
    )
    .unwrap();
    let campaign = mutants::read(dir.path()).unwrap().unwrap();
    assert_eq!(campaign.chain, Chain::default());
    assert_eq!(campaign.chain.scope, Scope::Full { why: String::new() });
    assert_eq!(campaign.survivors[0].found_on, None);
    let parts = mutants::parts(&campaign);
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].campaign.survivors.len(), 1);

    let running: mutants::Running = serde_json::from_str(
        "{\"fingerprint\":\"f\",\"head\":\"h\",\"started_at\":\"s\",\"container\":\"c\",\
         \"pid\":1,\"log\":\"/l\",\"deadline_minutes\":5}",
    )
    .unwrap();
    assert_eq!(running.chain.scope, Scope::Full { why: String::new() });
}

/// What a campaign's log claims of the chain is never read: which campaign
/// found a survivor is `nunki`'s to record.
#[test]
fn a_log_cannot_say_an_earlier_campaign_found_a_survivor() {
    let survivors =
        mutants::parse("{\"id\":\"a\",\"file\":\"f\",\"line\":1,\"found_on\":\"aaaaaaaaaaaa\"}\n");
    assert_eq!(survivors[0].found_on, None);
}

/// A finished partial campaign read back in its slot: what the monitor and
/// `nunki mission mutants` print, chain and all — and one whose chain broke
/// under it is forgotten and said, never asked about again and never
/// recorded alone.
#[test]
fn a_partial_campaign_is_read_back_with_its_chain_or_forgotten_when_it_broke() {
    use nunki::engine::{ExecOutput, fake::FakeEngine};

    let dir = tempfile::tempdir().unwrap();
    let (project, slot) = context(dir.path());
    let tree = slot.tree.clone();
    write(&tree, "src/lib.rs", "pub fn one() -> u8 { 2 }\n");
    git(&tree, &["commit", "-q", "-am", "L1"]);
    let first = git(&tree, &["rev-parse", "HEAD"]);
    write(&tree, "src/lib.rs", "pub fn one() -> u8 { 3 }\n");
    git(&tree, &["commit", "-q", "-am", "volet"]);
    let head = git(&tree, &["rev-parse", "HEAD"]);
    let profile = nunki::run::profile_path(&project, &slot.name);
    std::fs::create_dir_all(profile.parent().unwrap()).unwrap();
    std::fs::write(&profile, "services: {}\n").unwrap();
    let mission = dir.path().join("mission");
    std::fs::create_dir_all(&mission).unwrap();
    let log = mission.join("mutants.log");

    let read_back = |on_file: &Campaign| {
        mutants::write(&mission, on_file).unwrap();
        std::fs::write(&log, "{\"campaign\":\"done\",\"tried\":2,\"found\":2}\n").unwrap();
        mutants::write_running(
            &project.hq_root,
            &slot.name,
            &mutants::Running {
                chain: Chain {
                    scope: Scope::Partial {
                        since: first.clone(),
                    },
                    tooling: Some("tools-1".into()),
                    earlier: vec![],
                },
                fingerprint: "abc1234".into(),
                head: head.clone(),
                started_at: "2026-09-19T02:00:00Z".into(),
                container: "cafe1234".into(),
                pid: Some(41),
                log: log.clone(),
                deadline_minutes: 45,
            },
        )
        .unwrap();
        let engine: std::sync::Arc<dyn nunki::engine::Engine> = std::sync::Arc::new(
            FakeEngine::default()
                .with_liveness("cafe1234", nunki::engine::Liveness::Running)
                .with_exec(ExecOutput {
                    status: 0,
                    stdout: "nunki-run-ended\n".into(),
                    stderr: String::new(),
                }),
        );
        let progress = mutants::read_back(&project, &slot, engine, &mission)
            .unwrap()
            .expect("a campaign is filed");
        assert!(
            mutants::read_running(&project.hq_root, &slot.name)
                .unwrap()
                .is_none(),
            "an ended campaign is forgotten"
        );
        progress
    };

    // Continuing the campaign on file: recorded, and said campaign by campaign.
    match read_back(&on_file_at(&first, vec![], Some(12))) {
        mutants::Progress::Finished { survivors, chain } => {
            assert_eq!(survivors, 0);
            assert_eq!(chain.len(), 2, "{chain:?}");
            assert!(chain[0].starts_with("full at "), "{chain:?}");
            assert!(chain[0].contains("tried 12, killed 12"), "{chain:?}");
            assert!(chain[1].starts_with("partial since "), "{chain:?}");
            assert!(chain[1].contains("tried 2, killed 2"), "{chain:?}");
        }
        other => panic!("a finished partial campaign was read as {other:?}"),
    }

    // The campaign on file is another: lost, said, and nothing written.
    let other = on_file_at(&head, vec![], Some(5));
    match read_back(&other) {
        mutants::Progress::Lost(why) => {
            assert!(why.contains("is not recorded alone"), "{why}");
            assert!(why.contains("--again"), "{why}");
        }
        other => panic!("a broken chain was read as {other:?}"),
    }
    assert_eq!(mutants::read(&mission).unwrap(), Some(other));
}
