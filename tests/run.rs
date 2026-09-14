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
            model: None,
            bounds: Default::default(),
            credentials: None,
            run: None,
            services_file: None,
            permission_mode: "auto".to_string(),
            forge_protection: Default::default(),
        },
        dir.join("hq"),
    )
}

/// The same layering as the account, and for the same reason: what the
/// project declares holds until a mission says otherwise, and the choice is
/// frozen with the header rather than read again mid-mission.
///
/// No name is checked here. `hq` knows harnesses, not models: the harness
/// refuses what it does not know, in the container.
#[test]
fn a_model_is_chosen_by_the_mission_then_the_project() {
    let dir = tempfile::tempdir().unwrap();
    let coding = hq::mission::Integration::None {
        reason: "no external service is involved".to_string(),
    };

    // Neither declares one: the harness keeps its own default, and hq adds
    // no `--model` at all.
    let project = project(dir.path());
    assert_eq!(run::model_for(&project, &header_with(coding.clone())), None);

    // The project declares one, and no mission refines it.
    let mut declared = project;
    declared.config.model = Some("claude-sonnet-5".to_string());
    assert_eq!(
        run::model_for(&declared, &header_with(coding.clone())),
        Some("claude-sonnet-5".to_string())
    );

    // The mission declares another: its header wins over hq.yaml.
    let mut header = header_with(coding);
    header.model = Some("claude-opus-5".to_string());
    assert_eq!(
        run::model_for(&declared, &header),
        Some("claude-opus-5".to_string())
    );
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
        run: None,
        account: None,
        model: None,
        bounds: Default::default(),
    };
    hq::mission::dir::create(&project.hq_root, "m1", &header, "").unwrap();
    let engine: std::sync::Arc<dyn hq::engine::Engine> =
        std::sync::Arc::new(hq::engine::fake::FakeEngine::default());
    let err = run::start(&project, "m1", &slot, engine, "docker").unwrap_err();
    assert!(matches!(err, run::RunError::Account(_)), "{err}");
    assert!(!err.to_string().contains("rebuild"), "{err}");
}

