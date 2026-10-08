//! The host never executes a slot's configuration (SPEC 3.1, 4.1 bis, 4.2).
//!
//! A slot's `.git` is its agent's: mounted read-write in the container, it
//! holds whatever configuration, hooks and refs the agent wrote there. Every
//! git `nunki` runs on the host about a slot goes through the slot's host
//! mirror (`nunki::git::SlotGit`), and these tests are the proof of the class:
//!
//! - each plant an agent could leave is shown **live** first, by a plain git
//!   in the slot that runs it or reads what it says;
//! - then, with the plant in place, every host-side operation `nunki`
//!   performs on a slot is played, and no marker exists and every read is
//!   what it is in a slot without the plant;
//! - a slot whose `.git` is broken fails with an error that says so, and
//!   runs nothing;
//! - the source is scanned for a plain git on a slot, so that a new one
//!   cannot be added unnoticed.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use nunki::harness::Role;
use nunki::mission::flow::Flow;
use nunki::mission::{Bounds, Header, Integration, Lot, Security};
use nunki::project::{Config, Project, ProtectedPaths};
use nunki::state::{MissionState, Store};

/// Every commit of every world is made at this date, by this person, so two
/// worlds built alike hold the very same commit ids.
const DATE: &str = "2026-01-01T00:00:00+00:00";

/// A plain git, scrubbed of the caller's `GIT_*` and of any pager or
/// editor the environment names, so that what a plant does is the plant's.
fn plain(at: &Path) -> Command {
    let mut git = Command::new("git");
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            git.env_remove(key);
        }
    }
    for key in ["PAGER", "EDITOR", "VISUAL"] {
        git.env_remove(key);
    }
    git.env("GIT_AUTHOR_DATE", DATE)
        .env("GIT_COMMITTER_DATE", DATE)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .stdin(std::process::Stdio::null())
        .arg("-C")
        .arg(at);
    git
}

fn git(at: &Path, args: &[&str]) -> String {
    let out = plain(at).args(args).output().expect("git is on the path");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A plain git whose outcome does not matter, only what it ran.
fn attempt(at: &Path, args: &[&str]) -> String {
    let out = plain(at).args(args).output().expect("git is on the path");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write(at: &Path, path: &str, body: &str) {
    let full = at.join(path);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, body).unwrap();
}

/// A project, a mission `m1` on `mission/x` from `dev`, and its slot `one`
/// holding the agent's commit.
struct World {
    dir: tempfile::TempDir,
    project: Project,
    tree: PathBuf,
    header: Header,
    markers: PathBuf,
}

fn header() -> Header {
    Header {
        branch: "mission/x".into(),
        base: "dev".into(),
        lots: vec![Lot {
            id: "L1".into(),
            title: "one".into(),
        }],
        integration: Integration::None {
            reason: "none".into(),
        },
        security: Security::Gates,
        rigor: Default::default(),
        mutation_threshold: None,
        arbiter: None,
        run: None,
        account: None,
        model: None,
        bounds: Bounds::default(),
    }
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    let root = std::fs::canonicalize(&root).unwrap();
    git(&root, &["init", "-q", "-b", "dev"]);
    write(&root, "src.rs", "pub fn one() -> u8 {\n    1\n}\n");
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "base"]);

    // The slot, where `nunki::slot::find` looks: beside the repository.
    let tree = dir.path().join("repo-slots").join("one");
    git(
        dir.path(),
        &[
            "clone",
            "-q",
            "--no-hardlinks",
            &root.display().to_string(),
            &tree.display().to_string(),
        ],
    );
    git(&tree, &["checkout", "-q", "-b", "mission/x"]);
    // The agent's commit carries what a plant in the tree needs: attributes
    // that wire every file to a filter and a diff driver, and an ignored
    // file for `include.path` to point at.
    write(&tree, "src.rs", "pub fn one() -> u8 {\n    2\n}\n");
    write(&tree, ".gitattributes", "* filter=evil diff=evil\n");
    write(&tree, ".gitignore", "evil.config\n");
    git(&tree, &["add", "-A"]);
    git(&tree, &["commit", "-q", "-m", "the lot"]);

    let nunki_home = dir.path().join("nunki");
    let hq_root = nunki_home.join(nunki::project::HQ_DIR);
    for d in ["locks", "missions", "state/missions"] {
        std::fs::create_dir_all(hq_root.join(d)).unwrap();
    }
    let project = Project::at(
        root,
        Config {
            root: None,
            harness: "claude-code".into(),
            forge: vec![],
            stacks: vec!["rust".into()],
            protected_branches: vec!["main".into(), "dev".into()],
            protected_paths: ProtectedPaths::default(),
            account: None,
            model: None,
            bounds: Default::default(),
            credentials: None,
            run: None,
            services_file: None,
            permission_mode: "auto".to_string(),
            rigor: None,
            mutation_threshold: 80,
            mutation_jobs: None,
            forge_protection: Default::default(),
        },
        nunki_home,
    );
    let header = header();
    nunki::mission::dir::create(&hq_root, "m1", &header, "do it").unwrap();
    let head = git(&tree, &["rev-parse", "HEAD"]);
    let paths = nunki::mission::dir::Paths::of(&hq_root, "m1");
    std::fs::write(
        &paths.journal,
        format!("# Journal\n\n## ÉTAT DE REPRISE\n\nAt {head}.\n\nLot: L1 — done\n"),
    )
    .unwrap();
    Store::open(&hq_root)
        .unwrap()
        .save(&MissionState {
            id: "m1".into(),
            slot: "one".into(),
            flow: Flow::new(header.clone()).unwrap(),
            run: None,
            app: None,
            verdicts: Vec::new(),
            accepted: Vec::new(),
            stopped: None,
            harness_down: None,
            spent: Default::default(),
            spared: None,
            coder_session: None,
            pushed: None,
            updated_at: String::new(),
            revision: 0,
        })
        .unwrap();

    let markers = dir.path().join("markers");
    std::fs::create_dir_all(&markers).unwrap();
    World {
        dir,
        project,
        tree,
        header,
        markers,
    }
}

