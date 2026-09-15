//! `nunki init` (SPEC 4.2, 3.3): creates what is absent, never overwrites what
//! a human edits, keeps no manifest, and can be run again.

use std::path::Path;

use nunki::init::{Action, InitError, KNOWN_STACKS, init};

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
    let nunki = dir.path().join("nunki");
    std::fs::create_dir_all(&root).unwrap();
    (dir, root, nunki)
}

/// The home every test here gives `init`: beside the repository, and never
/// the real `~/.nunki`.
fn home(root: &Path) -> std::path::PathBuf {
    root.with_file_name("nunki")
}

#[test]
fn a_fresh_repository_gets_everything_it_needs() {
    let (_d, root, nunki) = fresh();
    let actions = init(&root, &nunki, &["rust".to_string()]).unwrap();

    for name in ["AGENTS.md", "CLAUDE.md", ".gitattributes"] {
        assert!(created(&actions, name), "{name} missing from {actions:?}");
        assert!(root.join(name).exists());
    }
    // And nothing else: the configuration and the fragments are tooling, and
    // tooling is not the project's to carry in its history.
    let mut in_tree: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    in_tree.sort();
    assert_eq!(in_tree, [".gitattributes", "AGENTS.md", "CLAUDE.md"]);

    assert!(created(&actions, "nunki.yaml"), "{actions:?}");
    assert!(nunki.join("nunki.yaml").is_file());
    // The HQ, outside the tree, is where everything nunki owns lives.
    for dir in ["state", "locks", "missions"] {
        assert!(nunki.join("hq").join(dir).is_dir(), "{dir} missing");
    }
    // A stack fragment that says something rather than an empty directory.
    let allow = std::fs::read_to_string(home(&root).join("stacks/rust/allow.txt")).unwrap();
    assert!(allow.contains("index.crates.io"), "{allow}");
    assert!(
        !allow.contains("github.com"),
        "a fragment must not carry a forge domain: {allow}"
    );
    assert!(home(&root).join("stacks/rust/prepush.sh").exists());
    // CLAUDE.md must import AGENTS.md or Claude Code never reads the rules.
    assert_eq!(
        std::fs::read_to_string(root.join("CLAUDE.md")).unwrap(),
        "@AGENTS.md\n"
    );
}

/// The rules an agent reads have to name what the gates actually refuse.
///
/// Measured on 2026-09-13, on the first mission `nunki` ran end to end: the
/// coder was never told that gate 3 wants `HEAD` named in the resume block,
/// nor that gate 5 wants `PR.md` written — and it lost one attempt to each,
/// on rules it had no way to learn. A gate that enforces what nothing
/// states is a gate that grades an agent on a secret.
#[test]
fn the_rules_it_writes_name_what_the_gates_require() {
    let (_d, root, nunki) = fresh();
    init(&root, &nunki, &["rust".to_string()]).unwrap();
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
        "nunki reads the block and not the whole file, so a line further down is \
         one it never sees: {rules}"
    );
}

/// The image a project builds carries the security updates published since
/// its base tag was cut.
///
/// Measured on 2026-09-13, on a scan that went red: `libpcre2-8-0` 10.42-1,
/// carrying CVE-2026-86145 and CVE-2026-89161 — both HIGH, both with a fix
/// already published. That package arrives with `debian:bookworm-slim` and
/// is installed by no line of this Dockerfile, so nothing but an `upgrade`
/// moves it; pulling the base tag again does nothing while the tag itself
/// has not been rebuilt.
#[test]
fn the_image_it_writes_applies_the_security_updates_of_its_base() {
    let (_d, root, nunki) = fresh();
    init(&root, &nunki, &["rust".to_string()]).unwrap();
    let dockerfile = std::fs::read_to_string(home(&root).join("stacks/rust/Dockerfile")).unwrap();

    assert!(
        dockerfile.contains("apt-get -qq -y upgrade"),
        "a freshly built image would keep the vulnerable packages its base \
         tag was cut with: {dockerfile}"
    );

    // The order carries as much as the presence: an upgrade run against a
    // stale package list upgrades nothing while looking right, and one run
    // after the install leaves the just-installed packages behind.
    let update = dockerfile
        .find("apt-get -qq update")
        .expect("the image updates its package list");
    let upgrade = dockerfile.find("apt-get -qq -y upgrade").unwrap();
    let install = dockerfile
        .find("apt-get -qq install")
        .expect("the image installs the toolchain");
    assert!(
        update < upgrade && upgrade < install,
        "update, then upgrade, then install — in that order: {dockerfile}"
    );
}

#[test]
fn the_battery_is_executable_or_nothing_can_run_it() {
    let (_d, root, nunki) = fresh();
    init(&root, &nunki, &["rust".to_string()]).unwrap();
    #[cfg(unix)]
    for script in ["prepush.sh", nunki::gate::SYSTEM_BATTERY] {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(home(&root).join("stacks/rust").join(script))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "{script}: {mode:o}");
    }
}

