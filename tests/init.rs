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

/// The rules an agent reads have to name what the gates actually refuse.
///
/// Measured on 2026-09-13, on the first mission `hq` ran end to end: the
/// coder was never told that gate 3 wants `HEAD` named in the resume block,
/// nor that gate 5 wants `PR.md` written — and it lost one attempt to each,
/// on rules it had no way to learn. A gate that enforces what nothing
/// states is a gate that grades an agent on a secret.
#[test]
fn the_rules_it_writes_name_what_the_gates_require() {
    let (_d, root, hq) = fresh();
    init(&root, &hq, &["rust".to_string()]).unwrap();
    let rules = std::fs::read_to_string(root.join("AGENTS.md")).unwrap();

    assert!(
        rules.contains("names the commit it describes"),
        "gate 3 refuses a resume block that does not carry HEAD: {rules}"
    );
    assert!(
        rules.contains("an empty one is a red gate"),
        "gate 5 asks for PR.md, so the rules have to ask for it: {rules}"
    );
    assert!(
        rules.contains("**inside** the block"),
        "hq reads the block and not the whole file, so a line further down is \
         one it never sees: {rules}"
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

#[test]
fn a_claude_md_that_is_a_link_to_agents_md_is_recognised() {
    // The other documented shape (SPEC 4.1). Reading through the link would
    // ask whether AGENTS.md mentions its own name, which is not the question
    // — and `init` got this wrong on nunki itself before this test.
    let (_d, root, hq) = fresh();
    std::fs::write(root.join("AGENTS.md"), "# rules with no self-reference\n").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("AGENTS.md", root.join("CLAUDE.md")).unwrap();

    let actions = init(&root, &hq, &[]).unwrap();
    let why = kept(&actions, "CLAUDE.md").expect("CLAUDE.md should be reported");
    assert!(why.contains("link to AGENTS.md"), "{why}");
    assert!(!why.contains("red"), "nothing is wrong here: {why}");
    assert!(root.join("CLAUDE.md").is_symlink(), "the link survived");
}

/// The three things that judge and fence an agent — the battery gate 6
/// replays, the allowlist the firewall is built from, and the image it runs
/// in — all live under `.hq/`, and none of them is that agent's to rewrite.
/// A project that starts with an empty refusal list starts with a gate an
/// agent can weaken in one commit.
#[test]
fn a_new_project_protects_the_fragments_that_gate_and_fence_its_agents() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init(&repo, &dir.path().join("hq"), &["rust".to_string()]).unwrap();

    let text = std::fs::read_to_string(repo.join("hq.yaml")).unwrap();
    let config: hq::project::Config = serde_yaml_ng::from_str(&text).unwrap();
    assert!(
        config.protected_paths.refuse.iter().any(|p| p == ".hq/**"),
        "{text}"
    );
    // And the battery it protects is really there.
    assert!(repo.join(".hq/stacks/rust/prepush.sh").is_file());
}

/// What `hq init` writes and what `hq` reads back must agree.
///
/// `permission_mode` ships **uncommented**, unlike `model`: it is the one
/// line in the template whose value every run of a new project depends on
/// from the first minute. A template that said `auto` while the field parsed
/// as something else would be invisible until an agent behaved oddly in a
/// container nobody is watching.
#[test]
fn the_hq_yaml_it_writes_parses_back_with_the_permission_mode_it_declares() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init(&repo, &dir.path().join("hq"), &["rust".to_string()]).unwrap();

    let text = std::fs::read_to_string(repo.join("hq.yaml")).unwrap();
    let config: hq::project::Config = serde_yaml_ng::from_str(&text).unwrap();
    assert_eq!(config.permission_mode, "auto", "{text}");
    // The model ships commented out, so a new project keeps the harness's
    // own default until somebody declares one.
    assert_eq!(config.model, None, "{text}");
}

/// The prose `hq init` deposits must read as prose.
///
/// The cause, measured on 2026-09-10: `cargo fmt` joins a `\`-continued
/// string literal onto one line and keeps the continuation's indentation as
/// **real spaces**. A template written to read nicely in the source arrives
/// on disk with nine-space indents, and Markdown renders those as a code
/// block. `FOLLOWUP_HQ.md` — the file an agent is told to read first — was
/// shipped that way for a fortnight.
///
/// Markdown and YAML only, on purpose: the Dockerfile and the shell scripts
/// indent continuation lines because that is how those languages read, and
/// they are raw strings in the source, which `cargo fmt` does not touch. It
/// is the prose that is at risk, and it is the prose this guards.
#[test]
fn the_prose_init_writes_reads_as_prose() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    hq::init::init(&root, &dir.path().join("hq"), &["rust".to_string()]).unwrap();

    let mut seen = 0;
    let mut stack: Vec<std::path::PathBuf> = vec![root.clone()];
    while let Some(at) = stack.pop() {
        for entry in std::fs::read_dir(&at).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let prose = path
                .extension()
                .is_some_and(|e| e == "md" || e == "yaml" || e == "yml" || e == "txt");
            if !prose {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            seen += 1;
            for (n, line) in text.lines().enumerate() {
                // A `\` continuation carries the source's own indentation,
                // which in this crate is eight spaces or more.
                assert!(
                    !line.contains("     "),
                    "{}:{}: a run of spaces inside a line — a `\\`-continued \
                     literal joined by cargo fmt?\n{line:?}",
                    path.display(),
                    n + 1
                );
                // And Markdown's own reading of four: a code block, whatever
                // the sentence in it says. Markdown only — YAML nests four
                // spaces deep because that is what YAML is.
                let markdown = path.extension().is_some_and(|e| e == "md");
                assert!(
                    !(markdown && line.starts_with("    ") && !line.trim().is_empty()),
                    "{}:{}: this renders as a code block\n{line:?}",
                    path.display(),
                    n + 1
                );
            }
        }
    }
    assert!(seen >= 4, "only {seen} prose files were read back");
}

/// The stack fragment ships the campaign gate 7 plays, and the image that can
/// run it. Measured on 2026-09-10: `mutation.sh` calls `cargo mutants`, and
/// nothing installed it — so every campaign would have died on a command that
/// is not there, in a log written by a detached process nobody was reading.
#[test]
fn the_rust_image_carries_what_the_mutation_campaign_calls() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    hq::init::init(&root, &dir.path().join("hq"), &["rust".to_string()]).unwrap();

    let script = std::fs::read_to_string(root.join(".hq/stacks/rust/mutation.sh")).unwrap();
    assert!(script.contains("cargo mutants"), "{script}");
    let dockerfile = std::fs::read_to_string(root.join(".hq/stacks/rust/Dockerfile")).unwrap();
    assert!(
        dockerfile.contains("cargo install cargo-mutants"),
        "the campaign calls a command the image does not carry:\n{dockerfile}"
    );
    // As the agent, after `USER agent`: installed as root it would land where
    // the only user that runs it cannot read (SPEC 4.2 bis).
    let user = dockerfile
        .rfind("USER ")
        .expect("the image drops to a user");
    let install = dockerfile
        .find("cargo install cargo-mutants")
        .expect("checked above");
    assert!(user < install, "{dockerfile}");
}