impl World {
    /// An executable outside the slot that leaves `markers/<name>` behind
    /// when anything runs it, and prints a file it is handed — what a
    /// textconv or an editor would.
    fn script(&self, name: &str) -> String {
        let bin = self.dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let path = bin.join(name);
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\ntouch '{}'\nif [ $# -eq 1 ] && [ -f \"$1\" ]; then cat \"$1\"; fi\nexit 0\n",
                self.markers.join(name).display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.display().to_string()
    }

    fn config(&self, key: &str, value: &str) {
        git(&self.tree, &["config", key, value]);
    }

    /// What ran: the markers left behind.
    fn ran(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.markers)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn slot(&self) -> nunki::slot::Slot {
        nunki::slot::find(&self.project, "one").unwrap()
    }

    fn paths(&self) -> nunki::mission::dir::Paths {
        nunki::mission::dir::Paths::of(&self.project.hq_root, "m1")
    }
}

/// Every hook git runs, on any verb.
const HOOKS: &[&str] = &[
    "applypatch-msg",
    "pre-applypatch",
    "post-applypatch",
    "pre-commit",
    "pre-merge-commit",
    "prepare-commit-msg",
    "commit-msg",
    "post-commit",
    "pre-rebase",
    "post-checkout",
    "post-merge",
    "pre-push",
    "pre-receive",
    "update",
    "proc-receive",
    "post-receive",
    "post-update",
    "reference-transaction",
    "push-to-checkout",
    "pre-auto-gc",
    "post-rewrite",
    "sendemail-validate",
    "fsmonitor-watchman",
    "post-index-change",
    "p4-changelist",
    "p4-prepare-changelist",
    "p4-post-changelist",
    "p4-pre-submit",
];

/// A plant: what an agent leaves in its slot's `.git`, by name.
struct Plant {
    name: &'static str,
    plant: fn(&World),
}

fn hooks_in(w: &World, dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    for hook in HOOKS {
        let script = w.script(&format!("hook-{hook}"));
        std::fs::copy(&script, dir.join(hook)).unwrap();
    }
}

const PLANTS: &[Plant] = &[
    Plant {
        name: "core.fsmonitor",
        plant: |w| w.config("core.fsmonitor", &w.script("fsmonitor")),
    },
    Plant {
        name: "core.hooksPath and every hook",
        plant: |w| {
            let dir = w.dir.path().join("hooks");
            hooks_in(w, &dir);
            w.config("core.hooksPath", &dir.display().to_string());
        },
    },
    Plant {
        name: "every hook in .git/hooks",
        plant: |w| hooks_in(w, &w.tree.join(".git/hooks")),
    },
    Plant {
        name: "a filter wired by .gitattributes",
        plant: |w| {
            w.config("filter.evil.clean", &w.script("filter-clean"));
            w.config("filter.evil.smudge", &w.script("filter-smudge"));
            w.config("filter.evil.process", &w.script("filter-process"));
            w.config("filter.evil.required", "true");
            write(&w.tree, ".git/info/attributes", "* filter=evil\n");
        },
    },
    Plant {
        name: "a textconv and diff.external",
        plant: |w| {
            w.config("diff.evil.textconv", &w.script("textconv"));
            w.config("diff.evil.command", &w.script("diff-command"));
            w.config("diff.external", &w.script("diff-external"));
        },
    },
    Plant {
        name: "aliases shadowing the verbs",
        plant: |w| {
            for verb in [
                "status",
                "rev-parse",
                "log",
                "diff",
                "fetch",
                "cat-file",
                "merge-base",
                "rev-list",
                "grep",
                "reset",
                "clean",
                "checkout",
                "hash-object",
                "ls-tree",
                "read-tree",
                "update-index",
            ] {
                w.config(&format!("alias.{verb}"), &format!("!{}", w.script("alias")));
            }
        },
    },
    Plant {
        name: "core.pager",
        plant: |w| {
            w.config("core.pager", &w.script("pager"));
            w.config("pager.log", &w.script("pager-log"));
            w.config("pager.status", &w.script("pager-status"));
        },
    },
    Plant {
        name: "core.editor",
        plant: |w| {
            w.config("core.editor", &w.script("editor"));
            w.config("sequence.editor", &w.script("sequence-editor"));
        },
    },
    Plant {
        name: "core.sshCommand and the remote's programs",
        plant: |w| {
            w.config("core.sshCommand", &w.script("ssh"));
            w.config("remote.origin.uploadpack", &w.script("uploadpack"));
            w.config("remote.origin.receivepack", &w.script("receivepack"));
        },
    },
    Plant {
        name: "credential.helper",
        plant: |w| w.config("credential.helper", &format!("!{}", w.script("credential"))),
    },
    Plant {
        name: "gpg.program",
        plant: |w| {
            w.config("gpg.program", &w.script("gpg"));
            w.config("gpg.ssh.program", &w.script("gpg-ssh"));
            w.config("commit.gpgSign", "true");
            w.config("log.showSignature", "true");
        },
    },
    Plant {
        name: "uploadpack.packObjectsHook",
        plant: |w| w.config("uploadpack.packObjectsHook", &w.script("pack-objects-hook")),
    },
    Plant {
        name: "an include.path towards a file in the tree",
        plant: |w| {
            std::fs::write(
                w.tree.join("evil.config"),
                format!(
                    "[core]\n\tfsmonitor = {}\n\tpager = {}\n",
                    w.script("include-fsmonitor"),
                    w.script("include-pager")
                ),
            )
            .unwrap();
            w.config("include.path", "../evil.config");
        },
    },
    Plant {
        name: "a replace ref",
        plant: |w| {
            // The agent's file made to read as the base's, and the agent's
            // commit as one with no parent.
            let base = git(&w.tree, &["rev-parse", "dev:src.rs"]);
            let mine = git(&w.tree, &["rev-parse", "HEAD:src.rs"]);
            git(&w.tree, &["replace", &mine, &base]);
            let orphan = git(&w.tree, &["commit-tree", "HEAD^{tree}", "-m", "the lot"]);
            git(&w.tree, &["replace", "HEAD", &orphan]);
        },
    },
    Plant {
        name: "a graft",
        plant: |w| {
            let head = git(&w.tree, &["rev-parse", "HEAD"]);
            write(&w.tree, ".git/info/grafts", &format!("{head}\n"));
        },
    },
];

/// What every host-side operation on a slot answered.
#[derive(Debug, PartialEq, Eq)]
struct Readings {
    gates: String,
    clean: bool,
    head: String,
    fork: String,
    touched: Vec<String>,
    fingerprint: String,
    not_attacked: Vec<String>,
    source: Option<String>,
    digest: Option<String>,
    changed: Vec<String>,
    unfetched: Vec<String>,
    fetched: String,
    branched: (String, String),
    back: (String, String),
    reset_discarded: bool,
    after_reset: (String, bool),
}

/// Play every operation `nunki` performs on a slot from the host: gates 1 to
/// 4 (and 8), `is_clean`, the fork point, the touched set, the fingerprint,
/// the verdict's head and what followed it, the registry's line read, the
/// campaign's record of what changed, `slot rm`'s question, `mission fetch`,
/// the launch's branching both ways, and `slot reset`.
fn play(w: &World) -> Readings {
    let slot = w.slot();
    let paths = w.paths();
    let report = nunki::gate::after_run(
        &nunki::gate::Subject {
            role: Role::Coder,
            tree: &slot.tree,
            journal: &paths.journal,
            pr: &paths.pr,
            verdict: &paths.verdict,
            mission_dir: &paths.dir,
            header: &w.header,
            protected_branches: &w.project.config.protected_branches,
            protected_paths: &w.project.config.protected_paths,
            coder_head: None,
        },
        &nunki::gate::Verification {
            project: &w.project,
            slot: &slot,
            engine: std::sync::Arc::new(nunki::engine::fake::FakeEngine::default()),
            stack: "rust",
        },
    )
    .unwrap();
    let clean = nunki::git::is_clean(&slot.tree).unwrap();
    let head = nunki::git::slot_head(&slot.tree).unwrap();
    let fork = nunki::gate::fork_point(&slot.tree, "dev").unwrap();
    let touched = nunki::gate::touched_since_base(&slot.tree, "dev").unwrap();
    let fingerprint = nunki::mutants::fingerprint(&slot.tree, &touched).unwrap();
    let not_attacked = nunki::push::not_attacked(&slot.tree, &fork, &head).unwrap();
    let source = nunki::equivalences::source_at(&slot.tree, &head, "src.rs");
    let digest = nunki::equivalences::digest_at(&slot.tree, &head, "src.rs", 2);
    let changed = nunki::mutants::changed_between(&slot.tree, &fork, &head)
        .unwrap()
        .into_iter()
        .collect();
    let unfetched = nunki::git::commits_not_in(&slot.tree, &w.project.root).unwrap();
    let fetched = nunki::push::fetch(&w.project, "m1").unwrap().head;

    let on = |slot: &nunki::slot::Slot| {
        (
            nunki::git::SlotGit::open(&slot.tree)
                .unwrap()
                .current_branch()
                .unwrap(),
            nunki::git::slot_head(&slot.tree).unwrap(),
        )
    };
    nunki::run::branch(&slot, &w.project.root, "mission/other", "dev").unwrap();
    let branched = on(&slot);
    nunki::run::branch(&slot, &w.project.root, "mission/x", "dev").unwrap();
    let back = on(&slot);

    let reset = nunki::slot::reset(&w.project, "one", "true", true).unwrap();
    let after_reset = (
        nunki::git::slot_head(&slot.tree).unwrap(),
        nunki::git::is_clean(&slot.tree).unwrap(),
    );
    Readings {
        gates: format!("{:?}", report.outcomes),
        clean,
        head,
        fork,
        touched,
        fingerprint,
        not_attacked,
        source,
        digest,
        changed,
        unfetched,
        fetched,
        branched,
        back,
        reset_discarded: reset.discarded,
        after_reset,
    }
}

/// The readings of a slot nobody planted anything in, checked against what
/// its commits hold by construction.
fn baseline() -> Readings {
    let w = world();
    let truth_head = git(&w.tree, &["rev-parse", "HEAD"]);
    let truth_fork = git(&w.tree, &["rev-parse", "dev"]);
    let readings = play(&w);
    assert_eq!(readings.head, truth_head);
    assert_eq!(readings.fork, truth_fork);
    assert_eq!(readings.touched, [".gitattributes", ".gitignore", "src.rs"]);
    assert_eq!(
        readings.source.as_deref(),
        Some("pub fn one() -> u8 {\n    2\n}\n")
    );
    assert_eq!(readings.not_attacked.len(), 1, "{readings:?}");
    assert!(readings.clean, "{readings:?}");
    assert!(
        readings.gates.matches("Passed").count() >= 4,
        "{}",
        readings.gates
    );
    assert_eq!(readings.fetched, truth_head);
    assert_eq!(readings.branched, ("mission/other".to_string(), truth_fork));
    assert_eq!(readings.back, ("mission/x".to_string(), truth_head.clone()));
    assert_eq!(readings.after_reset, (truth_head, true));
    assert!(w.ran().is_empty());
    readings
}

#[test]
fn nothing_an_agent_plants_in_its_slot_is_run_or_believed_by_the_host() {
    let expected = baseline();
    for plant in PLANTS {
        let w = world();
        (plant.plant)(&w);
        let readings = play(&w);
        assert_eq!(
            w.ran(),
            Vec::<String>::new(),
            "with {}, the host ran what the slot planted",
            plant.name
        );
        assert_eq!(
            readings, expected,
            "with {}, a read did not return what the commits hold",
            plant.name
        );
    }
}

/// The control: each plant does what it says to a plain git in the slot. A
/// plant that would do nothing anyway proves nothing about the mirror.
#[test]
fn every_plant_is_live_for_a_plain_git_in_the_slot() {
    let live = |name: &str, play: &dyn Fn(&World), marker: &str| {
        let w = world();
        let plant = PLANTS.iter().find(|p| p.name == name).unwrap();
        (plant.plant)(&w);
        play(&w);
        assert!(
            w.ran().iter().any(|m| m.starts_with(marker)),
            "{name}: a plain git did not run {marker}; ran {:?}",
            w.ran()
        );
    };
    let touch_src = |w: &World| write(&w.tree, "src.rs", "pub fn one() -> u8 {\n    3\n}\n");

    live(
        "core.fsmonitor",
        &|w| {
            attempt(&w.tree, &["status", "--porcelain"]);
        },
        "fsmonitor",
    );
    for name in ["core.hooksPath and every hook", "every hook in .git/hooks"] {
        live(
            name,
            &|w| {
                attempt(&w.tree, &["commit", "-q", "--allow-empty", "-m", "x"]);
            },
            "hook-pre-commit",
        );
        live(
            name,
            &|w| {
                attempt(&w.tree, &["checkout", "-q", "dev"]);
            },
            "hook-post-checkout",
        );
    }
    live(
        "a filter wired by .gitattributes",
        &|w| {
            touch_src(w);
            attempt(&w.tree, &["add", "src.rs"]);
        },
        "filter-process",
    );
    live(
        "a textconv and diff.external",
        &|w| {
            attempt(&w.tree, &["diff", "dev", "HEAD"]);
        },
        "diff-",
    );
    live(
        "a textconv and diff.external",
        &|w| {
            attempt(&w.tree, &["log", "-p", "--no-ext-diff", "-1"]);
        },
        "textconv",
    );
    // git starts a pager only on a terminal, which a test has not: what it
    // would start is what it names.
    {
        let w = world();
        (PLANTS
            .iter()
            .find(|p| p.name == "core.pager")
            .unwrap()
            .plant)(&w);
        assert_eq!(
            git(&w.tree, &["var", "GIT_PAGER"]),
            w.dir.path().join("bin/pager").display().to_string()
        );
    }
    live(
        "core.editor",
        &|w| {
            attempt(&w.tree, &["commit", "-q", "--allow-empty"]);
        },
        "editor",
    );
    live(
        "core.sshCommand and the remote's programs",
        &|w| {
            attempt(&w.tree, &["ls-remote", "ssh://example.invalid/x"]);
        },
        "ssh",
    );
    live(
        "core.sshCommand and the remote's programs",
        &|w| {
            attempt(&w.tree, &["fetch", "-q", "origin"]);
        },
        "uploadpack",
    );
    live(
        "credential.helper",
        &|w| {
            let mut git = plain(&w.tree);
            git.args(["credential", "fill"])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            let mut child = git.spawn().unwrap();
            use std::io::Write;
            let _ = child
                .stdin
                .take()
                .unwrap()
                .write_all(b"protocol=https\nhost=example.invalid\nusername=u\n\n");
            let _ = child.wait();
        },
        "credential",
    );
    live(
        "gpg.program",
        &|w| {
            attempt(&w.tree, &["commit", "-q", "--allow-empty", "-m", "x"]);
        },
        "gpg",
    );
    live(
        "an include.path towards a file in the tree",
        &|w| {
            attempt(&w.tree, &["status", "--porcelain"]);
        },
        "include-fsmonitor",
    );

    // What changes a read rather than running anything.
    let w = world();
    let mine = git(&w.tree, &["show", "HEAD:src.rs"]);
    (PLANTS
        .iter()
        .find(|p| p.name == "a replace ref")
        .unwrap()
        .plant)(&w);
    assert_ne!(git(&w.tree, &["show", "HEAD:src.rs"]), mine);
    assert_eq!(git(&w.tree, &["rev-list", "--count", "HEAD"]), "1");

    let w = world();
    assert_eq!(git(&w.tree, &["rev-list", "--count", "HEAD"]), "2");
    (PLANTS.iter().find(|p| p.name == "a graft").unwrap().plant)(&w);
    assert_eq!(git(&w.tree, &["rev-list", "--count", "HEAD"]), "1");
}

/// Two plants git ignores in a repository's own configuration, said so the
/// list above is read for what it is: `uploadpack.packObjectsHook` is
/// honoured only from protected configuration, and an alias never shadows
/// a built-in verb. They are planted all the same — git may change its
/// mind, and the mirror reads neither.
#[test]
fn the_plants_git_already_ignores_are_ignored_by_a_plain_git_too() {
    let w = world();
    for name in ["uploadpack.packObjectsHook", "aliases shadowing the verbs"] {
        (PLANTS.iter().find(|p| p.name == name).unwrap().plant)(&w);
    }
    attempt(&w.tree, &["status"]);
    let elsewhere = w.dir.path().join("elsewhere");
    attempt(
        w.dir.path(),
        &[
            "clone",
            "-q",
            "--no-local",
            &w.tree.display().to_string(),
            &elsewhere.display().to_string(),
        ],
    );
    assert!(
        elsewhere.join("src.rs").is_file(),
        "the clone did not happen"
    );
    assert_eq!(w.ran(), Vec::<String>::new());
}

/// A change made to a slot, by name: a way to break or to doctor it.
type Break = (&'static str, fn(&World));

/// A slot whose `.git` the host cannot read is an error that names the
/// slot and says nothing ran — never a plain git in it after all. Each
/// world carries a live `core.fsmonitor`, so a fallback would show.
#[test]
fn a_broken_slot_is_an_error_and_nothing_runs_in_it() {
    let breaks: &[Break] = &[
        ("an object missing", |w| {
            let blob = git(&w.tree, &["rev-parse", "HEAD:src.rs"]);
            std::fs::remove_file(
                w.tree
                    .join(".git/objects")
                    .join(&blob[..2])
                    .join(&blob[2..]),
            )
            .unwrap();
        }),
        ("an object rewritten", |w| {
            let blob = git(&w.tree, &["rev-parse", "HEAD:src.rs"]);
            let other = git(&w.tree, &["rev-parse", "dev:src.rs"]);
            let base = git(&w.tree, &["cat-file", "blob", &other]);
            // The base's blob is packed; write it loose to copy it over.
            let mut hash = plain(&w.tree);
            hash.args(["hash-object", "-w", "--stdin"])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null());
            let mut child = hash.spawn().unwrap();
            use std::io::Write;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(format!("{base}\n").as_bytes())
                .unwrap();
            child.wait().unwrap();
            let loose = |id: &str| w.tree.join(".git/objects").join(&id[..2]).join(&id[2..]);
            let target = loose(&blob);
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();
            std::fs::copy(loose(&other), &target).unwrap();
        }),
        ("HEAD that names no commit", |w| {
            std::fs::write(w.tree.join(".git/HEAD"), "garbage\n").unwrap();
        }),
        ("a branch holding no id", |w| {
            std::fs::write(w.tree.join(".git/refs/heads/mission/x"), "not an id\n").unwrap();
        }),
        (".git as a file", |w| {
            std::fs::rename(w.tree.join(".git"), w.dir.path().join("moved.git")).unwrap();
            std::fs::write(
                w.tree.join(".git"),
                format!("gitdir: {}\n", w.dir.path().join("moved.git").display()),
            )
            .unwrap();
        }),
        (".git/HEAD as a link", |w| {
            let target = w.dir.path().join("HEAD");
            std::fs::rename(w.tree.join(".git/HEAD"), &target).unwrap();
            std::os::unix::fs::symlink(&target, w.tree.join(".git/HEAD")).unwrap();
        }),
    ];
    for (name, break_it) in breaks {
        let w = world();
        w.config("core.fsmonitor", &w.script("fsmonitor"));
        break_it(&w);
        let err = nunki::git::is_clean(&w.tree).unwrap_err();
        assert!(
            matches!(err, nunki::git::GitError::Unreadable { .. }),
            "{name}: {err:?}"
        );
        let said = err.to_string();
        assert!(
            said.contains(&w.tree.display().to_string()),
            "{name}: {said}"
        );
        assert!(
            said.contains("nunki ran no git in that slot"),
            "{name}: {said}"
        );
        assert!(nunki::git::slot_head(&w.tree).is_err(), "{name}");
        assert!(
            nunki::slot::reset(&w.project, "one", "true", true).is_err(),
            "{name}"
        );
        assert!(nunki::push::fetch(&w.project, "m1").is_err(), "{name}");
        assert_eq!(w.ran(), Vec::<String>::new(), "{name}");
    }
}

/// What `nunki` writes into a slot it writes as files, and never through a
/// link the agent left in its `.git`: the host's write would land wherever
/// the link points.
#[test]
fn a_link_in_the_slots_git_is_never_written_through() {
    let w = world();
    // The base moves, so a new branch from it needs objects the slot lacks.
    write(
        &w.project.root,
        "src.rs",
        "pub fn one() -> u8 {\n    9\n}\n",
    );
    git(&w.project.root, &["commit", "-qam", "the base moves"]);
    // The slot's packs, moved out and linked back: git reads through the
    // link, and the host must not write through it.
    let outside = w.dir.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let pack = w.tree.join(".git/objects/pack");
    std::fs::rename(&pack, outside.join("pack")).unwrap();
    std::os::unix::fs::symlink(outside.join("pack"), &pack).unwrap();
    let before = std::fs::read_dir(outside.join("pack")).unwrap().count();

    let err = nunki::run::branch(&w.slot(), &w.project.root, "mission/other", "dev").unwrap_err();
    assert!(
        err.to_string().contains("nunki writes nothing through it"),
        "{err}"
    );
    assert_eq!(
        std::fs::read_dir(outside.join("pack")).unwrap().count(),
        before,
        "the host wrote through the agent's link"
    );
}

/// The mirror is the host's and nowhere an agent reaches: beside the slot,
/// never inside the tree its container mounts.
#[test]
fn the_mirror_lives_beside_the_slot_and_never_in_its_tree() {
    let w = world();
    let repo = nunki::git::SlotGit::open(&w.tree).unwrap();
    assert!(!repo.mirror().starts_with(&w.tree));
    assert_eq!(repo.mirror(), nunki::git::mirror_of(&w.tree));
    assert!(repo.mirror().join("HEAD").is_file());
    // A slot removed takes its mirror with it.
    nunki::slot::rm(&w.project, "one", true).unwrap();
    assert!(!nunki::git::mirror_of(&w.tree).exists());
}

/// Every plain git `nunki` spawns, by file and line: none of them on a slot.
///
/// The human's own repository is run as it is ([`nunki::git::run`]); a slot
/// only through [`nunki::git::SlotGit`]. A new plain call anywhere fails
/// this test until it is read and added here — or routed through the
/// mirror, if what it runs on is a slot.
#[test]
fn no_plain_git_runs_on_a_slot() {
    // (file, the call's line, trimmed, and what it runs on.)
    let allowed: &[(&str, &str)] = &[
        // `nunki slot add`: the clone of the human's repository, before any
        // agent has been in it.
        ("src/slot.rs", "git::run_anywhere(&["),
        // The human's identity, from their repository.
        (
            "src/human.rs",
            "if let Ok(name) = crate::git::run(at, &[\"config\", \"user.name\"]) {",
        ),
        (
            "src/human.rs",
            "let email = crate::git::run(at, &[\"config\", \"user.email\"])",
        ),
        // `--registry`: the human's repository at its own HEAD.
        ("src/equivalences.rs", "let out = Command::new(\"git\")"),
        // The project's remote, for `check`.
        (
            "src/check.rs",
            "let Ok(remote) = crate::git::run(&project.root, &[\"remote\", \"get-url\", crate::push::REMOTE])",
        ),
        // `nunki init`'s interview, in the human's repository.
        (
            "src/interview.rs",
            "let forge = crate::git::run(root, &[\"remote\", \"get-url\", \"origin\"])",
        ),
        (
            "src/interview.rs",
            "let has = |r: &str| crate::git::run(root, &[\"rev-parse\", \"--verify\", \"--quiet\", r]).is_ok();",
        ),
        // Finding the project from where `nunki` was started.
        (
            "src/project/mod.rs",
            "match crate::git::run(dir, &[\"rev-parse\", \"--show-toplevel\"]) {",
        ),
        // `mission fetch` and `push`, in the human's repository; the fetch
        // reads the slot's mirror, never the slot.
        (
            "src/push.rs",
            "if crate::git::current_branch(&project.root)? == branch {",
        ),
        ("src/push.rs", "let before = crate::git::run("),
        ("src/push.rs", "crate::git::run("),
        (
            "src/push.rs",
            "let after = crate::git::head_of(&project.root, &branch)?;",
        ),
        (
            "src/push.rs",
            "crate::git::run(&project.root, &[\"push\", REMOTE, &fetched.branch])?;",
        ),
        (
            "src/push.rs",
            "crate::git::run(&project.root, &[\"remote\", \"get-url\", REMOTE])",
        ),
    ];
    let plain = [
        "git::run(",
        "git::run_anywhere(",
        "git::head(",
        "git::current_branch(",
        "git::head_of(",
        "Command::new(\"git\")",
    ];
    let mut found: Vec<(String, String)> = Vec::new();
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![src.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|x| x != "rs") {
                continue;
            }
            let file = path
                .strip_prefix(env!("CARGO_MANIFEST_DIR"))
                .unwrap()
                .display()
                .to_string();
            // The mechanism itself: where the plain git and the mirror's
            // are both defined.
            if file == "src/git.rs" {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            for line in text.lines() {
                let line = line.trim();
                if line.starts_with("//") {
                    continue;
                }
                if plain.iter().any(|p| line.contains(p)) {
                    found.push((file.clone(), line.to_string()));
                }
            }
        }
    }
    found.sort();
    let mut allowed: Vec<(String, String)> = allowed
        .iter()
        .map(|(f, l)| (f.to_string(), l.to_string()))
        .collect();
    allowed.sort();
    assert_eq!(
        found, allowed,
        "a plain git call was added or removed; a slot goes through nunki::git::SlotGit"
    );
}

