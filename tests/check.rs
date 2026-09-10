//! `hq check` (SPEC 4.2): red when a restriction is not held, and always
//! explicit about what it could not establish.

use std::path::{Path, PathBuf};

use hq::check::{Report, Verdict, collisions, run};
use hq::project::{Config, Project, ProjectError, ProtectedPaths};

fn config() -> Config {
    Config {
        harness: "claude-code".to_string(),
        forge: vec!["github.com".to_string()],
        stacks: Vec::new(),
        protected_branches: vec!["main".to_string()],
        protected_paths: ProtectedPaths::default(),
        account: None,
        bounds: Default::default(),
        credentials: None,
        run: None,
    }
}

/// A project on disk that holds everything, so a test can break one thing.
fn sound(dir: &Path) -> Project {
    let root = dir.join("repo");
    let hq_root = dir.join("hq");
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::create_dir_all(&hq_root).unwrap();
    std::fs::write(root.join(".gitattributes"), "* text=auto eol=lf\n").unwrap();
    // The rules of the place, and the file the harness actually reads.
    std::fs::write(root.join("AGENTS.md"), "# rules\n").unwrap();
    std::fs::write(root.join("CLAUDE.md"), "@AGENTS.md\n").unwrap();
    Project::at(root, config(), hq_root)
}

fn verdict(report: &Report, what: &str) -> Verdict {
    report
        .checks
        .iter()
        .find(|c| c.what.contains(what))
        .unwrap_or_else(|| panic!("no check about {what:?} in {:?}", report.checks))
        .verdict
        .clone()
}

fn is_red(v: &Verdict) -> bool {
    matches!(v, Verdict::Red(_))
}

#[test]
fn a_sound_project_is_green_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let report = run(&sound(dir.path()));
    assert!(!report.is_red(), "{}", report.render());
    // Two, and both are things this machine cannot answer for rather than
    // things that are wrong: no stack is declared, and no harness token is
    // in reach.
    assert_eq!(report.unchecked(), 2, "{}", report.render());
    assert!(report.render().contains("2 could not be checked"));
}

#[test]
fn a_project_without_git_is_red_because_a_slot_is_a_clone() {
    let dir = tempfile::tempdir().unwrap();
    let project = sound(dir.path());
    std::fs::remove_dir_all(project.root.join(".git")).unwrap();
    assert!(is_red(&verdict(&run(&project), "git repository")));
}

#[test]
fn a_missing_hq_is_red_because_the_state_lives_there() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = sound(dir.path());
    project.hq_root = dir.path().join("nowhere");
    assert!(is_red(&verdict(&run(&project), "HQ exists")));
}

#[test]
fn a_windows_drive_under_wsl_is_red() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = sound(dir.path());
    project.root = PathBuf::from("/mnt/c/code/demo");
    let v = verdict(&run(&project), "a container can mount");
    assert!(is_red(&v), "{v:?}");
    match v {
        Verdict::Red(why) => assert!(why.contains("permissions git needs"), "{why}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_harness_without_an_adapter_is_red() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = sound(dir.path());
    project.config.harness = "some-future-agent".to_string();
    let v = verdict(&run(&project), "harness has an adapter");
    assert!(is_red(&v), "{v:?}");
}

#[test]
fn credentials_inside_the_tree_are_red_because_the_coder_would_read_them() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = sound(dir.path());
    project.config.credentials = Some(project.root.join("secrets"));
    assert!(is_red(&verdict(&run(&project), "credentials live outside")));

    project.config.credentials = Some(dir.path().join("elsewhere"));
    assert!(!is_red(&verdict(
        &run(&project),
        "credentials live outside"
    )));
}

#[test]
fn line_endings_are_red_when_pinned_wrongly_and_unchecked_when_not_pinned() {
    let dir = tempfile::tempdir().unwrap();
    let project = sound(dir.path());

    std::fs::write(project.root.join(".gitattributes"), "*.png binary\n").unwrap();
    assert!(is_red(&verdict(&run(&project), "line endings")));

    // Absent is not a violation: it is something this machine cannot answer
    // for, and saying so is the point.
    std::fs::remove_file(project.root.join(".gitattributes")).unwrap();
    assert!(matches!(
        verdict(&run(&project), "line endings"),
        Verdict::NotChecked(_)
    ));
}

