//! Slots (SPEC 4.2, 3.2, 3.3): a clone without hard links, beside the
//! repository, that refuses to be removed while it holds work.

use std::path::{Path, PathBuf};

use nunki::project::{Config, Project, ProtectedPaths};
use nunki::slot::{SlotError, add, find, list, rm, slots_dir};

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
            root: None,
            harness: "claude-code".to_string(),
            forge: Vec::new(),
            stacks: Vec::new(),
            protected_branches: vec!["main".to_string()],
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
            security_findings: Default::default(),
        },
        dir.join("nunki"),
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
    assert!(err.to_string().contains("nunki mission fetch"), "{err}");
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

/// `nunki slot reset` puts a slot back to a clean state without destroying it.
///
/// What it removes, and what it deliberately does not: the **clone stays** —
/// `nunki slot rm` is the verb that deletes one, and a reset that quietly did
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
    assert!(!nunki::git::is_clean(&slot.tree).unwrap());

    // An engine holding no volume: a volume never created is not an error —
    // a slot reset before its first run has none.
    let reset = nunki::slot::reset(
        &project,
        "one",
        &nunki::engine::fake::FakeEngine::default(),
        false,
    )
    .unwrap();
    assert!(reset.discarded);
    assert!(reset.volumes.is_empty());
    assert!(nunki::git::is_clean(&slot.tree).unwrap());
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

    let err = nunki::slot::reset(
        &project,
        "one",
        &nunki::engine::fake::FakeEngine::default(),
        false,
    )
    .unwrap_err();
    assert!(
        matches!(err, nunki::slot::SlotError::Unfetched { .. }),
        "{err}"
    );
    // The commit is still there: refused means refused.
    assert_eq!(
        nunki::git::commits_not_in(&slot.tree, &project.root)
            .unwrap()
            .len(),
        1
    );

    // And said explicitly, it goes.
    nunki::slot::reset(
        &project,
        "one",
        &nunki::engine::fake::FakeEngine::default(),
        true,
    )
    .unwrap();
    let _ = dir;
}

/// Which volumes a slot owns is listed from the project, not from a profile
/// file: a profile is regenerated at every launch and may not exist at all,
/// and a reset must be able to clean a slot whose last profile is gone.
#[test]
fn a_slots_volumes_are_known_without_a_profile_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = repository(dir.path());
    project.config.stacks = vec!["rust".into()];
    std::fs::create_dir_all(project.fragment("rust")).unwrap();
    std::fs::write(
        project.fragment("rust").join(nunki::project::WRITABLE_FILE),
        "target\n",
    )
    .unwrap();
    let slot = nunki::slot::Slot {
        name: "one".to_string(),
        tree: dir.path().join("nowhere"),
    };

    let names = nunki::slot::volumes_of(&project, &slot);
    assert!(
        names.contains(&nunki::exec::proof_volume("one")),
        "{names:?}"
    );
    assert!(
        names.contains(&"nunki-one-harness".to_string()),
        "{names:?}"
    );
    assert!(
        names.contains(&nunki::run::writable_volume("one", "target")),
        "{names:?}"
    );
}

/// The slot's Compose project, as `nunki` names it.
fn compose_project(project: &Project) -> String {
    nunki::compose::project_name(&project.session(), "one").unwrap()
}

/// A container of the slot's profile, up and mounting its volumes.
fn the_agent() -> nunki::engine::Container {
    nunki::engine::Container {
        id: "cafe1234".into(),
        service: "agent".into(),
        digest: None,
    }
}

