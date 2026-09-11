//! The mission's monitor (SPEC 4.3): when one is wanted, what it does after a
//! `verify`, when it wakes, and how it is started exactly once.

use std::path::{Path, PathBuf};

use hq::harness::{RunHandle, SessionId};
use hq::mission::flow::{Flow, Handover};
use hq::mission::{Bounds, Header, Integration, Lot, Security};
use hq::monitor::{Ensured, MonitorError, Next, after_verify, ensure, next_wake, running, wanted};
use hq::project::{Config, Project, ProtectedPaths};
use hq::state::MissionState;
use hq::verify::{Step, VerifyError};

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
        arbiter: None,
        run: None,
        account: None,
        bounds: Bounds::default(),
    }
}

fn project(dir: &Path) -> Project {
    std::fs::create_dir_all(dir.join("hq")).unwrap();
    Project::at(
        dir.join("repo"),
        Config {
            harness: "claude-code".into(),
            forge: vec![],
            stacks: vec![],
            protected_branches: vec!["main".into()],
            protected_paths: ProtectedPaths::default(),
            account: None,
            bounds: Default::default(),
            credentials: None,
            run: None,
            services_file: None,
            forge_protection: Default::default(),
        },
        dir.join("hq"),
    )
}

fn state(with_run: bool) -> MissionState {
    MissionState {
        id: "m1".into(),
        slot: "one".into(),
        flow: Flow::new(header()).unwrap(),
        run: with_run.then(|| RunHandle {
            session: SessionId("s1".into()),
            container: "cafe1234".into(),
            pid: Some(41),
            log: PathBuf::from("/dev/null"),
        }),
        app: None,
        verdicts: Vec::new(),
        accepted: Vec::new(),
        stopped: None,
        harness_down: None,
        spent: Default::default(),
        spared: None,
        coder_session: None,
        updated_at: String::new(),
    }
}

fn down_until(not_before: u64) -> Option<hq::backoff::HarnessDown> {
    Some(hq::backoff::HarnessDown {
        failures: 1,
        since: 0,
        not_before,
        last: "429".into(),
    })
}

/// Every arm, because the arms are the whole decision: go on while `hq` has
/// something to do on its own, stop the moment a human is needed.
#[test]
fn after_a_verify_the_monitor_goes_on_or_stops_for_a_human() {
    use hq::harness::Role::Integrator;
    let go = |step: Step| after_verify(&Ok(vec![step]));
    let stops = |next: Next| matches!(next, Next::Exit(_));

    assert_eq!(
        go(Step::Launched {
            role: Integrator,
            application: "started".into()
        }),
        Next::Continue
    );
    assert_eq!(
        go(Step::Saving {
            role: Integrator,
            account: "main".into(),
            window: "five-hour window".into(),
            per_mille: 950,
            stop_at_percent: 90,
            until: "later".into(),
        }),
        Next::Continue
    );
    assert_eq!(
        go(Step::Waiting {
            role: Integrator,
            until: "later".into(),
            failures: 1,
            last: "429".into(),
        }),
        Next::Continue
    );
    assert_eq!(
        go(Step::Unreachable {
            role: Integrator,
            why: "asleep".into()
        }),
        Next::Continue
    );

    assert!(stops(go(Step::Verified)));
    assert!(stops(go(Step::AwaitingHuman(Handover::Abandoned {
        reason: "no".into()
    }))));
    assert!(stops(go(Step::Held {
        role: Integrator,
        who: "hq".into(),
        date: "now".into(),
        reason: Some("the cap".into()),
    })));
    assert!(stops(go(Step::Findings {
        report: "one".into(),
        lifted: vec![],
    })));

    // A human driving the slot is not a failure: the monitor waits its turn.
    let held = VerifyError::Lock(hq::state::LockError::Held {
        slot: "one".into(),
        verb: "verify".into(),
        pid: 1,
        since: "now".into(),
    });
    assert_eq!(after_verify(&Err(held)), Next::Continue);
    assert_eq!(
        after_verify(&Err(VerifyError::RunInProgress {
            mission: "m1".into(),
            slot: "one".into()
        })),
        Next::Continue
    );
    assert_eq!(
        after_verify(&Err(VerifyError::Spared {
            mission: "m1".into(),
            account: "main".into(),
            window: "five-hour window".into(),
            percent: "95".into(),
            until: "later".into(),
        })),
        Next::Continue
    );
    // Anything else wants a human to look, not a loop retrying it all night.
    assert!(stops(after_verify(&Err(VerifyError::NotStarted(
        "m1".into()
    )))));
}