/// The integrator's gate 6 runs the stack's `system.sh`, and a stack that
/// ships none holds that gate red for a reason no agent is told — found on
/// 2026-09-15, before the first integration mission was launched.
///
/// What it runs matters as much as its presence. A system test needs the
/// services, and neither the coder's battery nor CI has them: the script
/// runs those tests and only those, and the coder's battery never does.
#[test]
fn the_integrators_battery_runs_the_system_tests_and_only_those() {
    let (_d, root, nunki) = fresh();
    init(&root, &nunki, &["rust".to_string()]).unwrap();
    let stack = home(&root).join("stacks/rust");

    let script = std::fs::read_to_string(stack.join(nunki::gate::SYSTEM_BATTERY)).unwrap();
    let line = script
        .lines()
        .find(|l| l.trim_start().starts_with("cargo test"))
        .expect("the system battery calls cargo test");
    assert!(line.contains("-- --ignored"), "{line}");
    assert!(
        line.contains("--tests"),
        "doc-tests stay out: `--ignored` would compile an `ignore` block: {line}"
    );
    // The convention it relies on is written where the integrator reads it.
    assert!(
        script.contains(r#"#[ignore = "system test: needs the services"]"#),
        "{script}"
    );

    let prepush = std::fs::read_to_string(stack.join("prepush.sh")).unwrap();
    assert!(
        !prepush.contains("ignored"),
        "the coder's battery would run tests that need services it cannot reach:\n{prepush}"
    );
}

#[test]
fn running_it_again_changes_nothing() {
    let (_d, root, nunki) = fresh();
    init(&root, &nunki, &["rust".to_string()]).unwrap();
    let before = snapshot(&root);

    let home_before = snapshot(&nunki);

    let second = init(&root, &nunki, &["rust".to_string()]).unwrap();
    assert_eq!(snapshot(&root), before, "the second run rewrote something");
    assert_eq!(
        snapshot(&nunki),
        home_before,
        "the second run rewrote something in the home"
    );
    assert!(
        second.iter().all(|a| !matches!(a, Action::Created(_))),
        "{second:?}"
    );
    assert!(kept(&second, "nunki.yaml").is_some());
}

#[test]
fn a_file_a_human_edits_is_never_overwritten() {
    let (_d, root, nunki) = fresh();
    std::fs::write(root.join("AGENTS.md"), "mine, and better\n").unwrap();
    std::fs::create_dir_all(&nunki).unwrap();
    std::fs::write(nunki.join("nunki.yaml"), "harness: claude-code\n").unwrap();

    init(&root, &nunki, &[]).unwrap();
    assert_eq!(
        std::fs::read_to_string(root.join("AGENTS.md")).unwrap(),
        "mine, and better\n"
    );
    assert_eq!(
        std::fs::read_to_string(nunki.join("nunki.yaml")).unwrap(),
        "harness: claude-code\n"
    );
}

#[test]
fn an_existing_claude_md_is_kept_and_the_missing_import_is_named() {
    let (_d, root, nunki) = fresh();
    std::fs::write(root.join("CLAUDE.md"), "# my own instructions\n").unwrap();
    let actions = init(&root, &nunki, &[]).unwrap();

    assert_eq!(
        std::fs::read_to_string(root.join("CLAUDE.md")).unwrap(),
        "# my own instructions\n",
        "nunki must not rewrite it"
    );
    let why = kept(&actions, "CLAUDE.md").expect("CLAUDE.md should be reported");
    assert!(why.contains("@AGENTS.md"), "{why}");
    assert!(
        why.contains("red"),
        "the human is told check will fail: {why}"
    );

    // And one that already imports is simply fine.
    std::fs::write(root.join("CLAUDE.md"), "@AGENTS.md\nplus my own\n").unwrap();
    let actions = init(&root, &nunki, &[]).unwrap();
    let why = kept(&actions, "CLAUDE.md").unwrap();
    assert!(why.contains("already imports"), "{why}");
}

#[test]
fn a_gitattributes_that_does_not_pin_lf_gets_a_suggestion_beside_it() {
    let (_d, root, nunki) = fresh();
    std::fs::write(root.join(".gitattributes"), "*.png binary\n").unwrap();
    let actions = init(&root, &nunki, &[]).unwrap();

    assert_eq!(
        std::fs::read_to_string(root.join(".gitattributes")).unwrap(),
        "*.png binary\n",
        "nunki does not decide a project's git configuration"
    );
    let suggestion = root.join(".gitattributes.nunki");
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
    let (_d, root, nunki) = fresh();
    let err = init(&root, &nunki, &["cobol".to_string()]).unwrap_err();
    assert!(matches!(err, InitError::UnknownStack(..)), "{err}");
    assert!(err.to_string().contains(KNOWN_STACKS[0]), "{err}");
    assert!(
        !nunki.join("stacks").exists(),
        "nothing should have been written"
    );
}

#[test]
fn the_generated_config_is_readable_by_the_reader() {
    let (_d, root, nunki) = fresh();
    init(&root, &nunki, &["rust".to_string()]).unwrap();
    // What init writes, `Project::open` must accept — otherwise the first
    // verb after `init` fails on the file `init` just produced.
    let project = nunki::project::Project::open_at(root.clone(), nunki.clone()).unwrap();
    assert_eq!(project.config.root, Some(root));
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
/// ordinary repository becomes one `nunki check` passes.
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

    let nunki = env!("CARGO_BIN_EXE_nunki");
    let init = std::process::Command::new(nunki)
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

    let check = std::process::Command::new(nunki)
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
    let (_d, root, nunki) = fresh();
    std::fs::write(root.join("AGENTS.md"), "# rules with no self-reference\n").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("AGENTS.md", root.join("CLAUDE.md")).unwrap();

    let actions = init(&root, &nunki, &[]).unwrap();
    let why = kept(&actions, "CLAUDE.md").expect("CLAUDE.md should be reported");
    assert!(why.contains("link to AGENTS.md"), "{why}");
    assert!(!why.contains("red"), "nothing is wrong here: {why}");
    assert!(root.join("CLAUDE.md").is_symlink(), "the link survived");
}

/// The three things that judge and fence an agent — the battery gate 6
/// replays, the allowlist the firewall is built from, and the image it runs
/// in — live in the project's home, and none of them in the tree the agent
/// writes. Nothing has to be refused for them: they are out of reach.
#[test]
fn a_new_project_keeps_what_gates_and_fences_its_agents_out_of_the_tree() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init(&repo, &home(&repo), &["rust".to_string()]).unwrap();

    for name in ["prepush.sh", "allow.txt", "Dockerfile"] {
        assert!(
            home(&repo).join("stacks/rust").join(name).is_file(),
            "{name}"
        );
    }
    assert!(
        !repo.join(".nunki").exists(),
        "a fragment landed in the tree"
    );

    let text = std::fs::read_to_string(home(&repo).join("nunki.yaml")).unwrap();
    let config: nunki::project::Config = serde_yaml_ng::from_str(&text).unwrap();
    assert!(
        config.protected_paths.refuse.is_empty(),
        "nothing of nunki's is left in the tree to refuse: {text}"
    );
}