/// `slot reset` really empties the caches while a profile stands: an engine
/// refuses to remove a volume a container mounts, so the slot's containers go
/// first — containers only, through a call that never takes a volume — and
/// then its named volumes, by name. A volume that is not the slot's is never
/// touched.
#[test]
fn resetting_takes_the_slots_containers_down_and_then_empties_its_caches() {
    use nunki::engine::fake::{Call, FakeEngine};

    let dir = tempfile::tempdir().unwrap();
    let project = repository(dir.path());
    let slot = add(&project, "one").unwrap();
    let cache = nunki::exec::proof_volume("one");
    let harness = "nunki-one-harness".to_string();
    let engine = FakeEngine::default()
        .with_lifted(the_agent())
        .with_volume(&cache)
        .with_volume(&harness)
        .with_volume("someone-elses-volume");

    // The control: with the container standing, the engine refuses.
    assert!(
        nunki::engine::Engine::remove_volume(
            &FakeEngine::default()
                .with_lifted(the_agent())
                .with_volume(&cache),
            &cache
        )
        .is_err(),
        "the fake does not model a volume in use, and this test proves nothing"
    );

    let reset = nunki::slot::reset(&project, "one", &engine, false).unwrap();

    assert_eq!(reset.taken_down, ["agent"]);
    let mut removed = reset.volumes.clone();
    removed.sort();
    let mut expected = vec![cache.clone(), harness.clone()];
    expected.sort();
    assert_eq!(removed, expected);
    assert!(reset.failed.is_empty(), "{:?}", reset.failed);
    assert_eq!(engine.volumes(), ["someone-elses-volume"]);

    // In that order, and the containers by id: never `down --volumes`.
    let calls = engine.calls.lock().unwrap().clone();
    let removed_at = calls
        .iter()
        .position(|c| *c == Call::Remove(vec!["cafe1234".into()]))
        .expect("the slot's containers were taken down");
    let first_volume = calls
        .iter()
        .position(|c| matches!(c, Call::RemoveVolume(_)))
        .unwrap();
    assert!(removed_at < first_volume, "{calls:?}");
    assert!(
        calls.contains(&Call::Containers(compose_project(&project))),
        "{calls:?}"
    );
    assert!(
        !calls.iter().any(|c| matches!(c, Call::Down(..))),
        "{calls:?}"
    );
    let _ = slot;
}

/// A volume the engine still will not remove is said, with the engine's
/// reason, rather than counted as one there was nothing to remove.
#[test]
fn a_volume_reset_could_not_remove_is_said() {
    use nunki::engine::fake::FakeEngine;

    let dir = tempfile::tempdir().unwrap();
    let project = repository(dir.path());
    add(&project, "one").unwrap();
    let cache = nunki::exec::proof_volume("one");
    let engine = FakeEngine::default().with_volume_held(&cache);

    let reset = nunki::slot::reset(&project, "one", &engine, false).unwrap();

    assert!(reset.volumes.is_empty());
    assert_eq!(reset.failed.len(), 1, "{:?}", reset.failed);
    assert_eq!(reset.failed[0].0, cache);
    assert!(reset.failed[0].1.contains("in use"), "{:?}", reset.failed);
}

/// Taking the slot's container down under a run, or under a campaign, would
/// end it under the mission that owns it: refused, and nothing touched.
#[test]
fn resetting_refuses_while_a_run_or_a_campaign_is_in_flight_in_the_slot() {
    use nunki::engine::fake::FakeEngine;

    let dir = tempfile::tempdir().unwrap();
    let project = repository(dir.path());
    let slot = add(&project, "one").unwrap();
    std::fs::write(slot.tree.join("half-written.rs"), "fn oops(").unwrap();
    let header = nunki::mission::Header {
        branch: "mission/x".into(),
        base: "main".into(),
        lots: vec![nunki::mission::Lot {
            id: "L1".into(),
            title: "one".into(),
        }],
        integration: nunki::mission::Integration::None {
            reason: "none".into(),
        },
        security: nunki::mission::Security::Gates,
        rigor: Default::default(),
        mutation_threshold: None,
        hardening: Vec::new(),
        arbiter: None,
        run: None,
        account: None,
        model: None,
        bounds: Default::default(),
    };
    nunki::mission::dir::create(&project.hq_root, "m1", &header, "do it").unwrap();
    let store = nunki::state::Store::open(&project.hq_root).unwrap();
    let mut state = nunki::state::MissionState {
        id: "m1".into(),
        slot: "one".into(),
        flow: nunki::mission::flow::Flow::new(header).unwrap(),
        run: Some(nunki::harness::RunHandle {
            session: nunki::harness::SessionId("s1".into()),
            container: "cafe1234".into(),
            pid: Some(41),
            log: PathBuf::from("/dev/null"),
        }),
        run_account: None,
        app: None,
        verdicts: Vec::new(),
        accepted: Vec::new(),
        stopped: None,
        harness_down: None,
        spent: Default::default(),
        spared: None,
        decided: None,
        coder_session: None,
        pushed: None,
        updated_at: "2026-09-10T00:00:00Z".into(),
        revision: 0,
    };
    store.save(&state).unwrap();
    let engine = FakeEngine::default().with_lifted(the_agent());

    let err = nunki::slot::reset(&project, "one", &engine, true).unwrap_err();
    assert!(matches!(err, SlotError::Busy { .. }), "{err}");
    assert!(err.to_string().contains("mission m1"), "{err}");
    assert!(
        slot.tree.join("half-written.rs").exists(),
        "refused means refused"
    );
    assert!(engine.calls.lock().unwrap().is_empty());

    // The run is over; a campaign rewrites the copy now.
    state.run = None;
    store.save(&state).unwrap();
    nunki::mutants::write_running(
        &project.hq_root,
        "one",
        &nunki::mutants::Running {
            off_mission: None,
            chain: Default::default(),
            fingerprint: "abc1234".into(),
            head: "def5678".into(),
            started_at: "2026-09-18T22:52:37Z".into(),
            container: "cafe1234".into(),
            pid: Some(41),
            log: PathBuf::from("/dev/null"),
            deadline_minutes: 45,
        },
    )
    .unwrap();
    let err = nunki::slot::reset(&project, "one", &engine, true).unwrap_err();
    assert!(err.to_string().contains("mutation campaign"), "{err}");

    // Neither: it resets.
    nunki::mutants::forget_running(&project.hq_root, "one").unwrap();
    nunki::slot::reset(&project, "one", &engine, true).unwrap();
    assert!(!slot.tree.join("half-written.rs").exists());
}