#[test]
fn a_stack_that_names_the_forge_is_red_and_one_that_does_not_is_green() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = sound(dir.path());
    project.config.stacks = vec!["rust".to_string()];
    let fragment = project.fragment("rust");
    std::fs::create_dir_all(&fragment).unwrap();

    std::fs::write(fragment.join("allow.txt"), "index.crates.io\ngithub.com\n").unwrap();
    let v = verdict(&run(&project), "allowlist for rust");
    assert!(is_red(&v), "{v:?}");

    std::fs::write(
        fragment.join("allow.txt"),
        "# what a build needs\nindex.crates.io\nstatic.crates.io\n",
    )
    .unwrap();
    assert!(!is_red(&verdict(&run(&project), "allowlist for rust")));
}

#[test]
fn a_stack_without_an_allowlist_is_unchecked_not_green() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = sound(dir.path());
    project.config.stacks = vec!["python".to_string()];
    assert!(matches!(
        verdict(&run(&project), "allowlist for python"),
        Verdict::NotChecked(_)
    ));
}

#[test]
fn paths_that_differ_only_by_case_are_found() {
    // Hand-written names: on a case-insensitive filesystem such a pair
    // cannot be created, which is exactly why it survives a commit made on
    // Linux and breaks the next clone.
    let clashes = collisions(&[
        "src/Main.rs".to_string(),
        "src/lib.rs".to_string(),
        "src/main.rs".to_string(),
    ]);
    assert_eq!(clashes, vec!["src/Main.rs and src/main.rs"]);
    assert!(collisions(&["a".to_string(), "b".to_string()]).is_empty());
}

#[test]
fn an_unchecked_thing_is_never_reported_as_a_pass() {
    let mut report = Report::default();
    report.checks.push(hq::check::Check {
        what: "something".to_string(),
        verdict: Verdict::NotChecked("no engine here".to_string()),
    });
    let text = report.render();
    assert!(!report.is_red(), "unchecked is not a violation");
    assert!(
        !text.contains("everything checked is held.\n"),
        "it must not read as a clean pass: {text}"
    );
    assert!(text.contains("1 could not be checked"), "{text}");
}

#[test]
fn a_project_is_found_by_walking_up_the_way_git_does() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    let deep = root.join("src").join("engine");
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(
        root.join("hq.yaml"),
        "harness: claude-code\nforge: [github.com]\nstacks: [rust]\n",
    )
    .unwrap();

    let project = Project::open(&deep).unwrap();
    assert_eq!(project.root, root);
    assert_eq!(project.config.harness, "claude-code");
    assert_eq!(project.config.stacks, vec!["rust".to_string()]);
    // Defaulted rather than demanded: a short file is a readable file.
    assert!(
        project
            .config
            .protected_branches
            .contains(&"main".to_string())
    );
    assert_eq!(project.config.bounds.max_volets, 3);
    // The HQ is never inside the repository.
    assert!(!project.hq_root.starts_with(&project.root));
    assert!(project.hq_root.ends_with(".hq/repo"));
}

#[test]
fn a_directory_that_is_not_a_project_says_which_file_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let err = Project::open(dir.path()).unwrap_err();
    assert!(matches!(err, ProjectError::NotAProject(_)));
    assert!(err.to_string().contains("hq.yaml"), "{err}");
    assert!(err.to_string().contains("hq init"), "{err}");
}

#[test]
fn a_malformed_config_names_the_file_and_the_reason() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("hq.yaml"), "harness: [not, a, string]\n").unwrap();
    let err = Project::open(dir.path()).unwrap_err();
    assert!(matches!(err, ProjectError::Invalid(..)), "{err}");
    assert!(err.to_string().contains("hq.yaml"), "{err}");
}

#[test]
fn a_report_holding_a_violation_is_red_and_says_how_many() {
    let mut report = Report::default();
    report.checks.push(hq::check::Check {
        what: "something".to_string(),
        verdict: Verdict::Green("fine".to_string()),
    });
    assert!(!report.is_red());

    report.checks.push(hq::check::Check {
        what: "something else".to_string(),
        verdict: Verdict::Red("not held".to_string()),
    });
    assert!(report.is_red(), "a Red must make the whole report red");
    assert!(
        report.render().contains("1 restriction(s) not held"),
        "{}",
        report.render()
    );
}

/// The verb itself, through the binary: what a human runs, and what a script
/// reads — the exit code.
#[test]
fn the_verb_exits_non_zero_when_a_restriction_is_not_held() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    // A project with no `.git`: a slot is a clone, so this cannot stand.
    std::fs::write(root.join("hq.yaml"), "harness: claude-code\n").unwrap();

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_hq"))
        .args(["-C"])
        .arg(&root)
        .arg("check")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("RED"), "{text}");
    assert_eq!(
        out.status.code(),
        Some(1),
        "a red check must fail a script, not just print: {text}"
    );

    // And a directory that is no project at all says so on stderr, without
    // pretending to have checked anything.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_hq"))
        .args(["-C"])
        .arg(dir.path())
        .arg("check")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        out.stdout.is_empty(),
        "nothing was checked, so nothing is reported"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("hq init"),
        "{:?}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// nunki orchestrates nunki: the first project the tool checks is the
