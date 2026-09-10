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
            account: None,
            bounds: Default::default(),
            credentials: None,
            run: None,
        },
        dir.join("hq"),
    )
}

#[test]
fn an_account_is_chosen_by_the_mission_then_the_project_then_the_default() {
    use hq::account::{Account, Accounts};
    use std::collections::BTreeMap;

    let dir = tempfile::tempdir().unwrap();
    let hq_home = dir.path();
    let mut accounts = BTreeMap::new();
    for name in ["perso", "pro"] {
        accounts.insert(
            name.to_string(),
            Account {
                harness: "claude-code".to_string(),
                token_file: std::path::PathBuf::from(format!("accounts/{name}")),
                note: None,
            },
        );
    }
    let index = Accounts {
        accounts,
        default: Some("perso".to_string()),
    };

    // The mission wins over the project, which wins over the default.
    assert_eq!(
        index.choose(hq_home, Some("pro"), Some("perso")).unwrap().0,
        "pro"
    );
    assert_eq!(index.choose(hq_home, None, Some("pro")).unwrap().0, "pro");
    assert_eq!(index.choose(hq_home, None, None).unwrap().0, "perso");

    // A name nobody declared is an error that lists what exists.
    let err = index.choose(hq_home, Some("ghost"), None).unwrap_err();
    assert!(err.to_string().contains("perso, pro"), "{err}");

    // With exactly one account and no default, that one is the answer rather
    // than a question.
    let only = Accounts {
        accounts: index
            .accounts
            .iter()
            .take(1)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        default: None,
    };
    assert_eq!(only.choose(hq_home, None, None).unwrap().0, "perso");

    // With none at all, hq says where to declare one.
    let empty = Accounts::default();
    let err = empty.choose(hq_home, None, None).unwrap_err();
    assert!(err.to_string().contains("accounts.yaml"), "{err}");
}

#[test]
fn a_token_lives_beside_the_accounts_and_whitespace_is_not_part_of_it() {
    use hq::account::{Account, Accounts};

    let dir = tempfile::tempdir().unwrap();
    let hq_home = dir.path();
    std::fs::create_dir_all(hq_home.join("accounts")).unwrap();
    let account = Account {
        harness: "claude-code".to_string(),
        token_file: std::path::PathBuf::from("accounts/perso"),
        note: None,
    };

    // Relative to ~/.hq, so an index can be written without absolute
    // paths — and never inside a repository.
    assert_eq!(account.token_path(hq_home), hq_home.join("accounts/perso"));

    assert!(account.token(hq_home, "perso").is_err());
    std::fs::write(hq_home.join("accounts/perso"), "  sk-ant-oat-example\n").unwrap();
    assert_eq!(
        account.token(hq_home, "perso").unwrap(),
        "sk-ant-oat-example"
    );

    // An empty file is no token, not an empty one.
    std::fs::write(hq_home.join("accounts/perso"), "\n").unwrap();
    let err = account.token(hq_home, "perso").unwrap_err();
    assert!(err.to_string().contains("claude setup-token"), "{err}");

    // And the index reads back what was written.
    let index = Accounts {
        accounts: [("perso".to_string(), account)].into_iter().collect(),
        default: None,
    };
    std::fs::write(
        hq_home.join("accounts.yaml"),
        serde_yaml_ng::to_string(&index).unwrap(),
    )
    .unwrap();
    assert_eq!(Accounts::load(hq_home).unwrap(), index);
}