/// Frame mission `id` in `project`'s HQ, as `mission new` does, with no
/// state: never started.
fn framed(project: &Project, id: &str) {
    let header = nunki::mission::Header {
        branch: "mission/x".into(),
        base: "main".into(),
        lots: vec![nunki::mission::Lot {
            id: "L1".into(),
            title: "one".into(),
        }],
        integration: nunki::mission::Integration::None {
            reason: "none".into(),
        },
        security: nunki::mission::Security::Gates,
        rigor: Default::default(),
        mutation_threshold: None,
        hardening: Vec::new(),
        arbiter: None,
        run: None,
        account: None,
        model: None,
        bounds: Default::default(),
    };
    nunki::mission::dir::create(&project.hq_root, id, &header, "do it").unwrap();
}

/// A state that cannot be read is not a state that says nothing runs: the
/// reset refuses, names the file and why, says how to get past it, and
/// changes nothing — not the tree, not a container, not a volume. Whether
/// the file is not JSON or cannot be read at all.
#[test]
fn resetting_refuses_when_a_missions_state_cannot_be_read() {
    use nunki::engine::fake::FakeEngine;

    let dir = tempfile::tempdir().unwrap();
    let project = repository(dir.path());
    let slot = add(&project, "one").unwrap();
    std::fs::write(slot.tree.join("half-written.rs"), "fn oops(").unwrap();
    framed(&project, "m1");
    let store = nunki::state::Store::open(&project.hq_root).unwrap();
    let file = store.root().join("missions/m1.json");
    let cache = nunki::exec::proof_volume("one");

    for unreadable in ["corrupt", "a directory"] {
        let _ = std::fs::remove_file(&file);
        let _ = std::fs::remove_dir(&file);
        match unreadable {
            "corrupt" => std::fs::write(&file, "{ half a state").unwrap(),
            _ => std::fs::create_dir(&file).unwrap(),
        }
        let engine = FakeEngine::default()
            .with_lifted(the_agent())
            .with_volume(&cache);

        let err = nunki::slot::reset(&project, "one", &engine, true).unwrap_err();
        assert!(
            matches!(err, SlotError::StateUnreadable { .. }),
            "{unreadable}: {err}"
        );
        let said = err.to_string();
        assert!(said.contains(&file.display().to_string()), "{said}");
        assert!(said.contains("mission m1"), "{said}");
        assert!(said.contains("nothing in it was changed"), "{said}");
        assert!(said.contains("nunki mission archive m1"), "{said}");
        assert!(
            slot.tree.join("half-written.rs").exists(),
            "{unreadable}: refused means refused"
        );
        assert!(engine.calls.lock().unwrap().is_empty(), "{unreadable}");
        assert_eq!(
            engine.volumes(),
            std::slice::from_ref(&cache),
            "{unreadable}"
        );
    }
}

