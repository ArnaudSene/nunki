//! Slots (SPEC 4.2, 3.2, 3.3): a clone without hard links, beside the
//! repository, that refuses to be removed while it holds work.

use std::path::{Path, PathBuf};

use hq::project::{Config, Project, ProtectedPaths};
use hq::slot::{SlotError, add, find, list, rm, slots_dir};

fn git(at: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(at)
        .args(args)
        .output()
        .expect("git is on the path");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A real repository with one commit, so the clone has something to carry.
fn repository(dir: &Path) -> Project {
    let root = dir.join("demo");
    std::fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.email", "t@example.com"]);
    git(&root, &["config", "user.name", "Test"]);
    std::fs::write(root.join("README.md"), "one\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-qm", "first"]);

    Project::at(
        root,
        Config {
            harness: "claude-code".to_string(),
            forge: Vec::new(),
            stacks: Vec::new(),
            protected_branches: vec!["main".to_string()],
            protected_paths: ProtectedPaths::default(),
            account: None,
            bounds: Default::default(),
            credentials: None,
            run: None,
            services_file: None,
        },
        dir.join("hq"),
    )
}

#[test]
fn a_slot_is_a_clone_beside_the_repository_with_no_hard_links() {
    let dir = tempfile::tempdir().unwrap();
    let project = repository(dir.path());
    let slot = add(&project, "one").unwrap();

    assert_eq!(slot.tree, slots_dir(&project).join("one"));
    assert_eq!(
        slot.tree.parent().unwrap().parent(),
        project.root.parent(),
        "the slot lives beside the repository, on the same filesystem"
    );
    assert_eq!(git(&slot.tree, &["log", "-1", "--format=%s"]), "first");

    // No hard links: a local clone shares its objects otherwise, and a raw
    // write in a container corrupts the human's history too (SPEC 3.2).
    assert!(!shares_objects(&project.root, &slot.tree));
}

/// Do the two repositories share an object file? A hard-linked clone does.
fn shares_objects(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let inodes = |root: &Path| -> Vec<(u64, u64)> {
        let mut out = Vec::new();
        fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
            for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else {
                    out.push(p);
                }
            }
        }
        let mut files = Vec::new();
        walk(&root.join(".git/objects"), &mut files);
        for f in files {
            if let Ok(m) = std::fs::metadata(&f) {
                out.push((m.dev(), m.ino()));
            }
        }
        out
    };
    let left = inodes(a);
    let right = inodes(b);
    left.iter().any(|i| right.contains(i))
}

#[test]
fn a_slot_keeps_the_repository_as_an_origin_a_container_cannot_reach() {
    let dir = tempfile::tempdir().unwrap();
    let project = repository(dir.path());
    let slot = add(&project, "one").unwrap();

    let origin = git(&slot.tree, &["remote", "get-url", "origin"]);
    // A host path, not a URL: nothing in the container mounts it, so there
    // is nowhere to push (SPEC 3.2).
    assert_eq!(origin, project.root.display().to_string());
    assert!(!origin.contains("://"), "{origin}");
}

#[test]
fn two_slots_of_the_same_name_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let project = repository(dir.path());
    add(&project, "one").unwrap();
    assert!(matches!(
        add(&project, "one").unwrap_err(),
        SlotError::Exists(..)
    ));
}

#[test]
fn a_name_that_compose_would_refuse_is_refused_here_first() {
    let dir = tempfile::tempdir().unwrap();
    let project = repository(dir.path());
    for bad in ["Feat/One", "-one", "", "un espace"] {
        assert!(
            matches!(add(&project, bad).unwrap_err(), SlotError::BadName(_)),
            "{bad:?} should be refused"
        );
    }
    assert!(add(&project, "lot_2-b").is_ok());
}

#[test]
fn slots_are_listed_in_order_and_found_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let project = repository(dir.path());
    add(&project, "two").unwrap();
    add(&project, "one").unwrap();

    let names: Vec<String> = list(&project).into_iter().map(|s| s.name).collect();
    assert_eq!(names, vec!["one", "two"]);
    assert_eq!(find(&project, "two").unwrap().name, "two");
    assert!(matches!(
        find(&project, "three").unwrap_err(),
        SlotError::Unknown(..)
    ));
}

#[test]
fn removing_a_slot_that_holds_commits_is_refused_and_names_the_branch() {
    let dir = tempfile::tempdir().unwrap();
    let project = repository(dir.path());
    let slot = add(&project, "one").unwrap();

    git(&slot.tree, &["config", "user.email", "t@example.com"]);
    git(&slot.tree, &["config", "user.name", "Test"]);
    git(&slot.tree, &["checkout", "-qb", "feat/work"]);
    std::fs::write(slot.tree.join("new.txt"), "work\n").unwrap();
    git(&slot.tree, &["add", "."]);
    git(&slot.tree, &["commit", "-qm", "the work nobody fetched"]);

    let err = rm(&project, "one", false).unwrap_err();
    match &err {
        SlotError::Unfetched { branch, count, .. } => {
            assert_eq!(branch, "feat/work", "the human is told where the work is");
            assert_eq!(*count, 1);
        }
        other => panic!("expected a refusal, got {other}"),
    }
    assert!(err.to_string().contains("hq mission fetch"), "{err}");
    assert!(slot.tree.exists(), "nothing was removed");

    // Said explicitly, it goes.
    rm(&project, "one", true).unwrap();
    assert!(!slot.tree.exists());
}