/// A run's profile carries the clean copy of `HEAD` that `hq exec` replays
/// proofs on, and its build cache with it — warmed once per slot and kept
/// (SPEC 4.2, and gate 7 of 4.4). Without it there is nowhere to put the
/// copy, and every proof would run in the tree the agent has been living in.
#[test]
fn a_run_profile_carries_the_clean_copy_of_head() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let slot = hq::slot::Slot {
        name: "one".to_string(),
        tree: dir.path().join("slot"),
    };
    let images = hq::image::Images {
        agent: "img/agent".into(),
        firewall: "img/fw".into(),
        prober: "img/probe".into(),
    };
    let paths = hq::mission::dir::Paths::of(&project.hq_root, "m1");
    let plan = run::plan(
        &project,
        &slot,
        "rust",
        &images,
        &paths,
        "stand-in-token",
        &hq::mission::Header {
            branch: "feat/alpha".to_string(),
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
            run: None,
            account: None,
            model: None,
            bounds: Default::default(),
        },
        hq::harness::Role::Coder,
    )
    .unwrap();
    let engine = hq::engine::fake::FakeEngine::default();
    let written = hq::compose::generate(&plan, hq::engine::Engine::dialect(&engine)).unwrap();
    let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(&written).unwrap();

    let mounts: Vec<String> = doc["services"][hq::compose::AGENT_SERVICE]["volumes"]
        .as_sequence()
        .expect("the agent has mounts")
        .iter()
        .map(|v| v.as_str().unwrap_or_default().to_string())
        .collect();
    let expected = format!("{}:{}", hq::exec::proof_volume("one"), hq::exec::PROOF_AT);
    assert!(mounts.contains(&expected), "{mounts:?}");
    let declared = doc["volumes"]
        .as_mapping()
        .expect("the document declares its named volumes");
    assert!(
        declared.contains_key(serde_yaml_ng::Value::from(hq::exec::proof_volume("one"))),
        "a named volume is declared as well as mounted: {written}"
    );
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
        // Its own files, named, and the one that is not its to write said
        // in the same breath (SPEC 4.1, 4.4).
        assert!(prompt.contains("JOURNAL.md"), "{prompt}");
        assert!(prompt.contains("VERDICT.json"), "{prompt}");
        assert!(prompt.contains("MUTANTS.triage.json"), "{prompt}");
        assert!(
            prompt.contains("not yours to give"),
            "the outcome nobody can check must be refused where the agent reads \
             its rules, not only where hq checks them: {prompt}"
        );
        // The shape of the answer, not just its name. Measured on 2026-09-13:
        // a coder left to guess it wrote MUTANTS.json's shape instead, and
        // `hq` refused the file with a serde error that stopped the whole
        // verification rather than reddening one gate.
        assert!(
            prompt.contains(r#"{"<survivor id>": {"kind": "killed", "test":"#),
            "the triage file's shape belongs where the agent reads its rules: {prompt}"
        );
        // Three outcomes and no fourth (SPEC 4.4). A prompt that sanctions
        // "unanswered" sends the agent at a gate that refuses it, and the
        // agent obeys the prompt — measured the same day.
        assert!(
            !prompt.contains("left unanswered"),
            "the prompt must not offer an outcome gate 7 refuses: {prompt}"
        );
        assert!(prompt.contains("no third answer of your own"), "{prompt}");
        // What the gates actually require, said where the agent reads it and
        // not only where `hq` checks it.
        assert!(
            prompt.contains("names the commit it describes"),
            "gate 3 refuses a resume block that does not carry HEAD: {prompt}"
        );
        assert!(
            prompt.contains("an empty one is a red gate"),
            "gate 5 asks for PR.md, so the run contract has to ask for it: {prompt}"
        );
        assert!(
            prompt.contains("inside the block"),
            "hq reads the block, not the file: a line further down is unseen: {prompt}"
        );
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
    // for the agent's own files — so the agent reads it and nothing in the
    // repository is touched (SPEC 3.3).
    assert_eq!(role::PROMPT_FILE, "ROLE.md");
    let writable = hq::compose::AGENT_WRITABLE;
    assert!(
        !writable.contains(&role::PROMPT_FILE),
        "the role prompt is not the agent's to rewrite"
    );
    // Nor is the campaign the gate reads: the coder answers in its own file
    // (SPEC 4.1, 4.4).
    assert!(
        !writable.contains(&hq::mutants::FILE),
        "the HQ's campaign file is not the agent's to rewrite"
    );
    assert!(writable.contains(&hq::mutants::TRIAGE_FILE));
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
    //
    // It ends on `USER agent`, as every stack image must (SPEC 4.2 bis): the
    // agent runs under the human's own id, and hq's harness layer is built
    // on top of whatever user the stack image leaves. A fixture that ended
    // as root ran this whole live test as root, and proved a shape no stack
    // image is allowed to have — caught by the check added the same day.
    let (uid, gid) = hq::image::host_ids();
    std::fs::write(
        root.join(".hq/stacks/rust/Dockerfile"),
        format!(
            "FROM alpine:3.22\n\
             RUN addgroup -g {gid} agent 2>/dev/null || true\n\
             RUN adduser -D -u {uid} -G $(getent group {gid} | cut -d: -f1) agent \
             2>/dev/null || true\n\
             RUN mkdir -p /work/tree /work/mission /run/hq \\\n\
              && chown -R {uid}:{gid} /work /run/hq\n\
             RUN printf '#!/bin/sh\\necho \\x27{{\"type\":\"system\",\"subtype\":\"init\"}}\\x27\\n\
             echo \\x27{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\
             \"usage\":{{\"input_tokens\":11,\"output_tokens\":7}}}}\\x27\\n' \
             > /usr/local/bin/claude \\\n\
              && chmod 0755 /usr/local/bin/claude\n\
             USER {uid}:{gid}\n"
        ),
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
        run: None,
        account: None,
        model: None,
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
    // Counted when `hq verify` reads it back, like every other run; and its
    // session is the one the next lot resumes.
    assert_eq!(state.spent.runs, 0);
    assert_eq!(state.coder_session.as_ref(), Some(&handle.session));
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
            RunState::Running(_) | RunState::Paused(_) => {
                std::thread::sleep(std::time::Duration::from_millis(200))
            }
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

// --- the profile of a role (SPEC 4.1, mounts per profile; 4.2, services) ---

fn header_with(integration: hq::mission::Integration) -> hq::mission::Header {
    hq::mission::Header {
        branch: "feat/alpha".to_string(),
        base: "dev".to_string(),
        lots: vec![hq::mission::Lot {
            id: "L1".to_string(),
            title: "one".to_string(),
        }],
        integration,
        security: hq::mission::Security::Agent,
        arbiter: None,
        run: None,
        account: None,
        model: None,
        bounds: Default::default(),
    }
}

fn with_services() -> hq::mission::Integration {
    hq::mission::Integration::Services {
        services: vec![hq::mission::Service {
            name: "db".to_string(),
            reach: vec!["db".to_string(), "10.4.0.7".to_string()],
            shared: false,
        }],
        wiring: vec!["compose.yaml".to_string()],
    }
}

/// Build a profile for one role and give back the plan and the YAML.
fn profile_for(
    project: &Project,
    slot: &hq::slot::Slot,
    header: &hq::mission::Header,
    role: Role,
) -> (hq::compose::Plan, serde_yaml_ng::Value) {
    let images = hq::image::Images {
        agent: "img/agent".into(),
        firewall: "img/fw".into(),
        prober: "img/probe".into(),
    };
    let paths = hq::mission::dir::Paths::of(&project.hq_root, "m1");
    let plan = run::plan(
        project,
        slot,
        "rust",
        &images,
        &paths,
        "stand-in-token",
        header,
        role,
    )
    .unwrap();
    let engine = hq::engine::fake::FakeEngine::default();
    let written = hq::compose::generate(&plan, hq::engine::Engine::dialect(&engine)).unwrap();
    (plan, serde_yaml_ng::from_str(&written).unwrap())
}

fn slot_at(dir: &Path) -> hq::slot::Slot {
    let tree = dir.join("slot");
    std::fs::create_dir_all(&tree).unwrap();
    hq::slot::Slot {
        name: "one".to_string(),
        tree,
    }
}

/// The mission's declared services reach the system profile's allowlist and
/// never the coder's — SPEC 4.1 bis, rule 6: the coder's list is what his
/// stack and his harness need, and nothing else.
#[test]
fn only_a_system_profile_carries_the_missions_services() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let slot = slot_at(dir.path());
    let header = header_with(with_services());

    let (coder, _) = profile_for(&project, &slot, &header, Role::Coder);
    assert!(
        !coder.perimeter.addresses.contains("10.4.0.7"),
        "the coder reaches no service: {:?}",
        coder.perimeter
    );

    let (integrator, _) = profile_for(&project, &slot, &header, Role::Integrator);
    assert!(
        integrator.perimeter.addresses.contains("10.4.0.7"),
        "{:?}",
        integrator.perimeter
    );
    assert!(
        integrator.perimeter.domains.contains("db"),
        "{:?}",
        integrator.perimeter
    );
}

/// The test credentials are mounted read-only on a system profile and on no
/// other (SPEC 3.1). Two guards say this — `compose::build` refuses them on a
/// mission profile as well — and that is deliberate.
#[test]
fn only_a_system_profile_mounts_the_test_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = project(dir.path()).config;
    let credentials = dir.path().join("credentials");
    std::fs::create_dir_all(&credentials).unwrap();
    std::fs::write(credentials.join("db.env"), "PGPASSWORD=x\n").unwrap();
    std::fs::write(credentials.join("api.env"), "TOKEN=y\n").unwrap();
    config.credentials = Some(credentials.clone());
    let project = Project::at(dir.path().join("repo"), config, dir.path().join("hq"));
    let slot = slot_at(dir.path());
    let header = header_with(with_services());

    let (coder, _) = profile_for(&project, &slot, &header, Role::Coder);
    assert!(coder.credentials.is_empty(), "{:?}", coder.credentials);

    let (integrator, doc) = profile_for(&project, &slot, &header, Role::Integrator);
    // Sorted, because the profile is regenerated at every launch and a diff
    // must mean a real change.
    assert_eq!(
        integrator
            .credentials
            .iter()
            .map(|(_, at)| at.display().to_string())
            .collect::<Vec<_>>(),
        vec![
            format!("{}/api.env", run::CREDENTIALS_AT),
            format!("{}/db.env", run::CREDENTIALS_AT),
        ]
    );
    let mounts: Vec<String> = doc["services"][hq::compose::AGENT_SERVICE]["volumes"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap_or_default().to_string())
        .collect();
    assert!(
        mounts
            .iter()
            .any(|m| m.ends_with(&format!("{}/db.env:ro", run::CREDENTIALS_AT))),
        "read-only, and nothing else: {mounts:?}"
    );
}