/// The slot's refs are read from its files as git stores them: packed,
/// loose over packed, a detached `HEAD`, and a new commit seen at once.
#[test]
fn the_mirror_reads_the_slots_refs_as_git_stores_them() {
    let w = world();
    let first = git(&w.tree, &["rev-parse", "HEAD"]);
    git(&w.tree, &["pack-refs", "--all"]);
    assert!(!w.tree.join(".git/refs/heads/mission/x").exists());
    assert_eq!(nunki::git::slot_head(&w.tree).unwrap(), first);

    // A commit writes a loose ref over the packed one, and the mirror
    // follows it.
    write(&w.tree, "src.rs", "pub fn one() -> u8 {\n    4\n}\n");
    git(&w.tree, &["commit", "-qam", "again"]);
    let second = git(&w.tree, &["rev-parse", "HEAD"]);
    assert_ne!(first, second);
    assert_eq!(nunki::git::slot_head(&w.tree).unwrap(), second);
    assert_eq!(
        nunki::git::on_slot(&w.tree, &["rev-list", "--count", "HEAD"]).unwrap(),
        "3"
    );

    // Detached, `HEAD` names the commit and no branch.
    git(&w.tree, &["checkout", "-q", "--detach", &first]);
    let repo = nunki::git::SlotGit::open(&w.tree).unwrap();
    assert_eq!(repo.head().unwrap(), first);
    assert_eq!(repo.current_branch().unwrap(), "HEAD");
    // The branch it left is still there, at its own commit.
    assert_eq!(repo.run(&["rev-parse", "mission/x"]).unwrap(), second);
}

