//! Starting a run (SPEC 4.3): what `nunki mission start` decides before any
//! container exists.

use std::path::Path;

use nunki::harness::Role;
use nunki::project::{Config, Project, ProtectedPaths};
use nunki::role;
use nunki::run;

fn project(dir: &Path) -> Project {
    Project::at(
        dir.join("repo"),
        Config {
            root: None,
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
        dir.join("nunki"),
    )
}

/// The same layering as the account, and for the same reason: what the
/// project declares holds until a mission says otherwise, and the choice is
/// frozen with the header rather than read again mid-mission.
///
/// No name is checked here. `nunki` knows harnesses, not models: the harness
/// refuses what it does not know, in the container.
#[test]
fn a_model_is_chosen_by_the_mission_then_the_project() {
    let dir = tempfile::tempdir().unwrap();
    let coding = nunki::mission::Integration::None {
        reason: "no external service is involved".to_string(),
    };

    // Neither declares one: the harness keeps its own default, and nunki adds
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

    // The mission declares another: its header wins over nunki.yaml.
    let mut header = header_with(coding);
    header.model = Some("claude-opus-5".to_string());
    assert_eq!(
        run::model_for(&declared, &header),
        Some("claude-opus-5".to_string())
    );
}

#[test]
fn an_account_is_chosen_by_the_mission_then_the_project_then_the_default() {
    use nunki::account::{Account, Accounts};
    use std::collections::BTreeMap;

    let dir = tempfile::tempdir().unwrap();
    let nunki_home = dir.path();
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
        index
            .choose(nunki_home, Some("pro"), Some("perso"))
            .unwrap()
            .0,
        "pro"
    );
    assert_eq!(
        index.choose(nunki_home, None, Some("pro")).unwrap().0,
        "pro"
    );
    assert_eq!(index.choose(nunki_home, None, None).unwrap().0, "perso");

    // A name nobody declared is an error that lists what exists.
    let err = index.choose(nunki_home, Some("ghost"), None).unwrap_err();
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
    assert_eq!(only.choose(nunki_home, None, None).unwrap().0, "perso");

    // With none at all, nunki says where to declare one.
    let empty = Accounts::default();
    let err = empty.choose(nunki_home, None, None).unwrap_err();
    assert!(err.to_string().contains("accounts.yaml"), "{err}");
}

#[test]
fn a_token_lives_beside_the_accounts_and_whitespace_is_not_part_of_it() {
    use nunki::account::{Account, Accounts};

    let dir = tempfile::tempdir().unwrap();
    let nunki_home = dir.path();
    std::fs::create_dir_all(nunki_home.join("accounts")).unwrap();
    let account = Account {
        harness: "claude-code".to_string(),
        token_file: std::path::PathBuf::from("accounts/perso"),
        note: None,
    };

    // Relative to ~/.nunki, so an index can be written without absolute
    // paths — and never inside a repository.
    assert_eq!(
        account.token_path(nunki_home),
        nunki_home.join("accounts/perso")
    );

    assert!(account.token(nunki_home, "perso").is_err());
    std::fs::write(nunki_home.join("accounts/perso"), "  sk-ant-oat-example\n").unwrap();
    assert_eq!(
        account.token(nunki_home, "perso").unwrap(),
        "sk-ant-oat-example"
    );

    // An empty file is no token, not an empty one.
    std::fs::write(nunki_home.join("accounts/perso"), "\n").unwrap();
    let err = account.token(nunki_home, "perso").unwrap_err();
    assert!(err.to_string().contains("claude setup-token"), "{err}");

    // And the index reads back what was written.
    let index = Accounts {
        accounts: [("perso".to_string(), account)].into_iter().collect(),
        default: None,
    };
    std::fs::write(
        nunki_home.join("accounts.yaml"),
        serde_yaml_ng::to_string(&index).unwrap(),
    )
    .unwrap();
    assert_eq!(Accounts::load(nunki_home).unwrap(), index);
}

#[test]
fn starting_a_mission_on_the_wrong_kind_of_account_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = project(dir.path());
    project.config.account = Some("openai".to_string());
    let nunki_home = project.nunki_home();
    std::fs::create_dir_all(nunki_home.join("accounts")).unwrap();
    std::fs::write(nunki_home.join("accounts/openai"), "sk-openai-example\n").unwrap();
    std::fs::write(
        nunki_home.join("accounts.yaml"),
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
    std::fs::write(nunki_home.join("accounts.yaml"), index).unwrap();
    std::fs::write(nunki_home.join("accounts/perso"), "sk-ant-oat-example\n").unwrap();
    let (name, _, token) = run::account_for(&project, Some("perso")).unwrap();
    assert_eq!(name, "perso");
    assert_eq!(token, "sk-ant-oat-example");
}

#[test]
fn an_account_for_another_harness_is_refused_rather_than_passed_along() {
    use nunki::account::{Account, Accounts};

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
    let slot = nunki::slot::Slot {
        name: "one".to_string(),
        tree: dir.path().join("slot"),
    };
    let engine: std::sync::Arc<dyn nunki::engine::Engine> =
        std::sync::Arc::new(nunki::engine::fake::FakeEngine::default());

    let err = run::start(&project, "m1", &slot, engine, "docker").unwrap_err();
    // The mission does not exist, so that is what it says — and it says it
    // before looking at this machine's images, because a missing image is
    // this machine's business and a missing mission is the mission's.
    assert!(matches!(err, run::RunError::Mission(_)), "{err}");

    // With the mission there but no account declared, the account is what is
    // named — still without a word about images.
    let header = nunki::mission::Header {
        branch: "feat/x".to_string(),
        base: "dev".to_string(),
        lots: vec![nunki::mission::Lot {
            id: "L1".to_string(),
            title: "one".to_string(),
        }],
        integration: nunki::mission::Integration::None {
            reason: "none".to_string(),
        },
        security: nunki::mission::Security::Gates,
        arbiter: None,
        run: None,
        account: None,
        model: None,
        bounds: Default::default(),
    };
    nunki::mission::dir::create(&project.hq_root, "m1", &header, "").unwrap();
    let engine: std::sync::Arc<dyn nunki::engine::Engine> =
        std::sync::Arc::new(nunki::engine::fake::FakeEngine::default());
    let err = run::start(&project, "m1", &slot, engine, "docker").unwrap_err();
    assert!(matches!(err, run::RunError::Account(_)), "{err}");
    assert!(!err.to_string().contains("rebuild"), "{err}");
}

