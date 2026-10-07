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

/// A shell to run a stack's script in, with no campaign variable inherited.
///
/// The battery runs inside a campaign too: cargo-mutants runs the test suite
/// against every mutant, with the campaign's `NUNKI_BASE` set. A template
/// spawned here inherited it, took the campaign's base for its own, and three
/// tests failed under the campaign and passed outside it, so the unmutated
/// baseline failed and the campaign measured nothing (security round 1 on
/// this mission). `NUNKI_MUTATION_JOBS` is removed for the same reason: a
/// template test asserting the command a campaign runs without it would see the
/// enclosing campaign's. A test that wants either variable sets it after this.
fn sh() -> std::process::Command {
    let mut command = std::process::Command::new("sh");
    command.env_remove(nunki::mutants::BASE_ENV);
    command.env_remove(nunki::mutants::JOBS_ENV);
    command
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
/// A coder never told that gate 3 wants `HEAD` named in the resume block,
/// nor that gate 5 wants `PR.md` written, loses one attempt to each, on
/// rules it has no way to learn. A gate that enforces what nothing
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
    // The third lot line, and the case it is for (the mission that added it).
    assert!(
        rules.contains("`Lot: <lot> — awaits ruling: <what>`")
            && rules.contains("not a way out of a hard lot")
            && rules.contains("Every backquoted span in `<what>` is read as a survivor id"),
        "the rules state every line nunki reads, and when the third is not one: {rules}"
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
/// A scan goes red on packages such as `libpcre2-8-0`, carrying HIGH CVEs
/// with a fix already published. Such a package arrives with `debian:bookworm-slim` and
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
/// ships none holds that gate red for a reason no agent is told.
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

/// The template names `rigor` and `mutation_threshold`, commented out with
/// the defaults they would have: uncommenting them changes nothing, and
/// leaving them commented reads as the same thing.
#[test]
fn the_nunki_yaml_it_writes_names_the_rigor_and_the_threshold_with_their_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init(&repo, &home(&repo), &["rust".to_string()]).unwrap();

    let text = std::fs::read_to_string(home(&repo).join("nunki.yaml")).unwrap();
    assert!(text.contains("\n# rigor: critical\n"), "{text}");
    assert!(text.contains("\n# mutation_threshold: 80\n"), "{text}");
    assert!(text.contains("\n# mutation_jobs: 1\n"), "{text}");
    let commented: nunki::project::Config = serde_yaml_ng::from_str(&text).unwrap();
    let uncommented: nunki::project::Config = serde_yaml_ng::from_str(
        &text
            .replace("\n# rigor:", "\nrigor:")
            .replace("\n# mutation_threshold:", "\nmutation_threshold:")
            .replace("\n# mutation_jobs:", "\nmutation_jobs:"),
    )
    .unwrap();
    assert_eq!(commented.rigor, None);
    assert_eq!(uncommented.rigor, Some(nunki::mission::Rigor::Critical));
    assert_eq!(commented.mutation_threshold, 80);
    assert_eq!(uncommented.mutation_threshold, 80);
    assert_eq!(commented.mutation_jobs, None);
    assert_eq!(commented.jobs(), 1);
    assert_eq!(uncommented.mutation_jobs, Some(1));
    assert_eq!(uncommented.jobs(), 1);
}

/// The prose `nunki init` deposits must read as prose.
///
/// The cause: `cargo fmt` joins a `\`-continued
/// string literal onto one line and keeps the continuation's indentation as
/// **real spaces**. A template written to read nicely in the source arrives
/// on disk with nine-space indents, and Markdown renders those as a code
/// block. `FOLLOWUP_HQ.md` — the file an agent is told to read first — is
/// exactly the kind of file that would ship that way unnoticed.
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
/// run it. `mutation.sh` calls `cargo mutants`, and if nothing installs it
/// every campaign dies on a command that
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

/// Every cargo in a Rust image builds with its debug info kept out of the
/// linked binaries, whether Rust is the project's stack or an add-on to
/// another.
///
/// Measured on a crate of 170,000 lines with 17 test targets: with the debug
/// info inside, concurrent links reached 13.6 GB and the kernel killed them;
/// an incremental `cargo test --no-run` took 51 s against 18 s without, and a
/// mutation campaign pays that per mutant. In the image's environment and not
/// in the repository's `Cargo.toml`, which is the project's.
#[test]
fn a_rust_image_links_without_copying_debug_info_into_every_binary() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    nunki::init::init(
        &root,
        &dir.path().join("nunki"),
        &["next".to_string(), "rust=backend".to_string()],
    )
    .unwrap();
    let setting = "ENV CARGO_PROFILE_DEV_SPLIT_DEBUGINFO=unpacked";
    for file in ["stacks/rust/Dockerfile", "stacks/rust/Dockerfile.addon"] {
        let text = std::fs::read_to_string(home(&root).join(file)).unwrap();
        assert!(text.contains(setting), "{file} lacks it:\n{text}");
    }
}

/// The campaign does not hand the coder a survivor nobody could answer.
///
/// cargo-mutants replaces a whole function body with `Default::default()`
/// whenever the return type allows it, and `fn main() -> ExitCode` always
/// allows it. No unit test calls `main`, so that mutant cannot be killed by
/// any test the coder is able to write, and gate 7 asks for every survivor to
/// be killed or frozen as a bug. A coder can spend a whole run extracting
/// `main`'s body into a testable function, only for the mutant to reappear
/// on the thin wrapper that is left.
///
/// Measured on a crate with a binary and a library: 19 mutants
/// without the exclusion, 18 with it — it removes `src/main.rs`'s whole body
/// and keeps `src/lib.rs`'s `replace run -> ExitCode`, the same shape in the
/// function `main` delegates to, which a test can and must kill.
/// The image carries what the **battery** calls, not only what the campaign
/// does.
///
/// `prepush.sh` runs `cargo deny check`. An image that does not install
/// `cargo-deny` fails gate 6 with `no such command: deny` — and no agent can
/// repair it, because the image is built from a Dockerfile no agent can
/// reach. A coder that diagnoses the hole correctly on all three attempts,
/// and says each time that it is outside its permission, still gets its
/// mission handed over with every lot built and committed: three runs to be
/// told what the generator could have said once.
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
/// (measured). It is a division of labour: the advisories question
/// needs the mounted database, the unfiltered view and the comparison against
/// the base, and gate 8 has all three.
///
/// Measured in the image this generator writes: with `cargo-deny`
/// installed the battery still comes back non-zero on
/// `failed to fetch advisory database … Could not resolve host: github.com`,
/// while `bans licenses sources` alone passes with `bans ok, licenses ok,
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
/// Measured on cargo-mutants 27.1.0. A campaign that reaches the
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
    let out = sh()
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
    let out = sh()
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
/// skipped the fragment loop entirely and said nothing about it: a project
/// missing a fragment file a release had added got only "kept" lines and no
/// hint that its fragments were never examined. The remedy existed and
/// nobody could guess it.
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