#[test]
fn removing_a_slot_with_uncommitted_work_is_refused_too() {
    let dir = tempfile::tempdir().unwrap();
    let project = repository(dir.path());
    let slot = add(&project, "one").unwrap();
    std::fs::write(slot.tree.join("scratch.txt"), "not committed\n").unwrap();

    assert!(matches!(
        rm(&project, "one", false).unwrap_err(),
        SlotError::Dirty(_)
    ));
    assert!(slot.tree.exists());
}

#[test]
fn a_slot_that_carries_nothing_new_is_removed_without_argument() {
    let dir = tempfile::tempdir().unwrap();
    let project = repository(dir.path());
    let slot = add(&project, "one").unwrap();
    rm(&project, "one", false).unwrap();
    assert!(!slot.tree.exists());
    assert!(list(&project).is_empty());
}

/// `hq slot reset` puts a slot back to a clean state without destroying it.
///
/// What it removes, and what it deliberately does not: the **clone stays** —
/// `hq slot rm` is the verb that deletes one, and a reset that quietly did
/// the same would be a name lying about a destructive act. What goes is the
/// work in progress, and the slot's named volumes, which is the real reason
/// to reach for it.
#[test]
fn resetting_discards_the_work_in_progress_and_keeps_the_clone() {
    let dir = tempfile::tempdir().unwrap();
    let project = repository(dir.path());
    let slot = add(&project, "one").unwrap();
    std::fs::write(slot.tree.join("half-written.rs"), "fn oops(").unwrap();
    std::fs::write(slot.tree.join("README.md"), "changed\n").unwrap();
    assert!(!hq::git::is_clean(&slot.tree).unwrap());

    // `false` as the engine binary: there is no engine here, and a volume
    // that cannot be removed is not an error — a slot reset before its first
    // run has none.
    let reset = hq::slot::reset(&project, "one", "false", false).unwrap();
    assert!(reset.discarded);
    assert!(reset.volumes.is_empty());
    assert!(hq::git::is_clean(&slot.tree).unwrap());
    assert!(!slot.tree.join("half-written.rs").exists());
    assert!(slot.tree.join(".git").is_dir(), "the clone stays");
    let _ = dir;
}

/// It refuses on the same grounds as `rm`, and for the same reason: commits
/// the repository does not have are work nobody else holds, and a verb that
/// discarded them because its name sounds mild would be the worst kind of
/// verb.
#[test]
fn resetting_refuses_while_the_slot_holds_work_the_repository_lacks() {
    let dir = tempfile::tempdir().unwrap();
    let project = repository(dir.path());
    let slot = add(&project, "one").unwrap();
    std::fs::write(slot.tree.join("new.rs"), "pub fn two() {}\n").unwrap();
    git(&slot.tree, &["add", "-A"]);
    git(
        &slot.tree,
        &[
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@example.com",
            "commit",
            "-qm",
            "work nobody else has",
        ],
    );

    let err = hq::slot::reset(&project, "one", "false", false).unwrap_err();
    assert!(
        matches!(err, hq::slot::SlotError::Unfetched { .. }),
        "{err}"
    );
    // The commit is still there: refused means refused.
    assert_eq!(
        hq::git::commits_not_in(&slot.tree, &project.root)
            .unwrap()
            .len(),
        1
    );

    // And said explicitly, it goes.
    hq::slot::reset(&project, "one", "false", true).unwrap();
    let _ = dir;
}

/// Which volumes a slot owns is listed from the project, not from a profile
/// file: a profile is regenerated at every launch and may not exist at all,
/// and a reset must be able to clean a slot whose last profile is gone.
#[test]
fn a_slots_volumes_are_known_without_a_profile_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = repository(dir.path());
    project.config.stacks = vec!["rust".to_string()];
    std::fs::create_dir_all(project.fragment("rust")).unwrap();
    std::fs::write(
        project.fragment("rust").join(hq::project::WRITABLE_FILE),
        "target\n",
    )
    .unwrap();
    let slot = hq::slot::Slot {
        name: "one".to_string(),
        tree: dir.path().join("nowhere"),
    };

    let names = hq::slot::volumes_of(&project, &slot);
    assert!(names.contains(&hq::exec::proof_volume("one")), "{names:?}");
    assert!(names.contains(&"hq-one-harness".to_string()), "{names:?}");
    assert!(
        names.contains(&hq::run::writable_volume("one", "target")),
        "{names:?}"
    );
}
