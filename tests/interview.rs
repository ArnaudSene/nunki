//! `nunki init`, asked (SPEC 4.2, `nunki init`): what is proposed from the
//! repository, what a human's answers become, and that nothing is written
//! until they say so.

use std::path::Path;
use std::process::Command;

use nunki::interview::{Detected, Given, InterviewError, Proposed, Scripted, detect, interview};
use nunki::project::{ForgeProtection, Project, Stack};

fn git(at: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(at)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

fn write(root: &Path, path: &str, body: &str) {
    let file = root.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, body).unwrap();
}

/// A repository laid out like a real polyglot project, and then some: a
/// Rust workspace at the root with member crates, a Next.js application in
/// `frontend/`, a Python service in `api/`, and the places a manifest lives
/// without being a stack.
fn repository(dir: &Path) -> std::path::PathBuf {
    let root = dir.join("repo");
    write(&root, "Cargo.toml", "[workspace]\n");
    write(
        &root,
        "crates/core/Cargo.toml",
        "[package]\nname = \"core\"\n",
    );
    write(
        &root,
        "frontend/package.json",
        r#"{"dependencies": {"next": "16.0.0"}}"#,
    );
    write(&root, "api/pyproject.toml", "[project]\nname = \"api\"\n");
    // A Node package that is not a Next.js application.
    write(
        &root,
        "tools/package.json",
        r#"{"devDependencies": {"prettier": "3"}}"#,
    );
    // Dependencies and vendored code: never a stack.
    write(
        &root,
        "frontend/node_modules/next/package.json",
        r#"{"dependencies": {"next": "1"}}"#,
    );
    write(
        &root,
        "vendor/ibapi/Cargo.toml",
        "[package]\nname = \"ibapi\"\n",
    );
    write(
        &root,
        "node_modules/some-app/package.json",
        r#"{"dependencies": {"next": "1"}}"#,
    );
    write(
        &root,
        "vendor/pylib/pyproject.toml",
        "[project]\nname = \"pylib\"\n",
    );
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "first"]);
    git(&root, &["branch", "dev"]);
    git(
        &root,
        &["remote", "add", "origin", "git@github.com:acme/x.git"],
    );
    root
}

#[test]
fn the_repository_proposes_its_stacks_its_forge_and_its_branches() {
    let dir = tempfile::tempdir().unwrap();
    let d = detect(&repository(dir.path()));
    let stacks: Vec<(String, String)> = d
        .stacks
        .iter()
        .map(|(s, m)| (s.to_string(), m.clone()))
        .collect();
    assert_eq!(
        stacks,
        [
            ("rust".into(), "Cargo.toml".into()),
            ("python in api/".into(), "api/pyproject.toml".into()),
            ("next in frontend/".into(), "frontend/package.json".into()),
        ]
    );
    assert_eq!(d.forge.as_deref(), Some("github.com"));
    assert_eq!(d.branches, ["main", "dev"]);
}

/// Pressing Enter at every question writes what was proposed, and the file
/// written is one nunki reads back as it was answered.
#[test]
fn enter_at_every_question_writes_what_the_repository_proposed() {
    let dir = tempfile::tempdir().unwrap();
    let root = repository(dir.path());
    let mut human = Scripted::new(&["", "", "", "", "", "y"]);
    let (stacks, answers) = interview(&detect(&root), &Given::default(), &mut human).unwrap();
    assert_eq!(stacks, ["rust", "python=api", "next=frontend"]);

    let home = dir.path().join("home");
    nunki::init::init_with(&root, &home, &stacks, &answers).unwrap();
    let p = Project::open_at(root, home).unwrap();
    assert_eq!(
        p.config.stacks,
        [
            Stack::root("rust"),
            Stack::new("python", "api").unwrap(),
            Stack::new("next", "frontend").unwrap()
        ]
    );
    assert_eq!(p.config.forge, ["github.com"]);
    assert_eq!(p.config.forge_protection, ForgeProtection::Forge);
    assert_eq!(p.config.permission_mode, "auto");
    assert_eq!(p.config.protected_branches, ["main", "dev"]);
}

