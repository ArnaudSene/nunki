//! Starting a run (SPEC 4.3): what `hq mission start` decides before any
//! container exists.

use std::path::Path;

use hq::harness::Role;
use hq::project::{Config, Project, ProtectedPaths};
use hq::role;
use hq::run;

fn project(dir: &Path) -> Project {
    Project::at(
        dir.join("repo"),
        Config {
            harness: "claude-code".to_string(),
            forge: vec!["github.com".to_string()],
            stacks: vec!["rust".to_string()],
            protected_branches: vec!["main".to_string()],
            protected_paths: ProtectedPaths::default(),
            bounds: Default::default(),
            credentials: None,
            run: None,
        },
        dir.join("hq"),
    )
}

#[test]
fn the_token_is_read_from_the_hq_and_never_from_the_tree() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    std::fs::create_dir_all(&project.hq_root).unwrap();

    // Its home is the HQ, outside the repository: a token in the tree would
    // be in every slot and every container (SPEC 3.3).
    let file = run::token_file(&project);
    assert!(!file.starts_with(&project.root));
    assert!(file.ends_with("token"));

    assert!(run::token(&project).is_none());
    std::fs::write(&file, "  sk-ant-oat-example\n").unwrap();
    assert_eq!(
        run::token(&project).as_deref(),
        Some("sk-ant-oat-example"),
        "surrounding whitespace is not part of a token"
    );

    // An empty file is no token, not an empty one.
    std::fs::write(&file, "\n").unwrap();
    assert!(run::token(&project).is_none());
}

#[test]
fn a_mission_that_cannot_authenticate_does_not_start() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    std::fs::create_dir_all(project.hq_root.join("locks")).unwrap();
    let slot = hq::slot::Slot {
        name: "one".to_string(),
        tree: dir.path().join("slot"),
    };
    let engine: std::sync::Arc<dyn hq::engine::Engine> =
        std::sync::Arc::new(hq::engine::fake::FakeEngine::default());

    let err = run::start(&project, "m1", &slot, engine, "docker").unwrap_err();
    // Refused before anything is lifted, and it says what to do.
    assert!(matches!(err, run::RunError::NoToken(_)), "{err}");
    assert!(err.to_string().contains("claude setup-token"), "{err}");
}

#[test]
fn each_profile_is_written_where_a_restarted_hq_finds_it() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let path = run::profile_path(&project, "lot2");
    // At the HQ, one per slot, regenerated at every launch (SPEC 4.2).
    assert!(path.starts_with(&project.hq_root));
    assert!(path.ends_with("lot2.yml"));
    assert!(!path.starts_with(&project.root));
}

#[test]
fn the_role_prompts_say_what_the_role_may_not_do() {
    let coder = role::prompt(Role::Coder);
    let integrator = role::prompt(Role::Integrator);
    let security = role::prompt(Role::Security);

    for prompt in [&coder, &integrator, &security] {
        // The corollary that makes an autonomous run safe (SPEC 3.2).
        assert!(prompt.contains("refusing is safe"), "{prompt}");
        assert!(prompt.contains("ÉTAT DE REPRISE"), "{prompt}");
        assert!(prompt.contains("never push"), "{prompt}");
        // Exactly three files, named.
        assert!(prompt.contains("JOURNAL.md"), "{prompt}");
        assert!(prompt.contains("VERDICT.json"), "{prompt}");
    }

    // And each says the thing that is its own.
    assert!(coder.contains("break the decision and check the test goes red"));
    assert!(integrator.contains("not a reviewer"));
    assert!(security.contains("read-only"));
    assert!(
        security.contains("findings, not fixes"),
        "the security agent must not repair what it finds"
    );
}

#[test]
fn the_prompt_travels_as_a_file_and_never_into_the_slot() {
    // It is written into the mission folder, which is mounted read-only but
    // for the agent's three files — so the agent reads it and nothing in the
    // repository is touched (SPEC 3.3).
    assert_eq!(role::PROMPT_FILE, "ROLE.md");
    let three = hq::compose::AGENT_WRITABLE;
    assert!(
        !three.contains(&role::PROMPT_FILE),
        "the role prompt is not the agent's to rewrite"
    );
}

#[test]
fn the_mounts_are_the_ones_every_profile_agrees_on() {
    // The paths the agent is told about, and the paths the generator mounts,
    // must be the same or the agent is told where nothing is.
    assert_eq!(run::TREE_AT, "/work/tree");
    assert_eq!(run::MISSION_AT, "/work/mission");
}

