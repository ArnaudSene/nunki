//! Several stacks in one project (SPEC 4.2, "plusieurs stacks"): how they are
//! declared, what one image carrying them is made of, and what is refused
//! while the gates judge only one.

use std::collections::BTreeMap;
use std::path::Path;

use nunki::image;
use nunki::project::{Config, Project, Stack};

fn config(yaml: &str) -> Result<Config, String> {
    serde_yaml_ng::from_str(&format!("harness: claude-code\n{yaml}")).map_err(|e| e.to_string())
}

/// Every `nunki.yaml` written before a stack could have a directory still
/// reads, and a stack in a directory reads beside it.
#[test]
fn a_stack_is_a_name_or_a_name_with_its_directory() {
    let c = config("stacks:\n  - rust\n  - next: frontend\n").unwrap();
    assert_eq!(
        c.stacks,
        vec![
            Stack::root("rust"),
            Stack {
                name: "next".into(),
                dir: "frontend".into()
            }
        ]
    );
    // And it is written back the way it was read.
    let written = serde_yaml_ng::to_string(&c.stacks).unwrap();
    assert_eq!(written, "- rust\n- next: frontend\n");
    assert_eq!(
        config("stacks: [rust]\n").unwrap().stacks,
        vec![Stack::root("rust")]
    );
    // `.` and slashes say the root and nothing more.
    assert_eq!(
        config("stacks:\n  - next: ./\n").unwrap().stacks,
        vec![Stack::root("next")]
    );
    assert!(
        config("stacks:\n  - next: /apps/web/\n").is_err(),
        "an absolute directory is outside the repository"
    );
}

/// A directory outside the repository, and a map that names two stacks at
/// once, are refused where they are read.
#[test]
fn a_directory_outside_the_repository_is_refused() {
    for bad in [
        "stacks:\n  - next: ../web\n",
        "stacks:\n  - next: web/../../x\n",
        "stacks:\n  - {next: web, rust: core}\n",
    ] {
        let err = config(bad).expect_err(bad);
        assert!(!err.is_empty(), "{bad}");
    }
    assert!(Stack::parse("next=../web").is_err());
    assert_eq!(
        Stack::parse("next=frontend/").unwrap(),
        Stack {
            name: "next".into(),
            dir: "frontend".into()
        }
    );
}

/// A repository with its configuration and fragments, the way `nunki init`
/// leaves them, for the stacks given as `--stack` takes them.
fn project(dir: &Path, stacks: &[&str]) -> Project {
    let root = dir.join("repo");
    let home = dir.join("home");
    std::fs::create_dir_all(&root).unwrap();
    let asked: Vec<String> = stacks.iter().map(|s| s.to_string()).collect();
    nunki::init::init(&root, &home, &asked).unwrap();
    Project::open_at(root, home).expect("init wrote a configuration open accepts")
}

/// One name twice is one fragment serving two directories: refused.
#[test]
fn a_stack_named_twice_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    let err = nunki::init::init(
        &root,
        &dir.path().join("home"),
        &["next".into(), "next=web".into()],
    )
    .unwrap_err();
    assert!(err.to_string().contains("twice"), "{err}");

    let p = project(dir.path(), &["rust"]);
    let file = p.home.join(nunki::project::CONFIG_FILE);
    let text = std::fs::read_to_string(&file)
        .unwrap()
        .replace("  - rust\n", "  - rust\n  - rust: core\n");
    std::fs::write(&file, text).unwrap();
    let err = Project::open_at(p.root.clone(), p.home.clone()).unwrap_err();
    assert!(err.to_string().contains("declared twice"), "{err}");
}

/// `nunki init --stack next=frontend` writes the directory into `nunki.yaml`,
/// and running it again without `--stack` keeps it.
#[test]
fn init_writes_the_directory_and_keeps_it_on_a_second_run() {
    let dir = tempfile::tempdir().unwrap();
    let p = project(dir.path(), &["rust", "next=frontend"]);
    assert_eq!(p.config.stacks[1].dir, "frontend");
    assert_eq!(p.stack("next").dir, "frontend");
    assert_eq!(p.stack_tree(&p.root, "next"), p.root.join("frontend"));
    assert_eq!(p.stack_tree(&p.root, "rust"), p.root);

    let again = nunki::init::init(&p.root, &p.home, &[]).unwrap();
    let reopened = Project::open_at(p.root.clone(), p.home.clone()).unwrap();
    assert_eq!(reopened.config.stacks, p.config.stacks, "{again:?}");
}