/// A run's profile carries the clean copy of `HEAD` that `nunki exec` replays
/// proofs on, and its build cache with it — warmed once per slot and kept
/// (SPEC 4.2, and gate 7 of 4.4). Without it there is nowhere to put the
/// copy, and every proof would run in the tree the agent has been living in.
#[test]
fn a_run_profile_carries_the_clean_copy_of_head() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let slot = nunki::slot::Slot {
        name: "one".to_string(),
        tree: dir.path().join("slot"),
    };
    let images = nunki::image::Images {
        agent: "img/agent".into(),
        firewall: "img/fw".into(),
        prober: "img/probe".into(),
    };
    let paths = nunki::mission::dir::Paths::of(&project.hq_root, "m1");
    let plan = run::plan(
        &project,
        &slot,
        "rust",
        &images,
        &paths,
        "stand-in-token",
        &nunki::mission::Header {
            branch: "feat/alpha".to_string(),
            base: "dev".to_string(),
            lots: vec![nunki::mission::Lot {
                id: "L1".to_string(),
                title: "one".to_string(),
            }],
            integration: nunki::mission::Integration::None {
                reason: "none".to_string(),
            },
            security: nunki::mission::Security::Gates,
            arbiter: None,
            run: None,
            account: None,
            model: None,
            bounds: Default::default(),
        },
        nunki::harness::Role::Coder,
    )
    .unwrap();
    let engine = nunki::engine::fake::FakeEngine::default();
    let written = nunki::compose::generate(&plan, nunki::engine::Engine::dialect(&engine)).unwrap();
    let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(&written).unwrap();

    let mounts: Vec<String> = doc["services"][nunki::compose::AGENT_SERVICE]["volumes"]
        .as_sequence()
        .expect("the agent has mounts")
        .iter()
        .map(|v| v.as_str().unwrap_or_default().to_string())
        .collect();
    let expected = format!(
        "{}:{}",
        nunki::exec::proof_volume("one"),
        nunki::exec::PROOF_AT
    );
    assert!(mounts.contains(&expected), "{mounts:?}");
    let declared = doc["volumes"]
        .as_mapping()
        .expect("the document declares its named volumes");
    assert!(
        declared.contains_key(serde_yaml_ng::Value::from(nunki::exec::proof_volume("one"))),
        "a named volume is declared as well as mounted: {written}"
    );
}

/// The allowlist a run reads is the perimeter its firewall enforces, role by
/// role — not the stack's `allow.txt`, which is one source of three. An
/// autonomous agent cannot ask why a name does not resolve; it reads the
/// list, or it retries a wall.
#[test]
fn the_agent_reads_the_perimeter_its_firewall_enforces() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let slot = nunki::slot::Slot {
        name: "one".to_string(),
        tree: dir.path().join("slot"),
    };
    let images = nunki::image::Images {
        agent: "img/agent".into(),
        firewall: "img/fw".into(),
        prober: "img/probe".into(),
    };
    let paths = nunki::mission::dir::Paths::of(&project.hq_root, "m1");
    let header = |integration| nunki::mission::Header {
        branch: "feat/alpha".to_string(),
        base: "dev".to_string(),
        lots: vec![nunki::mission::Lot {
            id: "L1".to_string(),
            title: "one".to_string(),
        }],
        integration,
        security: nunki::mission::Security::Gates,
        arbiter: None,
        run: None,
        account: None,
        model: None,
        bounds: Default::default(),
    };
    let lines = |text: &str, section: &str| -> std::collections::BTreeSet<String> {
        text.lines()
            .skip_while(|l| *l != section)
            .skip(1)
            .take_while(|l| !l.is_empty())
            .map(str::to_string)
            .collect()
    };

    for (role, integration) in [
        (
            Role::Coder,
            nunki::mission::Integration::None {
                reason: "none".to_string(),
            },
        ),
        (
            Role::Integrator,
            nunki::mission::Integration::Services {
                services: vec![nunki::mission::Service {
                    name: "db".to_string(),
                    reach: vec!["db".to_string(), "10.4.0.7".to_string()],
                    shared: false,
                }],
                wiring: vec!["tests/**".to_string()],
            },
        ),
    ] {
        let plan = run::plan(
            &project,
            &slot,
            "rust",
            &images,
            &paths,
            "stand-in-token",
            &header(integration),
            role,
        )
        .unwrap();
        let text = run::allowlist(role, &plan.perimeter);

        assert!(text.contains(role::slug(role)), "{text}");
        assert_eq!(lines(&text, "domains:"), plan.perimeter.domains, "{text}");
        if plan.perimeter.addresses.is_empty() {
            assert!(lines(&text, "addresses:").contains("(none)"), "{text}");
        } else {
            assert_eq!(
                lines(&text, "addresses:"),
                plan.perimeter.addresses,
                "{text}"
            );
        }
        // The service is the integrator's, and never the coder's.
        assert_eq!(
            text.lines().any(|l| l == "db"),
            role == Role::Integrator,
            "{role:?}: {text}"
        );
    }
}