/// A configuration may name a stack this release does not carry yet. The
/// declared list is not
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
    let out = sh()
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
/// A script that warns and carries on reports every finding as new, and
/// gate 8 goes red on a test credential the base already carries.
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

    let out = sh()
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
/// agent is told.
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
/// Measured on osv-scanner 2.6.0: the tool loads
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
/// Measured on mutmut 3.8.0: `mutmut run` exits **0** whether
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
    // Two terminal lines, and the order between them is the invariant: one
    // for a branch with nothing mutable in it, which is a measurement, and
    // one for a campaign that ran — with the guard for a campaign that could not run between
    // them, so a dead campaign reaches neither.
    assert_eq!(
        script.matches(r#"printf '{"campaign":"done""#).count(),
        2,
        "{script}"
    );
    let nothing = script
        .find(r#"printf '{"campaign":"done","tried":0,"found":0}\n'"#)
        .expect("a branch with nothing to mutate says so");
    let guard = script
        .find("the campaign could not run")
        .expect("the guard says why it stopped");
    let done = script
        .rfind(r#"printf '{"campaign":"done"}\n'"#)
        .expect("the campaign says when it got to the end");
    assert!(nothing < guard, "{script}");
    assert!(
        guard < done,
        "the terminal line is printed before the guard"
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
/// On a Python stack, gate 7's
/// campaign runs `mutmut`, which copies the whole tree into `mutants/` before
/// it generates anything. Git ignores that directory, so the `git clean -fd`
/// `nunki exec` performs leaves it where it is, and the next battery
/// type-checks two copies of every module — `Duplicate module named
/// "pygrep"`, exit 2.
///
/// Gate 6 is then green before the campaign and red after it, on a tree the
/// coder has not touched, and `nunki` opens a volet sending the agent to
/// repair code that was never broken. The first campaign of a project would
/// poison every battery after it.
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

    // And the campaign clears the copy itself, three times over, because the
    // three cover different failures:
    //
    // - a `trap`, set before anything can create it, for every ordinary exit
    //   — including the early ones `set -eu` leaves through, which is what a
    //   failed campaign takes;
    // - before the run, which is what clears the copy a *killed* campaign
    //   left, since no trap runs for one of those;
    // - at the end, once the survivors have been read out of it.
    //
    // The exclusion asserted above is still the guard rather than these: it
    // is the only one that does not depend on this script having run at all.
    let campaign = std::fs::read_to_string(stack.join("mutation.sh")).unwrap();
    assert!(
        campaign.contains("trap 'rm -rf mutants' EXIT"),
        "a campaign that exits early must not leave `mutants/` for gate 1 to \
         read as the agent's uncommitted change: {campaign}"
    );
    assert_eq!(
        campaign.matches("rm -rf mutants").count(),
        3,
        "the trap, before the run, and after it: {campaign}"
    );
}

/// A Next.js project gets every file its gates read, and the five that are
/// executed are executable.
#[test]
fn a_next_project_gets_every_file_its_gates_read() {
    let (_d, root, nunki) = fresh();
    let actions = init(&root, &nunki, &["next".to_string()]).unwrap();
    let stack = home(&root).join("stacks/next");

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
    assert!(allow.contains("registry.npmjs.org"), "{allow}");
    assert!(
        !allow.contains("github.com"),
        "a fragment must not carry a forge domain: {allow}"
    );
    // Playwright's CDN is on no allowlist and must not be: the browsers come
    // with the image, so a run can never fetch one.
    assert!(!allow.contains("playwright"), "{allow}");
    let dockerfile = std::fs::read_to_string(stack.join("Dockerfile")).unwrap();
    assert!(
        dockerfile.contains("PLAYWRIGHT_BROWSERS_PATH"),
        "{dockerfile}"
    );
    // Next.js phones home to a domain rule 6 does not name, and the refusal
    // would read as a flaky network rather than as the rule it is.
    assert!(
        dockerfile.contains("NEXT_TELEMETRY_DISABLED=1"),
        "{dockerfile}"
    );
    // Stryker spawns `ps` to find its children, and a slim image has none.
    assert!(dockerfile.contains("procps"), "{dockerfile}");
}

/// The Next.js campaign reads its report, and of its exit status trusts only
/// a failure: it says nothing when there is no report to read, and nothing
/// when Stryker did not exit 0.
///
/// Measured on Stryker 9.6.1: the status is **0** with survivors,
/// 0 when `--mutate` names a file that is not there, and 0 on a source that
/// does not parse. A campaign that trusted a 0 would call a run that never
/// happened a run with no survivor — gate 7 green on nothing. A status
/// other than 0 is a campaign that did not complete (final HQ review).
#[test]
fn the_next_campaign_trusts_its_report_and_only_a_failing_status() {
    let (_d, root, nunki) = fresh();
    init(&root, &nunki, &["next".to_string()]).unwrap();
    let script = std::fs::read_to_string(home(&root).join("stacks/next/mutation.sh")).unwrap();

    let run = script
        .lines()
        .find(|l| l.contains("stryker run") && !l.trim_start().starts_with('#'))
        .expect("the campaign runs stryker");
    // The sandbox goes even when the run fails, so a killed campaign leaves
    // no copy of the project beside the tree.
    assert!(run.contains("--cleanTempDir always"), "{run}");
    // Nothing on the command line overrides the project's own configuration:
    // its test runner is the one Stryker runs.
    assert!(!run.contains("--testRunner"), "{run}");
    // And the touched files are a flag, which is what the Python stack could
    // not do.
    assert!(script.contains("--mutate $path"), "{script}");

    // Two terminal lines, and the order between them is the invariant: one
    // for a branch with nothing mutable in it, which is a measurement, and
    // one for a campaign that ran — with the guard for a campaign that left no report between
    // them, so a dead campaign reaches neither.
    assert_eq!(
        script.matches(r#"printf '{"campaign":"done""#).count(),
        2,
        "{script}"
    );
    let nothing = script
        .find(r#"printf '{"campaign":"done","tried":0,"found":0}\n'"#)
        .expect("a branch with nothing to mutate says so");
    let guard = script
        .find("left no report at")
        .expect("the guard says why it stopped");
    let done = script
        .find(r#"printf '{"campaign":"done"}\n'"#)
        .expect("the campaign says when it got to the end");
    // A Stryker that did not exit 0 is stopped before the report is read.
    let status = script
        .find("stryker exited $status")
        .expect("a failing status stops the campaign");
    assert!(nothing < status && status < guard, "{script}");
    assert!(
        guard < done,
        "the terminal line is printed before the guard"
    );

    // The id is built rather than taken: Stryker numbers its mutants per file
    // and the numbers move between runs, so a coder answering "#7" would name
    // something else next time. And the replacement is in it because one
    // position carries several mutants — `ConditionalExpression` to `true`
    // and to `false` sit on the very same column.
    assert!(
        script.contains("[file, at.line, at.column, mutant.mutatorName, was].join(\":\")"),
        "{script}"
    );
    // Everything Stryker does not call detected or invalid owes an answer,
    // `NoCoverage` and `Pending` included.
    for status in [
        "Killed",
        "Timeout",
        "CompileError",
        "RuntimeError",
        "Ignored",
    ] {
        assert!(
            script.contains(status),
            "{status} missing from the answered set"
        );
    }
}

/// The Next.js audit reads both range shapes, and asks for a view the project
/// cannot filter.
///
/// npm advisories publish their fixed versions as
/// `SEMVER` ranges where PyPI uses `ECOSYSTEM`. A reader that looked only at
/// the second reported **every** npm finding as having no fix — `qs 6.15.1`
/// came back empty when 6.16.0 fixes it. An empty `fix` is not cosmetic: it
/// tells a human nothing can be done, and it is what stops `nunki` calling an
/// accepted finding an exception a fix has overtaken (SPEC 4.4).
#[test]
fn the_next_audit_reads_the_ranges_npm_publishes() {
    let (_d, root, nunki) = fresh();
    init(&root, &nunki, &["next".to_string()]).unwrap();
    let script =
        std::fs::read_to_string(home(&root).join("stacks/next").join(nunki::gate::SECURITY))
            .unwrap();

    assert!(
        script.contains(r#"span.type !== "SEMVER" && span.type !== "ECOSYSTEM""#),
        "{script}"
    );

    // And the same unfiltered view the Python stack had to be taught: every
    // scan whose answer is read as "everything there is" names a
    // configuration that ignores nothing.
    let joined = script.replace("\\\n", " ");
    let scans: Vec<&str> = joined
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("osv-scanner scan source"))
        .collect();
    assert_eq!(scans.len(), 3, "{scans:?}");
    for scan in scans
        .iter()
        .filter(|l| !l.contains("--config osv-scanner.toml"))
    {
        assert!(
            scan.contains(r#"--config "$work/none.toml""#),
            "this scan would be filtered by the tree's own osv-scanner.toml: {scan}"
        );
    }
}

/// No tool in either Next.js battery walks the copy the campaign leaves, and
/// the one that cannot be told to skip is named rather than hidden.
#[test]
fn the_next_batteries_do_not_read_what_the_campaign_leaves() {
    let (_d, root, nunki) = fresh();
    init(&root, &nunki, &["next".to_string()]).unwrap();
    let stack = home(&root).join("stacks/next");

    let writable = std::fs::read_to_string(stack.join(nunki::project::WRITABLE_FILE)).unwrap();
    for name in ["reports", ".stryker-tmp"] {
        assert!(
            writable.lines().any(|l| l.trim() == name),
            "the stack does not declare {name}: {writable}"
        );
    }

    let prepush = std::fs::read_to_string(stack.join("prepush.sh")).unwrap();
    let walkers: Vec<&str> = prepush
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter(|l| l.contains("pnpm exec"))
        .filter(|l| l.contains("eslint") || l.contains("vitest"))
        .filter(|l| !l.contains("--version"))
        .collect();
    assert_eq!(walkers.len(), 2, "eslint and vitest: {walkers:?}");
    for tool in walkers {
        assert!(
            tool.contains("reports") && tool.contains(".stryker-tmp"),
            "this walks what the campaign leaves: {tool}"
        );
    }
    // Prettier takes no such flag — measured, it answers "Ignored unknown
    // option" — and tsc is driven by the project's tsconfig. Both are said in
    // the script rather than quietly skipped.
    assert!(
        prepush.contains("Prettier takes no `--ignore-pattern`"),
        "{prepush}"
    );
    assert!(prepush.contains("`tsc` is the exception"), "{prepush}");
}

/// Every stack `nunki init` knows has its image scanned.
///
/// A stack ships a Dockerfile, and an image nobody scans is the hole that
/// workflow exists to close. A new stack is easy to forget in that list,
/// which is why it is checked against `KNOWN_STACKS` rather than read by eye.
#[test]
fn the_image_scan_covers_every_stack_that_ships_one() {
    let workflow = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/images.yml"),
    )
    .expect("the images workflow is in the repository");
    let matrix = workflow
        .lines()
        .find(|l| l.trim_start().starts_with("stack: ["))
        .expect("the scan runs over a matrix of stacks");
    for stack in KNOWN_STACKS {
        assert!(
            matrix.contains(stack),
            "{stack} ships an image the scan never builds: {matrix}"
        );
    }
}

/// A campaign with nothing to mutate says it finished, in every stack.
///
/// `nunki` reads a campaign that stops without its terminal line as one that
/// was killed: nothing is written, and gate 7 asks for another
/// (`mutants::Progress::Lost`). A branch that touched only tests, or only
/// documentation, has nothing mutable in it — and the three scripts left
/// without a word, so that branch would have been asked for a campaign for
/// ever.
///
/// Having nothing to mutate **is** a measurement: no mutants, therefore no
/// survivors, and gate 7 is satisfied. The scripts are run here rather than
/// read, because that early path exits before it needs any toolchain.
#[cfg(unix)]
#[test]
fn a_campaign_with_nothing_to_mutate_says_it_finished() {
    for stack in KNOWN_STACKS {
        let (_d, root, nunki) = fresh();
        init(&root, &nunki, &[stack.to_string()]).unwrap();
        let script = home(&root).join("stacks").join(stack).join("mutation.sh");

        // A path no stack mutates: the branch changed its README and nothing
        // else.
        let out = sh()
            .arg(&script)
            .arg("abc1234")
            .arg("README.md")
            .current_dir(root.parent().unwrap())
            .output()
            .expect("sh runs the campaign");

        assert!(
            out.status.success(),
            "{stack}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let said = String::from_utf8_lossy(&out.stdout);
        assert!(
            nunki::mutants::completed(&said),
            "{stack}: a campaign that stops without this line is read as one that was \
             killed, and gate 7 would ask again for ever — it said:\n{said}"
        );
        // Nothing was mutable, so nothing was tried, and it says so with a
        // number: a `standard` gate 7 passes a campaign that tried nothing.
        assert_eq!(nunki::mutants::tried(&said), Some(0), "{stack}: {said}");
        // And nothing it could be mistaken for a survivor.
        assert_eq!(
            said.lines().filter(|l| l.contains("\"file\"")).count(),
            0,
            "{stack}: {said}"
        );
    }
}

/// The campaign leaves the system tests alone, the way the battery beside it
/// does.
///
/// `prepush.sh` deselects them with `-m "not system"` because they reach
/// services and the coder's profile carries none. The campaign runs in that
/// same profile and runs the **whole** suite in its copy, so it has to
/// deselect them too — and it drives pytest itself, so it cannot be told on a
/// command line: measured on mutmut 3.8.0, `mutmut run` accepts
/// `--max-children` and nothing else.
///
/// On a project whose system tests open a database, the campaign dies in
/// collection on the driver's connection error, `nunki` reads that as a
/// campaign that could not run, and the mission stops at gate 7 with nothing
/// pointing at the cause.
///
/// The script is **run**, not read, with `uv` replaced by a stub that records
/// the environment it was called in. Asserting that the file contains the
/// line would pass just as well if the line were unreachable, which is the
/// shape of test this repository has been caught writing before.
#[cfg(unix)]
#[test]
fn the_campaign_deselects_the_system_tests() {
    let (_d, root, nunki) = fresh();
    init(&root, &nunki, &["python".to_string()]).unwrap();
    let script = home(&root)
        .join("stacks")
        .join("python")
        .join("mutation.sh");

    // A `uv` that writes down how it was called, and answers every probe the
    // script makes so it reaches the campaign.
    let bin = root.parent().unwrap().join("stub-bin");
    std::fs::create_dir_all(&bin).unwrap();
    let seen = root.parent().unwrap().join("pytest-addopts.txt");
    std::fs::write(
        bin.join("uv"),
        format!(
            "#!/bin/sh\nenv >> {}\necho '---' >> {}\nexit 0\n",
            seen.display(),
            seen.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(
        bin.join("uv"),
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .unwrap();

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = sh()
        .arg(&script)
        .arg("abc1234")
        .arg("src/pkg/thing.py")
        .current_dir(root.parent().unwrap())
        .env("PATH", path)
        .output()
        .expect("sh runs the campaign");

    let recorded = std::fs::read_to_string(&seen).unwrap_or_default();
    let calls: Vec<&str> = recorded
        .split("---")
        .filter(|c| !c.trim().is_empty())
        .collect();
    assert!(
        !calls.is_empty(),
        "the campaign never reached its toolchain, so this proves nothing: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    for (n, call) in calls.iter().enumerate() {
        let addopts = call
            .lines()
            .find_map(|l| l.strip_prefix("PYTEST_ADDOPTS="))
            .unwrap_or("<unset>");
        assert!(
            addopts.contains("not system"),
            "call {n} would run pytest without deselecting the system tests, \
             which reach services no campaign has — PYTEST_ADDOPTS was {addopts:?}"
        );
    }
}

/// A branch that touched only its own tests has nothing to mutate either, and
/// that is not the same statement as the one above: `README.md` is not a
/// source file in any language, while a test **is**, and a campaign that takes
/// it for one asks its tool a question the tool cannot answer.
///
/// On a branch that touches only files under `tests/` and no source at all,
/// mutmut mutates what `source_paths` names rather than what it is handed,
/// finds nothing, and says `Stopping early, because we could not find any
/// test case for any mutant` with a non-zero status. `nunki` reads that as a
/// campaign that could not run, gate 7 keeps asking, and the mission stops.
///
/// One stack is named and not run, deliberately. Rust hands the file to
/// `cargo mutants --file`, which answers this correctly on its own: measured
/// on cargo-mutants 27.1.0, a selection naming only
/// `tests/it.rs` exits 0, warns `No mutants found under the active filters`
/// and writes an empty `missed.txt`, which the fragment already reads as no
/// survivors. Running it here would need a crate and a toolchain to prove
/// something the tool already guarantees. A stack added later has to answer
/// A campaign that fails leaves nothing behind, because gate 1 reads what it
/// leaves as the agent's own uncommitted change.
///
/// `mutants/` is a copy of the tree. It is untracked, it is gitignored
/// nowhere, and `.gitignore` is outside a mission's perimeter — so a
/// directory the agent never created, and cannot ignore, fails its clean-tree
/// gate: a campaign that dies during mutmut's baseline collection makes the
/// next gate run come back "the tree holds 1 uncommitted change(s): ?? mutants/".
///
/// Clearing `mutants/` at the script's end is not enough. The end is the one place
/// a failing campaign never reaches, which is why this is a trap.
#[test]
fn a_campaign_that_fails_leaves_no_copy_of_the_tree_behind() {
    let (_d, root, nunki) = fresh();
    init(&root, &nunki, &["python".to_string()]).unwrap();
    let script = home(&root)
        .join("stacks")
        .join("python")
        .join("mutation.sh");

    // A `uv` that makes the directory the real one would, then fails the way
    // a campaign fails: non-zero, after writing nothing the script can read.
    let bin = root.parent().unwrap().join("stub-bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(
        bin.join("uv"),
        "#!/bin/sh\nmkdir -p mutants\necho copied > mutants/whatever.py\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(
        bin.join("uv"),
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .unwrap();

    let cwd = root.parent().unwrap();
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = sh()
        .arg(&script)
        .arg("abc1234")
        .arg("src/pkg/thing.py")
        .current_dir(cwd)
        .env("PATH", path)
        .output()
        .expect("sh runs the campaign");

    assert!(
        !out.status.success(),
        "this test is about a campaign that fails; it did not"
    );
    assert!(
        !cwd.join("mutants").exists(),
        "the campaign failed and left mutants/ behind, which gate 1 reads as \
         the agent's uncommitted change: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// this question rather than inherit an answer — hence the `panic!`.
#[cfg(unix)]
#[test]
fn a_campaign_whose_branch_touched_only_tests_says_it_finished() {
    for stack in KNOWN_STACKS {
        // What a test file of this stack's language looks like, or why this
        // stack is not asked here.
        let touched: &[&str] = match stack {
            "python" => &[
                "tests/test_search.py",
                "src/pkg/thing_test.py",
                "conftest.py",
            ],
            "next" => &["src/lib/search.test.ts", "e2e/smoke.spec.ts"],
            "rust" => continue,
            other => panic!(
                "{other} is a stack this test has never been told about: say what one of \
                 its test files is called, or why its tool already refuses to mutate one"
            ),
        };

        let (_d, root, nunki) = fresh();
        init(&root, &nunki, &[stack.to_string()]).unwrap();
        let script = home(&root).join("stacks").join(stack).join("mutation.sh");

        let mut command = sh();
        command.arg(&script).arg("abc1234");
        for path in touched {
            command.arg(path);
        }
        let out = command
            .current_dir(root.parent().unwrap())
            .output()
            .expect("sh runs the campaign");

        assert!(
            out.status.success(),
            "{stack}: a campaign told about tests alone must not reach its tool — it said:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let said = String::from_utf8_lossy(&out.stdout);
        assert!(
            nunki::mutants::completed(&said),
            "{stack}: a branch that touched only tests left gate 7 asking for ever — \
             it said:\n{said}"
        );
        // Nothing was mutable, so nothing was tried, and it says so with a
        // number: a `standard` gate 7 passes a campaign that tried nothing.
        assert_eq!(nunki::mutants::tried(&said), Some(0), "{stack}: {said}");
        assert_eq!(
            said.lines().filter(|l| l.contains("\"file\"")).count(),
            0,
            "{stack}: {said}"
        );
    }
}

/// Every argument a fragment's `versions.txt` reads must be one its Dockerfile
/// declares: an undeclared build argument is dropped by the engine with a
/// warning nobody reads, and the image keeps its default (SPEC 4.2).
#[test]
fn every_version_a_fragment_reads_is_an_argument_its_image_declares() {
    for stack in KNOWN_STACKS {
        let (_d, root, nunki) = fresh();
        init(&root, &nunki, &[stack.to_string()]).unwrap();
        let fragment = home(&root).join("stacks").join(stack);
        let file = fragment.join(nunki::versions::FILE);
        let sources = nunki::versions::parse(&std::fs::read_to_string(&file).unwrap(), &file)
            .unwrap_or_else(|e| panic!("{stack}: {e}"));
        assert!(!sources.is_empty(), "{stack} reads no version at all");
        let dockerfile = std::fs::read_to_string(fragment.join("Dockerfile")).unwrap();
        assert_eq!(
            nunki::versions::undeclared(&sources, &dockerfile),
            Vec::<String>::new(),
            "{stack}"
        );
    }
}

/// What each fragment reads out of a repository that pins everything, the
/// way that ecosystem writes it. A key misspelt in a fragment would leave the
/// image on its default without a word.
#[test]
fn each_fragment_reads_the_versions_its_ecosystem_pins() {
    // A stack, the files its repository holds, and what should be read.
    type Case<'a> = (&'a str, &'a [(&'a str, &'a str)], &'a [(&'a str, &'a str)]);
    let cases: [Case; 4] = [
        (
            "rust",
            &[(
                "rust-toolchain.toml",
                "[toolchain]\nchannel = \"1.98.0\"\nprofile = \"minimal\"\n\
                 components = [\"rustfmt\", \"clippy\"]\ntargets = [\"wasm32-unknown-unknown\"]\n",
            )],
            &[
                ("RUST_VERSION", "1.98.0"),
                ("RUST_COMPONENTS", "rustfmt,clippy"),
                ("RUST_TARGETS", "wasm32-unknown-unknown"),
            ],
        ),
        (
            "python",
            &[(".python-version", "3.13\n")],
            &[("PYTHON", "3.13")],
        ),
        (
            "next",
            &[
                (".nvmrc", "v24.1.0\n"),
                (
                    "package.json",
                    r#"{"engines": {"node": ">=20"}, "packageManager": "pnpm@11.15.0+sha512.0f",
                        "devDependencies": {"@playwright/test": "^1.51.1"}}"#,
                ),
                // The range in `package.json` is not the version; the lockfile
                // holds what was resolved, and the image's browsers must be
                // exactly those.
                (
                    "pnpm-lock.yaml",
                    "lockfileVersion: '9.0'\nimporters:\n  .:\n    devDependencies:\n\
                     \x20     '@playwright/test':\n        specifier: ^1.51.1\n\
                     \x20       version: 1.61.1\npackages:\n\
                     \x20 '@playwright/test@1.61.1':\n    resolution: {}\n",
                ),
            ],
            &[
                ("NODE", "24.1.0"),
                ("PNPM", "11.15.0"),
                ("PLAYWRIGHT", "1.61.1"),
            ],
        ),
        // npm's lockfile says the same thing another way.
        (
            "next",
            &[(
                "package-lock.json",
                r#"{"packages": {"node_modules/@playwright/test": {"version": "1.60.0"}}}"#,
            )],
            &[("PLAYWRIGHT", "1.60.0")],
        ),
    ];
    for (stack, files, want) in cases {
        let (_d, root, nunki) = fresh();
        init(&root, &nunki, &[stack.to_string()]).unwrap();
        for (name, body) in files {
            std::fs::write(root.join(name), body).unwrap();
        }
        let file = home(&root)
            .join("stacks")
            .join(stack)
            .join(nunki::versions::FILE);
        let sources =
            nunki::versions::parse(&std::fs::read_to_string(&file).unwrap(), &file).unwrap();
        let got = nunki::versions::pinned(&nunki::versions::resolve(&root, &sources));
        let want: std::collections::BTreeMap<String, String> = want
            .iter()
            .map(|(a, v)| (a.to_string(), v.to_string()))
            .collect();
        assert_eq!(got, want, "{stack}");
    }
}

/// A tree with a base commit and one branch commit on top, and the fork
/// point, for the campaign's diff.
#[cfg(unix)]
fn forked_tree(root: &std::path::Path) -> (std::path::PathBuf, String) {
    let tree = root.join("tree");
    std::fs::create_dir_all(tree.join("src")).unwrap();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(&tree)
            .args(["-c", "user.name=T", "-c", "user.email=t@t"])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    git(&["init", "-q", "-b", "dev"]);
    std::fs::write(
        tree.join("src/lib.rs"),
        "pub fn keep(a: i32) -> bool { a > 2 }\n\npub fn other(a: i32) -> i32 { a + 1 }\n",
    )
    .unwrap();
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "base"]);
    let base = git(&["rev-parse", "HEAD"]);
    std::fs::write(
        tree.join("src/lib.rs"),
        "pub fn keep(a: i32) -> bool { a > 2 }\n\npub fn other(a: i32) -> i32 { a + 2 }\n",
    )
    .unwrap();
    git(&["commit", "-q", "-am", "L1"]);
    (tree, base)
}

/// A `cargo` stub on the path, and the `PATH` that puts it first.
#[cfg(unix)]
fn stub_cargo(root: &std::path::Path, body: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    let stubs = root.join("stubs");
    std::fs::create_dir_all(&stubs).unwrap();
    std::fs::write(stubs.join("cargo"), body).unwrap();
    std::fs::set_permissions(stubs.join("cargo"), std::fs::Permissions::from_mode(0o755)).unwrap();
    format!(
        "{}:{}",
        stubs.display(),
        std::env::var("PATH").unwrap_or_default()
    )
}

/// The campaign answers for the lines the branch changed, not for every line
/// of the files it touched (SPEC 4.4).
///
/// Run, with a real `git` and a stub `cargo` that records what it was asked:
/// given the fork point, the script hands cargo-mutants the diff since it
/// (`--in-diff`), and no longer the whole files (`--file`). Measured on
/// cargo-mutants 27.1.0 against a real crate: a comment-only diff yields no
/// mutant, and a changed body yields only that function's.
#[test]
#[cfg(unix)]
fn the_campaign_mutates_what_the_branch_changed_and_not_whole_files() {
    let (_dir, root, _nunki) = fresh();
    let home = home(&root);
    init(&root, &home, &["rust".to_string()]).unwrap();
    let script = home
        .join(nunki::project::STACKS_DIR)
        .join("rust")
        .join(nunki::mutants::SCRIPT);
    let (tree, base) = forked_tree(&root);
    let asked = root.join("asked.txt");
    let path = stub_cargo(
        &root,
        &format!(
            "#!/bin/sh
echo \"$@\" > {asked}
out=\"\"
while [ $# -gt 0 ]; do
  case \"$1\" in
    --output) out=$2 ;;
    --in-diff) cat \"$2\" >> {asked} ;;
  esac
  shift
done
mkdir -p \"$out/mutants.out\"
printf '%s\\n' 'src/lib.rs:3:33: replace + with - in other' > \"$out/mutants.out/missed.txt\"
exit 2
",
            asked = asked.display()
        ),
    );

    let out = sh()
        .arg(&script)
        .arg("abc123")
        .arg("src/lib.rs")
        .env("PATH", path)
        .env(nunki::mutants::BASE_ENV, &base)
        .current_dir(&tree)
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(
        nunki::mutants::completed(&said),
        "{said}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(nunki::mutants::parse(&said).len(), 1, "{said}");

    let asked = std::fs::read_to_string(&asked).unwrap();
    let args = asked.lines().next().unwrap();
    assert!(
        args.contains("--in-diff"),
        "not restricted to the diff: {args}"
    );
    assert!(!args.contains("--file"), "still whole files: {args}");
    // The diff it was handed is the branch's own change, from the fork point.
    assert!(
        asked.contains("+pub fn other(a: i32) -> i32 { a + 2 }"),
        "{asked}"
    );
    assert!(
        asked.contains("-pub fn other(a: i32) -> i32 { a + 1 }"),
        "{asked}"
    );
}

/// A diff that reaches no mutant is a campaign that found nothing, not one
/// that could not run.
///
/// Measured on cargo-mutants 27.1.0: a comment-only diff makes it exit 0
/// with "No mutants to filter", and without writing `mutants.out` at all. A
/// crate that does not parse also leaves no `mutants.out`, but exits 1 —
/// `a_campaign_that_could_not_run_does_not_report_the_last_ones_survivors`
/// holds that side, with and without the fork point.
#[test]
#[cfg(unix)]
fn a_diff_that_reaches_no_mutant_is_a_campaign_that_found_nothing() {
    let (_dir, root, _nunki) = fresh();
    let home = home(&root);
    init(&root, &home, &["rust".to_string()]).unwrap();
    let script = home
        .join(nunki::project::STACKS_DIR)
        .join("rust")
        .join(nunki::mutants::SCRIPT);
    let (tree, base) = forked_tree(&root);
    let path = stub_cargo(
        &root,
        "#!/bin/sh\necho ' INFO No mutants to filter' >&2\nexit 0\n",
    );

    let out = sh()
        .arg(&script)
        .arg("abc123")
        .arg("src/lib.rs")
        .env("PATH", path)
        .env(nunki::mutants::BASE_ENV, &base)
        .current_dir(&tree)
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(
        nunki::mutants::completed(&said),
        "a campaign with nothing to mutate never finished: {said}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(nunki::mutants::parse(&said).is_empty(), "{said}");
    assert_eq!(out.status.code(), Some(0), "{out:?}");
}

/// A base the copy cannot diff against is a campaign that cannot run, said
/// as one — never a silent fall back to mutating whole files.
#[test]
#[cfg(unix)]
fn a_campaign_that_cannot_diff_against_its_base_says_so() {
    let (_dir, root, _nunki) = fresh();
    let home = home(&root);
    init(&root, &home, &["rust".to_string()]).unwrap();
    let script = home
        .join(nunki::project::STACKS_DIR)
        .join("rust")
        .join(nunki::mutants::SCRIPT);
    let (tree, _base) = forked_tree(&root);
    let called = root.join("called");
    let path = stub_cargo(
        &root,
        &format!("#!/bin/sh\ntouch {}\nexit 0\n", called.display()),
    );

    let out = sh()
        .arg(&script)
        .arg("abc123")
        .arg("src/lib.rs")
        .env("PATH", path)
        .env(
            nunki::mutants::BASE_ENV,
            "0123456789abcdef0123456789abcdef01234567",
        )
        .current_dir(&tree)
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(!nunki::mutants::completed(&said), "{said}");
    assert_ne!(out.status.code(), Some(0), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("cannot diff against the base"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!called.exists(), "it mutated anyway");
}

/// With the fork point, a campaign that left no `mutants.out` is either one
/// with nothing to mutate or one that could not run, and only cargo-mutants'
/// status tells them apart: a crate that does not parse exits 1 (measured on
/// 27.1.0). Read as "nothing to mutate", gate 7 would go green on a crate
/// nobody could mutate.
#[test]
#[cfg(unix)]
fn a_campaign_that_could_not_run_is_not_read_as_one_with_nothing_to_mutate() {
    let (_dir, root, _nunki) = fresh();
    let home = home(&root);
    init(&root, &home, &["rust".to_string()]).unwrap();
    let script = home
        .join(nunki::project::STACKS_DIR)
        .join("rust")
        .join(nunki::mutants::SCRIPT);
    let (tree, base) = forked_tree(&root);
    let path = stub_cargo(
        &root,
        "#!/bin/sh\necho 'cannot parse string into token stream' >&2\nexit 1\n",
    );

    let out = sh()
        .arg(&script)
        .arg("abc123")
        .arg("src/lib.rs")
        .env("PATH", path)
        .env(nunki::mutants::BASE_ENV, &base)
        .current_dir(&tree)
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(!nunki::mutants::completed(&said), "{said}");
    assert_ne!(out.status.code(), Some(0), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("left no"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The diff's paths are the stack's own, from where cargo-mutants runs.
///
/// A stack below the repository's root runs its campaign from its own
/// directory, where the crate's files are `src/…`; a diff that names them
/// `backend/src/…` matches none of them.
#[test]
#[cfg(unix)]
fn the_campaign_diff_names_files_from_the_stacks_own_directory() {
    let (_dir, root, _nunki) = fresh();
    let home = home(&root);
    init(&root, &home, &["rust".to_string()]).unwrap();
    let script = home
        .join(nunki::project::STACKS_DIR)
        .join("rust")
        .join(nunki::mutants::SCRIPT);
    let (tree, base) = forked_tree(&root);
    // The same crate, moved below the root after the fork point: its own
    // commit, so the change and the move are both in the diff.
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(&tree)
            .args(["-c", "user.name=T", "-c", "user.email=t@t"])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
    };
    std::fs::create_dir_all(tree.join("backend")).unwrap();
    git(&["mv", "src", "backend/src"]);
    git(&["commit", "-q", "-m", "move"]);
    let asked = root.join("asked.txt");
    let path = stub_cargo(
        &root,
        &format!(
            "#!/bin/sh
out=\"\"
while [ $# -gt 0 ]; do
  case \"$1\" in
    --output) out=$2 ;;
    --in-diff) cat \"$2\" > {asked} ;;
  esac
  shift
done
mkdir -p \"$out/mutants.out\"
: > \"$out/mutants.out/missed.txt\"
",
            asked = asked.display()
        ),
    );

    let out = sh()
        .arg(&script)
        .arg("abc123")
        .arg("src/lib.rs")
        .env("PATH", path)
        .env(nunki::mutants::BASE_ENV, &base)
        .current_dir(tree.join("backend"))
        .output()
        .unwrap();
    assert!(
        nunki::mutants::completed(&String::from_utf8_lossy(&out.stdout)),
        "{out:?}"
    );
    let asked = std::fs::read_to_string(&asked).unwrap();
    assert!(asked.contains("+++ b/src/lib.rs"), "{asked}");
    assert!(!asked.contains("backend/"), "{asked}");
}

// ---------------------------------------------------------------------------
// Every stack's campaign says how many mutants it tried (SPEC 4.4, gate 7 at
// `standard`). Each is run with stubs standing in for its tool, writing what
// the tool writes, so what is asserted is what the script does with it.
// ---------------------------------------------------------------------------

/// Stubs on a fresh `PATH`, each an executable shell script.
#[cfg(unix)]
fn stubs(at: &Path, scripts: &[(&str, &str)]) -> String {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(at).unwrap();
    for (name, body) in scripts {
        std::fs::write(at.join(name), body).unwrap();
        std::fs::set_permissions(at.join(name), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    format!(
        "{}:{}",
        at.display(),
        std::env::var("PATH").unwrap_or_default()
    )
}

#[cfg(unix)]
fn campaign_of(stack: &str, touched: &str, stub: &[(&str, &str)]) -> (String, String) {
    let (said, why, _) = campaign_in(stack, touched, stub, &[]);
    (said, why)
}

/// [`campaign_of`], in a tree holding `files`, and with the exit status.
#[cfg(unix)]
fn campaign_in(
    stack: &str,
    touched: &str,
    stub: &[(&str, &str)],
    files: &[(&str, &str)],
) -> (String, String, Option<i32>) {
    campaign_with(stack, touched, stub, files, &[])
}

/// [`campaign_in`], with `env` set on the script as `nunki` sets it.
#[cfg(unix)]
fn campaign_with(
    stack: &str,
    touched: &str,
    stub: &[(&str, &str)],
    files: &[(&str, &str)],
    env: &[(&str, &str)],
) -> (String, String, Option<i32>) {
    let (dir, root, nunki) = fresh();
    init(&root, &nunki, &[stack.to_string()]).unwrap();
    let script = home(&root).join("stacks").join(stack).join("mutation.sh");
    let tree = dir.path().join("tree");
    std::fs::create_dir_all(&tree).unwrap();
    for (path, body) in files {
        let at = tree.join(path);
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::write(at, body).unwrap();
    }
    let path = stubs(&dir.path().join("stub-bin"), stub);
    let out = sh()
        .arg(&script)
        .arg("abc123")
        .arg(touched)
        .env("PATH", path)
        .envs(env.iter().copied())
        .current_dir(&tree)
        .output()
        .expect("sh runs the campaign");
    let left: Vec<String> = std::fs::read_dir(&tree)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        format!(
            "{}\nleft in the tree: {left:?}",
            String::from_utf8_lossy(&out.stderr)
        ),
        out.status.code(),
    )
}

/// Rust: caught, missed and timed out are tried — read from cargo-mutants'
/// own outcome files — and unviable is not. Measured on 27.1.0, which
/// writes all four beside `missed.txt`.
#[cfg(unix)]
#[test]
fn the_rust_campaign_says_how_many_mutants_it_tried() {
    let cargo = "#!/bin/sh
out=\"\"
while [ $# -gt 0 ]; do
  if [ \"$1\" = --output ]; then out=$2; fi
  shift
done
o=\"$out/mutants.out\"
mkdir -p \"$o\"
printf 'src/lib.rs:2:5: replace keep -> bool with true\\nsrc/lib.rs:2:7: replace > with == in keep\\nsrc/lib.rs:2:7: replace > with < in keep\\n' > \"$o/caught.txt\"
printf 'src/lib.rs:2:7: replace > with >= in keep\\n' > \"$o/missed.txt\"
printf 'src/lib.rs:9:1: replace spin with ()\\n' > \"$o/timeout.txt\"
printf 'src/lib.rs:5:5: replace make -> Opaque with Default::default()\\nsrc/lib.rs:6:5: replace x\\n' > \"$o/unviable.txt\"
printf '[1,2,3,4,5,6,7]' > \"$o/mutants.json\"
exit 2
";
    let (said, why) = campaign_of("rust", "src/lib.rs", &[("cargo", cargo)]);
    assert!(nunki::mutants::completed(&said), "{said}\n{why}");
    assert_eq!(nunki::mutants::parse(&said).len(), 1, "{said}");
    assert_eq!(nunki::mutants::tried(&said), Some(5), "{said}\n{why}");
    // Seven mutants listed, two of them unviable: five testable found.
    assert_eq!(nunki::mutants::found(&said), Some(5), "{said}\n{why}");
}

/// Python: the survivors, filtered to the touched files, and **no count**.
/// mutmut mutates whole files under `source_paths`, so any `tried` would be
/// a share over whole files, where a well-tested file hides the branch's
/// new untested lines (HQ review). Without one, a `standard` gate 7 judges
/// as `critical` does. The stubs leave mutmut's per-file records exactly as
/// a count would read them, so a script that counted again would say so.
#[cfg(unix)]
#[test]
fn the_python_campaign_gives_no_count_over_whole_files() {
    let uv = r#"#!/bin/sh
case "$*" in
  *"mutmut run"*)
    mkdir -p mutants/src/pkg
    printf '%s' '{"exit_code_by_key":{"pkg.thing.x_a__mutmut_1":1,"pkg.thing.x_a__mutmut_2":3,"pkg.thing.x_a__mutmut_3":37,"pkg.thing.x_a__mutmut_4":0,"pkg.thing.x_a__mutmut_5":null,"pkg.thing.x_a__mutmut_6":34}}' > mutants/src/pkg/thing.py.meta
    ;;
esac
exit 0
"#;
    // The interpreter reads `mutmut results` and `mutmut show`; here it
    // prints what it would have found: the two mutants nothing proved dead.
    let python = r#"#!/bin/sh
cat > /dev/null
printf '%s\n' '{"id": "pkg.thing.x_a__mutmut_4", "file": "src/pkg/thing.py", "line": 3, "description": "survived: a -> b"}'
printf '%s\n' '{"id": "pkg.thing.x_a__mutmut_5", "file": "src/pkg/thing.py", "line": 4, "description": "not checked"}'
"#;
    let (said, why) = campaign_of(
        "python",
        "src/pkg/thing.py",
        &[("uv", uv), ("python3", python)],
    );
    assert!(nunki::mutants::completed(&said), "{said}\n{why}");
    assert_eq!(nunki::mutants::parse(&said).len(), 2, "{said}");
    assert_eq!(nunki::mutants::tried(&said), None, "{said}\n{why}");
    assert_eq!(nunki::mutants::found(&said), None, "{said}\n{why}");
    assert_eq!(
        said.lines().last(),
        Some(r#"{"campaign":"done"}"#),
        "{said}"
    );
    assert!(
        !why.contains("\"mutants\""),
        "mutants/ was left behind: {why}"
    );
}

/// A `node` standing in for the two scripts the Next.js campaign hands it,
/// told apart by their argument count (there is no node in this image).
/// With `-e` alone it is the locator, and answers where `stryker.config.json`
/// puts the json report, or Stryker's default. With the report after it, it
/// prints the survivors the report names, in the campaign's own shape.
#[cfg(unix)]
const NODE: &str = r#"#!/bin/sh
if [ $# -lt 3 ]; then
  jq -jr '.jsonReporter.fileName // "reports/mutation/mutation.json"' stryker.config.json 2>/dev/null \
    || printf 'reports/mutation/mutation.json'
  exit 0
fi
jq -c '.files | to_entries[] | .key as $f | .value.mutants[]
  | select(.status == "Survived" or .status == "NoCoverage" or .status == "Pending")
  | {id: ($f + ":" + .mutatorName), file: $f, line: .location.start.line, description: .status}' "$3"
"#;

/// Next.js: the survivors Stryker's report names, and **no count**, for the
/// same reason as Python: `--mutate` names whole touched files.
#[cfg(unix)]
#[test]
fn the_next_campaign_gives_no_count_over_whole_files() {
    let pnpm = r#"#!/bin/sh
case "$*" in
  *"stryker run"*)
    mkdir -p reports/mutation
    printf '%s' '{"files":{"app/page.ts":{"mutants":[
      {"status":"Killed"},{"status":"Timeout"},{"status":"Survived","mutatorName":"M","replacement":"x","location":{"start":{"line":1,"column":1}}},
      {"status":"NoCoverage","mutatorName":"N","replacement":"y","location":{"start":{"line":2,"column":1}}},
      {"status":"Pending","mutatorName":"P","replacement":"z","location":{"start":{"line":3,"column":1}}},
      {"status":"CompileError"},{"status":"RuntimeError"},{"status":"Ignored"}]}}}' > reports/mutation/mutation.json
    ;;
esac
exit 0
"#;
    let (said, why) = campaign_of("next", "app/page.ts", &[("pnpm", pnpm), ("node", NODE)]);
    assert!(nunki::mutants::completed(&said), "{said}\n{why}");
    assert_eq!(nunki::mutants::parse(&said).len(), 3, "{said}\n{why}");
    assert_eq!(nunki::mutants::tried(&said), None, "{said}\n{why}");
    assert_eq!(nunki::mutants::found(&said), None, "{said}\n{why}");
    assert_eq!(
        said.lines().last(),
        Some(r#"{"campaign":"done"}"#),
        "{said}"
    );
}

// ---------------------------------------------------------------------------
// The Rust campaign reads no configuration from the tree it mutates, while
// the Python and Next.js ones read theirs as on `dev` (final HQ review); and
// a campaign that did not complete never says it finished (security round 1
// on the rigor mission, HQ ruling).
// ---------------------------------------------------------------------------

/// A `cargo` that answers like cargo-mutants 27.1.0 on a crate with seven
/// survivors and four caught: with no `--no-config`, a tree
/// `.cargo/mutants.toml` setting `timeout_multiplier = 0` turns all eleven
/// into timeouts, and one with `exclude_re` empties the campaign. Measured
/// on the real tool before this stub was written; the stub keeps what the
/// tool does so the battery can ask the script what it passes.
const CARGO_READING_ITS_CONFIG: &str = r#"#!/bin/sh
out=""; noconfig=0
for arg in "$@"; do
  case "$arg" in --no-config) noconfig=1 ;; esac
done
while [ $# -gt 0 ]; do
  if [ "$1" = --output ]; then out=$2; fi
  shift
done
o="$out/mutants.out"
mkdir -p "$o"
: > "$o/unviable.txt"; : > "$o/caught.txt"; : > "$o/missed.txt"; : > "$o/timeout.txt"
config=.cargo/mutants.toml
if [ "$noconfig" = 0 ] && [ -f "$config" ] && grep -q exclude_re "$config"; then
  printf '[]' > "$o/mutants.json"
  exit 0
fi
printf '[1,2,3,4,5,6,7,8,9,10,11]' > "$o/mutants.json"
if [ "$noconfig" = 0 ] && [ -f "$config" ] && grep -q timeout_multiplier "$config"; then
  for n in 1 2 3 4 5 6 7 8 9 10 11; do echo "src/lib.rs:$n:1: replace m$n" >> "$o/timeout.txt"; done
  exit 3
fi
for n in 1 2 3 4; do echo "src/lib.rs:$n:1: replace c$n" >> "$o/caught.txt"; done
for n in 5 6 7 8 9 10 11; do echo "src/lib.rs:$n:1: replace m$n" >> "$o/missed.txt"; done
exit 2
"#;

/// A tree config that would hide every survivor, or empty the campaign,
/// changes nothing: the script runs cargo-mutants with `--no-config`.
#[cfg(unix)]
#[test]
fn a_tree_config_changes_nothing_in_the_rust_campaign() {
    let stub = [("cargo", CARGO_READING_ITS_CONFIG)];
    let (plain, why, _) = campaign_in("rust", "src/lib.rs", &stub, &[]);
    assert_eq!(nunki::mutants::parse(&plain).len(), 7, "{plain}\n{why}");
    assert_eq!(nunki::mutants::tried(&plain), Some(11), "{plain}");
    for config in [
        "timeout_multiplier = 0.0\nminimum_test_timeout = 0.0\n",
        "exclude_re = [\".*\"]\n",
    ] {
        let (said, why, _) = campaign_in(
            "rust",
            "src/lib.rs",
            &stub,
            &[(".cargo/mutants.toml", config)],
        );
        assert_eq!(said, plain, "{config} changed the campaign:\n{why}");
    }
}

/// A campaign whose tool did not complete never prints the done line,
/// whatever files it left: cargo-mutants exits 4 when the tests fail on the
/// unmutated code, with every outcome file empty — measured on 27.1.0, 12
/// mutants found and none tested.
#[cfg(unix)]
#[test]
fn a_rust_campaign_whose_baseline_fails_never_says_it_finished() {
    let cargo = r#"#!/bin/sh
out=""
while [ $# -gt 0 ]; do
  if [ "$1" = --output ]; then out=$2; fi
  shift
done
o="$out/mutants.out"
mkdir -p "$o"
: > "$o/caught.txt"; : > "$o/missed.txt"; : > "$o/timeout.txt"; : > "$o/unviable.txt"
printf '[1,2,3,4,5,6,7,8,9,10,11,12]' > "$o/mutants.json"
exit 4
"#;
    let (said, why, status) = campaign_in("rust", "src/lib.rs", &[("cargo", cargo)], &[]);
    assert!(!nunki::mutants::completed(&said), "{said}\n{why}");
    assert_ne!(status, Some(0), "{why}");
    assert!(why.contains("exited 4"), "{why}");
    // Any status but the three that mean it completed.
    for status in [1, 70] {
        let cargo = cargo.replace("exit 4", &format!("exit {status}"));
        let (said, why, _) = campaign_in("rust", "src/lib.rs", &[("cargo", &cargo)], &[]);
        assert!(!nunki::mutants::completed(&said), "{status}: {said}\n{why}");
    }
}

/// The python campaign runs with the tree's own mutmut configuration, as on
/// `dev`: mutmut needs `source_paths` to find a project's sources, and a
/// campaign that refused it would turn gate 7 red at the default rigor for
/// every project that sets it (final HQ review). That a tree configuration
/// can shape the campaign is deferred with the rest of campaign isolation.
#[cfg(unix)]
#[test]
fn the_python_campaign_runs_with_the_trees_own_mutmut_config() {
    let uv = "#!/bin/sh\necho \"uv $*\" >&2\nexit 0\n";
    let python = r#"#!/bin/sh
cat > /dev/null
printf '%s\n' '{"id": "pkg.thing.x_a__mutmut_4", "file": "src/pkg/thing.py", "line": 3, "description": "survived: a -> b"}'
"#;
    for (file, body) in [
        (
            "pyproject.toml",
            "[project]\nname = \"x\"\n\n[tool.mutmut]\nsource_paths = [\"src/\"]\n",
        ),
        ("setup.cfg", "[mutmut]\npaths_to_mutate = src/\n"),
    ] {
        let (said, why, status) = campaign_in(
            "python",
            "src/pkg/thing.py",
            &[("uv", uv), ("python3", python)],
            &[(file, body)],
        );
        assert_eq!(status, Some(0), "{file}: {why}");
        assert!(
            why.contains("mutmut run"),
            "{file}: mutmut never ran: {why}"
        );
        assert!(nunki::mutants::completed(&said), "{file}: {said}\n{why}");
        assert_eq!(nunki::mutants::parse(&said).len(), 1, "{file}: {said}");
        assert_eq!(
            said.lines().last(),
            Some(r#"{"campaign":"done"}"#),
            "{file}: {said}"
        );
    }
}

/// The Next.js campaign runs with the tree's own `stryker.config.json`, as
/// on `dev`: Stryker reads it — its test runner among it, which nothing on
/// the command line overrides — and the report is read where it says the
/// json reporter writes (final HQ review).
#[cfg(unix)]
#[test]
fn the_next_campaign_runs_with_the_trees_own_stryker_config() {
    // A Stryker that writes its report where the configuration tells it to,
    // and says on stderr what it was asked.
    let pnpm = r#"#!/bin/sh
case "$*" in
  *"stryker run"*)
    echo "stryker asked: $*" >&2
    where=$(jq -r '.jsonReporter.fileName // "reports/mutation/mutation.json"' stryker.config.json)
    mkdir -p "$(dirname "$where")"
    printf '%s' '{"files":{"app/page.ts":{"mutants":[{"status":"Killed"},
      {"status":"Survived","mutatorName":"M","replacement":"x","location":{"start":{"line":1,"column":1}}}]}}}' > "$where"
    ;;
esac
exit 0
"#;
    let config = r#"{"testRunner": "vitest", "jsonReporter": {"fileName": "out/stryker.json"}}"#;
    let (said, why, status) = campaign_in(
        "next",
        "app/page.ts",
        &[("pnpm", pnpm), ("node", NODE)],
        &[("stryker.config.json", config)],
    );
    assert_eq!(status, Some(0), "{why}");
    assert!(why.contains("stryker asked: "), "Stryker never ran: {why}");
    assert!(!why.contains("--testRunner"), "{why}");
    assert!(nunki::mutants::completed(&said), "{said}\n{why}");
    assert_eq!(nunki::mutants::parse(&said).len(), 1, "{said}\n{why}");
    assert_eq!(
        said.lines().last(),
        Some(r#"{"campaign":"done"}"#),
        "{said}"
    );
    // The report it read is the one it removes behind itself.
    assert!(!why.contains("\"out\""), "the report was left: {why}");
}

/// Stryker's status of 0 says little, but anything else is a campaign that
/// did not complete — a failing initial test run among them — and it never
/// says it finished, whatever report it left behind.
#[cfg(unix)]
#[test]
fn a_next_campaign_whose_stryker_fails_never_says_it_finished() {
    let pnpm = r#"#!/bin/sh
case "$*" in
  *"stryker run"*)
    mkdir -p reports/mutation
    printf '%s' '{"files":{"app/page.ts":{"mutants":[{"status":"Killed"}]}}}' > reports/mutation/mutation.json
    exit 1
    ;;
esac
exit 0
"#;
    let (said, why, status) = campaign_in(
        "next",
        "app/page.ts",
        &[("pnpm", pnpm), ("node", NODE)],
        &[],
    );
    assert!(!nunki::mutants::completed(&said), "{said}\n{why}");
    assert_ne!(status, Some(0), "{why}");
    assert!(why.contains("stryker exited 1"), "{why}");
}

// ---------------------------------------------------------------------------
// `mutation_jobs`, handed to every template in `NUNKI_MUTATION_JOBS`: each
// passes it to its tool's own parallelism option, keeps today's command
// without it, and refuses a value that is not a whole number of at least 1
// before the tool runs.
// ---------------------------------------------------------------------------

/// What the stub tool was asked, from the line it writes on stderr.
#[cfg(unix)]
fn asked(why: &str) -> String {
    why.lines()
        .find_map(|l| l.strip_prefix("asked: "))
        .unwrap_or_else(|| panic!("the tool never ran: {why}"))
        .to_string()
}

/// The values every template refuses, and that never reach the tool.
#[cfg(unix)]
const BAD_JOBS: [&str; 5] = ["0", "-1", "1.5", "four", "04"];

/// Rust: `1` is today's in-place run, argument for argument, and so is no
/// variable at all; above 1 the campaign runs `--jobs N`, in copies, and
/// keeps everything else. The terminal line and its counts are unchanged.
#[cfg(unix)]
#[test]
fn the_rust_campaign_runs_in_place_at_one_job_and_in_parallel_above() {
    let cargo = r#"#!/bin/sh
echo "asked: $*" >&2
out=""
while [ $# -gt 0 ]; do
  if [ "$1" = --output ]; then out=$2; fi
  shift
done
o="$out/mutants.out"
mkdir -p "$o"
printf 'src/lib.rs:2:7: replace > with == in keep\n' > "$o/caught.txt"
printf 'src/lib.rs:2:7: replace > with >= in keep\n' > "$o/missed.txt"
: > "$o/timeout.txt"; : > "$o/unviable.txt"
printf '[1,2]' > "$o/mutants.json"
exit 2
"#;
    let run =
        |env: &[(&str, &str)]| campaign_with("rust", "src/lib.rs", &[("cargo", cargo)], &[], env);
    let today = "mutants --no-config --in-place --no-shuffle --exclude-re replace main ->  \
                 --output target/mutants-abc123 --file src/lib.rs";

    let (said, why, status) = run(&[]);
    assert_eq!(status, Some(0), "{why}");
    assert_eq!(asked(&why), today, "without the variable");
    let (one, why, status) = run(&[(nunki::mutants::JOBS_ENV, "1")]);
    assert_eq!(status, Some(0), "{why}");
    assert_eq!(asked(&why), today, "at one job");
    assert_eq!(one, said, "one job says what no variable says");

    let (four, why, status) = run(&[(nunki::mutants::JOBS_ENV, "4")]);
    assert_eq!(status, Some(0), "{why}");
    assert_eq!(
        asked(&why),
        today.replace("--in-place", "--jobs 4 --copy-vcs true"),
        "at four jobs"
    );
    // The same answer: what changes is how the tool runs, not what it says.
    assert_eq!(four, said);
    assert!(nunki::mutants::completed(&four), "{four}");
    assert_eq!(nunki::mutants::tried(&four), Some(2), "{four}");
    assert_eq!(nunki::mutants::found(&four), Some(2), "{four}");
    assert_eq!(nunki::mutants::parse(&four).len(), 1, "{four}");

    for bad in BAD_JOBS {
        let (said, why, status) = run(&[(nunki::mutants::JOBS_ENV, bad)]);
        assert!(!nunki::mutants::completed(&said), "{bad}: {said}");
        assert_ne!(status, Some(0), "{bad}: {why}");
        assert!(!why.contains("asked: "), "{bad} reached the tool: {why}");
        assert!(
            why.contains("NUNKI_MUTATION_JOBS is a whole number of at least 1"),
            "{bad}: {why}"
        );
    }
}

/// Python: the value is mutmut's own `--max-children`; without the
/// variable nothing is passed, and mutmut keeps its default, as today.
#[cfg(unix)]
#[test]
fn the_python_campaign_hands_mutation_jobs_to_mutmut() {
    let uv = r#"#!/bin/sh
case "$*" in
  *"mutmut run"*) echo "asked: $*" >&2 ;;
esac
exit 0
"#;
    let python = "#!/bin/sh\ncat > /dev/null\n";
    let run = |env: &[(&str, &str)]| {
        campaign_with(
            "python",
            "src/pkg/thing.py",
            &[("uv", uv), ("python3", python)],
            &[],
            env,
        )
    };
    let today = "run --frozen --no-sync mutmut run";

    let (said, why, status) = run(&[]);
    assert_eq!(status, Some(0), "{why}");
    assert_eq!(asked(&why), today, "without the variable");
    assert_eq!(
        said.lines().last(),
        Some(r#"{"campaign":"done"}"#),
        "{said}"
    );
    for n in ["1", "6"] {
        let (said, why, status) = run(&[(nunki::mutants::JOBS_ENV, n)]);
        assert_eq!(status, Some(0), "{why}");
        assert_eq!(asked(&why), format!("{today} --max-children {n}"));
        assert_eq!(
            said.lines().last(),
            Some(r#"{"campaign":"done"}"#),
            "{said}"
        );
    }
    for bad in BAD_JOBS {
        let (said, why, status) = run(&[(nunki::mutants::JOBS_ENV, bad)]);
        assert!(!nunki::mutants::completed(&said), "{bad}: {said}");
        assert_ne!(status, Some(0), "{bad}: {why}");
        assert!(!why.contains("asked: "), "{bad} reached the tool: {why}");
        assert!(
            why.contains("NUNKI_MUTATION_JOBS is a whole number of at least 1"),
            "{bad}: {why}"
        );
    }
}

/// Next.js: the value is Stryker's own `--concurrency`; without the
/// variable nothing is passed, and Stryker keeps its default, as today.
#[cfg(unix)]
#[test]
fn the_next_campaign_hands_mutation_jobs_to_stryker() {
    let pnpm = r#"#!/bin/sh
case "$*" in
  *"stryker run"*)
    echo "asked: $*" >&2
    mkdir -p reports/mutation
    printf '%s' '{"files":{"app/page.ts":{"mutants":[{"status":"Killed"}]}}}' > reports/mutation/mutation.json
    ;;
esac
exit 0
"#;
    let run = |env: &[(&str, &str)]| {
        campaign_with(
            "next",
            "app/page.ts",
            &[("pnpm", pnpm), ("node", NODE)],
            &[],
            env,
        )
    };
    let today = "exec stryker run --mutate app/page.ts --cleanTempDir always --reporters json";

    let (said, why, status) = run(&[]);
    assert_eq!(status, Some(0), "{why}");
    assert_eq!(asked(&why), today, "without the variable");
    assert_eq!(
        said.lines().last(),
        Some(r#"{"campaign":"done"}"#),
        "{said}"
    );
    for n in ["1", "6"] {
        let (said, why, status) = run(&[(nunki::mutants::JOBS_ENV, n)]);
        assert_eq!(status, Some(0), "{why}");
        assert_eq!(
            asked(&why),
            today.replace(
                "--cleanTempDir",
                &format!("--concurrency {n} --cleanTempDir")
            )
        );
        assert_eq!(
            said.lines().last(),
            Some(r#"{"campaign":"done"}"#),
            "{said}"
        );
    }
    for bad in BAD_JOBS {
        let (said, why, status) = run(&[(nunki::mutants::JOBS_ENV, bad)]);
        assert!(!nunki::mutants::completed(&said), "{bad}: {said}");
        assert_ne!(status, Some(0), "{bad}: {why}");
        assert!(!why.contains("asked: "), "{bad} reached the tool: {why}");
        assert!(
            why.contains("NUNKI_MUTATION_JOBS is a whole number of at least 1"),
            "{bad}: {why}"
        );
    }
}

/// The shell every test here spawns a script in carries no campaign
/// variable, whatever this process carries: the battery is the same inside a
/// campaign and outside it. Asked of the command itself, because this
/// process has no `NUNKI_BASE` to inherit outside a campaign, and a test
/// that spawned one would prove nothing here.
#[test]
fn the_scripts_here_never_inherit_a_campaign_variable() {
    for variable in [nunki::mutants::BASE_ENV, nunki::mutants::JOBS_ENV] {
        let removed = sh()
            .get_envs()
            .any(|(key, value)| key == variable && value.is_none());
        assert!(removed, "{variable} reaches the scripts");
    }
}

// ---------------------------------------------------------------------------
// Each template's span output: `end_line`, the last line of the code a
// mutation replaces (HQ review 2, E). The tool is a stub; the template's own
// code reading its output runs for real. The Python and Next.js templates
// read it with their stack's interpreter, so their tests run where `python3`
// or `node` is on the path — as on CI's macOS runners — and say they were
// skipped where it is not, rather than pass on nothing.
// ---------------------------------------------------------------------------

/// Whether `program` runs here; when it does not, the reason the test is
/// skipped is printed.
#[cfg(unix)]
fn present(program: &str, test: &str) -> bool {
    let here = std::process::Command::new(program)
        .arg("--version")
        .output()
        .is_ok();
    if !here {
        eprintln!("{test}: skipped — no `{program}` on the path, so its template code cannot run");
    }
    here
}

/// The survivors' spans a campaign printed, by id.
#[cfg(unix)]
fn spans(said: &str) -> std::collections::BTreeMap<String, Option<(u32, u32)>> {
    nunki::mutants::parse(said)
        .into_iter()
        .map(|s| (s.id.clone(), s.span()))
        .collect()
}

/// Python: the span is the run of lines mutmut's diff removes, placed in the
/// file — one line, two lines, and a removal in a second hunk read from that
/// hunk's own first line, not the first hunk's.
#[cfg(unix)]
#[test]
fn the_python_campaign_carries_each_survivors_span_from_mutmuts_diff() {
    if !present(
        "python3",
        "the_python_campaign_carries_each_survivors_span_from_mutmuts_diff",
    ) {
        return;
    }
    let thing = "def x_a(a, b):\n    return a > b\n\n\ndef x_b(n):\n    y = n + 1\n    return y\n";
    let uv = r#"#!/bin/sh
case "$*" in
  *"mutmut results"*)
    printf 'pkg.thing.x_a__mutmut_1: survived\npkg.thing.x_b__mutmut_2: survived\npkg.thing.x_b__mutmut_3: survived\n'
    ;;
  *"mutmut show pkg.thing.x_a__mutmut_1"*)
    printf -- '--- src/pkg/thing.py\n+++ src/pkg/thing.py\n@@ -1,2 +1,2 @@\n def x_a(a, b):\n-    return a > b\n+    return a >= b\n'
    ;;
  *"mutmut show pkg.thing.x_b__mutmut_2"*)
    printf -- '--- src/pkg/thing.py\n+++ src/pkg/thing.py\n@@ -1,3 +1,2 @@\n def x_b(n):\n-    y = n + 1\n-    return y\n+    return None\n'
    ;;
  *"mutmut show pkg.thing.x_b__mutmut_3"*)
    printf -- '--- src/pkg/thing.py\n+++ src/pkg/thing.py\n@@ -1,2 +1,2 @@\n def x_a(a, b):\n     return a > b\n@@ -5,3 +5,3 @@\n def x_b(n):\n-    y = n + 1\n+    y = n - 1\n     return y\n'
    ;;
esac
exit 0
"#;
    let (said, why, code) = campaign_in(
        "python",
        "src/pkg/thing.py",
        &[("uv", uv)],
        &[("src/pkg/thing.py", thing)],
    );
    assert_eq!(code, Some(0), "{said}\n{why}");
    let spans = spans(&said);
    assert_eq!(spans["pkg.thing.x_a__mutmut_1"], Some((2, 2)), "{said}");
    assert_eq!(spans["pkg.thing.x_b__mutmut_2"], Some((6, 7)), "{said}");
    assert_eq!(
        spans["pkg.thing.x_b__mutmut_3"],
        Some((6, 6)),
        "a second hunk counts from its own first line: {said}"
    );
}

/// Next.js: the span is the report's `location.end.line` when it is a whole
/// number from 1 up, and 0 — unknown — when it is anything else, as the
/// Rust template does with what jq reads.
#[cfg(unix)]
#[test]
fn the_next_campaign_carries_each_survivors_span_and_sanitises_it() {
    if !present(
        "node",
        "the_next_campaign_carries_each_survivors_span_and_sanitises_it",
    ) {
        return;
    }
    let pnpm = r#"#!/bin/sh
case "$*" in
  *"stryker run"*)
    mkdir -p reports/mutation
    printf '%s' '{"files":{"app/page.ts":{"mutants":[
      {"status":"Survived","mutatorName":"A","replacement":"a","location":{"start":{"line":2,"column":1},"end":{"line":4,"column":2}}},
      {"status":"Survived","mutatorName":"B","replacement":"b","location":{"start":{"line":5,"column":1},"end":{"line":"7","column":2}}},
      {"status":"Survived","mutatorName":"C","replacement":"c","location":{"start":{"line":6,"column":1},"end":{"line":6.5,"column":2}}},
      {"status":"Survived","mutatorName":"D","replacement":"d","location":{"start":{"line":7,"column":1},"end":{"line":-1,"column":2}}},
      {"status":"Survived","mutatorName":"E","replacement":"e","location":{"start":{"line":8,"column":1}}}]}}}' > reports/mutation/mutation.json
    ;;
esac
exit 0
"#;
    let (said, why) = campaign_of("next", "app/page.ts", &[("pnpm", pnpm)]);
    assert!(nunki::mutants::completed(&said), "{said}\n{why}");
    let ends: Vec<Option<u32>> = nunki::mutants::parse(&said)
        .into_iter()
        .map(|s| s.end_line)
        .collect();
    assert_eq!(
        ends,
        vec![Some(4), Some(0), Some(0), Some(0), Some(0)],
        "{said}"
    );
}

// ---------------------------------------------------------------------------
// A campaign leaves the build cache as it found it (SPEC 4.4, gate 7): what
// its mutants built is removed however it ends, and so are the copies the
// tool builds in above one job.
// ---------------------------------------------------------------------------

/// A stand-in `cargo mutants` that builds like one: it adds "mutant
/// artefacts" to the cache — in `deps`, in a fresh incremental session, in a
/// fingerprint of its own, and over the crate's own library, whose name a
/// mutant shares with the baseline — a new name hard-linked to an old file,
/// as rustc relinks unchanged units, and a copy under `$TMPDIR` above one
/// job. Then it ends the way `STUB_END` says.
#[cfg(unix)]
const CARGO_THAT_BUILDS: &str = r#"#!/bin/sh
out=""
jobs=""
while [ $# -gt 0 ]; do
  if [ "$1" = --output ]; then out=$2; fi
  if [ "$1" = --jobs ]; then jobs=$2; fi
  shift
done
mkdir -p target/debug/deps target/debug/incremental/crate-1/s-new target/debug/.fingerprint/crate-new
echo mutant > target/debug/deps/crate-mutant.rcgu.o
echo mutant > target/debug/deps/libcrate-0001.rlib
echo mutant > target/debug/incremental/crate-1/s-new/a.o
echo mutant > target/debug/.fingerprint/crate-new/lib-crate
# What a real rebuild leaves most of: an unchanged unit hard-linked under a
# new name, which keeps the old file's time.
ln target/debug/deps/libdep-1.rlib target/debug/deps/dep-relinked.rcgu.dwo
if [ -n "$jobs" ]; then
  mkdir -p "$TMPDIR/cargo-mutants-tree-x/target"
  echo copy > "$TMPDIR/cargo-mutants-tree-x/target/big"
fi
o="$out/mutants.out"
case "$STUB_END" in
  caught|missed|baseline)
    mkdir -p "$o"
    : > "$o/caught.txt"; : > "$o/missed.txt"; : > "$o/timeout.txt"; : > "$o/unviable.txt"
    printf '[1]' > "$o/mutants.json"
    ;;
esac
case "$STUB_END" in
  caught) echo 'src/lib.rs:1:1: replace a' > "$o/caught.txt"; exit 0 ;;
  missed) echo 'src/lib.rs:1:1: replace a' > "$o/missed.txt"; exit 2 ;;
  baseline) exit 4 ;;
  crash) exit 1 ;;
  stopped) kill -TERM "$PPID"; exec sleep 30 ;;