/// A stack's writable directories are relative to its own directory in the
/// fragment, and to the tree everywhere they are used — one place adds the
/// prefix, so the image's mount point and the slot's volume cannot disagree.
#[test]
fn a_stack_in_a_directory_writes_where_it_lives() {
    let dir = tempfile::tempdir().unwrap();
    let p = project(dir.path(), &["rust", "next=frontend"]);
    let rust = p.stack_writable("rust");
    let next = p.stack_writable("next");
    assert!(rust.contains(&"target".to_string()), "{rust:?}");
    assert!(!next.is_empty());
    for path in &next {
        assert!(path.starts_with("frontend/"), "{next:?}");
    }
}

/// Every stack `nunki init` knows can be added onto another's image: it ships
/// an add-on, and the versions it reads are arguments its add-on declares.
#[test]
fn every_stack_ships_an_add_on_declaring_what_it_reads() {
    for stack in nunki::init::KNOWN_STACKS {
        let dir = tempfile::tempdir().unwrap();
        let primary = if stack == "rust" { "next" } else { "rust" };
        let p = project(dir.path(), &[primary, &format!("{stack}=sub")]);
        let text = image::contribution(&p, &p.config.stacks, &p.config.stacks[1])
            .unwrap_or_else(|e| panic!("{stack}: {e}"));
        let sources = p.stack_versions(stack).unwrap();
        assert_eq!(
            nunki::versions::undeclared(&sources, &text),
            Vec::<String>::new(),
            "{stack}"
        );
        // Starts from an image that ends as the agent, and ends as the agent.
        let users: Vec<&str> = text.lines().filter(|l| l.starts_with("USER ")).collect();
        assert_eq!(users.last(), Some(&"USER agent"), "{stack}: {users:?}");
        // And never `BASE`, which the primary's Dockerfile owns.
        assert!(
            !nunki::versions::declared(&text).contains("BASE"),
            "{stack}"
        );
    }
}

/// The steps a stack's image and its add-on share are one text: cut out of
/// both files, they are byte for byte the same.
#[test]
fn a_stack_and_its_add_on_share_their_steps_byte_for_byte() {
    let dir = tempfile::tempdir().unwrap();
    let p = project(dir.path(), &["rust", "python=py", "next=web"]);
    let read =
        |stack: &str, file: &str| std::fs::read_to_string(p.fragment(stack).join(file)).unwrap();
    let cut = |text: &str, from: &str, to: &str| -> String {
        let i = text.find(from).unwrap_or_else(|| panic!("no {from:?}"));
        let j = text[i..].find(to).unwrap_or_else(|| panic!("no {to:?}")) + i + to.len();
        text[i..j].to_string()
    };
    for (stack, from, to) in [
        (
            "rust",
            "ENV RUSTUP_HOME=",
            "rm -rf /home/agent/.cargo/registry/*\n",
        ),
        ("python", "ARG OSV_SCANNER=", "osv-scanner --version\n"),
        (
            "python",
            "# `UV_PYTHON_DOWNLOADS=never`",
            "/home/agent/.harness\n",
        ),
        ("next", "ARG OSV_SCANNER=", "osv-scanner --version\n"),
        ("next", "# pnpm, baked in", "! command -v npm\n"),
        (
            "next",
            "# `NEXT_TELEMETRY_DISABLED`",
            "/home/agent/.harness\n",
        ),
    ] {
        let own = cut(&read(stack, "Dockerfile"), from, to);
        let added = cut(&read(stack, nunki::project::ADDON_FILE), from, to);
        assert_eq!(own, added, "{stack}: {from}");
    }
}