/// The list is `nunki`'s, like `MISSION.md`: in the mission folder, which the
/// agent reads, and among none of the files it may write — an agent that could
/// edit its allowlist would only mislead itself, but a list that lies is worse
/// than no list.
#[test]
fn the_allowlist_is_read_only_for_the_agent_and_every_prompt_names_it() {
    use nunki::mission::dir::ALLOWLIST_FILE;
    assert!(!nunki::compose::AGENT_WRITABLE.contains(&ALLOWLIST_FILE));
    let paths = nunki::mission::dir::Paths::of(Path::new("/hq"), "m1");
    assert_eq!(paths.allowlist, paths.dir.join(ALLOWLIST_FILE));
    for role in [Role::Coder, Role::Integrator, Role::Security] {
        assert!(
            role::prompt(role).contains(ALLOWLIST_FILE),
            "{role:?} is not told where its allowlist is"
        );
    }
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
        // A commit that names the tool that typed it says nothing about
        // the change, and the author already says which role wrote it.
        assert!(
            prompt.contains("Co-Authored-By"),
            "the prompt names the trailer it forbids, so there is nothing to \
             interpret: {prompt}"
        );
        assert!(
            prompt.contains("no trailer naming a harness, a model or a tool"),
            "{prompt}"
        );
        assert!(
            prompt.contains("not yours to give"),
            "the outcome nobody can check must be refused where the agent reads \
             its rules, not only where nunki checks them: {prompt}"
        );
        // The shape of the answer, not just its name. Measured on 2026-09-13:
        // a coder left to guess it wrote MUTANTS.json's shape instead, and
        // `nunki` refused the file with a serde error that stopped the whole
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
        // not only where `nunki` checks it.
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
            "nunki reads the block, not the file: a line further down is unseen: {prompt}"
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

/// The integrator is graded by gates of its own — the wiring list, a system
/// battery, a completed `PR.md`, a verdict pinned to `HEAD` — and each has to
/// be said where it reads its rules. Found on 2026-09-15, before the first
/// integration mission was launched: none of the four was, so its first run
/// would have gone red at gates 5 and 6 on rules it had no way to learn.
#[test]
fn the_integrator_is_told_what_its_own_gates_require() {
    let integrator = role::prompt(Role::Integrator);

    assert!(integrator.contains("wiring list"), "{integrator}");
    let battery = format!("{}/{}", nunki::run::STACK_AT, nunki::gate::SYSTEM_BATTERY);
    assert!(
        integrator.contains(&battery),
        "gate 6 runs {battery}, so the prompt has to name it: {integrator}"
    );
    assert!(
        integrator.contains("`Integration`"),
        "gate 5 looks for that heading in PR.md: {integrator}"
    );

    // The verdict's shape, and one `nunki` actually reads. A shape given only
    // in prose drifts from the struct that parses it, and the agent obeys the
    // prose — the triage file taught that on 2026-09-13.
    let line = integrator
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("{\"role\""))
        .expect("the prompt shows the verdict's shape");
    let file: nunki::mission::VerdictFile = serde_json::from_str(line)
        .unwrap_or_else(|e| panic!("the shape in the prompt is not one nunki reads: {e}\n{line}"));
    assert_eq!(file.role, Role::Integrator);
    assert_eq!(file.verdict, nunki::mission::Verdict::Integrated);
    assert!(integrator.contains("`BROKEN`"), "{integrator}");
}

/// The security agent commits nothing, so its gates are its journal and its
/// report — and its verdict has a shape `nunki` refuses when it is wrong.
/// Found on 2026-09-15, before the first security mission: its prompt named
/// neither `CLEAR`, nor `FINDINGS`, nor the file the verdict goes in, so its
/// first run would have been refused for a verdict it was never shown.
#[test]
fn the_security_agent_is_told_what_its_own_gates_require() {
    let security = role::prompt(Role::Security);

    assert!(security.contains("ÉTAT DE REPRISE"), "{security}");
    assert!(
        security.contains("VERDICT.json"),
        "its report is its deliverable, and that is where it goes: {security}"
    );

    let line = security
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("{\"role\""))
        .expect("the prompt shows the verdict's shape");
    let file: nunki::mission::VerdictFile = serde_json::from_str(line)
        .unwrap_or_else(|e| panic!("the shape in the prompt is not one nunki reads: {e}\n{line}"));
    assert_eq!(file.role, Role::Security);
    assert_eq!(file.verdict, nunki::mission::Verdict::Clear);
    assert!(security.contains("`FINDINGS`"), "{security}");

    // And both roles that meet a running application are told it may still be
    // starting: `nunki` launches it and does not wait for it to serve.
    for role in [Role::Integrator, Role::Security] {
        assert!(
            role::prompt(role).contains("still be compiling"),
            "{role:?} is not told the application may not answer yet"
        );
    }
}