esac
"#;

/// Every file under `dir`, relative to it, with its content, and every
/// directory, the way a test compares a cache before and after.
#[cfg(unix)]
fn cache_of(dir: &Path) -> std::collections::BTreeMap<String, Option<String>> {
    fn walk(root: &Path, at: &Path, into: &mut std::collections::BTreeMap<String, Option<String>>) {
        for entry in std::fs::read_dir(at).unwrap().flatten() {
            let path = entry.path();
            let name = path.strip_prefix(root).unwrap().display().to_string();
            if path.is_dir() {
                into.insert(format!("{name}/"), None);
                walk(root, &path, into);
            } else {
                into.insert(
                    name,
                    Some(std::fs::read_to_string(&path).unwrap_or_default()),
                );
            }
        }
    }
    let mut into = std::collections::BTreeMap::new();
    if dir.is_dir() {
        walk(dir, dir, &mut into);
    }
    into
}

/// Write `body` at `path` with a modification time `minutes` ago.
#[cfg(unix)]
fn aged(path: &Path, body: &str, minutes: u64) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
    let then = std::time::SystemTime::now() - std::time::Duration::from_secs(60 * minutes);
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(then)
        .unwrap();
}

/// The Rust campaign, run against [`CARGO_THAT_BUILDS`] in a tree whose
/// cache was built an hour ago — and which an earlier campaign, killed
/// outright, left with its file list, a mutant's artefact, its answer and a
/// copy. Each way a campaign ends, at one job and at three: afterwards the
/// cache holds what was built before the campaign and nothing else; only the
/// crate's own library, which a mutant overwrote under the same name, is
/// gone, to be rebuilt once. The campaign's own output stays.
#[cfg(unix)]
#[test]
fn a_rust_campaign_leaves_the_build_cache_as_it_found_it_however_it_ends() {
    for jobs in ["1", "3"] {
        for end in ["caught", "missed", "baseline", "crash", "stopped"] {
            let (dir, root, nunki) = fresh();
            init(&root, &nunki, &["rust".to_string()]).unwrap();
            let script = home(&root).join("stacks/rust/mutation.sh");
            let tree = dir.path().join("tree");
            let target = tree.join("target");
            aged(&tree.join("src/lib.rs"), "pub fn a() {}\n", 90);
            aged(&target.join("debug/deps/libdep-1.rlib"), "dep", 60);
            aged(&target.join("debug/deps/libcrate-0001.rlib"), "crate", 60);
            aged(&target.join("debug/.fingerprint/dep-1/lib-dep"), "fp", 60);
            aged(&target.join("release/keep"), "release", 60);
            let built = cache_of(&target);
            // What a campaign killed outright left behind: the list it
            // recorded, a mutant's artefact, an answer and a copy.
            // An earlier campaign's answer, there before it: on its list.
            aged(
                &target.join("mutants-earlier/mutants.out/missed.txt"),
                "x",
                50,
            );
            let mut listed: Vec<String> = built
                .iter()
                .filter(|(_, body)| body.is_some())
                .map(|(name, _)| format!("target/{name}\n"))
                .collect();
            listed.push("target/mutants-earlier/mutants.out/missed.txt\n".to_string());
            listed.sort();
            aged(&target.join("nunki-campaign.list"), &listed.concat(), 30);
            aged(&target.join("debug/deps/killed-mutant.o"), "mutant", 20);

            let path = stubs(
                &dir.path().join("stub-bin"),
                &[("cargo", CARGO_THAT_BUILDS)],
            );
            // Where copies would go if the script did not say: the
            // system's temporary directory, which nothing cleans.
            let system_tmp = dir.path().join("system-tmp");
            aged(
                &system_tmp.join("nunki-campaign-copies/cargo-mutants-tree-y/big"),
                "copy",
                20,
            );
            let started = std::time::Instant::now();
            let out = sh()
                .arg(&script)
                .arg("abc123")
                .arg("src/lib.rs")
                .env("PATH", path)
                .env("STUB_END", end)
                .env(nunki::mutants::JOBS_ENV, jobs)
                .env("TMPDIR", &system_tmp)
                .current_dir(&tree)
                .output()
                .expect("sh runs the campaign");
            let took = started.elapsed();
            let said = String::from_utf8_lossy(&out.stdout);
            let why = String::from_utf8_lossy(&out.stderr);
            let case = format!("{end} at {jobs} job(s)");

            match end {
                "caught" | "missed" => {
                    assert!(nunki::mutants::completed(&said), "{case}: {said}\n{why}");
                    assert_eq!(
                        nunki::mutants::parse(&said).len(),
                        usize::from(end == "missed"),
                        "{case}: {said}"
                    );
                }
                _ => assert!(!nunki::mutants::completed(&said), "{case}: {said}\n{why}"),
            }
            if end == "stopped" {
                assert_eq!(out.status.code(), Some(143), "{case}: {why}");
                // At once, and not when the tool happens to finish: the
                // stand-in would have run on for thirty seconds.
                assert!(
                    took < std::time::Duration::from_secs(20),
                    "{case}: the stop waited for the tool ({took:?})"
                );
            }
            assert!(
                cache_of(&system_tmp).is_empty(),
                "{case}: a copy was left behind: {:?}",
                cache_of(&system_tmp)
            );

            let mut left = cache_of(&target);
            let own: Vec<String> = left
                .keys()
                .filter(|k| k.starts_with("mutants-abc123/"))
                .cloned()
                .collect();
            if end != "crash" && end != "stopped" {
                assert!(
                    own.iter().any(|k| k.ends_with("mutants.json")),
                    "{case}: the campaign's own output went: {own:?}"
                );
            }
            for k in own {
                left.remove(&k);
            }
            let mut expected = built.clone();
            expected.remove("debug/deps/libcrate-0001.rlib");
            assert_eq!(left, expected, "{case}: the cache is not as it was\n{why}");
        }
    }
}