#[test]
fn a_monitor_is_wanted_for_a_run_or_a_wait_and_not_for_a_held_mission() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let now = 1_000;

    assert!(wanted(&project, &state(true), now), "a run to watch");
    assert!(!wanted(&project, &state(false), now), "nothing to watch");

    let mut waiting = state(false);
    waiting.harness_down = down_until(now + 60);
    assert!(
        wanted(&project, &waiting, now),
        "a wait that ends by itself"
    );
    waiting.harness_down = down_until(now);
    assert!(!wanted(&project, &waiting, now), "a wait already over");

    let mut held = state(true);
    held.hold("Arnaud", false);
    assert!(!wanted(&project, &held, now), "held: a human's to lift");
}

#[test]
fn the_monitor_wakes_every_minute_during_a_run_and_at_the_deadline_otherwise() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let now = 1_000;

    assert_eq!(next_wake(&project, &state(true), now), now + 60);
    // A run goes, whatever wait is recorded: it is watched every minute.
    let mut going = state(true);
    going.harness_down = down_until(now + 6_000);
    assert_eq!(next_wake(&project, &going, now), now + 60);
    let mut waiting = state(false);
    waiting.harness_down = down_until(now + 600);
    assert_eq!(next_wake(&project, &waiting, now), now + 600);
    waiting.harness_down = down_until(now + 10);
    assert_eq!(
        next_wake(&project, &waiting, now),
        now + 60,
        "never sooner than a minute"
    );
}

/// A stand-in for the `hq` binary: named `hq`, it sleeps, and its command
/// line carries what `ensure` gave it.
fn fake_hq(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let hq = bin.join("hq");
    std::fs::write(&hq, "#!/bin/sh\nsleep 30\n").unwrap();
    std::fs::set_permissions(&hq, std::fs::Permissions::from_mode(0o755)).unwrap();
    hq
}

fn kill(pid: u32) {
    let _ = std::process::Command::new("kill")
        .arg(pid.to_string())
        .status();
}

/// Started once: a live monitor is found and not doubled; a dead one is
/// replaced. The pid alone is not trusted — `ps` must say it is this
/// mission's monitor.
#[test]
fn a_monitor_is_started_once_and_replaced_when_dead() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let exe = fake_hq(dir.path());

    let first = match ensure(&project, "m1", &exe).unwrap() {
        Ensured::Started(pid) => pid,
        other => panic!("{other:?}"),
    };
    // `ps` may need a moment to see the exec'd command line.
    for _ in 0..50 {
        if running(&project.hq_root, "m1") == Some(first) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(running(&project.hq_root, "m1"), Some(first));
    assert_eq!(
        ensure(&project, "m1", &exe).unwrap(),
        Ensured::Running(first)
    );
    // One per mission: another mission's pidfile naming this pid is not a
    // monitor of that mission — `ps` must see the mission's own id.
    std::fs::write(
        hq::monitor::pidfile(&project.hq_root, "other"),
        format!("{first}\n"),
    )
    .unwrap();
    assert_eq!(running(&project.hq_root, "other"), None, "one per mission");

    kill(first);
    for _ in 0..50 {
        if running(&project.hq_root, "m1").is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let second = match ensure(&project, "m1", &exe).unwrap() {
        Ensured::Started(pid) => pid,
        other => panic!("a dead monitor is replaced: {other:?}"),
    };
    assert_ne!(first, second);
    kill(second);
}

/// Only the `hq` binary starts a monitor: a test harness calling this would
/// otherwise fork itself into a detached process.
#[test]
fn only_the_hq_binary_starts_a_monitor() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let exe = std::env::current_exe().unwrap();
    assert!(matches!(
        ensure(&project, "m1", &exe),
        Err(MonitorError::NotHq(_))
    ));
    assert!(running(&project.hq_root, "m1").is_none());
}