#[test]
fn starting_a_mission_on_the_wrong_kind_of_account_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = project(dir.path());
    project.config.account = Some("openai".to_string());
    let hq_home = project.hq_home();
    std::fs::create_dir_all(hq_home.join("accounts")).unwrap();
    std::fs::write(hq_home.join("accounts/openai"), "sk-openai-example\n").unwrap();
    std::fs::write(
        hq_home.join("accounts.yaml"),
        "accounts:\n  openai:\n    harness: codex\n    token_file: accounts/openai\n",
    )
    .unwrap();

    // An OpenAI subscription does not authenticate Claude Code. Passing it
    // along would fail inside a container, with nobody there to read the
    // error (SPEC 3.2: refusing is safe, asking is not).
    let err = run::account_for(&project, None).unwrap_err();
    assert!(matches!(err, run::RunError::WrongHarness { .. }), "{err}");
    let text = err.to_string();
    assert!(
        text.contains("codex") && text.contains("claude-code"),
        "{text}"
    );

    // And the mission's own choice is what is checked, not the project's.
    let index = concat!(
        "accounts:\n",
        "  openai:\n",
        "    harness: codex\n",
        "    token_file: accounts/openai\n",
        "  perso:\n",
        "    harness: claude-code\n",
        "    token_file: accounts/perso\n",
    );
    std::fs::write(hq_home.join("accounts.yaml"), index).unwrap();
    std::fs::write(hq_home.join("accounts/perso"), "sk-ant-oat-example\n").unwrap();
    let (name, _, token) = run::account_for(&project, Some("perso")).unwrap();
    assert_eq!(name, "perso");
    assert_eq!(token, "sk-ant-oat-example");
}

#[test]
fn an_account_for_another_harness_is_refused_rather_than_passed_along() {
    use hq::account::{Account, Accounts};

    let dir = tempfile::tempdir().unwrap();
    let mut project = project(dir.path());
    project.config.account = Some("openai".to_string());
    let index = Accounts {
        accounts: [(
            "openai".to_string(),
            Account {
                harness: "codex".to_string(),
                token_file: std::path::PathBuf::from("accounts/openai"),
                note: None,
            },
        )]
        .into_iter()
        .collect(),
        default: None,
    };
    // An OpenAI subscription does not authenticate Claude Code, and passing
    // it along would fail inside a container with nobody to read the error.
    let (name, account) = index.choose(dir.path(), None, Some("openai")).unwrap();
    assert_eq!(name, "openai");
    assert_ne!(account.harness, project.config.harness);
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
    // The mission does not exist, so that is what it says — and it says it
    // before looking at this machine's images, because a missing image is
    // this machine's business and a missing mission is the mission's.
    assert!(matches!(err, run::RunError::Mission(_)), "{err}");

    // With the mission there but no account declared, the account is what is
    // named — still without a word about images.
    let header = hq::mission::Header {
        branch: "feat/x".to_string(),
        base: "dev".to_string(),
        lots: vec![hq::mission::Lot {
            id: "L1".to_string(),
            title: "one".to_string(),
        }],
        integration: hq::mission::Integration::None {
            reason: "none".to_string(),
        },
        security: hq::mission::Security::Gates,
        arbiter: None,
        account: None,
        bounds: Default::default(),
    };
    hq::mission::dir::create(&project.hq_root, "m1", &header, "").unwrap();
    let engine: std::sync::Arc<dyn hq::engine::Engine> =
        std::sync::Arc::new(hq::engine::fake::FakeEngine::default());
    let err = run::start(&project, "m1", &slot, engine, "docker").unwrap_err();
    assert!(matches!(err, run::RunError::Account(_)), "{err}");
    assert!(!err.to_string().contains("rebuild"), "{err}");
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
        arbiter: None,
        account: None,
        bounds: Default::default(),
    };
    hq::mission::dir::create(&hq_root, "alpha", &header, "Do the thing.").unwrap();
    // An account of this test's own, under its own ~/.hq: nothing here
    // reads the machine's.
    let hq_home = project.hq_home();
    std::fs::create_dir_all(hq_home.join("accounts")).unwrap();
    std::fs::write(hq_home.join("accounts/stand-in"), "stand-in-token\n").unwrap();
    std::fs::write(
        hq_home.join("accounts.yaml"),
        "default: stand-in\naccounts:\n  stand-in:\n    harness: claude-code\n    token_file: accounts/stand-in\n",
    )
    .unwrap();

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