/// The whole path, with a real engine and a stand-in for the harness: the
/// slot goes on its branch, the profile lifts, a run starts **inside** the
/// agent's container, its stream is captured on the host, and `hq` reads it
/// back.
///
/// The image carries a `claude` that prints one captured-looking stream and
/// exits. That is deliberate: what is being proved here is the plumbing, and
/// spending the subscription would prove the same thing more slowly. The real
/// CLI has its own live test (`tests/claude_code.rs`).
///
/// ```text
/// cargo test --test run -- --ignored --nocapture
/// ```
#[test]
#[ignore = "builds images and lifts containers; run by hand"]
fn live_a_mission_starts_and_its_run_is_read_back() {
    use hq::harness::{Harness, RunState};

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    let hq_root = dir.path().join("hq");
    std::fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q", "-b", "dev"]);
    git(&root, &["config", "user.email", "t@example.com"]);
    git(&root, &["config", "user.name", "Test"]);

    hq::init::init(&root, &hq_root, &["rust".to_string()]).unwrap();
    // A stand-in for the harness: it prints a result event and exits.
    std::fs::write(
        root.join(".hq/stacks/rust/Dockerfile"),
        "FROM alpine:3.22\n\
         RUN mkdir -p /work/tree /work/mission /run/hq\n\
         RUN printf '#!/bin/sh\\necho \\x27{\"type\":\"system\",\"subtype\":\"init\"}\\x27\\n\
         echo \\x27{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\
         \"usage\":{\"input_tokens\":11,\"output_tokens\":7}}\\x27\\n' > /usr/local/bin/claude \\\n\
         && chmod 0755 /usr/local/bin/claude\n",
    )
    .unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-qm", "first"]);

    let opened = Project::open(&root).unwrap();
    let project = Project::at(opened.root, opened.config, hq_root.clone());
    let engine_bin = std::env::var("HQ_ENGINE").unwrap_or_else(|_| "docker".to_string());
    hq::image::build(&project, "rust", &engine_bin).expect("the images build");

    let slot = hq::slot::add(&project, "one").expect("the slot is cloned");
    let header = hq::mission::Header {
        branch: "feat/alpha".to_string(),
        base: "dev".to_string(),
        lots: vec![hq::mission::Lot {
            id: "L1".to_string(),
            title: "the first lot".to_string(),
        }],
        integration: hq::mission::Integration::None {
            reason: "nothing external".to_string(),
        },
        security: hq::mission::Security::Gates,
        bounds: Default::default(),
    };
    hq::mission::dir::create(&hq_root, "alpha", &header, "Do the thing.").unwrap();
    std::fs::write(run::token_file(&project), "stand-in-token\n").unwrap();

    let engine: std::sync::Arc<dyn hq::engine::Engine> =
        std::sync::Arc::new(hq::engine::docker::Docker::real());
    let compose_project = hq::compose::project_name("one").unwrap();
    let profile = run::profile_path(&project, "one");
    let _ = engine.down(&profile, &compose_project, true);

    let state = run::start(&project, "alpha", &slot, engine.clone(), &engine_bin)
        .expect("the mission starts");

    // The slot is on the mission's branch, taken from its base.
    assert_eq!(
        git(&slot.tree, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "feat/alpha"
    );

    // The token reaches the agent through the profile's environment, and
    // nowhere else: not a file in the slot, not a layer in the image.
    let written = std::fs::read_to_string(&profile).unwrap();
    let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(&written).unwrap();
    assert_eq!(
        doc["services"][hq::compose::AGENT_SERVICE]["environment"]["CLAUDE_CODE_OAUTH_TOKEN"]
            .as_str(),
        Some("stand-in-token"),
        "the run would have nothing to authenticate with"
    );
    assert!(
        !std::fs::read_to_string(slot.tree.join(".git/config"))
            .unwrap_or_default()
            .contains("stand-in-token"),
        "and it never reaches the slot"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&profile).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o077,
            0,
            "a file holding the token is the human's alone, got {mode:o}"
        );
    }

    let handle = state.run.clone().expect("a run was launched");
    assert!(!handle.container.is_empty(), "it runs in a container");
    // The log is on the host, where hq reads it — the container has nowhere
    // to write it.
    assert!(handle.log.starts_with(&hq_root), "{:?}", handle.log);

    let spawner = hq::engine::spawn::ContainerSpawner::new(
        engine.clone(),
        profile.clone(),
        &compose_project,
        hq::compose::AGENT_SERVICE,
    );
    let harness = hq::harness::claude_code::ClaudeCode::new(Default::default(), Box::new(spawner));

    let mut outcome = None;
    for _ in 0..50 {
        match harness.state(&handle).unwrap() {
            RunState::Finished(o) => {
                outcome = Some(o);
                break;
            }
            RunState::Running(_) => std::thread::sleep(std::time::Duration::from_millis(200)),
        }
    }
    match outcome.expect("the run ended") {
        hq::harness::Outcome::Finished(usage) => {
            println!("read back: {usage:?}");
            assert_eq!(usage.output_tokens, 7, "hq read the run's own numbers");
        }
        other => panic!("expected a finished run, got {other:?}"),
    }

    // Starting it twice is refused: the state already says where it is.
    let again = run::start(&project, "alpha", &slot, engine.clone(), &engine_bin);
    assert!(matches!(again, Err(run::RunError::AlreadyStarted(_))));

    engine.down(&profile, &compose_project, true).unwrap();
    let _ = hq::slot::rm(&project, "one", true);
}

fn git(at: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(at)
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

#[test]
fn a_slot_already_driven_by_another_hq_is_not_driven_twice() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let locks = project.hq_root.join("locks");
    std::fs::create_dir_all(&locks).unwrap();
    std::fs::create_dir_all(&project.hq_root).unwrap();
    // A token exists, so a refusal here can only be the lock.
    std::fs::write(run::token_file(&project), "stand-in\n").unwrap();

    let held = hq::state::SlotLock::acquire(&locks, "one", "something else").unwrap();
    let slot = hq::slot::Slot {
        name: "one".to_string(),
        tree: dir.path().join("slot"),
    };
    let engine: std::sync::Arc<dyn hq::engine::Engine> =
        std::sync::Arc::new(hq::engine::fake::FakeEngine::default());

    let err = run::start(&project, "m1", &slot, engine, "docker").unwrap_err();
    // Two `hq` on one slot is two agents in one tree (SPEC 4.2).
    assert!(matches!(err, run::RunError::Lock(_)), "{err}");
    drop(held);
}
