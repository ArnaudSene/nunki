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
            bounds: Default::default(),
            credentials: None,
            run: None,
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
