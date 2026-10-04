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
        "dev",
        45,
        mutants::Replay::WhenChanged,
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
        "dev",
        45,
        mutants::Replay::WhenChanged,
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
        fingerprint: "abc1234".into(),
        head: "def5678".into(),
        tried: None,
        date: "2026-09-10T12:00:00Z".into(),
        survivors: vec![Survivor {
            id: "src/lib.rs:3".into(),
            file: "src/lib.rs".into(),
            line: 3,
            description: "replace one with 0".into(),
            outcome: Some(Triage::Equivalent {
                why: "the branch is unreachable from any caller".into(),
                carried_from: None,
            }),
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
            carried_from: None
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
            "dev",
            45,
            mutants::Replay::WhenChanged,
        )
        .unwrap()
    };

    match go() {
        Progress::Started { fingerprint } => println!("started {fingerprint}"),
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
        Progress::Finished { survivors } => assert_eq!(survivors, 2),
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
        Progress::Finished { survivors } => assert_eq!(
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
            fingerprint: "abc1234".into(),
            head: "def5678".into(),
            tried: None,
            date: "2026-09-10T12:00:00Z".into(),
            survivors: vec![Survivor {
                id: "src/lib.rs:3".into(),
                file: "src/lib.rs".into(),
                line: 3,
                description: "replace one with 0".into(),
                outcome: None,
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
        id: id.into(),
        file: "src/cells.py".into(),
        line,
        description: description.into(),
        outcome: None,
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
/// is what a commit moves; the mutation is what was ruled on.
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
        Some(Triage::Equivalent {
            why: "'f' and 'F' agree on finite decimals".into(),
            carried_from: Some("def5678".into()),
        })
    );
}

/// A tool that renumbers its mutants must not lose the ruling either, as long
/// as the mutation is told apart without a doubt.
#[test]
fn a_ruling_follows_a_renamed_mutant_when_its_mutation_is_unique() {
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
    assert!(matches!(
        campaign.survivors[0].outcome,
        Some(Triage::Equivalent { .. })
    ));
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

/// A ruling carried twice still names the campaign it was first given on.
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
        Some(Triage::Equivalent { carried_from: Some(first), .. }) if first == "def5678"
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
            carried_from: None
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
            carried_from: None
        }
        .is_the_coders_to_give()
    );
    assert_eq!(
        Triage::Equivalent {
            why: "x".into(),
            carried_from: None
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
            id: format!("src/lib.rs:{i}:1: replace a with b"),
            file: "src/lib.rs".into(),
            line: i as u32 + 1,
            description: "replace a with b".into(),
            outcome: None,
        })
        .collect();
    mutants::write(
        dir,
        &mutants::Campaign {
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
        "dev",
        45,
        mutants::Replay::WhenChanged,
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
        "dev",
        45,
        mutants::Replay::WhenChanged,
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