/// one whose rules it enforces.
///
/// With a home of its own. The first version of this test read the
/// developer's, where the HQ happens to exist — so it was green here and red
/// on both CI runners, having asserted something about the machine rather
/// than about the repository.
#[test]
fn the_verb_is_green_on_this_very_repository() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".hq").join("nunki")).unwrap();

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_hq"))
        .env("HOME", home.path())
        .args(["-C"])
        .arg(root)
        .arg("check")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{text}");
    assert!(text.contains("the coder's allowlist for rust"), "{text}");
    // The container probes are named as not run, never silently skipped.
    assert!(text.contains("could not be checked"), "{text}");
}

#[test]
fn the_hq_is_the_projects_own_directory_under_the_home() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("some-project");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("hq.yaml"), "harness: claude-code\n").unwrap();

    let project = Project::open(&root).unwrap();
    let home = PathBuf::from(std::env::var("HOME").unwrap());
    assert_eq!(project.hq_root, home.join(".hq").join("some-project"));
}

#[test]
fn rules_the_harness_cannot_read_are_red() {
    let dir = tempfile::tempdir().unwrap();
    let project = sound(dir.path());

    // Claude Code reads CLAUDE.md and not AGENTS.md: without the import, a
    // mission starts without the rules of the place (SPEC 4.1).
    std::fs::write(project.root.join("CLAUDE.md"), "# my own\n").unwrap();
    assert!(is_red(&verdict(&run(&project), "rules of the place")));

    std::fs::remove_file(project.root.join("CLAUDE.md")).unwrap();
    assert!(is_red(&verdict(&run(&project), "rules of the place")));

    std::fs::write(project.root.join("CLAUDE.md"), "@AGENTS.md\n").unwrap();
    assert!(!is_red(&verdict(&run(&project), "rules of the place")));

    std::fs::remove_file(project.root.join("AGENTS.md")).unwrap();
    assert!(is_red(&verdict(&run(&project), "rules of the place")));
}

#[test]
fn the_account_a_project_spends_is_checked_not_assumed() {
    use hq::account::{Account, Accounts};

    let dir = tempfile::tempdir().unwrap();
    let project = sound(dir.path());
    let hq_home = project.hq_home();

    // Nothing declared: something to do, not something broken.
    let v = verdict(&run(&project), "can authenticate");
    assert!(matches!(v, Verdict::NotChecked(_)), "{v:?}");

    let write_index = |index: &Accounts| {
        std::fs::write(
            hq_home.join("accounts.yaml"),
            serde_yaml_ng::to_string(index).unwrap(),
        )
        .unwrap()
    };
    let account = |harness: &str, file: &str| Account {
        harness: harness.to_string(),
        token_file: std::path::PathBuf::from(file),
        note: None,
    };

    // Declared, but no token yet: still something to do, with the command.
    write_index(&Accounts {
        accounts: [(
            "perso".to_string(),
            account("claude-code", "accounts/perso"),
        )]
        .into_iter()
        .collect(),
        default: None,
    });
    match verdict(&run(&project), "can authenticate") {
        Verdict::NotChecked(why) => assert!(why.contains("claude setup-token"), "{why}"),
        other => panic!("{other:?}"),
    }

    // A token that authenticates another harness is refused rather than
    // passed along to fail inside a container.
    write_index(&Accounts {
        accounts: [("openai".to_string(), account("codex", "accounts/openai"))]
            .into_iter()
            .collect(),
        default: None,
    });
    assert!(is_red(&verdict(&run(&project), "can authenticate")));

    // The real thing, with the right harness and a private file.
    write_index(&Accounts {
        accounts: [(
            "perso".to_string(),
            account("claude-code", "accounts/perso"),
        )]
        .into_iter()
        .collect(),
        default: None,
    });
    std::fs::create_dir_all(hq_home.join("accounts")).unwrap();
    let token = hq_home.join("accounts/perso");
    std::fs::write(&token, "sk-ant-oat-example\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Readable by others is not good enough for a secret.
        std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(is_red(&verdict(&run(&project), "can authenticate")));
        std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert!(!is_red(&verdict(&run(&project), "can authenticate")));
}
