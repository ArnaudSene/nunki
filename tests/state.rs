//! Persisted state and the slot lock (SPEC 4.2).

use std::fs;
use std::process::{Command, Stdio};

use hq::harness::{Outcome, RunHandle, SessionId, Usage};
use hq::mission::flow::{Event, Flow, Stage, Work};
use hq::mission::{Bounds, Header, Integration, Lot, Security};
use hq::state::{LockError, MissionState, SlotLock, StateError, Store};

fn header() -> Header {
    Header {
        branch: "feat/x".into(),
        base: "dev".into(),
        lots: vec![
            Lot {
                id: "L1".into(),
                title: "one".into(),
            },
            Lot {
                id: "L2".into(),
                title: "two".into(),
            },
        ],
        integration: Integration::None {
            reason: "pure domain".into(),
        },
        security: Security::Gates,
        arbiter: None,
        run: None,
        account: None,
        bounds: Bounds::default(),
    }
}

fn state(id: &str) -> MissionState {
    MissionState {
        id: id.into(),
        slot: "s1".into(),
        flow: Flow::new(header()).unwrap(),
        run: None,
        app: None,
        verdicts: Vec::new(),
        accepted: Vec::new(),
        stopped: None,
        updated_at: String::new(),
    }
}

#[test]
fn a_mission_state_round_trips_through_disk() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let mut st = state("m1");
    store
        .set_run(
            &mut st,
            Some(RunHandle {
                session: SessionId("sess-1".into()),
                container: "hq-m1-coder".into(),
                pid: Some(4242),
                log: std::path::PathBuf::from("/tmp/runs/sess-1.jsonl"),
            }),
        )
        .unwrap();
    let back = store.load("m1").unwrap();
    assert_eq!(back, st);
    assert_eq!(store.missions().unwrap(), vec!["m1".to_string()]);
    assert!(!back.updated_at.is_empty());
}

#[test]
fn apply_persists_only_a_valid_transition() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let mut st = state("m2");
    store.save(&st).unwrap();

    // Invalid: nothing written, flow unchanged.
    let err = store.apply(&mut st, Event::GatesPassed).unwrap_err();
    assert!(matches!(err, StateError::Flow(_)));
    assert_eq!(store.load("m2").unwrap().flow, Flow::new(header()).unwrap());

    // Valid: written.
    store
        .apply(
            &mut st,
            Event::RunEnded {
                outcome: Outcome::Finished(Usage::default()),
                lot_done: true,
            },
        )
        .unwrap();
    let back = store.load("m2").unwrap();
    assert_eq!(
        back.flow.stage(),
        &Stage::Coding {
            work: Work::Lot(1),
            attempt: 1
        }
    );
}

#[test]
fn a_write_is_atomic_and_leaves_no_temp_file() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let st = state("m3");
    store.save(&st).unwrap();
    let names: Vec<_> = fs::read_dir(store.root().join("missions"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["m3.json".to_string()]);
}

#[test]
fn a_missing_or_corrupt_state_is_named_not_invented() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    assert!(matches!(
        store.load("nope").unwrap_err(),
        StateError::Missing(_)
    ));
    fs::write(store.root().join("missions/bad.json"), b"{ not json").unwrap();
    assert!(matches!(
        store.load("bad").unwrap_err(),
        StateError::Corrupt { .. }
    ));
}

#[test]
fn a_lock_held_by_a_live_process_refuses_a_second_taker() {
    let dir = tempfile::tempdir().unwrap();
    let locks = dir.path().join("locks");
    fs::create_dir_all(&locks).unwrap();
    let first = SlotLock::acquire(&locks, "s1", "verify").unwrap();
    assert!(first.lifted().is_none());
    let err = SlotLock::acquire(&locks, "s1", "reset").unwrap_err();
    match err {
        LockError::Held {
            slot, verb, pid, ..
        } => {
            assert_eq!(
                (slot.as_str(), verb.as_str(), pid),
                ("s1", "verify", std::process::id())
            );
        }
        other => panic!("expected Held, got {other:?}"),
    }
    // Another slot is free.
    let _other = SlotLock::acquire(&locks, "s2", "reset").unwrap();
}

#[test]
fn releasing_frees_the_slot() {
    let dir = tempfile::tempdir().unwrap();
    let locks = dir.path().join("locks");
    fs::create_dir_all(&locks).unwrap();
    {
        let _l = SlotLock::acquire(&locks, "s1", "verify").unwrap();
        assert!(SlotLock::holder(&locks, "s1").unwrap().is_some());
    }
    assert!(SlotLock::holder(&locks, "s1").unwrap().is_none());
    let _again = SlotLock::acquire(&locks, "s1", "verify").unwrap();
}

