//! `hq init` (SPEC 4.2, 3.3): creates what is absent, never overwrites what
//! a human edits, keeps no manifest, and can be run again.

use std::path::Path;

use hq::init::{Action, InitError, KNOWN_STACKS, init};

fn created(actions: &[Action], name: &str) -> bool {
    actions
        .iter()
        .any(|a| matches!(a, Action::Created(p) if p.ends_with(name)))
}

fn kept(actions: &[Action], name: &str) -> Option<String> {
    actions.iter().find_map(|a| match a {
        Action::LeftAlone(p, why) if p.ends_with(name) => Some(why.clone()),
        _ => None,
    })
}

fn fresh() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    let hq = dir.path().join("hq");
    std::fs::create_dir_all(&root).unwrap();
    (dir, root, hq)
}

#[test]
fn a_fresh_repository_gets_everything_it_needs() {
    let (_d, root, hq) = fresh();
    let actions = init(&root, &hq, &["rust".to_string()]).unwrap();

    for name in ["hq.yaml", "AGENTS.md", "CLAUDE.md", ".gitattributes"] {
        assert!(created(&actions, name), "{name} missing from {actions:?}");
        assert!(root.join(name).exists());
    }
    // The HQ, outside the tree, is where everything hq owns lives.
    for dir in ["state", "locks", "missions"] {
        assert!(hq.join(dir).is_dir(), "{dir} missing");
    }
    // A stack fragment that says something rather than an empty directory.
    let allow = std::fs::read_to_string(root.join(".hq/stacks/rust/allow.txt")).unwrap();
    assert!(allow.contains("index.crates.io"), "{allow}");
    assert!(
        !allow.contains("github.com"),
        "a fragment must not carry a forge domain: {allow}"
    );
    assert!(root.join(".hq/stacks/rust/prepush.sh").exists());
    // CLAUDE.md must import AGENTS.md or Claude Code never reads the rules.
    assert_eq!(
        std::fs::read_to_string(root.join("CLAUDE.md")).unwrap(),
        "@AGENTS.md\n"
    );
}

#[test]
fn the_battery_is_executable_or_nothing_can_run_it() {
    let (_d, root, hq) = fresh();
    init(&root, &hq, &["rust".to_string()]).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(root.join(".hq/stacks/rust/prepush.sh"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "{mode:o}");
    }
}

#[test]
fn running_it_again_changes_nothing() {
    let (_d, root, hq) = fresh();
    init(&root, &hq, &["rust".to_string()]).unwrap();
    let before = snapshot(&root);

    let second = init(&root, &hq, &["rust".to_string()]).unwrap();
    assert_eq!(snapshot(&root), before, "the second run rewrote something");
    assert!(
        second.iter().all(|a| !matches!(a, Action::Created(_))),
        "{second:?}"
    );
    assert!(kept(&second, "hq.yaml").is_some());
}

#[test]
fn a_file_a_human_edits_is_never_overwritten() {
    let (_d, root, hq) = fresh();
    std::fs::write(root.join("AGENTS.md"), "mine, and better\n").unwrap();
    std::fs::write(root.join("hq.yaml"), "harness: claude-code\n").unwrap();

    init(&root, &hq, &[]).unwrap();
    assert_eq!(
        std::fs::read_to_string(root.join("AGENTS.md")).unwrap(),
        "mine, and better\n"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("hq.yaml")).unwrap(),
        "harness: claude-code\n"
    );
}