/// A reset leaves the slot as its own git sees it clean: the tree at
/// `HEAD`, nothing untracked or ignored, and an index that says so — the
/// one `nunki` built, since no git ran there to write its own.
#[test]
fn a_reset_leaves_a_slot_its_own_git_finds_clean() {
    let w = world();
    write(&w.tree, "src.rs", "changed\n");
    write(&w.tree, "untracked.rs", "new\n");
    write(&w.tree, "evil.config", "ignored\n");
    git(&w.tree, &["add", "src.rs"]);
    assert!(!nunki::git::is_clean(&w.tree).unwrap());

    let reset = nunki::slot::reset(&w.project, "one", "true", true).unwrap();
    assert!(reset.discarded);
    assert!(nunki::git::is_clean(&w.tree).unwrap());
    assert!(!w.tree.join("untracked.rs").exists());
    assert!(
        !w.tree.join("evil.config").exists(),
        "-x removes the ignored"
    );
    assert_eq!(
        std::fs::read_to_string(w.tree.join("src.rs")).unwrap(),
        "pub fn one() -> u8 {\n    2\n}\n"
    );
    assert_eq!(git(&w.tree, &["status", "--porcelain"]), "");
}

/// A slot whose `HEAD` names a branch with no commit yet is an error, not
/// an empty answer: there is nothing to judge.
#[test]
fn a_slot_on_an_unborn_branch_is_an_error() {
    let w = world();
    std::fs::write(w.tree.join(".git/HEAD"), "ref: refs/heads/nothing-yet\n").unwrap();
    assert!(nunki::git::slot_head(&w.tree).is_err());
    assert!(nunki::git::is_clean(&w.tree).is_err());
}

