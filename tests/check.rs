//! `nunki check` (SPEC 4.2): red when a restriction is not held, and always
//! explicit about what it could not establish.

use std::path::{Path, PathBuf};

use nunki::check::{Report, Verdict, collisions, run};
use nunki::project::{Config, Project, ProjectError, ProtectedPaths};

mod common;
use common::serve;

fn config() -> Config {
    Config {
        root: None,
        harness: "claude-code".to_string(),
        forge: vec!["github.com".to_string()],
        stacks: Vec::new(),
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
    }
}

/// A project on disk that holds everything, so a test can break one thing.
fn sound(dir: &Path) -> Project {
    let root = dir.join("repo");
    let home = dir.join("nunki");
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::create_dir_all(home.join(nunki::project::HQ_DIR)).unwrap();
    std::fs::write(root.join(".gitattributes"), "* text=auto eol=lf\n").unwrap();
    // The rules of the place, and the file the harness actually reads.
    std::fs::write(root.join("AGENTS.md"), "# rules\n").unwrap();
    std::fs::write(root.join("CLAUDE.md"), "@AGENTS.md\n").unwrap();
    // And a human who said who they are — otherwise the fixture would read
    // this machine's global git configuration, and be green here and amber
    // on a runner that has none.
    std::fs::write(dir.join("me.yaml"), "name: Arnaud\n").unwrap();
    Project::at(root, config(), home)
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
    // Named rather than counted: a count says "two" and a name says which,
    // and only the second survives a check being added.
    let unchecked: Vec<&str> = report
        .checks
        .iter()
        .filter(|c| matches!(c.verdict, Verdict::NotChecked(_)))
        .map(|c| c.what.as_str())
        .collect();
    assert_eq!(
        unchecked,
        vec![
            "the account this project spends can authenticate",
            "the coder's allowlist names no forge domain",
        ],
        "{}",
        report.render()
    );
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
    report.checks.push(nunki::check::Check {
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
fn a_project_is_found_from_anywhere_in_its_repository_and_read_from_its_home() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    let home = common::project_home(
        &root,
        dir.path(),
        "harness: claude-code\nforge: [github.com]\nstacks: [rust]\n",
    );
    let deep = root.join("src").join("engine");
    std::fs::create_dir_all(&deep).unwrap();

    // The repository's top level, the way git finds it.
    let found = Project::find_root(&deep).unwrap();
    assert_eq!(found, std::fs::canonicalize(&root).unwrap());

    let project = Project::open_at(found, home.clone()).unwrap();
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
    // Nothing of nunki's is inside the repository: the HQ and the fragments
    // are in the home, and the accounts one level above it.
    assert!(!project.hq_root.starts_with(&project.root));
    assert_eq!(project.hq_root, home.join("hq"));
    assert_eq!(project.fragment("rust"), home.join("stacks/rust"));
    assert_eq!(project.nunki_home(), dir.path().join(".nunki"));
}

#[test]
fn a_directory_outside_any_repository_is_not_a_project() {
    let dir = tempfile::tempdir().unwrap();
    let err = Project::find_root(dir.path()).unwrap_err();
    assert!(matches!(err, ProjectError::NotARepository(_)), "{err}");
    assert!(err.to_string().contains("git repository"), "{err}");
}

#[test]
fn a_repository_nunki_was_never_given_says_which_file_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    let home = common::project_home(&root, dir.path(), "harness: claude-code\n");
    std::fs::remove_file(home.join("nunki.yaml")).unwrap();

    let err = Project::open_at(root, home).unwrap_err();
    assert!(matches!(err, ProjectError::NotAProject { .. }), "{err}");
    assert!(err.to_string().contains("nunki.yaml"), "{err}");
    assert!(err.to_string().contains("nunki init"), "{err}");
}

#[test]
fn a_malformed_config_names_the_file_and_the_reason() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    let home = common::project_home(&root, dir.path(), "harness: [not, a, string]\n");
    let err = Project::open_at(root, home).unwrap_err();
    assert!(matches!(err, ProjectError::Invalid(..)), "{err}");
    assert!(err.to_string().contains("nunki.yaml"), "{err}");
}