#[test]
fn the_prompt_travels_as_a_file_and_never_into_the_slot() {
    // It is written into the mission folder, which is mounted read-only but
    // for the agent's own files — so the agent reads it and nothing in the
    // repository is touched (SPEC 3.3).
    assert_eq!(role::PROMPT_FILE, "ROLE.md");
    let writable = nunki::compose::AGENT_WRITABLE;
    assert!(
        !writable.contains(&role::PROMPT_FILE),
        "the role prompt is not the agent's to rewrite"
    );
    // Nor is the campaign the gate reads: the coder answers in its own file
    // (SPEC 4.1, 4.4).
    assert!(
        !writable.contains(&nunki::mutants::FILE),
        "the HQ's campaign file is not the agent's to rewrite"
    );
    assert!(writable.contains(&nunki::mutants::TRIAGE_FILE));
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
/// agent's container, its stream is captured on the host, and `nunki` reads it
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
    use nunki::harness::{Harness, RunState};

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    let home = dir.path().join("nunki");
    std::fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q", "-b", "dev"]);
    git(&root, &["config", "user.email", "t@example.com"]);
    git(&root, &["config", "user.name", "Test"]);

    nunki::init::init(&root, &home, &["rust".to_string()]).unwrap();
    // A stand-in for the harness: it prints a result event and exits.
    //
    // It ends on `USER agent`, as every stack image must (SPEC 4.2 bis): the
    // agent runs under the human's own id, and nunki's harness layer is built
    // on top of whatever user the stack image leaves. A fixture that ended
    // as root ran this whole live test as root, and proved a shape no stack
    // image is allowed to have — caught by the check added the same day.
    let (uid, gid) = nunki::image::host_ids();
    std::fs::write(
        home.join("stacks/rust/Dockerfile"),
        format!(
            "FROM alpine:3.22\n\
             RUN addgroup -g {gid} agent 2>/dev/null || true\n\
             RUN adduser -D -u {uid} -G $(getent group {gid} | cut -d: -f1) agent \
             2>/dev/null || true\n\
             RUN mkdir -p /work/tree /work/mission /run/nunki \\\n\
              && chown -R {uid}:{gid} /work /run/nunki\n\
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

    let project = Project::open_at(root.clone(), home.clone()).unwrap();
    let hq_root = project.hq_root.clone();
    let engine_bin = std::env::var("HQ_ENGINE").unwrap_or_else(|_| "docker".to_string());
    nunki::image::build(&project, "rust", &engine_bin).expect("the images build");

    let slot = nunki::slot::add(&project, "one").expect("the slot is cloned");
    let header = nunki::mission::Header {
        branch: "feat/alpha".to_string(),
        base: "dev".to_string(),
        lots: vec![nunki::mission::Lot {
            id: "L1".to_string(),
            title: "the first lot".to_string(),
        }],
        integration: nunki::mission::Integration::None {
            reason: "nothing external".to_string(),
        },
        security: nunki::mission::Security::Gates,
        arbiter: None,
        run: None,
        account: None,
        model: None,
        bounds: Default::default(),
    };
    nunki::mission::dir::create(&hq_root, "alpha", &header, "Do the thing.").unwrap();
    // An account of this test's own, under its own ~/.nunki: nothing here
    // reads the machine's.
    let nunki_home = project.nunki_home();
    std::fs::create_dir_all(nunki_home.join("accounts")).unwrap();
    std::fs::write(nunki_home.join("accounts/stand-in"), "stand-in-token\n").unwrap();
    std::fs::write(
        nunki_home.join("accounts.yaml"),
        "default: stand-in\naccounts:\n  stand-in:\n    harness: claude-code\n    token_file: accounts/stand-in\n",
    )
    .unwrap();

    let engine: std::sync::Arc<dyn nunki::engine::Engine> =
        std::sync::Arc::new(nunki::engine::docker::Docker::real());
    let compose_project = nunki::compose::project_name(&project.session(), "one").unwrap();
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
        doc["services"][nunki::compose::AGENT_SERVICE]["environment"]["CLAUDE_CODE_OAUTH_TOKEN"]
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
    // Counted when `nunki verify` reads it back, like every other run; and its
    // session is the one the next lot resumes.
    assert_eq!(state.spent.runs, 0);
    assert_eq!(state.coder_session.as_ref(), Some(&handle.session));
    // The log is on the host, where nunki reads it — the container has nowhere
    // to write it.
    assert!(handle.log.starts_with(&hq_root), "{:?}", handle.log);

    let spawner = nunki::engine::spawn::ContainerSpawner::new(
        engine.clone(),
        profile.clone(),
        &compose_project,
        nunki::compose::AGENT_SERVICE,
    );
    let harness =
        nunki::harness::claude_code::ClaudeCode::new(Default::default(), Box::new(spawner));

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
        nunki::harness::Outcome::Finished(usage) => {
            println!("read back: {usage:?}");
            assert_eq!(usage.output_tokens, 7, "nunki read the run's own numbers");
        }
        other => panic!("expected a finished run, got {other:?}"),
    }

    // Starting it twice is refused: the state already says where it is.
    let again = run::start(&project, "alpha", &slot, engine.clone(), &engine_bin);
    assert!(matches!(again, Err(run::RunError::AlreadyStarted(_))));

    engine.down(&profile, &compose_project, true).unwrap();
    let _ = nunki::slot::rm(&project, "one", true);
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

    let held = nunki::state::SlotLock::acquire(&locks, "one", "something else").unwrap();
    let slot = nunki::slot::Slot {
        name: "one".to_string(),
        tree: dir.path().join("slot"),
    };
    let engine: std::sync::Arc<dyn nunki::engine::Engine> =
        std::sync::Arc::new(nunki::engine::fake::FakeEngine::default());

    let err = run::start(&project, "m1", &slot, engine, "docker").unwrap_err();
    // Two `nunki` on one slot is two agents in one tree (SPEC 4.2).
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
/// answering "not alive" for it is how `nunki` came to blame an agent for its
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
    use nunki::engine::spawn::ContainerSpawner;
    use nunki::harness::spawn::{CommandSpec, Presence, Spawned, Spawner};

    let dir = tempfile::tempdir().unwrap();
    let engine_bin = std::env::var("HQ_ENGINE").unwrap_or_else(|_| "docker".to_string());
    let profile = dir.path().join("probe.yml");
    let project = "nunki-pidprobe";
    std::fs::write(
        &profile,
        concat!(
            "services:\n",
            "  agent:\n",
            "    image: alpine:3.20\n",
            // The agent's own writable directory, where a run publishes its
            // pid. Without it the wrapper has nowhere to write and the
            // launch times out — which is what a real profile mounts.
            "    tmpfs:\n      - /run/nunki\n",
            "    command: [\"sleep\", \"600\"]\n",
        ),
    )
    .unwrap();

    let engine: std::sync::Arc<dyn nunki::engine::Engine> =
        std::sync::Arc::new(nunki::engine::docker::Docker::real());
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

fn header_with(integration: nunki::mission::Integration) -> nunki::mission::Header {
    nunki::mission::Header {
        branch: "feat/alpha".to_string(),
        base: "dev".to_string(),
        lots: vec![nunki::mission::Lot {
            id: "L1".to_string(),
            title: "one".to_string(),
        }],
        integration,
        security: nunki::mission::Security::Agent,
        arbiter: None,
        run: None,
        account: None,
        model: None,
        bounds: Default::default(),
    }
}

fn with_services() -> nunki::mission::Integration {
    nunki::mission::Integration::Services {
        services: vec![nunki::mission::Service {
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
    slot: &nunki::slot::Slot,
    header: &nunki::mission::Header,
    role: Role,
) -> (nunki::compose::Plan, serde_yaml_ng::Value) {
    let images = nunki::image::Images {
        agent: "img/agent".into(),
        firewall: "img/fw".into(),
        prober: "img/probe".into(),
    };
    let paths = nunki::mission::dir::Paths::of(&project.hq_root, "m1");
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
    let engine = nunki::engine::fake::FakeEngine::default();
    let written = nunki::compose::generate(&plan, nunki::engine::Engine::dialect(&engine)).unwrap();
    (plan, serde_yaml_ng::from_str(&written).unwrap())
}

fn slot_at(dir: &Path) -> nunki::slot::Slot {
    let tree = dir.join("slot");
    std::fs::create_dir_all(&tree).unwrap();
    nunki::slot::Slot {
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
    let project = Project::at(dir.path().join("repo"), config, dir.path().join("nunki"));
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
    let mounts: Vec<String> = doc["services"][nunki::compose::AGENT_SERVICE]["volumes"]
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
    let project = Project::at(dir.path().join("repo"), config, dir.path().join("nunki"));
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
    // Verbatim: a key nunki does not know about survives, because re-typing the
    // block would lose it.
    assert_eq!(
        doc["services"]["db"]["volumes"][0].as_str(),
        Some("dbdata:/var/lib/postgresql/data")
    );
    // And the firewall attaches to the network the project declared — it is
    // the one that can, the agent having `network_mode` instead.
    assert_eq!(
        doc["services"][nunki::compose::FIREWALL_SERVICE]["networks"][0].as_str(),
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

/// A services file that `nunki.yaml` names and the commit does not carry is
/// said by name, before a profile is lifted without the services the
/// integrator was called to wire.
#[test]
fn a_services_file_that_is_not_on_the_commit_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = project(dir.path()).config;
    config.services_file = Some(std::path::PathBuf::from("compose.yaml"));
    let project = Project::at(dir.path().join("repo"), config, dir.path().join("nunki"));
    let slot = slot_at(dir.path());
    let images = nunki::image::Images {
        agent: "img/agent".into(),
        firewall: "img/fw".into(),
        prober: "img/probe".into(),
    };
    let paths = nunki::mission::dir::Paths::of(&project.hq_root, "m1");
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

        // And a commit made in that container says the same role, from the
        // run's environment — git reads these before any config file, so the
        // slot's config cannot answer for them.
        //
        // Measured on `notes-2` on 2026-09-16, before this existed: nunki set
        // no identity, the agents set one in the slot's config, and the
        // coder of a second mission committed as `nunki integrator` — the
        // identity the previous mission's integrator had left there.
        //
        // The **committer** carries it. The author is the human's, and
        // `a_commit_is_attributed_to_the_human_who_owns_it` covers that.
        for (var, value) in [
            ("GIT_COMMITTER_NAME", format!("nunki {said}")),
            ("GIT_COMMITTER_EMAIL", format!("{said}@nunki.local")),
        ] {
            assert_eq!(
                plan.environment.get(var),
                Some(&value),
                "{var} on the {role:?} profile"
            );
        }
    }
}

/// A commit an agent makes is **authored** by the human whose project it is.
///
/// Not a nicety: a forge reads the author line and nothing else. Measured on
/// 2026-09-21, on a real repository — GitHub resolves a commit's author email
/// to an account and leaves `author` null when it cannot, so every commit
/// authored as `coder@nunki.local` was attributed to nobody, absent from the
/// contribution graph of the person who owns the repository, framed the
/// mission, authorised the push and merged it.
///
/// The role is not lost: it moves to the committer, which the test above
/// pins, and which a forge shows beside the author.
#[test]
fn a_commit_is_attributed_to_the_human_who_owns_it() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let slot = slot_at(dir.path());
    let header = header_with(with_services());

    std::fs::create_dir_all(project.nunki_home()).unwrap();
    std::fs::write(
        project.nunki_home().join(nunki::human::ME_FILE),
        "name: Ada Lovelace\nemail: ada@example.org\n",
    )
    .unwrap();

    for role in [Role::Coder, Role::Integrator, Role::Security] {
        let (plan, _) = profile_for(&project, &slot, &header, role);
        assert_eq!(
            plan.environment.get("GIT_AUTHOR_NAME").map(String::as_str),
            Some("Ada Lovelace"),
            "{role:?}"
        );
        assert_eq!(
            plan.environment.get("GIT_AUTHOR_EMAIL").map(String::as_str),
            Some("ada@example.org"),
            "{role:?}: a forge resolves this address to an account, or to nobody"
        );
        // And the role is still on the commit, in the other line.
        assert_eq!(
            plan.environment
                .get("GIT_COMMITTER_EMAIL")
                .map(String::as_str),
            Some(format!("{}@nunki.local", nunki::role::slug(role)).as_str()),
            "{role:?}"
        );
    }
}

/// A name with no address attributes to nobody, so `nunki` does not pretend.
///
/// An invented address — `ada@localhost`, the machine's hostname, anything —
/// would read as attribution on every commit and resolve to no account on
/// any forge. The role signs both lines instead, which is visibly wrong
/// rather than quietly wrong.
#[test]
fn a_human_without_an_address_does_not_get_an_invented_one() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let slot = slot_at(dir.path());
    let header = header_with(with_services());

    std::fs::create_dir_all(project.nunki_home()).unwrap();
    std::fs::write(
        project.nunki_home().join(nunki::human::ME_FILE),
        "name: Ada Lovelace\n",
    )
    .unwrap();

    let (plan, _) = profile_for(&project, &slot, &header, Role::Coder);
    assert_eq!(
        plan.environment.get("GIT_AUTHOR_EMAIL").map(String::as_str),
        Some("coder@nunki.local"),
    );
    assert_eq!(
        plan.environment.get("GIT_AUTHOR_NAME").map(String::as_str),
        Some("nunki coder"),
    );
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
        project.fragment("rust").join(nunki::project::WRITABLE_FILE),
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
        "nunki-one-security-target"
    );
    assert_eq!(
        run::writable_volume("one", "packages/web/node_modules"),
        "nunki-one-security-packages-web-node_modules"
    );
    // Declared as well as mounted, or Compose refuses the whole project.
    let declared = doc["volumes"].as_mapping().unwrap();
    assert!(
        declared.contains_key(serde_yaml_ng::Value::from(
            "nunki-one-security-target".to_string()
        )),
        "{declared:?}"
    );
    // And the tree stays read-only around them.
    let mounts: Vec<String> = doc["services"][nunki::compose::AGENT_SERVICE]["volumes"]
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
        project.fragment("rust").join(nunki::project::WRITABLE_FILE),
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
/// file system`, measured 2026-09-10. So `nunki` makes the directory in the
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
    let given = nunki::harness::SessionId("s-coder".into());
    assert_eq!(run::session_for(Some(&given)), (given.clone(), true));
    let (fresh, resume) = run::session_for(None);
    assert!(!resume);
    assert_ne!(fresh, given);
    assert_ne!(run::session_for(None).0, fresh, "a fresh one each time");
}

/// The coder is told to stand in for what it cannot reach, because a gate
/// that requires what nothing states grades an agent on a secret.
///
/// SPEC 4.4 has said "bouchonné, ou un composant local jetable" since the
/// first day; the prompt had not. Measured on `notes-2` on 2026-09-16: the
/// coder wrote a PostgreSQL store with no seam, the mutation campaign left
/// five survivors nobody in that container could kill, and gate 7 spent all
/// three attempts on a triage that could not be won.
#[test]
fn the_coder_is_told_to_mock_what_its_container_cannot_reach() {
    let coder = role::prompt(Role::Coder);

    assert!(
        coder.contains("What you cannot reach, you stand in for"),
        "{coder}"
    );
    assert!(
        coder.contains("seam"),
        "the rule names what to write against, not only what not to do: {coder}"
    );
    // The half that is not the coder's, named in the same breath, so the
    // rule does not read as "test the database yourself".
    assert!(coder.contains("is the integrator's"), "{coder}");
    // And the escape hatch is closed where an agent would reach for it.
    assert!(coder.contains("`#[ignore]`"), "{coder}");

    // The other two roles are not told this: the integrator has the service,
    // and the security agent commits nothing.
    for other in [role::prompt(Role::Integrator), role::prompt(Role::Security)] {
        assert!(
            !other.contains("What you cannot reach, you stand in for"),
            "{other}"
        );
    }
}

/// A slot is a clone, and a clone writes its own `dev` once. Nothing moves it
/// again — so every mission after the first branched from the base as it
/// stood the day the slot was made, and its pull request opened against a
/// base that had moved. Bit for real twice on `notes-api`, worked around by
/// hand both times.
///
/// A slot's `origin` is the project on this machine rather than the forge, so
/// refreshing it is local and needs no network.
#[test]
fn a_second_mission_branches_from_a_base_the_slot_refreshed() {
    let dir = tempfile::tempdir().unwrap();
    let origin = dir.path().join("origin");
    std::fs::create_dir_all(&origin).unwrap();
    git(&origin, &["init", "-q", "-b", "dev"]);
    git(&origin, &["config", "user.email", "a@b.c"]);
    git(&origin, &["config", "user.name", "t"]);
    std::fs::write(origin.join("f"), "one").unwrap();
    git(&origin, &["add", "-A"]);
    git(&origin, &["commit", "-qm", "base"]);

    let tree = dir.path().join("slot");
    git(
        dir.path(),
        &["clone", "-q", origin.to_str().unwrap(), "slot"],
    );
    git(&tree, &["config", "user.email", "a@b.c"]);
    git(&tree, &["config", "user.name", "t"]);
    let slot = nunki::slot::Slot {
        name: "one".into(),
        tree: tree.clone(),
    };

    // The first mission, then the base moves the way a merged pull request
    // moves it.
    nunki::run::branch(&slot, "mission/first", "dev").unwrap();
    std::fs::write(origin.join("f"), "two").unwrap();
    git(&origin, &["add", "-A"]);
    git(&origin, &["commit", "-qm", "what the first mission merged"]);
    let moved = git(&origin, &["rev-parse", "HEAD"]);

    nunki::run::branch(&slot, "mission/second", "dev").unwrap();

    assert_eq!(
        git(&tree, &["rev-parse", "HEAD"]),
        moved,
        "the second mission started from the slot's own stale dev"
    );
}

/// `checkout -B` repoints a branch at its start. Measured on 2026-09-17:
/// `checkout -B mission/x dev` on a branch holding one commit of work left it
/// holding none. Any launch that found the slot on another branch — a human
/// looking at something, a role switch that did not come back — spent the
/// mission's work to get back to it.
#[test]
fn a_branch_that_already_holds_work_is_checked_out_and_never_moved() {
    let dir = tempfile::tempdir().unwrap();
    let origin = dir.path().join("origin");
    std::fs::create_dir_all(&origin).unwrap();
    git(&origin, &["init", "-q", "-b", "dev"]);
    git(&origin, &["config", "user.email", "a@b.c"]);
    git(&origin, &["config", "user.name", "t"]);
    std::fs::write(origin.join("f"), "one").unwrap();
    git(&origin, &["add", "-A"]);
    git(&origin, &["commit", "-qm", "base"]);

    let tree = dir.path().join("slot");
    git(
        dir.path(),
        &["clone", "-q", origin.to_str().unwrap(), "slot"],
    );
    git(&tree, &["config", "user.email", "a@b.c"]);
    git(&tree, &["config", "user.name", "t"]);
    let slot = nunki::slot::Slot {
        name: "one".into(),
        tree: tree.clone(),
    };

    nunki::run::branch(&slot, "mission/x", "dev").unwrap();
    std::fs::write(tree.join("w"), "the agent's work").unwrap();
    git(&tree, &["add", "-A"]);
    git(&tree, &["commit", "-qm", "the agent's work"]);
    let work = git(&tree, &["rev-parse", "HEAD"]);

    // Someone leaves the slot somewhere else, and the next role launches.
    git(&tree, &["checkout", "-q", "dev"]);
    nunki::run::branch(&slot, "mission/x", "dev").unwrap();

    assert_eq!(
        git(&tree, &["rev-parse", "HEAD"]),
        work,
        "the mission's commit was thrown away getting back to its branch"
    );
    assert_eq!(
        git(&tree, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "mission/x"
    );
}

/// A cache belongs to a toolchain, and `nunki` is agnostic to the stack: the
/// principle (SPEC, section 1) names caches among the declared fragments,
/// beside the battery and the domains.
///
/// It was not. `/home/agent/.cargo/registry` was mounted from `nunki`'s own
/// code for **every stack and every role**, so a Python project would have
/// carried an empty `cargo` volume and none for pip — the first wall the
/// second stack would have hit.
#[test]
fn the_caches_a_slot_keeps_are_the_ones_its_stack_declares() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    std::fs::create_dir_all(project.fragment("rust")).unwrap();
    std::fs::write(
        project.fragment("rust").join(nunki::project::CACHES_FILE),
        "# what this toolchain keeps outside the tree\n\
         /home/agent/.cache/pip\n\n\
         /home/agent/.venv\n",
    )
    .unwrap();
    let slot = slot_at(dir.path());
    let header = header_with(with_services());

    for role in [Role::Coder, Role::Integrator, Role::Security] {
        let (plan, _) = profile_for(&project, &slot, &header, role);
        let at: Vec<String> = plan
            .volumes
            .iter()
            .map(|v| v.at.display().to_string())
            .collect();
        assert!(at.contains(&"/home/agent/.cache/pip".to_string()), "{at:?}");
        assert!(at.contains(&"/home/agent/.venv".to_string()), "{at:?}");
        // And nothing the core invented: this stack declares no cargo.
        assert!(
            !at.iter().any(|p| p.contains("cargo")),
            "the core named a cache the stack did not declare: {at:?}"
        );
        // Named after the path, so two caches of one stack cannot collide.
        let names: Vec<String> = plan.volumes.iter().map(|v| v.name.clone()).collect();
        assert!(
            names.contains(&run::cache_volume("one", "/home/agent/.cache/pip")),
            "{names:?}"
        );
        assert_ne!(
            run::cache_volume("one", "/home/agent/.cache/pip"),
            run::cache_volume("one", "/home/agent/.venv")
        );
    }
}

/// A stack that declares no cache gets none, rather than one `nunki` chose.
#[test]
fn a_stack_that_declares_no_cache_carries_none() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    std::fs::create_dir_all(project.fragment("rust")).unwrap();
    let slot = slot_at(dir.path());
    let header = header_with(with_services());

    let (plan, _) = profile_for(&project, &slot, &header, Role::Coder);

    // The exact set, not the absence of one spelling: the cache this replaces
    // was named `-cargo`, so a test looking for `-cache-` would have watched
    // the old code put it back and called it green.
    let names: Vec<String> = plan.volumes.iter().map(|v| v.name.clone()).collect();
    assert_eq!(
        names,
        vec![
            nunki::exec::proof_volume("one"),
            format!("nunki-{}-harness", slot.name),
        ],
        "the core carried a volume nothing declared"
    );
}

/// Where a harness keeps its sessions is the harness's to say, through the one
/// place a name becomes an adapter. `nunki` wrote `/home/agent/.claude` in its
/// own volume list — one harness's default, spelled out in the core, for a
/// system that claims to be agnostic to the harness (SPEC, section 1).
///
/// This one pins the wiring and **cannot fail today**: the literal it replaced
/// is the same path this adapter declares, and there is only one adapter. It
/// is [`a_harness_that_keeps_nothing_carries_no_volume`] that bites — measured,
/// by putting the literal back and watching that one go red. Said here rather
/// than left for a reader to discover.
#[test]
fn the_harness_says_where_it_keeps_its_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    std::fs::create_dir_all(project.fragment("rust")).unwrap();
    let slot = slot_at(dir.path());
    let header = header_with(with_services());

    let (plan, _) = profile_for(&project, &slot, &header, Role::Coder);

    let declared = nunki::image::harness_provisioning(&project.config.harness)
        .config_dir
        .expect("the harness says where it keeps its sessions");
    let volume = plan
        .volumes
        .iter()
        .find(|v| v.name == format!("nunki-{}-harness", slot.name))
        .expect("the harness volume is there");
    assert_eq!(
        volume.at, declared,
        "the core chose the path, not the adapter"
    );
}

/// A harness that keeps nothing gets no volume — and the answer comes from the
/// adapter, not from a name the core recognises.
#[test]
fn a_harness_that_keeps_nothing_carries_no_volume() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = project(dir.path());
    std::fs::create_dir_all(project.fragment("rust")).unwrap();
    // The stack carries the perimeter here: an unknown harness declares no
    // domain, and `nunki` refuses an empty perimeter rather than lifting a
    // container that can reach nothing.
    std::fs::write(
        project.fragment("rust").join("allow.txt"),
        "index.crates.io\n",
    )
    .unwrap();
    project.config.harness = "a-harness-nunki-has-no-adapter-for".into();
    let slot = slot_at(dir.path());
    let header = header_with(with_services());

    let (plan, _) = profile_for(&project, &slot, &header, Role::Coder);

    assert!(
        !plan.volumes.iter().any(|v| v.name.ends_with("-harness")),
        "{:?}",
        plan.volumes
    );
}