/// Only the missions not archived count: once the broken mission is
/// archived — its folder and its state out of `missions/` — the reset
/// proceeds. So does one beside a mission framed and never started, which
/// has no state and nothing in flight.
#[test]
fn an_archived_broken_state_or_a_mission_never_started_does_not_block_a_reset() {
    use nunki::engine::fake::FakeEngine;

    let dir = tempfile::tempdir().unwrap();
    let project = repository(dir.path());
    let slot = add(&project, "one").unwrap();
    std::fs::write(slot.tree.join("half-written.rs"), "fn oops(").unwrap();
    framed(&project, "never-started");
    framed(&project, "broken");
    let store = nunki::state::Store::open(&project.hq_root).unwrap();
    let file = store.root().join("missions/broken.json");
    std::fs::write(&file, "{ half a state").unwrap();
    let engine = FakeEngine::default().with_lifted(the_agent());
    assert!(nunki::slot::reset(&project, "one", &engine, true).is_err());

    // What `mission archive` leaves: both under `archive/`.
    let archive = project.hq_root.join("archive/broken");
    std::fs::create_dir_all(project.hq_root.join("archive")).unwrap();
    std::fs::rename(project.hq_root.join("missions/broken"), &archive).unwrap();
    std::fs::rename(&file, archive.join("broken.json")).unwrap();

    let reset = nunki::slot::reset(&project, "one", &engine, true).unwrap();
    assert!(reset.discarded);
    assert_eq!(reset.taken_down, ["agent"]);
    assert!(!slot.tree.join("half-written.rs").exists());
}

/// A state store that will not open says nothing about what runs either:
/// refused, with the reason, and nothing changed.
#[test]
fn resetting_refuses_when_the_state_store_will_not_open() {
    use nunki::engine::fake::FakeEngine;

    let dir = tempfile::tempdir().unwrap();
    let project = repository(dir.path());
    let slot = add(&project, "one").unwrap();
    std::fs::write(slot.tree.join("half-written.rs"), "fn oops(").unwrap();
    let state = project.hq_root.join("state");
    let _ = std::fs::remove_dir_all(&state);
    std::fs::create_dir_all(&project.hq_root).unwrap();
    std::fs::write(&state, "not a directory").unwrap();
    let engine = FakeEngine::default().with_lifted(the_agent());

    let err = nunki::slot::reset(&project, "one", &engine, true).unwrap_err();
    assert!(matches!(err, SlotError::StoreUnreadable { .. }), "{err}");
    assert!(
        err.to_string().contains("nothing in it was changed"),
        "{err}"
    );
    assert!(slot.tree.join("half-written.rs").exists());
    assert!(engine.calls.lock().unwrap().is_empty());
}

/// A run in another slot is not this slot's: the reset proceeds.
#[test]
fn a_run_in_another_slot_does_not_block_a_reset() {
    use nunki::engine::fake::FakeEngine;

    let dir = tempfile::tempdir().unwrap();
    let project = repository(dir.path());
    add(&project, "one").unwrap();
    framed(&project, "m1");
    let header = nunki::mission::dir::read_header(&project.hq_root, "m1").unwrap();
    let store = nunki::state::Store::open(&project.hq_root).unwrap();
    let mut state = nunki::state::MissionState {
        id: "m1".into(),
        slot: "two".into(),
        flow: nunki::mission::flow::Flow::new(header).unwrap(),
        run: Some(nunki::harness::RunHandle {
            session: nunki::harness::SessionId("s1".into()),
            container: "cafe1234".into(),
            pid: Some(41),
            log: PathBuf::from("/dev/null"),
        }),
        run_account: None,
        app: None,
        verdicts: Vec::new(),
        accepted: Vec::new(),
        stopped: None,
        harness_down: None,
        spent: Default::default(),
        spared: None,
        decided: None,
        coder_session: None,
        pushed: None,
        updated_at: "2026-09-10T00:00:00Z".into(),
        revision: 0,
    };
    store.save(&state).unwrap();

    let engine = FakeEngine::default().with_lifted(the_agent());
    nunki::slot::reset(&project, "one", &engine, true).unwrap();
    // The control: the same run, in this slot, refuses.
    state.slot = "one".into();
    store.save(&state).unwrap();
    assert!(nunki::slot::reset(&project, "one", &engine, true).is_err());
}

/// A live run of mission `id` in slot `one`, recorded in the state store,
/// its mission framed as `mission new` frames it.
fn running(project: &Project, id: &str) -> nunki::state::MissionState {
    framed(project, id);
    let header = nunki::mission::dir::read_header(&project.hq_root, id).unwrap();
    let state = nunki::state::MissionState {
        id: id.into(),
        slot: "one".into(),
        flow: nunki::mission::flow::Flow::new(header).unwrap(),
        run: Some(nunki::harness::RunHandle {
            session: nunki::harness::SessionId("s1".into()),
            container: "cafe1234".into(),
            pid: Some(41),
            log: PathBuf::from("/dev/null"),
        }),
        run_account: None,
        app: None,
        verdicts: Vec::new(),
        accepted: Vec::new(),
        stopped: None,
        harness_down: None,
        spent: Default::default(),
        spared: None,
        decided: None,
        coder_session: None,
        pushed: None,
        updated_at: "2026-09-10T00:00:00Z".into(),
        revision: 0,
    };
    nunki::state::Store::open(&project.hq_root)
        .unwrap()
        .save(&state)
        .unwrap();
    state
}