/// The project's own services are merged verbatim into the system profile —
/// `include:` is unusable on one of the two engines (SPEC 4.2, engine table)
/// — and into no mission profile: the coder's allowlist forbids it to talk
/// to them.
#[test]
fn the_projects_services_are_merged_into_the_system_profile_only() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = project(dir.path()).config;
    config.services_file = Some(std::path::PathBuf::from("compose.yaml"));
    let project = Project::at(dir.path().join("repo"), config, dir.path().join("hq"));
    let slot = slot_at(dir.path());
    std::fs::write(
        slot.tree.join("compose.yaml"),
        "services:\n  db:\n    image: postgres:16\n    volumes:\n      - dbdata:/var/lib/postgresql/data\nnetworks:\n  back: {}\nvolumes:\n  dbdata: null\n",
    )
    .unwrap();
    let header = header_with(with_services());

    let (_, coder) = profile_for(&project, &slot, &header, Role::Coder);
    assert!(coder["services"]["db"].is_null(), "{coder:?}");

    let (_, doc) = profile_for(&project, &slot, &header, Role::Integrator);
    assert_eq!(doc["services"]["db"]["image"].as_str(), Some("postgres:16"));
    // Verbatim: a key hq does not know about survives, because re-typing the
    // block would lose it.
    assert_eq!(
        doc["services"]["db"]["volumes"][0].as_str(),
        Some("dbdata:/var/lib/postgresql/data")
    );
    // And the firewall attaches to the network the project declared — it is
    // the one that can, the agent having `network_mode` instead.
    assert_eq!(
        doc["services"][hq::compose::FIREWALL_SERVICE]["networks"][0].as_str(),
        Some("back")
    );
    // The volumes block travels with them: a service naming a volume the
    // document does not declare makes the whole project invalid (measured).
    assert!(
        doc["volumes"]
            .as_mapping()
            .unwrap()
            .contains_key(serde_yaml_ng::Value::from("dbdata")),
        "{doc:?}"
    );
}