#[test]
fn an_orphan_lock_is_lifted_and_said() {
    let dir = tempfile::tempdir().unwrap();
    let locks = dir.path().join("locks");
    fs::create_dir_all(&locks).unwrap();
    // A process that has already exited: its pid is certainly dead.
    let child = Command::new("true").stdout(Stdio::null()).spawn().unwrap();
    let dead_pid = child.id();
    let _ = child.wait_with_output().unwrap();
    let orphan = serde_json::json!({
        "slot": "s1", "verb": "verify", "pid": dead_pid, "since": "2026-09-09T00:00:00Z"
    });
    fs::write(locks.join("s1.lock"), serde_json::to_vec(&orphan).unwrap()).unwrap();
    assert!(SlotLock::holder(&locks, "s1").unwrap().is_none());

    let lock = SlotLock::acquire(&locks, "s1", "reset").unwrap();
    let lifted = lock.lifted().expect("the orphan is reported");
    assert_eq!((lifted.verb.as_str(), lifted.pid), ("verify", dead_pid));
    assert_eq!(lock.info().verb, "reset");
}

#[test]
fn an_unreadable_lock_file_is_lifted_too() {
    let dir = tempfile::tempdir().unwrap();
    let locks = dir.path().join("locks");
    fs::create_dir_all(&locks).unwrap();
    fs::write(locks.join("s1.lock"), b"garbage").unwrap();
    let lock = SlotLock::acquire(&locks, "s1", "verify").unwrap();
    assert!(lock.lifted().is_none());
}

#[test]
fn dropping_a_guard_never_deletes_a_lock_retaken_by_another_process() {
    let dir = tempfile::tempdir().unwrap();
    let locks = dir.path().join("locks");
    fs::create_dir_all(&locks).unwrap();
    let mine = SlotLock::acquire(&locks, "s1", "verify").unwrap();
    // Meanwhile another, live process takes the slot over (as it would after
    // lifting an orphan): its lock file replaces ours.
    let mut other = Command::new("sleep")
        .arg("30")
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let theirs = serde_json::json!({
        "slot": "s1", "verb": "reset", "pid": other.id(), "since": "2026-09-09T00:00:00Z"
    });
    fs::write(locks.join("s1.lock"), serde_json::to_vec(&theirs).unwrap()).unwrap();
    drop(mine);
    let still = SlotLock::holder(&locks, "s1").unwrap();
    other.kill().unwrap();
    let _ = other.wait();
    assert_eq!(
        still.map(|h| h.pid),
        Some(theirs["pid"].as_u64().unwrap() as u32)
    );
}

/// A hold changes what `hq` will do next, so a read verb that does not
/// mention it reports a held mission as one nobody touched. This drives the
/// real binary, because the omission was in what the CLI prints and nothing
/// below it could have caught it.
#[test]
fn status_says_a_mission_is_held_and_who_held_it() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("hq.yaml"), "harness: claude-code\n").unwrap();
    let hq_root = dir.path().join("home/.hq/repo");

    let status = || {
        let out = Command::new(env!("CARGO_BIN_EXE_hq"))
            .args(["-C"])
            .arg(&root)
            .args(["mission", "status", "m1"])
            .env("HOME", dir.path().join("home"))
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    };

    hq::mission::dir::create(&hq_root, "m1", &header(), "do it").unwrap();
    let store = Store::open(&hq_root).unwrap();
    let mut state = state("m1");
    store.save(&state).unwrap();
    assert!(
        !status().contains("held"),
        "the probe is worth nothing unless an unheld mission says nothing: {}",
        status()
    );

    state.hold("Arnaud", false);
    store.save(&state).unwrap();
    let text = status();
    assert!(text.contains("held"), "{text}");
    assert!(text.contains("Arnaud"), "it names who held it: {text}");
    assert!(text.contains("resume m1"), "and how to lift it: {text}");
}

/// `watch` on a held mission with no run said "none in progress" and left —
/// which reads as "it is between two runs" when it means "nothing is
/// coming". The hold is reported before the run, and it is the one that
/// decides whether anything follows.
#[test]
fn watch_says_the_hold_before_it_says_there_is_no_run() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("hq.yaml"), "harness: claude-code\n").unwrap();
    let hq_root = dir.path().join("home/.hq/repo");

    hq::mission::dir::create(&hq_root, "m1", &header(), "do it").unwrap();
    let store = Store::open(&hq_root).unwrap();
    let mut state = state("m1");
    state.hold("Arnaud", false);
    store.save(&state).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_hq"))
        .args(["-C"])
        .arg(&root)
        .args(["mission", "watch", "m1"])
        .env("HOME", dir.path().join("home"))
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let held = text.find("held").unwrap_or_else(|| panic!("{text}"));
    let none = text
        .find("none in progress")
        .unwrap_or_else(|| panic!("{text}"));
    assert!(held < none, "the hold comes first: {text}");
    assert!(text.contains("Arnaud"), "{text}");
}