/// What `nunki init` writes and what `nunki` reads back must agree.
///
/// `permission_mode` ships **uncommented**, unlike `model`: it is the one
/// line in the template whose value every run of a new project depends on
/// from the first minute. A template that said `auto` while the field parsed
/// as something else would be invisible until an agent behaved oddly in a
/// container nobody is watching.
#[test]
fn the_nunki_yaml_it_writes_parses_back_with_the_permission_mode_it_declares() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init(&repo, &home(&repo), &["rust".to_string()]).unwrap();

    let text = std::fs::read_to_string(home(&repo).join("nunki.yaml")).unwrap();
    let config: nunki::project::Config = serde_yaml_ng::from_str(&text).unwrap();
    assert_eq!(config.permission_mode, "auto", "{text}");
    // And it names the repository it belongs to.
    assert_eq!(config.root, Some(repo), "{text}");
    // The model ships commented out, so a new project keeps the harness's
    // own default until somebody declares one.
    assert_eq!(config.model, None, "{text}");
}

/// The prose `nunki init` deposits must read as prose.
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
    nunki::init::init(&root, &dir.path().join("nunki"), &["rust".to_string()]).unwrap();

    let mut seen = 0;
    let mut stack: Vec<std::path::PathBuf> = vec![root.clone(), home(&root)];
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
    nunki::init::init(&root, &dir.path().join("nunki"), &["rust".to_string()]).unwrap();

    let script = std::fs::read_to_string(home(&root).join("stacks/rust/mutation.sh")).unwrap();
    assert!(script.contains("cargo mutants"), "{script}");
    let dockerfile = std::fs::read_to_string(home(&root).join("stacks/rust/Dockerfile")).unwrap();
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