/// Where the advisory database lives is the stack's to say, as a path on the
/// host: `nunki` mounts it read-only and never fills it, because the tool owns
/// its own layout (SPEC 4.4, gate 8).
#[test]
fn a_stack_says_where_its_advisory_database_lives() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    std::fs::create_dir_all(project.fragment("rust")).unwrap();
    let file = project
        .fragment("rust")
        .join(nunki::project::ADVISORIES_FILE);

    // Absent: the stack declares none, and that is not an error here — the
    // gate is what says a database is missing.
    assert_eq!(project.stack_advisories("rust"), None);

    // A path a human can type, tilde and all.
    std::fs::write(&file, "# where it lives\n~/.cargo/advisory-db\n").unwrap();
    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    assert_eq!(
        project.stack_advisories("rust"),
        Some(home.join(".cargo/advisory-db"))
    );

    // Relative is refused rather than resolved against whatever directory
    // `nunki` happened to be started in.
    std::fs::write(&file, "advisory-db\n").unwrap();
    assert_eq!(project.stack_advisories("rust"), None);

    // Comments and blank lines are not a path.
    std::fs::write(&file, "# only a comment\n\n").unwrap();
    assert_eq!(project.stack_advisories("rust"), None);
}

/// Gate 8's script reaches the container the way every script that judges the
/// agent does: read-only, one file at a time, at `/work/stack`.
#[test]
fn the_security_script_reaches_the_container_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    std::fs::create_dir_all(project.fragment("rust")).unwrap();
    let script = project.fragment("rust").join(nunki::gate::SECURITY);
    std::fs::write(&script, "#!/bin/sh\n").unwrap();
    let slot = slot_at(dir.path());
    let header = header_with(with_services());

    for role in [Role::Coder, Role::Integrator, Role::Security] {
        let (_, doc) = profile_for(&project, &slot, &header, role);
        let mounts: Vec<String> = doc["services"][nunki::compose::AGENT_SERVICE]["volumes"]
            .as_sequence()
            .expect("the agent has mounts")
            .iter()
            .map(|v| v.as_str().unwrap_or_default().to_string())
            .collect();
        let at = format!("{}/{}", run::STACK_AT, nunki::gate::SECURITY);
        assert!(
            mounts.iter().any(|m| m.contains(&at) && m.ends_with(":ro")),
            "{role:?} does not carry {at} read-only: {mounts:?}"
        );
    }
}