/// Liveness must be about **this** run — not about whatever holds its
/// number, and not about the container it needed to ask through.
///
/// Process ids inside a container are low and recycled within seconds, so a
/// finished run reads as running the moment an unrelated `exec` takes its
/// pid — measured on a real container, where the very shell asking the
/// question had become pid 7. And the check has to be able to fail to
/// answer: a container taken down under a run is not a run that died, and
/// answering "not alive" for it is how `hq` came to blame an agent for its
/// engine.
///
/// Three assertions, in the three directions: a live run reads as running,
/// an impostor pid does not, and a container that went away is neither.
///
/// ```text
/// cargo test --test run -- --ignored --nocapture
/// ```
#[test]
#[ignore = "lifts real containers; run by hand"]
fn live_liveness_tells_this_run_from_a_stranger_and_from_a_lost_container() {
    use hq::engine::spawn::ContainerSpawner;
    use hq::harness::spawn::{CommandSpec, Presence, Spawned, Spawner};

    let dir = tempfile::tempdir().unwrap();
    let engine_bin = std::env::var("HQ_ENGINE").unwrap_or_else(|_| "docker".to_string());
    let profile = dir.path().join("probe.yml");
    let project = "hq-pidprobe";
    std::fs::write(
        &profile,
        concat!(
            "services:\n",
            "  agent:\n",
            "    image: alpine:3.20\n",
            // The agent's own writable directory, where a run publishes its
            // pid. Without it the wrapper has nowhere to write and the
            // launch times out — which is what a real profile mounts.
            "    tmpfs:\n      - /run/hq\n",
            "    command: [\"sleep\", \"600\"]\n",
        ),
    )
    .unwrap();

    let engine: std::sync::Arc<dyn hq::engine::Engine> =
        std::sync::Arc::new(hq::engine::docker::Docker::real());
    let _ = engine.down(&profile, project, true);
    engine.up(&profile, project).unwrap();

    let session = "11111111-2222-4333-8444-555555555555";
    let spawner = ContainerSpawner::new(engine.clone(), profile.clone(), project, "agent")
        .identified_by(session);

    let log = dir.path().join("runs").join(format!("{session}.jsonl"));
    let spawned = spawner
        .spawn(
            &CommandSpec {
                program: "sh".to_string(),
                // A loop, not `sleep`: a shell whose script ends in one
                // simple command execs it and its own command line
                // disappears. The real harness is exec'd directly by the
                // wrapper, so its `--session-id` is right there in
                // /proc/<pid>/cmdline; this stands in for that.
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
    let pid = spawned.pid.expect("a pid was published");
    println!("in-container pid {pid}");
    assert_eq!(
        spawner.alive(&spawned).unwrap(),
        Presence::Running,
        "it is running, and a liveness check that cannot say so about a live \
         run is worth nothing"
    );

    // A pid that is not this run — pid 1 is always there, and is never the
    // harness. `kill -0` alone would call it alive.
    let impostor = Spawned {
        pid: Some(1),
        container: spawned.container.clone(),
    };
    assert_eq!(
        spawner.alive(&impostor).unwrap(),
        Presence::Ended,
        "pid 1 is alive and is not the run; liveness must be about identity"
    );

    // The container goes away under the run. That is not the process ending
    // — SPEC 4.2 makes it an interruption the agent does not pay for — and
    // the answer must say so in its own words, naming the container.
    engine.down(&profile, project, true).unwrap();
    match spawner.alive(&spawned).unwrap() {
        Presence::Vanished(why) => {
            println!("vanished, and it says why: {why}");
            assert!(
                why.contains(&spawned.container[..12]),
                "it must name the container: {why}"
            );
        }
        other => panic!("a container that went away is not the run ending: {other:?}"),
    }

    let _ = engine_bin;
}