/// Gate 1 as the coder's gates play it.
fn clean_tree_gate(w: &World) -> nunki::gate::Decision {
    let slot = w.slot();
    let paths = w.paths();
    nunki::gate::after_run(
        &nunki::gate::Subject {
            role: Role::Coder,
            tree: &slot.tree,
            journal: &paths.journal,
            pr: &paths.pr,
            verdict: &paths.verdict,
            mission_dir: &paths.dir,
            header: &w.header,
            protected_branches: &w.project.config.protected_branches,
            protected_paths: &w.project.config.protected_paths,
            coder_head: None,
        },
        &nunki::gate::Verification {
            project: &w.project,
            slot: &slot,
            engine: std::sync::Arc::new(nunki::engine::fake::FakeEngine::default()),
            stack: "rust",
        },
    )
    .unwrap()
    .outcomes
    .into_iter()
    .find(|o| o.gate == nunki::gate::Gate::CleanTree)
    .unwrap()
    .decision
}

/// A file's modification time, set back to a fixed past.
fn age(path: &Path) {
    let past = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(past)
        .unwrap();
}

/// A slot's index, or its configuration, doctored so that the slot's own
/// git sees nothing to commit, still leaves a tree that is "not clean": the
/// question is answered by `nunki`'s comparison of the tree with `HEAD`, not
/// by what the slot says (SPEC 4.4, gate 1).
#[test]
fn a_slot_index_doctored_to_hide_a_change_is_still_not_clean() {
    let changed = "pub fn one() -> u8 {\n    7\n}\n";
    let doctorings: &[Break] = &[
        ("assume-unchanged", |w| {
            write(&w.tree, "src.rs", "pub fn one() -> u8 {\n    7\n}\n");
            git(&w.tree, &["update-index", "--assume-unchanged", "src.rs"]);
        }),
        ("skip-worktree", |w| {
            write(&w.tree, "src.rs", "pub fn one() -> u8 {\n    7\n}\n");
            git(&w.tree, &["update-index", "--skip-worktree", "src.rs"]);
        }),
        ("a stat the slot no longer checks", |w| {
            let file = w.tree.join("src.rs");
            age(&file);
            git(&w.tree, &["update-index", "--refresh"]);
            // nunki's own index is warm too: it must not trust it either.
            assert!(nunki::git::is_clean(&w.tree).unwrap());
            w.config("core.checkStat", "minimal");
            w.config("core.trustctime", "false");
            std::fs::write(&file, "pub fn one() -> u8 {\n    7\n}\n").unwrap();
            age(&file);
        }),
        ("an untracked file excluded in .git/info", |w| {
            write(&w.tree, ".git/info/exclude", "hidden.rs\n");
            write(&w.tree, "hidden.rs", "pub fn hidden() {}\n");
        }),
        ("an untracked file excluded by core.excludesFile", |w| {
            let list = w.dir.path().join("excluded");
            std::fs::write(&list, "hidden.rs\n").unwrap();
            w.config("core.excludesFile", &list.display().to_string());
            write(&w.tree, "hidden.rs", "pub fn hidden() {}\n");
        }),
    ];
    for (name, doctor) in doctorings {
        let w = world();
        doctor(&w);
        // The control: the slot's own git is fooled.
        assert_eq!(git(&w.tree, &["status", "--porcelain"]), "", "{name}");

        assert!(!nunki::git::is_clean(&w.tree).unwrap(), "{name}");
        assert!(
            matches!(clean_tree_gate(&w), nunki::gate::Decision::Failed(_)),
            "{name}"
        );
        if !name.contains("untracked") {
            assert_eq!(
                std::fs::read_to_string(w.tree.join("src.rs")).unwrap(),
                changed
            );
        }
    }
}