/// Next.js: the sandbox a killed Stryker left, `.stryker-tmp/`, is cleared
/// before the next run — the one thing Stryker leaves behind that
/// `--cleanTempDir always` cannot remove, since a killed process runs nothing.
#[cfg(unix)]
#[test]
fn a_next_campaign_clears_the_sandbox_a_killed_one_left() {
    let pnpm = r#"#!/bin/sh
case "$*" in
  *"stryker run"*)
    if [ -e .stryker-tmp ]; then echo "sandbox still there" >&2; fi
    mkdir -p reports/mutation
    printf '%s' '{"files":{"app/page.ts":{"mutants":[{"status":"Killed"}]}}}' > reports/mutation/mutation.json
    ;;
esac
exit 0
"#;
    let (said, why, status) = campaign_in(
        "next",
        "app/page.ts",
        &[("pnpm", pnpm), ("node", NODE)],
        &[(".stryker-tmp/sandbox-1234/app/page.ts", "a mutant\n")],
    );
    assert_eq!(status, Some(0), "{why}");
    assert!(nunki::mutants::completed(&said), "{said}\n{why}");
    assert!(!why.contains("sandbox still there"), "{why}");
    assert!(!why.contains(".stryker-tmp"), "the sandbox was left: {why}");
}

/// Python: what mutmut leaves per mutant lives in `mutants/` — the copy of
/// the tree its mutants are written into, with their results — and a killed
/// campaign leaves it all. It is cleared before mutmut runs, not only when
/// the script exits: mutmut would otherwise start from the dead campaign's
/// copy.
#[cfg(unix)]
#[test]
fn a_python_campaign_clears_what_a_killed_one_left_before_it_runs() {
    let uv = r#"#!/bin/sh
case "$*" in
  *"mutmut run"*)
    if [ -e mutants/left-by-the-killed-one.py ]; then echo "old copy still there" >&2; fi
    ;;