/// What the composed Dockerfile looks like, frozen: regenerate with
/// `HQ_BLESS=1 cargo test --test stacks` and read the diff.
///
/// The shape it holds: every global `ARG` before any `FROM` — the primary's
/// `FROM ${BASE}` needs its own `BASE` global — the add-ons' stages before the
/// primary's first `FROM`, and the bodies at the end in the declared order.
#[test]
fn the_composed_dockerfile_is_the_primary_with_stages_first_and_add_ons_last() {
    let dir = tempfile::tempdir().unwrap();
    let p = project(dir.path(), &["rust", "next=frontend", "python=tools"]);
    let text = image::compose_stacks(&p, &p.config.stacks).unwrap();

    let froms: Vec<&str> = text.lines().filter(|l| l.starts_with("FROM ")).collect();
    assert_eq!(
        froms,
        [
            "FROM node:${NODE}-bookworm-slim AS nunki-next-node",
            "FROM python:${PYTHON}-slim-bookworm AS nunki-python-runtime",
            "FROM ghcr.io/astral-sh/uv:${UV_VERSION} AS nunki-python-uv",
            "FROM ${BASE}",
        ]
    );
    let first_from = text.find("\nFROM ").unwrap();
    for arg in ["ARG BASE=", "ARG NODE=", "ARG PYTHON=", "ARG UV_VERSION="] {
        let at = text.find(arg).unwrap_or_else(|| panic!("no {arg}"));
        assert!(
            at < first_from,
            "{arg} comes after a FROM, so it is not global"
        );
    }
    let next = text.find("# Next.js, added by nunki").unwrap();
    let python = text.find("# Python, added by nunki").unwrap();
    let rust_tail = text.find("rm -rf /home/agent/.cargo/registry/*").unwrap();
    assert!(
        rust_tail < next && next < python,
        "the add-ons follow the primary, in order"
    );
    assert!(
        text.trim_end()
            .ends_with("RUN mkdir -p /home/agent/.cache/uv /home/agent/.harness")
    );

    let golden = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/composed-rust-next-python.Dockerfile");
    if std::env::var_os("HQ_BLESS").is_some() {
        std::fs::write(&golden, &text).unwrap();
    }
    assert_eq!(text, std::fs::read_to_string(&golden).unwrap());
}

/// Each stack's versions are read in its own directory, and the image is
/// built with all of them and labelled with all of them.
#[test]
fn the_image_reads_each_stack_in_its_own_directory() {
    let dir = tempfile::tempdir().unwrap();
    let p = project(dir.path(), &["rust", "next=frontend"]);
    std::fs::write(
        p.root.join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"1.98.0\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(p.root.join("frontend")).unwrap();
    std::fs::write(
        p.root.join("frontend/package.json"),
        r#"{"packageManager": "pnpm@11.15.0"}"#,
    )
    .unwrap();
    // At the root it would be read by nothing: next lives in `frontend/`.
    std::fs::write(p.root.join(".nvmrc"), "20\n").unwrap();

    let build = image::stack_build(&p, "rust", 501, 20).unwrap();
    assert_eq!(
        build.args,
        ["UID=501", "GID=20", "PNPM=11.15.0", "RUST_VERSION=1.98.0"]
    );
    assert_eq!(
        build.label,
        "nunki.versions=PNPM=11.15.0;RUST_VERSION=1.98.0"
    );
    let wanted: BTreeMap<String, String> = image::pinned_now(&p, &p.root, "rust").unwrap();
    assert_eq!(wanted.len(), 2);
}

/// The image is the project's: named after its primary stack, and built
/// from it alone. Asking for it by a secondary stack's name is refused.
#[test]
fn the_image_is_asked_for_by_its_primary_stack() {
    let dir = tempfile::tempdir().unwrap();
    let p = project(dir.path(), &["rust", "next=frontend"]);
    let err = image::stack_build(&p, "next", 501, 20).unwrap_err();
    assert!(matches!(err, image::ImageError::NotPrimary { .. }), "{err}");
}

/// Two stacks reading one argument would have one value win for both.
#[test]
fn two_stacks_reading_one_argument_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let p = project(dir.path(), &["rust", "next=frontend"]);
    let versions = p.fragment("next").join(nunki::versions::FILE);
    let mut text = std::fs::read_to_string(&versions).unwrap();
    text.push_str("RUST_VERSION .nvmrc\n");
    std::fs::write(&versions, text).unwrap();
    let addon = p.fragment("next").join(nunki::project::ADDON_FILE);
    let mut body = std::fs::read_to_string(&addon).unwrap();
    body.push_str("ARG RUST_VERSION\n");
    std::fs::write(&addon, body).unwrap();

    let err = image::stack_build(&p, "rust", 501, 20).unwrap_err();
    assert!(
        matches!(err, image::ImageError::SharedArgument { .. }),
        "{err}"
    );
}

