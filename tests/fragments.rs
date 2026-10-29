//! The fragments follow nunki (SPEC 4.2, "les fragments suivent nunki"): a
//! file an older nunki wrote and nobody touched is replaced by `--refresh`,
//! a file the project rewrote is kept with today's version beside it, and
//! nothing is ever decided from a record read on disk.

use std::path::{Path, PathBuf};

use nunki::init::{Action, Answers, Fragments, Kept, init_as, kept};

/// A stack fragment file as a nunki released before today wrote it: bytes
/// that are in the shipped list and are not today's.
fn older(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/older")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn fresh(stacks: &[&str]) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    let home = dir.path().join("home");
    std::fs::create_dir_all(&root).unwrap();
    let asked: Vec<String> = stacks.iter().map(|s| s.to_string()).collect();
    nunki::init::init(&root, &home, &asked).unwrap();
    (dir, root, home)
}

fn run(root: &Path, home: &Path, how: Fragments) -> Vec<Action> {
    init_as(root, home, &[], &Answers::default(), how).unwrap()
}

fn today(stack: &str, name: &str) -> String {
    nunki::init::fragment(stack)
        .into_iter()
        .find(|(n, _, _)| *n == name)
        .unwrap()
        .1
}

/// Every file nunki writes today is in the list of what it shipped: without
/// it, next release's refresh would read today's files as the project's and
/// never replace them. Regenerate with `scripts/shipped-fragments.sh`.
#[test]
fn everything_nunki_writes_today_is_in_the_shipped_list() {
    let (_d, _root, home) = fresh(&["rust", "python", "next"]);
    for stack in nunki::init::KNOWN_STACKS {
        for (name, body, _) in nunki::init::fragment(stack) {
            let path = home.join("stacks").join(stack).join(name);
            assert_eq!(
                kept(stack, name, &path, &body),
                Kept::Current,
                "{stack}/{name}"
            );
            let hash = nunki::init::sha256(body.as_bytes());
            assert!(
                include_str!("../src/shipped-fragments.txt")
                    .lines()
                    .any(|l| l == format!("{hash} {stack}/{name}")),
                "{stack}/{name} is not in src/shipped-fragments.txt — run \
                 scripts/shipped-fragments.sh > src/shipped-fragments.txt"
            );
        }
    }
}

/// An older nunki's file, untouched: kept by a plain `init`, which says why;
/// replaced by `--refresh`, with the mode a script needs to run.
#[test]
fn an_untouched_older_file_is_replaced_by_refresh_only() {
    let (_d, root, home) = fresh(&["next"]);
    let dockerfile = home.join("stacks/next/Dockerfile");
    let security = home.join("stacks/next/security.sh");
    std::fs::write(&dockerfile, older("next-Dockerfile")).unwrap();
    std::fs::write(&security, older("next-security.sh")).unwrap();
    assert_eq!(
        kept(
            "next",
            "Dockerfile",
            &dockerfile,
            &today("next", "Dockerfile")
        ),
        Kept::Older
    );

    let plain = run(&root, &home, Fragments::Keep);
    assert!(
        plain.iter().any(|a| matches!(a, Action::LeftAlone(p, why)
        if p == &dockerfile && why.contains("--refresh"))),
        "{plain:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&dockerfile).unwrap(),
        older("next-Dockerfile")
    );

    let refreshed = run(&root, &home, Fragments::Refresh);
    assert!(
        refreshed
            .iter()
            .any(|a| matches!(a, Action::Replaced(p) if p == &dockerfile))
    );
    assert_eq!(
        std::fs::read_to_string(&dockerfile).unwrap(),
        today("next", "Dockerfile")
    );
    assert_eq!(
        std::fs::read_to_string(&security).unwrap(),
        today("next", "security.sh")
    );
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(&security).unwrap().permissions().mode();
    assert_eq!(
        mode & 0o111,
        0o111,
        "a replaced script must still run: {mode:o}"
    );
    // Nothing staged is left behind.
    let litter: Vec<_> = std::fs::read_dir(home.join("stacks/next"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".nunki-writing"))
        .collect();
    assert!(litter.is_empty(), "{litter:?}");
}

/// A file the project rewrote is the project's: never replaced, even by
/// `--refresh`, and today's version lies beside it — rewritten when it goes
/// stale, so what is beside it is never an older suggestion.
#[test]
fn a_rewritten_file_is_kept_and_todays_lies_beside_it() {
    let (_d, root, home) = fresh(&["rust"]);
    let battery = home.join("stacks/rust/prepush.sh");
    std::fs::write(&battery, "#!/bin/sh\ncargo test\n").unwrap();
    let beside = home.join("stacks/rust/prepush.sh.nunki");
    std::fs::write(&beside, "an older suggestion\n").unwrap();

    let actions = run(&root, &home, Fragments::Refresh);
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, Action::DepositedBeside { kept, suggestion }
        if kept == &battery && suggestion == &beside)),
        "{actions:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&battery).unwrap(),
        "#!/bin/sh\ncargo test\n"
    );
    assert_eq!(
        std::fs::read_to_string(&beside).unwrap(),
        today("rust", "prepush.sh")
    );
}