/// What the human types wins over what was proposed, all the way into the
/// file.
#[test]
fn a_human_answer_replaces_the_proposal() {
    let dir = tempfile::tempdir().unwrap();
    let root = repository(dir.path());
    let mut human = Scripted::new(&[
        "next=frontend rust",
        "github.com, gitlab.example.org",
        "by_hand",
        "dontAsk",
        "main",
        "y",
    ]);
    let (stacks, answers) = interview(&detect(&root), &Given::default(), &mut human).unwrap();
    assert_eq!(stacks, ["next=frontend", "rust"]);
    let home = dir.path().join("home");
    nunki::init::init_with(&root, &home, &stacks, &answers).unwrap();
    let p = Project::open_at(root, home).unwrap();
    assert_eq!(p.config.stacks[0], Stack::new("next", "frontend").unwrap());
    assert_eq!(p.config.forge, ["github.com", "gitlab.example.org"]);
    assert_eq!(p.config.forge_protection, ForgeProtection::ByHand);
    assert_eq!(p.config.permission_mode, "dontAsk");
    assert_eq!(p.config.protected_branches, ["main"]);
}

/// A flag answers its question in advance: it is not asked again.
#[test]
fn a_flag_is_a_question_already_answered() {
    let dir = tempfile::tempdir().unwrap();
    let root = repository(dir.path());
    let given = Given {
        forge_protection: Some(ForgeProtection::ByHand),
        permission_mode: Some("dontAsk".into()),
        ..Given::default()
    };
    let mut human = Scripted::new(&["", "", "", "y"]);
    let (_, answers) = interview(&detect(&root), &given, &mut human).unwrap();
    assert_eq!(answers.forge_protection, ForgeProtection::ByHand);
    assert!(
        !human
            .heard
            .iter()
            .any(|q| q.contains("forge/by_hand") || q.contains("dontAsk [")),
        "{:#?}",
        human.heard
    );
}

/// An answer that does not read is asked again; three, and the interview
/// stops without a file.
#[test]
fn an_answer_that_does_not_read_is_asked_again_then_refused() {
    let dir = tempfile::tempdir().unwrap();
    let root = repository(dir.path());
    let mut human = Scripted::new(&["rust, go", "rust=../up", "", "", "", "", "", "y"]);
    let (stacks, _) = interview(&detect(&root), &Given::default(), &mut human).unwrap();
    assert_eq!(stacks, ["rust", "python=api", "next=frontend"]);
    assert!(
        human
            .heard
            .iter()
            .any(|h| h.contains("go is not a stack nunki knows"))
    );

    let mut stubborn = Scripted::new(&["go", "go", "go"]);
    let err = interview(&detect(&root), &Given::default(), &mut stubborn).unwrap_err();
    assert!(matches!(err, InterviewError::Refused { .. }), "{err}");
}

/// "n" at the end writes nothing.
#[test]
fn nothing_is_written_until_the_human_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let root = repository(dir.path());
    let mut human = Scripted::new(&["", "", "", "", "", "n"]);
    let err = interview(&detect(&root), &Given::default(), &mut human).unwrap_err();
    assert!(matches!(err, InterviewError::NotConfirmed), "{err}");
}

/// `--yes` takes every proposal, and says nothing it would need an answer to.
#[test]
fn yes_takes_every_proposal() {
    let dir = tempfile::tempdir().unwrap();
    let root = repository(dir.path());
    let (stacks, answers) = interview(&detect(&root), &Given::default(), &mut Proposed).unwrap();
    assert_eq!(stacks, ["rust", "python=api", "next=frontend"]);
    assert_eq!(answers.forge, ["github.com"]);

    // A repository with nothing to propose still gets a configuration: no
    // stack, no forge, the usual branches.
    let empty = Detected::default();
    let (stacks, answers) = interview(&empty, &Given::default(), &mut Proposed).unwrap();
    assert!(stacks.is_empty());
    assert!(answers.forge.is_empty());
    assert_eq!(answers.protected_branches, ["main", "dev"]);
}