/// A fragment written before add-ons existed cannot be added: said, with
/// the command that writes the missing file.
#[test]
fn a_secondary_stack_without_an_add_on_says_how_to_get_one() {
    let dir = tempfile::tempdir().unwrap();
    let p = project(dir.path(), &["rust", "next=frontend"]);
    std::fs::remove_file(p.fragment("next").join(nunki::project::ADDON_FILE)).unwrap();
    let err = image::compose_stacks(&p, &p.config.stacks).unwrap_err();
    assert!(matches!(err, image::ImageError::NoAddon { .. }), "{err}");
    assert!(err.to_string().contains("nunki init --stack next"), "{err}");
}

/// A secondary stack's versions are checked against its add-on, which is what
/// it puts into the image — not against its standalone Dockerfile.
#[test]
fn check_reads_a_secondary_stack_against_its_add_on() {
    let dir = tempfile::tempdir().unwrap();
    let p = project(dir.path(), &["rust", "next=frontend"]);
    let find = |report: &nunki::check::Report, what: &str| {
        report
            .checks
            .iter()
            .find(|c| c.what.contains(what))
            .unwrap_or_else(|| panic!("no {what:?}"))
            .verdict
            .clone()
    };
    assert!(matches!(
        find(&nunki::check::run(&p), "the next image follows"),
        nunki::check::Verdict::Green(_)
    ));

    let addon = p.fragment("next").join(nunki::project::ADDON_FILE);
    let text = std::fs::read_to_string(&addon)
        .unwrap()
        .replace("ARG PNPM=12.5.1\n", "");
    std::fs::write(&addon, text).unwrap();
    match find(&nunki::check::run(&p), "the next image follows") {
        nunki::check::Verdict::Red(why) => {
            assert!(why.contains("PNPM"), "{why}");
            assert!(why.contains(nunki::project::ADDON_FILE), "{why}");
        }
        other => panic!("{other:?}"),
    }
}

/// What a profile carries for several stacks: every stack's scripts, the
/// primary's where one stack's have always been and the others' beside them;
/// and what each lets the coder reach, once each.
#[test]
fn a_profile_mounts_every_stack_and_reaches_what_each_needs() {
    let dir = tempfile::tempdir().unwrap();
    let p = project(dir.path(), &["rust", "next=frontend"]);
    let mounted: Vec<String> = nunki::run::stack_scripts(&p, "rust")
        .into_iter()
        .map(|(_, at)| at.display().to_string())
        .collect();
    for at in [
        "/work/stack/prepush.sh",
        "/work/stack/mutation.sh",
        "/work/stack-next/prepush.sh",
        "/work/stack-next/security.sh",
    ] {
        assert!(mounted.contains(&at.to_string()), "{at} in {mounted:?}");
    }
    let domains = nunki::run::stack_domains(&p, "rust");
    assert!(
        domains.contains(&"index.crates.io".to_string()),
        "{domains:?}"
    );
    assert!(
        domains.contains(&"registry.npmjs.org".to_string()),
        "{domains:?}"
    );
    let mut unique = domains.clone();
    unique.dedup();
    assert_eq!(unique.len(), domains.len());
    // The security agent writes where every stack builds.
    let writable = nunki::run::writable(&p, "rust");
    assert!(writable.contains(&"target".to_string()), "{writable:?}");
    assert!(
        writable.iter().any(|w| w == "frontend/node_modules"),
        "{writable:?}"
    );
}