/// A stack that ships no `security.sh` mounts none. A bind mount of a missing
/// source makes the engine create a directory in its place, and the gate would
/// read a directory instead of saying the script is absent.
#[test]
fn a_stack_without_a_security_script_mounts_nothing_in_its_place() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    std::fs::create_dir_all(project.fragment("rust")).unwrap();
    let slot = slot_at(dir.path());
    let header = header_with(with_services());

    let (plan, _) = profile_for(&project, &slot, &header, Role::Coder);

    assert!(
        !plan
            .stack_scripts
            .iter()
            .any(|(_, at)| at.ends_with(nunki::gate::SECURITY)),
        "{:?}",
        plan.stack_scripts
    );
}

/// The advisory database is the host's, mounted read-only at a path `nunki`
/// fixes. The stack says where it lives; `nunki` says where it lands, because
/// a path the agent could influence would let it point the audit at an empty
/// directory — no findings, and a green gate.
#[test]
fn the_advisory_database_is_mounted_read_only_where_nunki_says() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    std::fs::create_dir_all(project.fragment("rust")).unwrap();
    let db = dir.path().join("advisory-db");
    std::fs::create_dir_all(&db).unwrap();
    std::fs::write(
        project
            .fragment("rust")
            .join(nunki::project::ADVISORIES_FILE),
        format!("{}\n", db.display()),
    )
    .unwrap();
    let slot = slot_at(dir.path());
    let header = header_with(with_services());

    let (plan, doc) = profile_for(&project, &slot, &header, Role::Coder);

    assert_eq!(
        plan.advisories,
        Some((db.clone(), std::path::PathBuf::from(run::ADVISORIES_AT)))
    );
    let mounts: Vec<String> = doc["services"][nunki::compose::AGENT_SERVICE]["volumes"]
        .as_sequence()
        .expect("the agent has mounts")
        .iter()
        .map(|v| v.as_str().unwrap_or_default().to_string())
        .collect();
    assert!(
        mounts
            .iter()
            .any(|m| m.contains(run::ADVISORIES_AT) && m.ends_with(":ro")),
        "{mounts:?}"
    );
}