/// A services file that `hq.yaml` names and the commit does not carry is
/// said by name, before a profile is lifted without the services the
/// integrator was called to wire.
#[test]
fn a_services_file_that_is_not_on_the_commit_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = project(dir.path()).config;
    config.services_file = Some(std::path::PathBuf::from("compose.yaml"));
    let project = Project::at(dir.path().join("repo"), config, dir.path().join("hq"));
    let slot = slot_at(dir.path());
    let images = hq::image::Images {
        agent: "img/agent".into(),
        firewall: "img/fw".into(),
        prober: "img/probe".into(),
    };
    let paths = hq::mission::dir::Paths::of(&project.hq_root, "m1");
    let err = run::plan(
        &project,
        &slot,
        "rust",
        &images,
        &paths,
        "t",
        &header_with(with_services()),
        Role::Integrator,
    )
    .unwrap_err();
    assert!(matches!(err, run::RunError::NoServicesFile(_)), "{err}");
    assert!(err.to_string().contains("compose.yaml"), "{err}");
}

/// The container is told which role it is running, and the spelling comes
/// from one place.
#[test]
fn the_profile_names_its_role() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let slot = slot_at(dir.path());
    let header = header_with(with_services());
    for (role, said) in [
        (Role::Coder, "coder"),
        (Role::Integrator, "integrator"),
        (Role::Security, "security"),
    ] {
        let (plan, _) = profile_for(&project, &slot, &header, role);
        assert_eq!(
            plan.environment.get("HQ_ROLE").map(String::as_str),
            Some(said)
        );
        assert_eq!(plan.role, role);
    }
}