/// A list of missions that cannot be read is not an empty list: with a live
/// run in the slot and `missions/` unreadable — at mode 000, or not a
/// directory at all — the reset refuses, names the directory and why, and
/// changes nothing: not the tree, not a container, not a volume.
#[test]
fn resetting_refuses_when_the_list_of_missions_cannot_be_read() {
    use nunki::engine::fake::FakeEngine;
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let project = repository(dir.path());
    let slot = add(&project, "one").unwrap();
    std::fs::write(slot.tree.join("half-written.rs"), "fn oops(").unwrap();
    running(&project, "m1");
    let missions = project.hq_root.join("missions");
    let cache = nunki::exec::proof_volume("one");

    for unreadable in ["mode 000", "a file"] {
        match unreadable {
            "mode 000" => {
                std::fs::set_permissions(&missions, std::fs::Permissions::from_mode(0o000))
                    .unwrap();
                // Under a uid that reads through any mode, this case
                // proves nothing; the other one still does.
                if std::fs::read_dir(&missions).is_ok() {
                    eprintln!("mode 000 does not stop this uid: case not measured");
                    std::fs::set_permissions(&missions, std::fs::Permissions::from_mode(0o755))
                        .unwrap();
                    continue;
                }
            }
            _ => {
                std::fs::set_permissions(&missions, std::fs::Permissions::from_mode(0o755))
                    .unwrap();
                std::fs::rename(&missions, dir.path().join("missions-aside")).unwrap();
                std::fs::write(&missions, "not a directory").unwrap();
            }
        }
        let engine = FakeEngine::default()
            .with_lifted(the_agent())
            .with_volume(&cache);

        let err = nunki::slot::reset(&project, "one", &engine, true).unwrap_err();
        assert!(
            matches!(err, SlotError::MissionsUnreadable { .. }),
            "{unreadable}: {err}"
        );
        let said = err.to_string();
        assert!(said.contains(&missions.display().to_string()), "{said}");
        assert!(said.contains("nothing in it was changed"), "{said}");
        assert!(
            slot.tree.join("half-written.rs").exists(),
            "{unreadable}: refused means refused"
        );
        assert!(engine.calls.lock().unwrap().is_empty(), "{unreadable}");
        assert_eq!(
            engine.volumes(),
            std::slice::from_ref(&cache),
            "{unreadable}"
        );
    }
}

/// A run the state store records in this slot blocks the reset whether or
/// not its mission folder is listed: without its `MISSION.md`, or without
/// its folder at all. The control: the same mission with no run in flight,
/// everything readable, and the reset proceeds as it always did.
#[test]
fn a_run_the_state_records_blocks_a_reset_without_its_mission_folder() {
    use nunki::engine::fake::FakeEngine;

    let dir = tempfile::tempdir().unwrap();
    let project = repository(dir.path());
    let slot = add(&project, "one").unwrap();
    std::fs::write(slot.tree.join("half-written.rs"), "fn oops(").unwrap();
    let mut state = running(&project, "m1");
    let folder = project.hq_root.join("missions/m1");
    let engine = FakeEngine::default().with_lifted(the_agent());

    std::fs::remove_file(folder.join("MISSION.md")).unwrap();
    let err = nunki::slot::reset(&project, "one", &engine, true).unwrap_err();
    assert!(matches!(err, SlotError::Busy { .. }), "{err}");
    assert!(err.to_string().contains("mission m1"), "{err}");

    std::fs::remove_dir_all(&folder).unwrap();
    let err = nunki::slot::reset(&project, "one", &engine, true).unwrap_err();
    assert!(matches!(err, SlotError::Busy { .. }), "{err}");
    assert!(slot.tree.join("half-written.rs").exists());
    assert!(engine.calls.lock().unwrap().is_empty());

    framed(&project, "m1");
    state.run = None;
    nunki::state::Store::open(&project.hq_root)
        .unwrap()
        .save(&state)
        .unwrap();
    let reset = nunki::slot::reset(&project, "one", &engine, true).unwrap();
    assert_eq!(reset.taken_down, ["agent"]);
    assert!(!slot.tree.join("half-written.rs").exists());
}