#[test]
fn an_existing_claude_md_is_kept_and_the_missing_import_is_named() {
    let (_d, root, hq) = fresh();
    std::fs::write(root.join("CLAUDE.md"), "# my own instructions\n").unwrap();
    let actions = init(&root, &hq, &[]).unwrap();

    assert_eq!(
        std::fs::read_to_string(root.join("CLAUDE.md")).unwrap(),
        "# my own instructions\n",
        "hq must not rewrite it"
    );
    let why = kept(&actions, "CLAUDE.md").expect("CLAUDE.md should be reported");
    assert!(why.contains("@AGENTS.md"), "{why}");
    assert!(
        why.contains("red"),
        "the human is told check will fail: {why}"
    );

    // And one that already imports is simply fine.
    std::fs::write(root.join("CLAUDE.md"), "@AGENTS.md\nplus my own\n").unwrap();
    let actions = init(&root, &hq, &[]).unwrap();
    let why = kept(&actions, "CLAUDE.md").unwrap();
    assert!(why.contains("already imports"), "{why}");
}

#[test]
fn a_gitattributes_that_does_not_pin_lf_gets_a_suggestion_beside_it() {
    let (_d, root, hq) = fresh();
    std::fs::write(root.join(".gitattributes"), "*.png binary\n").unwrap();
    let actions = init(&root, &hq, &[]).unwrap();

    assert_eq!(
        std::fs::read_to_string(root.join(".gitattributes")).unwrap(),
        "*.png binary\n",
        "hq does not decide a project's git configuration"
    );
    let suggestion = root.join(".gitattributes.hq");
    assert!(suggestion.exists(), "{actions:?}");
    assert!(
        std::fs::read_to_string(&suggestion)
            .unwrap()
            .contains("eol=lf"),
        "the suggestion is the thing it could not write"
    );
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, Action::DepositedBeside { .. })),
        "{actions:?}"
    );
}

#[test]
fn an_unknown_stack_is_refused_rather_than_left_as_an_empty_directory() {
    let (_d, root, hq) = fresh();
    let err = init(&root, &hq, &["cobol".to_string()]).unwrap_err();
    assert!(matches!(err, InitError::UnknownStack(..)), "{err}");
    assert!(err.to_string().contains(KNOWN_STACKS[0]), "{err}");
    assert!(
        !root.join(".hq").exists(),
        "nothing should have been written"
    );
}

#[test]
fn the_generated_config_is_readable_by_the_reader() {
    let (_d, root, hq) = fresh();
    init(&root, &hq, &["rust".to_string()]).unwrap();
    // What init writes, `Project::open` must accept — otherwise the first
    // verb after `init` fails on the file `init` just produced.
    let project = hq::project::Project::open(&root).unwrap();
    assert_eq!(project.config.harness, "claude-code");
    assert_eq!(project.config.stacks, vec!["rust".to_string()]);
    assert!(project.config.forge.is_empty(), "the human fills that in");
    assert!(
        project
            .config
            .protected_branches
            .contains(&"main".to_string())
    );
}

fn snapshot(root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, out);
            } else {
                out.push((
                    path.strip_prefix(root).unwrap().display().to_string(),
                    std::fs::read_to_string(&path).unwrap_or_default(),
                ));
            }
        }
    }
    walk(root, root, &mut out);
    out.sort();
    out
}

/// The two verbs in the order a human runs them, through the binary: an
/// ordinary repository becomes one `hq check` passes.
#[test]
fn init_then_check_is_green_on_a_repository_that_had_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("demo");
    let home = dir.path().join("home");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .arg(&root)
            .status()
            .unwrap()
            .success()
    );

    let hq = env!("CARGO_BIN_EXE_hq");
    let init = std::process::Command::new(hq)
        .env("HOME", &home)
        .args(["-C"])
        .arg(&root)
        .args(["init", "--stack", "rust"])
        .output()
        .unwrap();
    assert!(
        init.status.success(),
        "{:?}",
        String::from_utf8_lossy(&init.stderr)
    );

    let check = std::process::Command::new(hq)
        .env("HOME", &home)
        .args(["-C"])
        .arg(&root)
        .arg("check")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&check.stdout);
    assert_eq!(check.status.code(), Some(0), "{text}");
    assert!(
        text.contains("the rules of the place reach the harness"),
        "{text}"
    );
    // And it still says what it could not establish.
    assert!(text.contains("could not be checked"), "{text}");
}