esac
exit 0
"#;
    let python = "#!/bin/sh\ncat > /dev/null\n";
    let (said, why, status) = campaign_in(
        "python",
        "src/pkg/thing.py",
        &[("uv", uv), ("python3", python)],
        &[("mutants/left-by-the-killed-one.py", "a mutant\n")],
    );
    assert_eq!(status, Some(0), "{why}");
    assert!(nunki::mutants::completed(&said), "{said}\n{why}");
    assert!(!why.contains("old copy still there"), "{why}");
    assert!(!why.contains("\"mutants\""), "mutants/ was left: {why}");
}

/// What `nunki` judges reached by a later diff and what the Rust campaign is
/// handed to mutate again are the same diff, read the same way (HQ ruling
/// on security round 1). Here, across a `git mv`: the template's own diff,
/// as the stub `cargo` is handed it, and [`nunki::mutants::Changes::between`]
/// agree on every survivor — in the old file and in the new — and the code
/// the survivors stood on is in what the campaign mutates. With git's
/// rename detection in either one, they part: a pure rename gave the tool no
/// hunk, and the survivors were dropped with nothing tried in their place.
#[test]
#[cfg(unix)]
fn a_renamed_file_is_dropped_and_mutated_again_by_the_same_diff() {
    let (_dir, root, _nunki) = fresh();
    let home = home(&root);
    init(&root, &home, &["rust".to_string()]).unwrap();
    let script = home
        .join(nunki::project::STACKS_DIR)
        .join("rust")
        .join(nunki::mutants::SCRIPT);
    let (tree, _base) = forked_tree(&root);
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(&tree)
            .args(["-c", "user.name=T", "-c", "user.email=t@t"])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    // The previous campaign's commit, then a volet that only renames.
    let first = git(&["rev-parse", "HEAD"]);
    git(&["mv", "src/lib.rs", "src/moved.rs"]);
    git(&["commit", "-q", "-m", "volet: a rename"]);

    let asked = root.join("asked.diff");
    let path = stub_cargo(
        &root,
        &format!(
            "#!/bin/sh
while [ $# -gt 0 ]; do
  case \"$1\" in
    --in-diff) cp \"$2\" {asked} ;;
  esac
  shift
done
exit 0
",
            asked = asked.display()
        ),
    );
    let out = sh()
        .arg(&script)
        .arg("abc123")
        .arg("src/lib.rs")
        .arg("src/moved.rs")
        .env("PATH", path)
        .env(nunki::mutants::BASE_ENV, &first)
        .current_dir(&tree)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let handed = std::fs::read_to_string(&asked).expect("the tool was handed a diff");

    let theirs = nunki::mutants::Changes::parse(&handed);
    let ours = nunki::mutants::Changes::between(&tree, &first, "HEAD").unwrap();
    let survivor = |file: &str, line: u32| nunki::mutants::Survivor {
        found_on: None,
        id: format!("{file}:{line}"),
        file: file.into(),
        line,
        end_line: Some(line),
        description: "replace > with >=".into(),
        outcome: None,
        refused: None,
    };
    for s in [
        survivor("src/lib.rs", 1),
        survivor("src/lib.rs", 3),
        survivor("src/moved.rs", 1),
    ] {
        assert_eq!(
            ours.kept(&s),
            theirs.kept(&s),
            "nunki and the campaign read {}:{} differently\n{handed}",
            s.file,
            s.line
        );
        if s.file == "src/lib.rs" {
            assert_eq!(
                ours.kept(&s),
                None,
                "{}:{} survived the rename",
                s.file,
                s.line
            );
        }
    }
    // What was dropped is mutated again: the whole file, under its new name.
    assert!(handed.contains("+++ b/src/moved.rs"), "{handed}");
    assert!(
        handed.contains("+pub fn keep(a: i32) -> bool { a > 2 }"),
        "{handed}"
    );
}