/// The campaign does not hand the coder a survivor nobody could answer.
///
/// cargo-mutants replaces a whole function body with `Default::default()`
/// whenever the return type allows it, and `fn main() -> ExitCode` always
/// allows it. No unit test calls `main`, so that mutant cannot be killed by
/// any test the coder is able to write, and gate 7 asks for every survivor to
/// be killed or frozen as a bug. Measured on 2026-09-13: a coder spent a run
/// extracting `main`'s body into a testable function, and the mutant
/// reappeared on the thin wrapper that was left.
///
/// Measured the same day, on a crate with a binary and a library: 19 mutants
/// without the exclusion, 18 with it — it removes `src/main.rs`'s whole body
/// and keeps `src/lib.rs`'s `replace run -> ExitCode`, the same shape in the
/// function `main` delegates to, which a test can and must kill.
/// The image carries what the **battery** calls, not only what the campaign
/// does.
///
/// `prepush.sh` runs `cargo deny check`. An image that does not install
/// `cargo-deny` fails gate 6 with `no such command: deny` — and no agent can
/// repair it, because the image is built from a Dockerfile no agent can
/// reach. Measured on 2026-09-14, on the first mission the renamed tool drove:
/// the coder diagnosed the hole correctly on all three attempts, said each
/// time that it was outside its permission, and the mission was handed over
/// with both its lots built and committed. Three runs to be told what the
/// generator could have said once.
#[test]
fn the_rust_image_carries_what_the_battery_calls() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    nunki::init::init(&root, &dir.path().join("nunki"), &["rust".to_string()]).unwrap();

    let battery = std::fs::read_to_string(home(&root).join("stacks/rust/prepush.sh")).unwrap();
    assert!(battery.contains("cargo deny check"), "{battery}");
    let dockerfile = std::fs::read_to_string(home(&root).join("stacks/rust/Dockerfile")).unwrap();
    assert!(
        dockerfile.contains("cargo install cargo-deny"),
        "the battery calls a command the image does not carry:\n{dockerfile}"
    );
    // As the agent, after `USER agent`: installed as root it would land where
    // the only user that runs it cannot read (SPEC 4.2 bis).
    let user = dockerfile
        .rfind("USER ")
        .expect("the image drops to a user");
    let install = dockerfile
        .find("cargo install cargo-deny")
        .expect("checked above");
    assert!(user < install, "{dockerfile}");
}

/// The battery asks only for what the coder's container can reach.
///
/// `cargo deny check` runs four stages, and `advisories` fetches its database
/// from github.com. The coder's allowlist names no forge (SPEC 4.1 bis, rule
/// 6), so that stage can never succeed in the container — not for want of a
/// network, but by design.
///
/// Measured on 2026-09-14, in the image this generator writes: with
/// `cargo-deny` installed the battery still came back non-zero on
/// `failed to fetch advisory database … Could not resolve host: github.com`,
/// while `bans licenses sources` alone passed with `bans ok, licenses ok,
/// sources ok`. Asking for the whole thing holds gate 6 red for a reason no
/// agent can repair — the same shape as the missing command it replaced.
#[test]
fn the_battery_asks_only_for_what_the_container_can_reach() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    nunki::init::init(&root, &dir.path().join("nunki"), &["rust".to_string()]).unwrap();

    let battery = std::fs::read_to_string(home(&root).join("stacks/rust/prepush.sh")).unwrap();
    assert!(
        battery.contains("cargo deny check bans licenses sources"),
        "{battery}"
    );
    // Never the bare form: it drags the forge-bound stage in. The check is on
    // the line, not the substring — the corrected form contains the bare one.
    assert!(
        !battery.lines().any(|l| l.trim() == "cargo deny check"),
        "the battery asks for a stage the container cannot reach:\n{battery}"
    );

    // And this holds only because the allowlist keeps the forge out: if a
    // forge domain were ever added, both this test and `nunki check` would
    // have something to say.
    let allow = std::fs::read_to_string(home(&root).join("stacks/rust/allow.txt")).unwrap();
    assert!(!allow.contains("github.com"), "{allow}");
}

#[test]
fn the_campaign_does_not_mutate_a_binarys_entry_point() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    nunki::init::init(&root, &dir.path().join("nunki"), &["rust".to_string()]).unwrap();

    let script = std::fs::read_to_string(home(&root).join("stacks/rust/mutation.sh")).unwrap();
    let line = script
        .lines()
        .find(|l| l.trim_start().starts_with("cargo mutants"))
        .expect("the campaign calls cargo mutants");
    assert!(
        line.contains(r#"--exclude-re "replace main -> ""#),
        "the campaign it writes still mutates an entry point no test can \
         reach: {line}"
    );

    // On the mutation, and never on the file. A `main.rs` carrying real code
    // — this project's own is 79K — still owes every mutant in it, so an
    // exclusion aimed at the file would buy the gate's silence by dropping
    // coverage that is genuinely the coder's to answer.
    assert!(
        !line.contains("--exclude src/main.rs") && !line.contains("--exclude-glob"),
        "the file is excluded, not the mutation: {line}"
    );
    assert!(
        !script.contains("exclude_globs"),
        "a mutants.toml excluding the file would do the same damage:\n{script}"
    );
}