/// Everything mounted **only when it exists** lands under `NUNKI_AT`, where
/// the agent cannot make it.
///
/// Measured in a container on 2026-09-18: as the agent, `mkdir -p
/// /work/advisories` succeeds — the Dockerfile chowns `/work` — and `mkdir
/// /nunki` is refused. A conditional mount under `/work` is therefore, on the
/// day it is absent, a path the agent writes: its own exceptions for the
/// secrets file, and for the advisory database a forged empty directory that
/// turns gate 8's 69 into a green gate (`exit=0`, no findings, measured).
#[test]
fn what_is_mounted_only_when_it_exists_lands_where_no_agent_could_make_it() {
    let root = format!("{}/", run::NUNKI_AT);
    for at in [run::ADVISORIES_AT, nunki::secrets::AT] {
        assert!(at.starts_with(&root), "{at} is not under {}", run::NUNKI_AT);
    }
    assert!(
        !run::NUNKI_AT.starts_with("/work"),
        "{} is the agent's to write",
        run::NUNKI_AT
    );
    // And it is a top level, so the agent cannot create it either: only `/`
    // is above it, and `/` is root's.
    assert_eq!(
        run::NUNKI_AT.matches('/').count(),
        1,
        "{} has a parent the agent might own",
        run::NUNKI_AT
    );
}