/// The home is named after the repository's directory, so two repositories
/// with the same name reach the same home. The second is refused rather than
/// handed the first one's configuration, HQ and missions.
#[test]
fn a_home_belongs_to_one_repository_and_refuses_another_of_the_same_name() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("a").join("repo");
    let home = common::project_home(&first, dir.path(), "harness: claude-code\n");
    let second = dir.path().join("b").join("repo");
    std::fs::create_dir_all(&second).unwrap();

    assert!(Project::open_at(first, home.clone()).is_ok());
    let err = Project::open_at(second, home).unwrap_err();
    assert!(matches!(err, ProjectError::ClaimedBy { .. }), "{err}");
    assert!(err.to_string().contains("rename"), "{err}");

    // And a file that names no repository at all is not taken as anyone's.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    let home = common::project_home(&root, dir.path(), "harness: claude-code\n");
    std::fs::write(home.join("nunki.yaml"), "harness: claude-code\n").unwrap();
    let err = Project::open_at(root, home).unwrap_err();
    assert!(matches!(err, ProjectError::Unclaimed { .. }), "{err}");
    assert!(err.to_string().contains("root:"), "{err}");
}

/// A project set up before the configuration left the repository is said to
/// be one, with the moves to make, and nothing is moved for the human.
#[test]
fn the_old_layout_is_named_with_the_moves_to_make() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    let home = common::project_home(&root, dir.path(), "harness: claude-code\n");

    // The configuration still in the tree.
    std::fs::rename(home.join("nunki.yaml"), root.join("nunki.yaml")).unwrap();
    let err = Project::open_at(root.clone(), home.clone()).unwrap_err();
    assert!(matches!(err, ProjectError::ConfigInTheTree { .. }), "{err}");
    assert!(err.to_string().contains(".nunki/stacks/"), "{err}");
    assert!(root.join("nunki.yaml").is_file(), "nothing was moved");

    // The HQ at the top of the home.
    std::fs::rename(root.join("nunki.yaml"), home.join("nunki.yaml")).unwrap();
    std::fs::remove_dir_all(home.join("hq")).unwrap();
    std::fs::create_dir_all(home.join("state")).unwrap();
    let err = Project::open_at(root, home.clone()).unwrap_err();
    assert!(matches!(err, ProjectError::FlatHq { .. }), "{err}");
    assert!(home.join("state").is_dir(), "nothing was moved");
}