/// The security agent's tree is read-only, and what an execution must still
/// write is declared by the stack and mounted as a volume of its own (SPEC
/// 4.2, rule 3). The other two roles write the tree, so they carry none.
#[test]
fn only_the_security_profile_carries_the_stacks_writable_volumes() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    std::fs::create_dir_all(project.fragment("rust")).unwrap();
    std::fs::write(
        project.fragment("rust").join(hq::project::WRITABLE_FILE),
        "# what a build writes\ntarget\n\npackages/web/node_modules\n",
    )
    .unwrap();
    let slot = slot_at(dir.path());
    let header = header_with(with_services());

    for role in [Role::Coder, Role::Integrator] {
        let (plan, _) = profile_for(&project, &slot, &header, role);
        assert!(
            !plan.volumes.iter().any(|v| v.at.ends_with("target")),
            "{role:?} writes the tree itself: {:?}",
            plan.volumes
        );
    }

    let (plan, doc) = profile_for(&project, &slot, &header, Role::Security);
    let at: Vec<String> = plan
        .volumes
        .iter()
        .map(|v| v.at.display().to_string())
        .collect();
    assert!(at.contains(&"/work/tree/target".to_string()), "{at:?}");
    assert!(
        at.contains(&"/work/tree/packages/web/node_modules".to_string()),
        "{at:?}"
    );
    // Named per profile, not per slot: a `target/` shared with the coder's
    // would hand the security agent the build tree it is meant to attack
    // from outside.
    assert_eq!(
        run::writable_volume("one", "target"),
        "hq-one-security-target"
    );
    assert_eq!(
        run::writable_volume("one", "packages/web/node_modules"),
        "hq-one-security-packages-web-node_modules"
    );
    // Declared as well as mounted, or Compose refuses the whole project.
    let declared = doc["volumes"].as_mapping().unwrap();
    assert!(
        declared.contains_key(serde_yaml_ng::Value::from(
            "hq-one-security-target".to_string()
        )),
        "{declared:?}"
    );
    // And the tree stays read-only around them.
    let mounts: Vec<String> = doc["services"][hq::compose::AGENT_SERVICE]["volumes"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap_or_default().to_string())
        .collect();
    assert!(
        mounts.iter().any(|m| m.ends_with("/work/tree:ro")),
        "{mounts:?}"
    );
}

/// `writable.txt` is a list a human edits, and what it may not do is escape
/// the tree it describes.
#[test]
fn a_writable_declaration_may_not_climb_out_of_the_tree() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    std::fs::create_dir_all(project.fragment("rust")).unwrap();
    std::fs::write(
        project.fragment("rust").join(hq::project::WRITABLE_FILE),
        "# a comment\n\n  target  \n/.next/\n..\n../../etc\nsrc/../..\n.\n",
    )
    .unwrap();
    assert_eq!(
        project.stack_writable("rust"),
        vec!["target".to_string(), ".next".to_string()]
    );
}