/// The secrets a human ruled on reach the container read-only, from the HQ,
/// at a path `nunki` fixes.
#[test]
fn the_rulings_reach_the_container_read_only_where_no_agent_could_write_them() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    std::fs::create_dir_all(project.fragment("rust")).unwrap();
    let at = nunki::secrets::file(&project);
    nunki::secrets::accept(&at, "abc:src/store.rs:Postgres:22", "a disposable test DSN").unwrap();
    let slot = slot_at(dir.path());
    let header = header_with(with_services());

    let (plan, doc) = profile_for(&project, &slot, &header, Role::Coder);

    assert_eq!(
        plan.secrets,
        Some((at, std::path::PathBuf::from(nunki::secrets::AT)))
    );
    let mounts: Vec<String> = doc["services"][nunki::compose::AGENT_SERVICE]["volumes"]
        .as_sequence()
        .expect("the agent has mounts")
        .iter()
        .map(|v| v.as_str().unwrap_or_default().to_string())
        .collect();
    assert!(
        mounts
            .iter()
            .any(|m| m.contains(nunki::secrets::AT) && m.ends_with(":ro")),
        "{mounts:?}"
    );
}

/// A project nobody has ruled on for mounts nothing, rather than an empty
/// file: the engine would make a **directory** at that path, and the script
/// would be reading a folder.
#[test]
fn a_project_that_has_ruled_on_nothing_mounts_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    std::fs::create_dir_all(project.fragment("rust")).unwrap();
    let slot = slot_at(dir.path());
    let header = header_with(with_services());

    let (plan, _) = profile_for(&project, &slot, &header, Role::Coder);

    assert_eq!(plan.secrets, None);
}

/// A database the host has not filled is not mounted at all. The engine would
/// create an empty directory in its place, and gate 8 would read "nothing to
/// report" where it should say it has no database.
#[test]
fn a_database_the_host_never_filled_is_not_mounted_empty() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    std::fs::create_dir_all(project.fragment("rust")).unwrap();
    std::fs::write(
        project
            .fragment("rust")
            .join(nunki::project::ADVISORIES_FILE),
        format!("{}/never-fetched\n", dir.path().display()),
    )
    .unwrap();
    let slot = slot_at(dir.path());
    let header = header_with(with_services());

    let (plan, _) = profile_for(&project, &slot, &header, Role::Coder);

    assert_eq!(plan.advisories, None);
}