#[test]
fn a_report_holding_a_violation_is_red_and_says_how_many() {
    let mut report = Report::default();
    report.checks.push(nunki::check::Check {
        what: "something".to_string(),
        verdict: Verdict::Green("fine".to_string()),
    });
    assert!(!report.is_red());

    report.checks.push(nunki::check::Check {
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
    // A project whose rules no harness would read: no AGENTS.md, no import.
    common::project_home(&root, dir.path(), "harness: claude-code\n");

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_nunki"))
        .env("HOME", dir.path())
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
    let elsewhere = tempfile::tempdir().unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_nunki"))
        .env("HOME", dir.path())
        .args(["-C"])
        .arg(elsewhere.path())
        .arg("check")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        out.stdout.is_empty(),
        "nothing was checked, so nothing is reported"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("git repository"),
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
    let root = std::fs::canonicalize(env!("CARGO_MANIFEST_DIR")).unwrap();
    let home = tempfile::tempdir().unwrap();
    // This repository's configuration is not in it, like any project's: the
    // test gives it one, in its own home.
    let project_home = home.path().join(".nunki").join(root.file_name().unwrap());
    std::fs::create_dir_all(project_home.join("hq")).unwrap();
    std::fs::create_dir_all(project_home.join("stacks/rust")).unwrap();
    std::fs::write(
        project_home.join("nunki.yaml"),
        format!(
            "root: {}\n\
             harness: claude-code\n\
             forge: [github.com]\n\
             stacks: [rust]\n\
             protected_branches: [main, dev]\n\
             forge_protection: forge\n",
            root.display()
        ),
    )
    .unwrap();
    std::fs::write(
        project_home.join("stacks/rust/allow.txt"),
        "static.crates.io\nindex.crates.io\ncrates.io\n",
    )
    .unwrap();

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_nunki"))
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
    // And the forge is part of the verb, not only of the library. This
    // repository lets the forge hold the rule, and a home of its own has no
    // credential to ask it with: every protected branch is named as not
    // checked, and why — never read off the forge from a test, never red.
    assert!(text.contains("main is protected on the forge"), "{text}");
    assert!(text.contains("no forge credential"), "{text}");
}

#[test]
fn the_home_is_the_projects_own_directory_under_nunkis() {
    let root = PathBuf::from("/somewhere/some-project");
    let user = PathBuf::from(std::env::var("HOME").unwrap());
    let home = Project::home_for(&root).unwrap();
    assert_eq!(home, user.join(".nunki").join("some-project"));

    // And the HQ is a directory of that home, never its top.
    let project = Project::at(root, config(), home.clone());
    assert_eq!(project.hq_root, home.join("hq"));
    assert_eq!(project.nunki_home(), user.join(".nunki"));
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
    use nunki::account::{Account, Accounts};

    let dir = tempfile::tempdir().unwrap();
    let project = sound(dir.path());
    let nunki_home = project.nunki_home();

    // Nothing declared: something to do, not something broken.
    let v = verdict(&run(&project), "can authenticate");
    assert!(matches!(v, Verdict::NotChecked(_)), "{v:?}");

    let write_index = |index: &Accounts| {
        std::fs::write(
            nunki_home.join("accounts.yaml"),
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
    std::fs::create_dir_all(nunki_home.join("accounts")).unwrap();
    let token = nunki_home.join("accounts/perso");
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

// --- the forge's own protection (SPEC 4.1 bis) ------------------------------

/// A real repository whose `origin` reads as GitHub, protecting two branches.
fn on_github(dir: &Path, remote: &str) -> Project {
    let root = dir.join("repo");
    let home = dir.join("nunki");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(home.join(nunki::project::HQ_DIR)).unwrap();
    for args in [vec!["init", "-q"], vec!["remote", "add", "origin", remote]] {
        assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(&args)
                .status()
                .unwrap()
                .success()
        );
    }
    Project::at(
        root,
        Config {
            protected_branches: vec!["main".to_string(), "dev".to_string()],
            ..config()
        },
        home,
    )
}

fn with_token(project: &Project) {
    std::fs::write(
        project.hq_root.join(nunki::forge::TOKEN_FILE),
        "tok-human\n",
    )
    .unwrap();
}

fn forge_verdicts(report: &Report) -> Vec<(String, Verdict)> {
    report
        .checks
        .iter()
        .filter(|c| c.what.ends_with("is protected on the forge"))
        .map(|c| (c.what.clone(), c.verdict.clone()))
        .collect()
}

/// A protected branch is green, an unprotected one is red — the forge would
/// take the push, and gate 2 is left alone — and each question goes to the
/// branch itself, with the human's token.
#[test]
fn the_forge_is_asked_branch_by_branch_and_an_unprotected_one_is_red() {
    let dir = tempfile::tempdir().unwrap();
    let project = on_github(dir.path(), "https://github.com/o/r.git");
    with_token(&project);
    let (api, server) = serve(vec![
        (200, r#"{"name":"main","protected":true}"#),
        (200, r#"{"name":"dev","protected":false}"#),
    ]);

    let mut report = Report::default();
    nunki::check::forge_protection(&project, &api, &mut report);
    let verdicts = forge_verdicts(&report);

    assert!(matches!(&verdicts[0].1, Verdict::Green(_)), "{verdicts:?}");
    match &verdicts[1].1 {
        Verdict::Red(why) => assert!(why.contains("gate 2"), "{why}"),
        other => panic!("{other:?}"),
    }
    assert!(report.is_red());

    let seen = server.join().unwrap();
    assert!(
        seen[0].starts_with("GET /repos/o/r/branches/main "),
        "{}",
        seen[0]
    );
    assert!(
        seen[1].starts_with("GET /repos/o/r/branches/dev "),
        "{}",
        seen[1]
    );
    assert!(
        seen.iter().all(|r| r
            .to_ascii_lowercase()
            .contains("authorization: bearer tok-human")),
        "{seen:?}"
    );
}

/// Without the human's credential the forge is not asked — and the check
/// says so for every branch rather than passing over them in silence.
#[test]
fn without_a_credential_every_branch_is_named_as_not_checked() {
    let dir = tempfile::tempdir().unwrap();
    let project = on_github(dir.path(), "https://github.com/o/r.git");

    let mut report = Report::default();
    nunki::check::forge_protection(&project, "http://127.0.0.1:9", &mut report);
    let verdicts = forge_verdicts(&report);

    assert_eq!(verdicts.len(), 2, "{verdicts:?}");
    for (_, verdict) in &verdicts {
        match verdict {
            Verdict::NotChecked(why) => assert!(why.contains(nunki::forge::TOKEN_FILE), "{why}"),
            other => panic!("{other:?}"),
        }
    }
    assert!(!report.is_red());
}

/// Measured on GitHub: a missing branch answers 404 "Branch not found", and a
/// repository the token cannot see answers 404 "Not Found". The first is
/// about the branch; the second is the forge declining to say — and reading
/// it as either green or red would be inventing an answer.
#[test]
fn a_404_is_an_absent_branch_only_when_the_forge_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let project = on_github(dir.path(), "https://github.com/o/r.git");
    with_token(&project);
    let (api, server) = serve(vec![
        (404, r#"{"message":"Branch not found","status":"404"}"#),
        (404, r#"{"message":"Not Found","status":"404"}"#),
    ]);

    let mut report = Report::default();
    nunki::check::forge_protection(&project, &api, &mut report);
    server.join().unwrap();
    let verdicts = forge_verdicts(&report);

    match &verdicts[0].1 {
        Verdict::NotChecked(why) => assert!(why.contains("does not exist"), "{why}"),
        other => panic!("{other:?}"),
    }
    match &verdicts[1].1 {
        Verdict::NotChecked(why) => {
            assert!(why.contains("404") && why.contains("Not Found"), "{why}");
            assert!(!why.contains("does not exist"), "{why}");
        }
        other => panic!("{other:?}"),
    }
    assert!(!report.is_red());
}

/// `by_hand` asks nothing, even with a credential on a GitHub remote: every
/// branch is named as held by hand, and the check is not red. The api here
/// is unreachable, so a forge actually asked would answer "the forge did not
/// say" instead.
#[test]
fn by_hand_asks_no_forge_and_names_every_branch_as_held_by_hand() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = on_github(dir.path(), "https://github.com/o/r.git");
    project.config.forge_protection = nunki::project::ForgeProtection::ByHand;
    with_token(&project);

    let mut report = Report::default();
    nunki::check::forge_protection(&project, "http://127.0.0.1:9", &mut report);
    let verdicts = forge_verdicts(&report);

    assert_eq!(verdicts.len(), 2, "{verdicts:?}");
    for (_, verdict) in &verdicts {
        match verdict {
            Verdict::NotChecked(why) => assert!(why.contains("by_hand"), "{why}"),
            other => panic!("{other:?}"),
        }
    }
    assert!(!report.is_red());
}

/// The field reads from `nunki.yaml` as written there, and is `forge` when
/// absent — the default asks the forge.
#[test]
fn forge_protection_reads_from_the_file_and_defaults_to_the_forge() {
    use nunki::project::ForgeProtection;
    let base = "harness: claude-code\n";
    let absent: Config = serde_yaml_ng::from_str(base).unwrap();
    assert_eq!(absent.forge_protection, ForgeProtection::Forge);
    let by_hand: Config =
        serde_yaml_ng::from_str(&format!("{base}forge_protection: by_hand\n")).unwrap();
    assert_eq!(by_hand.forge_protection, ForgeProtection::ByHand);
}

/// A remote off GitHub is named as such, and no forge is asked.
#[test]
fn a_remote_off_github_is_not_asked_and_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let project = on_github(dir.path(), "git@gitlab.com:team/thing.git");
    with_token(&project);

    let mut report = Report::default();
    nunki::check::forge_protection(&project, "http://127.0.0.1:9", &mut report);
    for (_, verdict) in forge_verdicts(&report) {
        match verdict {
            Verdict::NotChecked(why) => assert!(why.contains("not on GitHub"), "{why}"),
            other => panic!("{other:?}"),
        }
    }
}
