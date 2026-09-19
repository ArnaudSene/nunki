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
    let tree = repo(dir.path());
    let home = dir.path().join("nunki");
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
            forge_protection: Default::default(),
        },
        home,
    );
    let slot = nunki::slot::Slot {
        name: "one".into(),
        tree: tree.clone(),
    };
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
        &mission,
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
    assert!(mutants::read_running(&mission).unwrap().is_none());
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
        date: "2026-09-10T12:00:00Z".into(),
        survivors: vec![Survivor {
            id: "src/lib.rs:3".into(),
            file: "src/lib.rs".into(),
            line: 3,
            description: "replace one with 0".into(),
            outcome: Some(Triage::Equivalent {
                why: "the branch is unreachable from any caller".into(),
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
    assert_eq!(Triage::Equivalent { why: "x".into() }.test(), None);
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
         echo '{\"id\":\"src/lib.rs:1\",\"file\":\"src/lib.rs\",\"line\":1,\
         \"description\":\"replace one with 0\"}'\n\
         echo 'not a survivor, just chatter'\n\
         echo '{\"id\":\"src/lib.rs:1b\",\"file\":\"src/lib.rs\",\"line\":1,\
         \"description\":\"replace one with 255\"}'\n",
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
             sleep 600\"]\n\
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
    assert!(mutants::read_running(&mission).unwrap().is_some());

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
        mutants::read_running(&mission).unwrap().is_none(),
        "the record is cleared when the campaign is on file"
    );

    let campaign = mutants::read(&mission).unwrap().expect("it is on file");
    assert_eq!(campaign.survivors.len(), 2, "the chatter is not a survivor");
    assert_eq!(
        campaign.fingerprint,
        mutants::fingerprint(&tree, &touched).unwrap()
    );
    assert!(campaign.survivors.iter().all(|s| s.outcome.is_none()));

    // Asked again on the same content, it does not spend another campaign.
    match go() {
        Progress::Fresh { survivors } => assert_eq!(survivors, 2),
        other => panic!("it replays only when the touched files change: {other:?}"),
    }

    // And forced to run again on that same content — what a human does when
    // a campaign's answer looked wrong — it reports what **this** run found,
    // not that plus what the one before it found.
    //
    // Measured on `notes-3` on 2026-09-16: four campaigns on one fingerprint
    // wrote into one log, because the spawner appends and the name carries
    // the fingerprint. The runs found 5, then 1, then none, then none;
    // `MUTANTS.json` said six, every one of them dead, and gate 7 sent a
    // coder back three times for mutants that no longer existed.
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
            why: "no caller reaches it".into()
        })
    );
    // And it landed in the HQ's file, not in the agent's.
    assert!(mutants::read_triage(dir.path()).unwrap().is_empty());
}

#[test]
fn only_the_two_outcomes_that_rest_on_a_test_are_the_coders_to_give() {
    assert!(Triage::Killed { test: "x".into() }.is_the_coders_to_give());
    assert!(Triage::Bug { test: "x".into() }.is_the_coders_to_give());
    // The judgement nobody can check.
    assert!(!Triage::Equivalent { why: "x".into() }.is_the_coders_to_give());
    assert_eq!(Triage::Equivalent { why: "x".into() }.kind(), "equivalent");
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
/// It had never been run. Every test of gate 7 used a stub that echoed a JSON
/// line, and the shipped script was wrong in two ways that only running it
/// could show — measured against cargo-mutants 27.1.0 on 2026-09-10:
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
/// The sentence used to ask for "one of the three outcomes" whatever the
/// campaign found — measured on `notes-api` on 2026-09-16, where a clean
/// campaign still demanded outcomes for nobody. A line that reads the same
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
/// Measured on 2026-09-16: `nunki verify` printed
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
/// Before this existed the only way past it was deleting `MUTANTS.json` by
/// hand, and on 2026-09-17 that took `MUTANTS.triage.json` with it: the engine
/// replaced the missing file with a directory and the mission came down on
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