/// The `nunki check` line for slots.
fn slots_line(w: &World) -> nunki::check::Verdict {
    nunki::check::run(&w.project)
        .checks
        .into_iter()
        .find(|c| c.what == "the host runs no git inside a slot")
        .expect("check reports the line")
        .verdict
}

/// `nunki check` says the host runs no git inside a slot: green on a slot
/// nobody planted anything in, and, on one whose `.git/config` carries keys
/// git would execute, a line that names each key and says nothing ran.
#[test]
fn check_is_green_on_a_fresh_slot_and_names_a_planted_key() {
    let w = world();
    match slots_line(&w) {
        nunki::check::Verdict::Green(said) => {
            assert!(said.contains("one at "), "{said}");
            assert!(said.contains(".nunki-git"), "{said}");
        }
        other => panic!("a fresh slot: {other:?}"),
    }

    w.config("core.fsmonitor", &w.script("fsmonitor"));
    w.config("filter.evil.clean", &w.script("filter-clean"));
    w.config("include.path", "../evil.config");
    match slots_line(&w) {
        nunki::check::Verdict::Amber(said) => {
            assert!(said.contains("slot one's .git/config"), "{said}");
            assert!(
                said.contains("core.fsmonitor, filter.evil.clean, include.path"),
                "{said}"
            );
            assert!(said.contains("nunki ran none of them"), "{said}");
        }
        other => panic!("a planted slot: {other:?}"),
    }
    assert_eq!(w.ran(), Vec::<String>::new());
}