/// nunki's own text saved with CRLF endings is neither the project's nor
/// nunki's to replace: it is said, since a script ending in `\r` fails.
#[test]
fn crlf_is_said_rather_than_kept_as_the_projects() {
    let (_d, _root, home) = fresh(&["rust"]);
    let battery = home.join("stacks/rust/prepush.sh");
    let body = today("rust", "prepush.sh");
    std::fs::write(&battery, body.replace('\n', "\r\n")).unwrap();
    assert_eq!(
        kept("rust", "prepush.sh", &battery, &body),
        Kept::LineEndings
    );
}

/// What the image is built from moves together: an older Dockerfile is held
/// back while the project's own `versions.txt` stands beside it, or today's
/// Dockerfile could lack an argument that file reads — and the reverse.
#[test]
fn the_files_that_make_the_image_move_together() {
    let (_d, root, home) = fresh(&["rust"]);
    let dockerfile = home.join("stacks/rust/Dockerfile");
    let versions = home.join("stacks/rust/versions.txt");
    std::fs::write(&dockerfile, older("rust-Dockerfile")).unwrap();
    std::fs::write(
        &versions,
        "RUST_VERSION rust-toolchain.toml toolchain.channel\n",
    )
    .unwrap();

    let actions = run(&root, &home, Fragments::Refresh);
    assert_eq!(
        std::fs::read_to_string(&dockerfile).unwrap(),
        older("rust-Dockerfile")
    );
    assert!(
        actions.iter().any(|a| matches!(a, Action::LeftAlone(p, why)
        if p == &dockerfile && why.contains("move together"))),
        "{actions:?}"
    );

    // Once the project's file is today's again, the group moves.
    std::fs::write(&versions, today("rust", "versions.txt")).unwrap();
    run(&root, &home, Fragments::Refresh);
    assert_eq!(
        std::fs::read_to_string(&dockerfile).unwrap(),
        today("rust", "Dockerfile")
    );
}

/// `nunki check` says which files are behind, and never reds a file the
/// project made its own.
#[test]
fn check_reds_what_refresh_can_fix_and_only_that() {
    let (_d, root, home) = fresh(&["next"]);
    let project = nunki::project::Project::open_at(root.clone(), home.clone()).unwrap();
    let verdict = |p: &nunki::project::Project| {
        nunki::check::run(p)
            .checks
            .into_iter()
            .find(|c| c.what == "the next fragment is today's")
            .unwrap()
            .verdict
    };
    assert!(matches!(verdict(&project), nunki::check::Verdict::Green(_)));

    std::fs::write(
        home.join("stacks/next/prepush.sh"),
        "#!/bin/sh\npnpm test\n",
    )
    .unwrap();
    match verdict(&project) {
        nunki::check::Verdict::Green(said) => assert!(said.contains("prepush.sh"), "{said}"),
        other => panic!("{other:?}"),
    }

    std::fs::write(
        home.join("stacks/next/Dockerfile"),
        older("next-Dockerfile"),
    )
    .unwrap();
    match verdict(&project) {
        nunki::check::Verdict::Red(why) => {
            assert!(
                why.contains("Dockerfile") && why.contains("--refresh"),
                "{why}"
            );
            assert!(
                why.contains("prepush.sh"),
                "the project's file is still said: {why}"
            );
        }
        other => panic!("{other:?}"),
    }
}

/// The project's own steps go last, after every stack, and end as the agent;
/// the Dockerfile's hash moves with them, so an image built before they were
/// added reads as stale.
#[test]
fn the_projects_own_steps_come_last_and_change_the_images_hash() {
    let (_d, root, home) = fresh(&["rust", "next=web"]);
    let project = nunki::project::Project::open_at(root.clone(), home.clone()).unwrap();
    let stacks = project.config.stacks.clone();
    let before = nunki::image::dockerfile(&project, &stacks).unwrap();

    std::fs::write(
        home.join(nunki::project::PROJECT_ADDON_FILE),
        "USER root\nRUN apt-get install -y libssl-dev\n",
    )
    .unwrap();
    let after = nunki::image::dockerfile(&project, &stacks).unwrap();
    assert_ne!(
        nunki::init::sha256(before.as_bytes()),
        nunki::init::sha256(after.as_bytes())
    );
    let project_at = after.find("RUN apt-get install -y libssl-dev").unwrap();
    let next_at = after.find("# Next.js, added by nunki").unwrap();
    assert!(
        next_at < project_at,
        "the project's steps follow every stack"
    );
    assert!(
        after.trim_end().ends_with("USER agent"),
        "{}",
        &after[after.len() - 80..]
    );

    // With one stack too: the project's steps turn it into a composition.
    let (_d2, root2, home2) = fresh(&["rust"]);
    std::fs::write(home2.join(nunki::project::PROJECT_ADDON_FILE), "RUN true\n").unwrap();
    let one = nunki::project::Project::open_at(root2, home2).unwrap();
    let text = nunki::image::dockerfile(&one, &one.config.stacks).unwrap();
    assert!(
        text.contains("RUN true\n\nUSER agent") || text.contains("RUN true\nUSER agent"),
        "{text}"
    );
}

/// An image built from another Dockerfile than today's is red in `check`.
#[test]
fn an_image_built_from_another_dockerfile_is_red() {
    assert!(nunki::check::dockerfile_drift("abc", "abc").is_none());
    let why = nunki::check::dockerfile_drift("abc", "def").unwrap();
    assert!(why.contains("slot rebuild"), "{why}");
}