/// A volume can only be mounted at a path the read-only bind already carries:
/// runc creates the mount point in the assembled root filesystem, and under a
/// `:ro` bind it cannot — `create mountpoint for /work/tree/target: read-only
/// file system`, measured 2026-09-10. So `hq` makes the directory in the
/// slot's tree first, and nothing else: an empty directory the project
/// ignores, invisible to `git status`, so gate 1 stays green.
#[test]
fn the_declared_directories_are_made_in_the_tree_before_the_profile_is_lifted() {
    let dir = tempfile::tempdir().unwrap();
    let slot = slot_at(dir.path());
    let git = |args: &[&str]| {
        assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(&slot.tree)
                .args(["-c", "user.name=T", "-c", "user.email=t@example.com"])
                .args(args)
                .status()
                .unwrap()
                .success(),
            "git {args:?}"
        );
    };
    git(&["init", "-q", "-b", "x"]);
    std::fs::write(slot.tree.join(".gitignore"), "target\n").unwrap();
    git(&["add", "-A"]);
    git(&["commit", "-qm", "base"]);

    let declared = vec![
        "target".to_string(),
        "packages/web/node_modules".to_string(),
    ];
    run::seed_writable(&slot, &declared).unwrap();
    assert!(slot.tree.join("target").is_dir());
    assert!(slot.tree.join("packages/web/node_modules").is_dir());

    // Twice changes nothing — a role that follows another finds them there.
    run::seed_writable(&slot, &declared).unwrap();

    // And the tree gate does not see them. `packages/` is not ignored and is
    // still invisible: git tracks files, not directories.
    let dirty = String::from_utf8(
        std::process::Command::new("git")
            .arg("-C")
            .arg(&slot.tree)
            .args(["status", "--porcelain"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert!(dirty.trim().is_empty(), "gate 1 would go red on: {dirty}");
}

/// A session identifier has to be unique across its **whole** length.
///
/// The identity check greps a container's `/proc/<pid>/cmdline` for it (see
/// `engine::spawn`), and a prefix everybody shares tells nothing apart. The
/// mix that came before left the top 48 bits at zero — every identifier began
/// `00000000-0000-`, as this project's own run logs show — because
/// nanoseconds since 1970 need 61 bits, a pid shifted by 64 reaches 80, and
/// nothing filled the rest.
#[test]
fn a_session_identifier_is_unique_over_its_whole_length() {
    let ids: Vec<String> = (0..64).map(|_| run::session_id()).collect();

    for id in &ids {
        assert_eq!(id.len(), 36, "{id}");
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(
            parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
            vec![8, 4, 4, 4, 12],
            "{id}"
        );
        assert!(
            id.chars().all(|c| c.is_ascii_hexdigit() || c == '-'),
            "{id}"
        );
        // v4, and the RFC 4122 variant, so a harness that validates the shape
        // accepts it.
        assert!(parts[2].starts_with('4'), "{id}");
        assert!(
            matches!(parts[3].chars().next(), Some('8' | '9' | 'a' | 'b')),
            "{id}"
        );
    }

    // No shared prefix: the head of the identifier must carry entropy, not a
    // constant. Sixteen distinct first-halves out of sixty-four is far below
    // what randomness gives and far above what a constant gives.
    let heads: std::collections::BTreeSet<&str> = ids.iter().map(|id| &id[..18]).collect();
    assert!(
        heads.len() > 16,
        "the first half of every identifier is nearly the same: {:?}",
        heads.iter().take(4).collect::<Vec<_>>()
    );
    let whole: std::collections::BTreeSet<&String> = ids.iter().collect();
    assert_eq!(whole.len(), ids.len(), "two runs share an identifier");
}

/// A session given is resumed; none given is a fresh one, never the same
/// twice.
#[test]
fn a_given_session_is_resumed_and_none_starts_a_fresh_one() {
    let given = hq::harness::SessionId("s-coder".into());
    assert_eq!(run::session_for(Some(&given)), (given.clone(), true));
    let (fresh, resume) = run::session_for(None);
    assert!(!resume);
    assert_ne!(fresh, given);
    assert_ne!(run::session_for(None).0, fresh, "a fresh one each time");
}