/// A slot the host cannot read is said by `check` too, and a project with no
/// slot has nothing to hold yet.
#[test]
fn check_names_a_slot_the_host_cannot_read_and_holds_with_none() {
    let w = world();
    std::fs::write(w.tree.join(".git/HEAD"), "garbage\n").unwrap();
    match slots_line(&w) {
        nunki::check::Verdict::Amber(said) => {
            assert!(said.contains("slot one:"), "{said}");
            assert!(said.contains("cannot be read from the host"), "{said}");
        }
        other => panic!("a broken slot: {other:?}"),
    }

    nunki::slot::rm(&w.project, "one", true).unwrap();
    assert!(matches!(
        slots_line(&w),
        nunki::check::Verdict::Green(said) if said.contains("no slot yet")
    ));
}

/// Which keys of a slot's configuration are named as executable, and which
/// are not: everything git runs, or reads another file for, and nothing it
/// only reads as a value.
#[test]
fn the_keys_named_are_the_ones_git_would_execute() {
    let w = world();
    let executable = [
        "alias.st",
        "browser.x.cmd",
        "core.alternaterefscommand",
        "core.askpass",
        "core.editor",
        "core.fsmonitor",
        "core.gitproxy",
        "core.hookspath",
        "core.pager",
        "core.sshcommand",
        "credential.helper",
        "credential.https://example.com.helper",
        "diff.e.command",
        "diff.e.textconv",
        "diff.external",
        "difftool.x.cmd",
        "filter.e.clean",
        "filter.e.process",
        "filter.e.smudge",
        "gpg.program",
        "gpg.ssh.program",
        "include.path",
        "includeif.gitdir:/x/.path",
        "man.x.cmd",
        "merge.e.driver",
        "mergetool.x.cmd",
        "pager.log",
        "protocol.allow",
        "protocol.ext.allow",
        "remote.origin.receivepack",
        "remote.origin.uploadpack",
        "sequence.editor",
        "uploadpack.packobjectshook",
        "web.browser",
    ];
    let inert = [
        "credential.username",
        "diff.e.binary",
        "diff.textconv",
        "filter.clean",
        "filter.e.required",
        "gpg.format",
        "includeif.path",
        "merge.e.name",
        "protocol.version",
        "remote.origin.pushurl",
        "uploadpack.allowfilter",
        "user.name",
        "x.y",
        "credential.https://example.com.username",
        "gpg.ssh.allowedsignersfile",
        "difftool.cmd",
        "difftool.x.path",
        "mergetool.x.path",
        "man.x.path",
        "browser.x.path",
        "protocol.ext.other",
    ];
    for key in executable.iter().chain(inert.iter()) {
        git(&w.tree, &["config", "--add", key, "v"]);
    }
    let mut expected: Vec<String> = executable.iter().map(|k| k.to_string()).collect();
    expected.sort();
    assert_eq!(nunki::git::executable_keys(&w.tree).unwrap(), expected);
    // A fresh clone's configuration names nothing.
    let fresh = world();
    assert_eq!(
        nunki::git::executable_keys(&fresh.tree).unwrap(),
        Vec::<String>::new()
    );
}

/// What `executable_keys` makes of a configuration it cannot read as one:
/// none at all is nothing to name, and anything but a regular file — a
/// `.git` that is a file, a config that is a link — is an error, never a
/// file followed.
#[test]
fn a_config_that_is_not_a_plain_file_is_never_followed() {
    let w = world();
    std::fs::remove_file(w.tree.join(".git/config")).unwrap();
    assert_eq!(
        nunki::git::executable_keys(&w.tree).unwrap(),
        Vec::<String>::new()
    );

    let elsewhere = w.dir.path().join("elsewhere.config");
    std::fs::write(&elsewhere, "[core]\n\tfsmonitor = /x\n").unwrap();
    std::os::unix::fs::symlink(&elsewhere, w.tree.join(".git/config")).unwrap();
    let err = nunki::git::executable_keys(&w.tree).unwrap_err();
    assert!(
        matches!(err, nunki::git::GitError::Unreadable { .. }),
        "{err:?}"
    );

    let w = world();
    std::fs::rename(w.tree.join(".git"), w.dir.path().join("moved.git")).unwrap();
    std::fs::write(w.tree.join(".git"), "gitdir: elsewhere\n").unwrap();
    assert!(nunki::git::executable_keys(&w.tree).is_err());
}

/// The slot's files say what is wrong with them, each in its own words:
/// the error a human reads names the fault, not git's reaction to it.
#[test]
fn a_slot_read_from_its_files_says_what_is_wrong() {
    let fault = |break_it: &dyn Fn(&World), said: &str| {
        let w = world();
        break_it(&w);
        let err = nunki::git::SlotGit::open(&w.tree).unwrap_err().to_string();
        assert!(err.contains(said), "wanted {said:?}, got {err}");
    };
    let id = |w: &World| git(&w.tree, &["rev-parse", "HEAD"]);
    fault(
        &|w| {
            std::fs::rename(w.tree.join(".git"), w.dir.path().join("moved.git")).unwrap();
            std::fs::write(w.tree.join(".git"), "gitdir: elsewhere\n").unwrap();
        },
        "a slot is a plain clone",
    );
    fault(
        &|w| std::fs::write(w.tree.join(".git/HEAD"), "ref: refs/remotes/origin/dev\n").unwrap(),
        "which is not a branch",
    );
    fault(
        &|w| std::fs::write(w.tree.join(".git/HEAD"), "garbage\n").unwrap(),
        "which is no commit id",
    );
    fault(
        &|w| {
            std::fs::write(
                w.tree.join(".git/refs/heads/mission/x"),
                format!("{}\n", "z".repeat(40)),
            )
            .unwrap()
        },
        "which is no object id",
    );
    fault(
        &|w| {
            std::fs::write(
                w.tree.join(".git/refs/heads/mission/x"),
                format!("{}\n", &id(w)[..20]),
            )
            .unwrap()
        },
        "which is no object id",
    );
    fault(
        &|w| std::fs::write(w.tree.join(".git/refs/heads/a b"), format!("{}\n", id(w))).unwrap(),
        "a ref is named",
    );
    fault(
        &|w| {
            std::fs::write(
                w.tree.join(".git/refs/heads/long"),
                format!("{}\n", "a".repeat(64)),
            )
            .unwrap()
        },
        "mix SHA-1 and SHA-256",
    );
}

/// A refs directory the host cannot list is an error, never a slot read as
/// if it had no branch there.
#[test]
fn a_refs_directory_that_cannot_be_listed_is_an_error() {
    let w = world();
    let heads = w.tree.join(".git/refs/heads");
    std::fs::set_permissions(&heads, std::fs::Permissions::from_mode(0o000)).unwrap();
    let opened = nunki::git::SlotGit::open(&w.tree);
    std::fs::set_permissions(&heads, std::fs::Permissions::from_mode(0o755)).unwrap();
    let err = opened.unwrap_err().to_string();
    assert!(err.contains("Permission denied"), "{err}");
}

