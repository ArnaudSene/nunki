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
    // Not a gate, and said so in the pull request: the rules of the place are
    // where a house style lives, and this one is a house style.
    // The same rule the coder's prompt carries: a project's rules say it
    // too, for whoever reads them first.
    assert!(
        rules.contains("no database, no queue"),
        "the rules say what an agent cannot reach, and what to do about it: {rules}"
    );
    assert!(
        rules.contains("Co-Authored-By"),
        "the rules name the trailer they forbid rather than describe it: {rules}"
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
/// 6), so the bare command fails here.
///
/// Not because the audit is impossible in a container — gate 8 runs it,
/// `--offline`, against the database the host filled and `nunki` mounts
/// (measured 2026-09-17). This comment said the stage could never pass here,
/// and that was wrong. It is a division of labour: the advisories question
/// needs the mounted database, the unfiltered view and the comparison against
/// the base, and gate 8 has all three.
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

/// A campaign that could not run reports nothing, rather than the answer of
/// the campaign before it.
///
/// Run, with a stub in place of `cargo`: the script swallows the tool's status
/// with `|| true` — right, because a campaign with survivors exits 2 and that
/// is a result — so what it does with a run that produced no answer at all is
/// the whole question, and reading the source would only prove it says what it
/// says.
///
/// Measured on cargo-mutants 27.1.0, 2026-09-18. A campaign that reaches the
/// mutants rotates `mutants.out` to `mutants.out.old` and writes a fresh one,
/// so a second campaign that kills everything correctly leaves `missed.txt`
/// empty. A campaign that **cannot run** — a crate that does not parse —
/// fails before creating anything, and `mutants.out` is still the previous
/// campaign's: survivors from a campaign that never happened, on code that has
/// changed. A replay of the same fingerprint reuses the same path, so it is
/// reachable.
#[test]
#[cfg(unix)]
fn a_campaign_that_could_not_run_does_not_report_the_last_ones_survivors() {
    let (_dir, root, _nunki) = fresh();
    let home = home(&root);
    init(&root, &home, &["rust".to_string()]).unwrap();
    let script = home
        .join(nunki::project::STACKS_DIR)
        .join("rust")
        .join(nunki::mutants::SCRIPT);

    // A `cargo` that fails the way a crate that does not parse makes it fail:
    // no output directory, no answer, a non-zero status.
    let stubs = root.join("stubs");
    std::fs::create_dir_all(&stubs).unwrap();
    std::fs::write(
        stubs.join("cargo"),
        "#!/bin/sh\necho 'error: expected `!`' >&2\nexit 1\n",
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(stubs.join("cargo"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
    }

    // What the campaign before it left, at the path a replay of the same
    // fingerprint comes back to.
    let tree = root.join("tree");
    let stale = tree.join("target/mutants-abc123/mutants.out");
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(
        stale.join("missed.txt"),
        "src/lib.rs:2:7: replace > with >= in keep\n",
    )
    .unwrap();

    let path = format!(
        "{}:{}",
        stubs.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = std::process::Command::new("sh")
        .arg(&script)
        .arg("abc123")
        .arg("src/lib.rs")
        .env("PATH", path)
        .current_dir(&tree)
        .output()
        .expect("sh is on the path");

    let said = String::from_utf8_lossy(&out.stdout);
    assert!(
        !said.contains("replace > with >="),
        "it reported a campaign that never ran: {said}"
    );
    assert_ne!(out.status.code(), Some(0), "it passed on nothing: {out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("left no"),
        "nothing said why: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // And it never says it finished, which is the line `nunki` reads: without
    // it a stopped campaign and one that found nothing are the same log, and
    // gate 7 goes green on a measurement nobody made.
    assert!(
        !nunki::mutants::completed(&said),
        "it said it had finished: {said}"
    );
}

/// And a campaign that does reach the end says so, on its last line.
///
/// Run, with a stub `cargo` that leaves the file a real campaign leaves: what
/// matters is that the script's own last act is the line `nunki` requires, and
/// that it comes after the survivors rather than instead of them.
#[test]
#[cfg(unix)]
fn a_campaign_that_reached_the_end_says_so_on_its_last_line() {
    let (_dir, root, _nunki) = fresh();
    let home = home(&root);
    init(&root, &home, &["rust".to_string()]).unwrap();
    let script = home
        .join(nunki::project::STACKS_DIR)
        .join("rust")
        .join(nunki::mutants::SCRIPT);

    let stubs = root.join("stubs");
    std::fs::create_dir_all(&stubs).unwrap();
    // `--output DIR` writes into `DIR/mutants.out/`; the script reads
    // `missed.txt` there. One survivor, so the two lines can be told apart.
    std::fs::write(
        stubs.join("cargo"),
        "#!/bin/sh
out=\"\"
while [ $# -gt 0 ]; do
  if [ \"$1\" = --output ]; then out=$2; fi
  shift
done
mkdir -p \"$out/mutants.out\"
printf '%s\\n' 'src/lib.rs:2:7: replace > with >= in keep' > \"$out/mutants.out/missed.txt\"
",
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(stubs.join("cargo"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
    }

    let tree = root.join("tree");
    std::fs::create_dir_all(&tree).unwrap();
    let path = format!(
        "{}:{}",
        stubs.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = std::process::Command::new("sh")
        .arg(&script)
        .arg("abc123")
        .arg("src/lib.rs")
        .env("PATH", path)
        .current_dir(&tree)
        .output()
        .expect("sh is on the path");

    let said = String::from_utf8_lossy(&out.stdout);
    assert!(
        nunki::mutants::completed(&said),
        "it never said it finished: {said}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(nunki::mutants::parse(&said).len(), 1, "{said}");
    // Last, so that a log truncated anywhere loses it.
    assert!(
        said.trim_end()
            .lines()
            .next_back()
            .unwrap()
            .contains("done"),
        "the line that says it finished is not the last one: {said}"
    );
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

/// A release that adds a fragment file must reach the projects that already
/// exist. `init` never overwrites, but it does create what is missing — so
/// re-running it is the migration path.
///
/// It was undiscoverable: `--stack` has no default, so a bare `nunki init`
/// skipped the fragment loop entirely and said nothing about it. Measured on
/// 2026-09-17 — a project missing the `caches.txt` a release had added got
/// four "kept" lines and no hint that its fragments were never examined. The
/// remedy existed and nobody could guess it.
#[test]
fn a_second_init_tops_up_the_stacks_the_project_already_declares() {
    let (_dir, root, nunki) = fresh();
    let home = home(&root);
    init(&root, &home, &["rust".to_string()]).unwrap();

    // A release adds a file to the fragment: the project is missing it.
    let missing = home
        .join(nunki::project::STACKS_DIR)
        .join("rust")
        .join(nunki::project::CACHES_FILE);
    std::fs::remove_file(&missing).unwrap();

    // Re-run the way a human does, without remembering the stack's name.
    let actions = init(&root, &home, &[]).unwrap();

    assert!(missing.is_file(), "the missing fragment was not put back");
    assert!(
        created(&actions, nunki::project::CACHES_FILE),
        "{actions:?}"
    );
    // And what was there is untouched, as always.
    assert!(kept(&actions, "prepush.sh").is_some(), "{actions:?}");
    let _ = nunki;
}

/// The declared stacks come from the project's own configuration, so a
/// project that declares none still gets none — `init` does not guess a stack
/// for a repository that never named one.
#[test]
fn a_first_init_without_a_stack_writes_no_fragment() {
    let (_dir, root, _nunki) = fresh();
    let home = home(&root);

    let actions = init(&root, &home, &[]).unwrap();

    assert!(
        !home.join(nunki::project::STACKS_DIR).join("rust").exists(),
        "{actions:?}"
    );
}

/// A configuration may name a stack this release does not carry yet — the
/// owner plans Python, TypeScript and Solidity. The declared list is not
/// checked against the known one, because it comes from a file a human wrote
/// rather than from a flag; so a stack with no fragments is skipped, and does
/// not leave an empty folder behind.
#[test]
fn a_declared_stack_nunki_has_no_fragment_for_leaves_nothing_behind() {
    let (_dir, root, _nunki) = fresh();
    let home = home(&root);
    init(&root, &home, &["rust".to_string()]).unwrap();

    let config = home.join(nunki::project::CONFIG_FILE);
    let text = std::fs::read_to_string(&config).unwrap();
    std::fs::write(&config, text.replace("- rust", "- rust\n  - solidity")).unwrap();

    let actions = init(&root, &home, &[]).unwrap();

    assert!(
        !home
            .join(nunki::project::STACKS_DIR)
            .join("solidity")
            .exists(),
        "an empty folder was left for a stack with no fragments: {actions:?}"
    );
    // And the stack it does carry is still examined.
    assert!(kept(&actions, "prepush.sh").is_some(), "{actions:?}");
}

/// Gate 8 is a stack's to declare, like the battery and the campaign, and it
/// reaches the container read-only. A script that judges the agent is not the
/// agent's to weaken.
#[test]
fn the_rust_stack_ships_its_mechanical_security() {
    let (_dir, root, _nunki) = fresh();
    let home = home(&root);

    let actions = init(&root, &home, &["rust".to_string()]).unwrap();

    assert!(created(&actions, nunki::gate::SECURITY), "{actions:?}");
    let script = home
        .join(nunki::project::STACKS_DIR)
        .join("rust")
        .join(nunki::gate::SECURITY);
    assert!(script.is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&script).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "nothing could run it: {mode:o}");
    }

    let body = std::fs::read_to_string(&script).unwrap();
    // The database is a positional argument, never an environment variable:
    // the agent owns its environment inside the container, and a variable
    // would let it point this at an empty directory — no findings, and a
    // green gate.
    assert!(!body.contains("NUNKI_ADVISORIES"), "{body}");
    assert!(
        body.contains(&format!("db=\"${{2:-{}}}\"", nunki::run::ADVISORIES_AT)),
        "the database is not the second argument, at {}:\n{body}",
        nunki::run::ADVISORIES_AT
    );
    // The seven fields of the contract, and no eighth.
    for field in [
        "id",
        "kind",
        "where",
        "via",
        "fix",
        "accepted",
        "was_at_base",
    ] {
        assert!(
            body.contains(&format!("{field}:$")),
            "{field} is not emitted"
        );
    }
    // It carries the two families the battery does not: the dependency audit
    // and the secret scan. Static analysis is `clippy`, at gate 6.
    assert!(body.contains("cargo deny"), "no dependency audit:\n{body}");
    assert!(body.contains("trufflehog git"), "no secret scan:\n{body}");
}

/// A ruling is matched on the whole id, not on a prefix of it.
///
/// Run, with a stub in place of `trufflehog`: what the script does with two
/// rulings whose ids differ only by the line number is the point, and reading
/// the source would only prove the source says what it says.
///
/// The first version was `sed "s|^$1[[:space:]]*||p"`, a prefix match: asked
/// for `…:Postgres:1`, it matched the line for `…:Postgres:10` and returned
/// `0  <that reason>`. A human ruling on one line silenced another.
#[test]
#[cfg(unix)]
fn a_ruling_is_matched_on_the_whole_id_and_not_on_a_prefix_of_it() {
    let (_dir, root, _nunki) = fresh();
    let home = home(&root);
    init(&root, &home, &["rust".to_string()]).unwrap();
    let script = home
        .join(nunki::project::STACKS_DIR)
        .join("rust")
        .join(nunki::gate::SECURITY);

    let repo = root.join("tree");
    std::fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| -> String {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["-c", "user.name=Init Test", "-c", "user.email=init@test"])
            .args(args)
            .output()
            .expect("git is on the path");
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    git(&["init", "-q"]);
    std::fs::write(repo.join("README.md"), "one\n").unwrap();
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "the base"]);
    let base = git(&["rev-parse", "HEAD"]);
    std::fs::write(repo.join("README.md"), "two\n").unwrap();
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "the branch"]);
    let head = git(&["rev-parse", "HEAD"]);

    // One leak, on the branch's commit, at line 1.
    let stubs = root.join("stubs");
    std::fs::create_dir_all(&stubs).unwrap();
    std::fs::write(
        stubs.join("trufflehog"),
        format!(
            "#!/bin/sh\nprintf '%s\\n' '{{\"SourceMetadata\":{{\"Data\":{{\"Git\":\
             {{\"commit\":\"{head}\",\"file\":\"src/leak.rs\",\"line\":1}}}}}},\
             \"DetectorName\":\"Postgres\"}}'\n"
        ),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            stubs.join("trufflehog"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }

    // Two rulings: the one that applies, and one whose id it is a prefix of.
    let secrets = root.join("SECRETS.txt");
    std::fs::write(
        &secrets,
        format!(
            "# a comment, which is not a ruling\n\
             {head}:src/leak.rs:Postgres:10  the wrong line\n\
             {head}:src/leak.rs:Postgres:1  the right line\n"
        ),
    )
    .unwrap();
    let db = root.join("db");
    std::fs::create_dir_all(&db).unwrap();

    let path = format!(
        "{}:{}",
        stubs.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = std::process::Command::new("sh")
        .arg(&script)
        .arg(&base)
        .arg(&db)
        .arg(&root)
        .arg(&secrets)
        .env("PATH", path)
        .current_dir(&repo)
        .output()
        .expect("sh is on the path");

    let said = String::from_utf8_lossy(&out.stdout);
    let line = said
        .lines()
        .find(|l| l.contains("\"kind\":\"secret\""))
        .unwrap_or_else(|| {
            panic!(
                "no secret reported: {said}\n{}",
                String::from_utf8_lossy(&out.stderr)
            )
        });
    assert!(
        line.contains("\"accepted\":\"the right line\""),
        "the wrong ruling applied: {line}"
    );
    // And the id it reports is the one a human would rule on.
    assert!(
        line.contains(&format!("\"id\":\"{head}:src/leak.rs:Postgres:1\"")),
        "{line}"
    );
}

/// A base it cannot read stops it, and does not become an audit against
/// nothing.
///
/// Run, not read: the script is a script, and what it does with a base it
/// cannot resolve is the whole point. It gets a real repository and a base
/// that is not in it, and never reaches `cargo deny`.
///
/// Measured on 2026-09-18 against a real container, where it warned and
/// carried on: every finding came back new, and notes-api's gate 8 went red on
/// a test credential its base already carried.
#[test]
#[cfg(unix)]
fn a_base_it_cannot_read_stops_the_script_instead_of_counting_everything_as_new() {
    let (_dir, root, _nunki) = fresh();
    let home = home(&root);
    init(&root, &home, &["rust".to_string()]).unwrap();
    let script = home
        .join(nunki::project::STACKS_DIR)
        .join("rust")
        .join(nunki::gate::SECURITY);

    // A repository with one commit, and a database directory that exists so
    // the script gets past its own 69.
    let repo = root.join("tree");
    std::fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["-c", "user.name=Init Test", "-c", "user.email=init@test"])
            .args(args)
            .output()
            .expect("git is on the path");
        assert!(out.status.success(), "git {args:?}: {out:?}");
    };
    git(&["init", "-q"]);
    std::fs::write(repo.join("README.md"), "one\n").unwrap();
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "one"]);
    let db = root.join("db");
    std::fs::create_dir_all(&db).unwrap();

    let out = std::process::Command::new("sh")
        .arg(&script)
        .arg("0000000000000000000000000000000000000000")
        .arg(&db)
        .arg(&root)
        .current_dir(&repo)
        .output()
        .expect("sh is on the path");

    assert_eq!(out.status.code(), Some(70), "{out:?}");
    assert!(
        out.stdout.is_empty(),
        "it reported findings without a base to compare them to: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// A secret has no `fix` and no `via`: it is revoked, not upgraded, and
/// nothing brought it in but the commit that wrote it. That commit is in the
/// fingerprint, which is what tells `was_at_base` exactly.
#[test]
fn the_secret_scan_reads_its_exceptions_and_reports_them() {
    let (_dir, root, _nunki) = fresh();
    let home = home(&root);
    init(&root, &home, &["rust".to_string()]).unwrap();
    let body = std::fs::read_to_string(
        home.join(nunki::project::STACKS_DIR)
            .join("rust")
            .join(nunki::gate::SECURITY),
    )
    .unwrap();

    // `--no-ignore-tag` is a rule and not a setting: trufflehog's own
    // exception is a comment **in the source line**, which the agent edits
    // legitimately. Without it, six characters silence a secret.
    //
    // The invocation, not the word: this script explains the flag in a
    // comment just above it, so asserting the word alone watched the flag
    // leave the command line and still called itself green. That trap has
    // caught this file four times now.
    assert!(
        body.contains("--json --no-update --no-ignore-tag"),
        "the agent could silence its own secret:\n{body}"
    );
    // What is accepted is decided outside the container, and reaches the
    // script as an argument at a path `nunki` chooses. The default is the
    // constant, so the two cannot drift apart.
    assert!(
        body.contains(&format!("secrets=\"${{4:-{}}}\"", nunki::secrets::AT)),
        "the secrets file is not the fourth argument, at {}:\n{body}",
        nunki::secrets::AT
    );
    // And not in the mission folder, which goes to `archive/` with the
    // mission: the same secret would stop the next one.
    assert!(
        !body.contains("$mission/SECRETS.txt"),
        "an exception that lives with a mission is lost with it:\n{body}"
    );
    assert!(
        !body.contains("MISSION_DIR"),
        "the mission folder is an argument, not an environment the agent owns:\n{body}"
    );
    // Which commit introduced it, and not merely that it is there.
    assert!(body.contains("merge-base --is-ancestor"), "{body}");
}

/// The tool that finds the secrets is pinned and checked. A binary nobody
/// verifies is a dependency nobody reviewed (SPEC section 7).
#[test]
fn the_secret_scanner_is_pinned_and_its_download_is_checked() {
    let (_dir, root, _nunki) = fresh();
    let home = home(&root);
    init(&root, &home, &["rust".to_string()]).unwrap();
    let dockerfile = std::fs::read_to_string(
        home.join(nunki::project::STACKS_DIR)
            .join("rust/Dockerfile"),
    )
    .unwrap();

    // A version, not a moving name: `ARG GITLEAKS=` alone stays true for
    // `latest`, and a first version of this test watched that pass.
    let pinned = dockerfile
        .lines()
        .find_map(|l| l.strip_prefix("ARG TRUFFLEHOG="))
        .expect("the scanner's version is declared");
    assert!(
        pinned
            .split('.')
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())),
        "the scanner is not pinned to a version: {pinned:?}"
    );
    assert!(
        dockerfile.contains("sha256sum -c -"),
        "the download is not checked:\n{dockerfile}"
    );
    // Both architectures nunki targets, or the build says so rather than
    // producing an image with no scanner in it.
    assert!(dockerfile.contains("amd64)"), "{dockerfile}");
    assert!(dockerfile.contains("arm64)"), "{dockerfile}");
    assert!(dockerfile.contains("ships no build for"), "{dockerfile}");
}

/// The image must carry a JSON parser, because the script parses JSON. A
/// fragment that cannot run is a gate that cannot be played.
#[test]
fn the_image_carries_what_the_security_script_needs() {
    let (_dir, root, _nunki) = fresh();
    let home = home(&root);
    init(&root, &home, &["rust".to_string()]).unwrap();

    let dockerfile = std::fs::read_to_string(
        home.join(nunki::project::STACKS_DIR)
            .join("rust/Dockerfile"),
    )
    .unwrap();

    assert!(
        dockerfile.contains(" jq "),
        "the image has no JSON parser:\n{dockerfile}"
    );
}

/// A Python project gets every file its gates read, and the two that are
/// executed are executable.
///
/// The list is not decoration: gate 6 runs `prepush.sh`, gate 6 for the
/// integrator runs `system.sh`, gate 7 runs `mutation.sh` and gate 8 runs
/// `security.sh`. A fragment missing one holds that gate red for a reason no
/// agent is told, which is what happened to the Rust stack on 2026-09-15.
#[test]
fn a_python_project_gets_every_file_its_gates_read() {
    let (_d, root, nunki) = fresh();
    let actions = init(&root, &nunki, &["python".to_string()]).unwrap();
    let stack = home(&root).join("stacks/python");

    for name in [
        "allow.txt",
        "prepush.sh",
        "mutation.sh",
        nunki::gate::SECURITY,
        "run.sh",
        nunki::gate::SYSTEM_BATTERY,
        "Dockerfile",
        "writable.txt",
        "advisories.txt",
        "caches.txt",
    ] {
        assert!(created(&actions, name), "{name} missing from {actions:?}");
        assert!(stack.join(name).is_file(), "{name}");
    }

    #[cfg(unix)]
    for script in [
        "prepush.sh",
        "mutation.sh",
        nunki::gate::SECURITY,
        "run.sh",
        nunki::gate::SYSTEM_BATTERY,
    ] {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(stack.join(script))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "{script}: {mode:o}");
    }

    let allow = std::fs::read_to_string(stack.join("allow.txt")).unwrap();
    assert!(allow.contains("pypi.org"), "{allow}");
    assert!(
        !allow.contains("github.com"),
        "a fragment must not carry a forge domain: {allow}"
    );
    // And the interpreter is not fetched from one either: uv downloads a
    // Python of its own unless it is told not to, from a forge rule 6 does
    // not name — a refusal that would read as a flaky network.
    let dockerfile = std::fs::read_to_string(stack.join("Dockerfile")).unwrap();
    assert!(
        dockerfile.contains("UV_PYTHON_DOWNLOADS=never"),
        "{dockerfile}"
    );
}

/// Gate 8's unfiltered view has to be one the project cannot filter.
///
/// Measured on osv-scanner 2.6.0, 2026-09-20: the tool loads
/// `osv-scanner.toml` **from the scanned tree on its own** — "Loaded filter
/// from: /w/osv-scanner.toml" with no `--config` given at all. The first
/// draft of this fragment relied on that absence, so the run meant to see
/// everything was filtered by the very file whose effect it exists to
/// measure: every acceptance came back empty, and an agent could have
/// silenced any finding by writing that file.
///
/// So every scan whose answer is read as "everything there is" — the branch's
/// and the base's — names a configuration that ignores nothing, and the only
/// scan that reads the project's own is the one measuring what it accepts.
#[test]
fn the_python_audit_asks_for_a_view_the_project_cannot_filter() {
    let (_d, root, nunki) = fresh();
    init(&root, &nunki, &["python".to_string()]).unwrap();
    let script = std::fs::read_to_string(
        home(&root)
            .join("stacks/python")
            .join(nunki::gate::SECURITY),
    )
    .unwrap();
    // The scans are written across continuations; read them as one line each.
    let joined = script.replace("\\\n", " ");
    let scans: Vec<&str> = joined
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("osv-scanner scan source"))
        .collect();
    assert_eq!(
        scans.len(),
        3,
        "the branch, its base, and what it accepts: {scans:?}"
    );

    let unfiltered: Vec<&&str> = scans
        .iter()
        .filter(|l| !l.contains("--config osv-scanner.toml"))
        .collect();
    assert_eq!(unfiltered.len(), 2, "{scans:?}");
    for scan in unfiltered {
        assert!(
            scan.contains(r#"--config "$work/none.toml""#),
            "this scan would be filtered by the tree's own osv-scanner.toml: {scan}"
        );
    }
    // And that configuration really ignores nothing.
    let none = script
        .lines()
        .find(|l| l.contains(r#"> "$work/none.toml""#))
        .expect("the fragment writes the empty configuration");
    assert!(!none.contains("IgnoredVulns"), "{none}");
}

/// A campaign that could not run must not reach the line that says it
/// finished.
///
/// Measured on mutmut 3.8.0, 2026-09-20: `mutmut run` exits **0** whether
/// every mutant was killed or some survived, and **1** when it could not run
/// at all. The first draft swallowed that status and guarded on the
/// `mutants/` directory instead — which mutmut creates *before* it generates
/// anything, so a source file it could not parse produced
/// `{"campaign":"done"}` with no survivor. Gate 7 green on nothing, which is
/// the one failure this contract exists to prevent (SPEC 4.4, gate 7).
#[test]
fn the_python_campaign_says_nothing_when_it_could_not_run() {
    let (_d, root, nunki) = fresh();
    init(&root, &nunki, &["python".to_string()]).unwrap();
    let script = std::fs::read_to_string(home(&root).join("stacks/python/mutation.sh")).unwrap();

    let run = script
        .lines()
        .find(|l| l.contains("mutmut run") && !l.trim_start().starts_with('#'))
        .expect("the campaign runs mutmut");
    assert!(
        !run.contains("|| true"),
        "the status is the only thing that tells a dead campaign from a green one: {run}"
    );
    assert!(run.trim_start().starts_with("if !"), "{run}");

    // The terminal line comes last, and after the guard that leaves without
    // it: `nunki` reads no other line as the campaign having measured
    // anything.
    let guard = script
        .find("the campaign could not run")
        .expect("the guard says why it stopped");
    let done = script
        .find(r#"printf '{"campaign":"done"}\n'"#)
        .expect("the campaign says when it got to the end");
    assert!(
        guard < done,
        "the terminal line is printed before the guard"
    );
    assert_eq!(
        script.matches(r#"printf '{"campaign":"done"}"#).count(),
        1,
        "the terminal line is printed in more than one place: {script}"
    );
}

/// The integrator's gate 6 runs the system tests, and the coder's battery
/// never does: they need the mission's services, which only the system
/// profile has.
#[test]
fn the_python_integrators_battery_runs_the_system_tests_and_only_those() {
    let (_d, root, nunki) = fresh();
    init(&root, &nunki, &["python".to_string()]).unwrap();
    let stack = home(&root).join("stacks/python");

    let script = std::fs::read_to_string(stack.join(nunki::gate::SYSTEM_BATTERY)).unwrap();
    let line = script
        .lines()
        .find(|l| l.contains("uv run") && l.contains("pytest"))
        .expect("the system battery calls pytest");
    assert!(line.contains("-m system"), "{line}");
    assert!(script.contains("@pytest.mark.system"), "{script}");
    // A battery that ran nothing proved nothing: pytest answers 5 when it
    // collected none, and 0 when it collected them and skipped them all.
    assert!(script.contains(r#""$status" -eq 5"#), "{script}");
    assert!(script.contains(r#""$ran" -eq 0"#), "{script}");

    let prepush = std::fs::read_to_string(stack.join("prepush.sh")).unwrap();
    let coder = prepush
        .lines()
        .find(|l| l.contains("uv run") && l.contains("pytest"))
        .expect("the coder's battery calls pytest");
    assert!(
        coder.contains(r#"-m "not system""#),
        "the coder's battery would run tests that need services it cannot reach: {coder}"
    );
}

/// The battery does not read what the campaign leaves behind.
///
/// Measured on 2026-09-20, on the first mission of the Python bench. Gate 7's
/// campaign runs `mutmut`, which copies the whole tree into `mutants/` before
/// it generates anything. Git ignores that directory, so the `git clean -fd`
/// `nunki exec` performs leaves it where it is, and the next battery type-checked
/// two copies of every module — `Duplicate module named "pygrep"`, exit 2.
///
/// Gate 6 was green before the campaign and red after it, on a tree the coder
/// had not touched, and `nunki` opened a volet sending the agent to repair
/// code that was never broken. The first campaign of a project would have
/// poisoned every battery after it.
#[test]
fn the_python_battery_does_not_read_what_the_campaign_leaves() {
    let (_d, root, nunki) = fresh();
    init(&root, &nunki, &["python".to_string()]).unwrap();
    let stack = home(&root).join("stacks/python");

    // The directory really is the campaign's: the stack declares it writable
    // because `mutmut` writes there, which is what makes it the battery's
    // problem rather than a stray folder nobody put there.
    let writable = std::fs::read_to_string(stack.join(nunki::project::WRITABLE_FILE)).unwrap();
    assert!(
        writable.lines().any(|l| l.trim() == "mutants"),
        "the stack does not declare the campaign's directory: {writable}"
    );

    // **Every** tool the two batteries run, and not a list written by hand.
    // The first fix named ruff and mypy, left pytest out, and the battery
    // was red again one command further down — a second reason wearing the
    // same exit status. A test that enumerates what was fixed proves only
    // that it was fixed; this one asks the property of whatever is there.
    for script in ["prepush.sh", nunki::gate::SYSTEM_BATTERY] {
        let text = std::fs::read_to_string(stack.join(script)).unwrap();
        let walkers: Vec<&str> = text
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .filter(|l| l.contains("uv run"))
            // `uv sync` and `--version` probes read no tree.
            .filter(|l| l.contains("ruff") || l.contains("mypy") || l.contains("pytest"))
            .filter(|l| !l.contains("--version"))
            .collect();
        assert!(!walkers.is_empty(), "{script} runs nothing");
        for tool in walkers {
            assert!(
                tool.contains("mutants"),
                "{script}: this walks the copy the campaign left in `mutants/`: {tool}"
            );
        }
    }

    // And the campaign clears it when it gets to the end — a courtesy, since
    // one that is killed never does, which is why the exclusion above is the
    // guard and this is not.
    let campaign = std::fs::read_to_string(stack.join("mutation.sh")).unwrap();
    assert_eq!(
        campaign.matches("rm -rf mutants").count(),
        2,
        "cleared before the run and after it: {campaign}"
    );
}