/// An annotated tag, packed, leaves a peeled line in `packed-refs`: read as
/// what it is, not as a ref.
#[test]
fn a_packed_annotated_tag_is_read_with_its_peeled_line() {
    let w = world();
    git(&w.tree, &["tag", "-a", "v1", "-m", "one"]);
    git(&w.tree, &["pack-refs", "--all"]);
    let packed = std::fs::read_to_string(w.tree.join(".git/packed-refs")).unwrap();
    assert!(packed.lines().any(|l| l.starts_with('^')), "{packed}");
    let repo = nunki::git::SlotGit::open(&w.tree).unwrap();
    assert_eq!(
        repo.run(&["rev-parse", "v1^{commit}"]).unwrap(),
        git(&w.tree, &["rev-parse", "HEAD"])
    );
}

/// A git that fails in the mirror says which slot it was about.
#[test]
fn a_failure_in_the_mirror_names_the_slot() {
    let w = world();
    let err = nunki::git::on_slot(&w.tree, &["rev-parse", "--verify", "no-such-thing"])
        .unwrap_err()
        .to_string();
    assert!(
        err.contains(&format!("{} (through its host mirror)", w.tree.display())),
        "{err}"
    );
}

/// What a new branch hands the slot is what the slot lacks, and nothing
/// it already holds: one pack, with the base's new commit, tree and blob.
#[test]
fn a_new_branch_hands_the_slot_only_the_objects_it_lacks() {
    let w = world();
    write(
        &w.project.root,
        "src.rs",
        "pub fn one() -> u8 {\n    9\n}\n",
    );
    git(&w.project.root, &["commit", "-qam", "the base moves"]);
    let packs = w.tree.join(".git/objects/pack");
    let before: Vec<PathBuf> = std::fs::read_dir(&packs)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    nunki::run::branch(&w.slot(), &w.project.root, "mission/other", "dev").unwrap();
    let new: Vec<PathBuf> = std::fs::read_dir(&packs)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| !before.contains(p) && p.extension().is_some_and(|x| x == "idx"))
        .collect();
    assert_eq!(new.len(), 1, "{new:?}");
    let out = Command::new("git")
        .args(["show-index"])
        .stdin(std::fs::File::open(&new[0]).unwrap())
        .output()
        .unwrap();
    let objects = String::from_utf8_lossy(&out.stdout).lines().count();
    assert_eq!(objects, 3, "the pack carries objects the slot already had");
}

/// The mirror forgets what it last synced before it fetches again: a sync
/// that cannot clear its record fetches nothing.
#[test]
fn a_sync_that_cannot_clear_its_record_fetches_nothing() {
    let w = world();
    let mirror = nunki::git::SlotGit::open(&w.tree)
        .unwrap()
        .mirror()
        .to_path_buf();
    let before = git(&mirror, &["rev-parse", "refs/heads/mission/x"]);
    let record = mirror.join("nunki-synced");
    std::fs::remove_file(&record).unwrap();
    std::fs::create_dir(&record).unwrap();
    write(&w.tree, "src.rs", "pub fn one() -> u8 {\n    5\n}\n");
    git(&w.tree, &["commit", "-qam", "more"]);

    assert!(nunki::git::SlotGit::open(&w.tree).is_err());
    assert_eq!(
        git(&mirror, &["rev-parse", "refs/heads/mission/x"]),
        before,
        "the mirror fetched while its record still said otherwise"
    );
}

/// A commit asked for by id is fetched only when the mirror lacks it, and
/// a name that is not an id is never fetched at all.
#[test]
fn only_a_commit_the_mirror_lacks_is_fetched_by_its_id() {
    let w = world();
    let repo = nunki::git::SlotGit::open(&w.tree).unwrap();
    let head = repo.head().unwrap();
    repo.fetch_commit(&head).unwrap();
    repo.fetch_commit("not-an-id").unwrap();
    assert_eq!(
        repo.run(&["for-each-ref", "refs/nunki/kept"]).unwrap(),
        "",
        "a commit the mirror had was fetched again"
    );
    // One no ref reaches is fetched, and kept.
    let dangling = git(&w.tree, &["commit-tree", "HEAD^{tree}", "-m", "dangling"]);
    repo.fetch_commit(&dangling).unwrap();
    assert_eq!(
        repo.run(&["rev-parse", &format!("refs/nunki/kept/{dangling}")])
            .unwrap(),
        dangling
    );
}

/// A file in the tree shaped like a configuration — whatever its name —
/// is never read as one by a git the host runs with the tree as work tree.
#[test]
fn a_config_file_in_the_tree_is_never_read() {
    let w = world();
    let body = format!("[core]\n\tfsmonitor = {}\n", w.script("tree-config"));
    for name in ["xyzzy", ".gitconfig", "config", "NUL"] {
        std::fs::write(w.tree.join(name), &body).unwrap();
    }
    assert!(!nunki::git::is_clean(&w.tree).unwrap());
    assert_eq!(w.ran(), Vec::<String>::new());
}

/// Host-side readers of one slot may run at once, and each leaves nothing
/// of its own behind in the mirror.
#[test]
fn readers_of_one_slot_run_at_once_and_leave_nothing_behind() {
    let w = world();
    assert!(nunki::git::is_clean(&w.tree).unwrap());
    let readers: Vec<_> = (0..8)
        .map(|_| {
            let tree = w.tree.clone();
            std::thread::spawn(move || {
                (0..5)
                    .map(|_| nunki::git::is_clean(&tree).map_err(|e| e.to_string()))
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    for reader in readers {
        for answer in reader.join().unwrap() {
            assert_eq!(answer, Ok(true));
        }
    }
    let mirror = nunki::git::mirror_of(&w.tree);
    let left: Vec<String> = std::fs::read_dir(&mirror)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("index") || n.starts_with("shim") || n.starts_with("pack-"))
        .collect();
    assert_eq!(left, Vec::<String>::new());
}

/// `slot rm` that cannot remove the mirror says so, rather than leaving it
/// for the next slot of that name to start from.
#[test]
fn a_mirror_that_cannot_be_removed_fails_slot_rm() {
    let w = world();
    let mirror = nunki::git::SlotGit::open(&w.tree)
        .unwrap()
        .mirror()
        .to_path_buf();
    std::fs::set_permissions(&mirror, std::fs::Permissions::from_mode(0o555)).unwrap();
    let removed = nunki::slot::rm(&w.project, "one", true);
    std::fs::set_permissions(&mirror, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(removed.is_err());
}
