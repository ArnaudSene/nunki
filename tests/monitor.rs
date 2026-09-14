//! The mission's monitor (SPEC 4.3): when one is wanted, what it does after a
//! `verify`, when it wakes, and how it is started exactly once.

use std::path::{Path, PathBuf};

use nunki::harness::{RunHandle, SessionId};
use nunki::mission::flow::{Flow, Handover};
use nunki::mission::{Bounds, Header, Integration, Lot, Security};
use nunki::monitor::{
    Ensured, MonitorError, Next, after_verify, ensure, next_wake, running, wanted,
};
use nunki::project::{Config, Project, ProtectedPaths};
use nunki::state::MissionState;
use nunki::verify::{Step, VerifyError};

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
        model: None,
        bounds: Bounds::default(),
    }
}

fn project(dir: &Path) -> Project {
    std::fs::create_dir_all(dir.join("nunki")).unwrap();
    Project::at(
        dir.join("repo"),
        Config {
            harness: "claude-code".into(),
            forge: vec![],
            stacks: vec![],
            protected_branches: vec!["main".into()],
            protected_paths: ProtectedPaths::default(),
            account: None,
            model: None,
            bounds: Default::default(),
            credentials: None,
            run: None,
            services_file: None,
            permission_mode: "auto".to_string(),
            forge_protection: Default::default(),
        },
        dir.join("nunki"),
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

fn down_until(not_before: u64) -> Option<nunki::backoff::HarnessDown> {
    Some(nunki::backoff::HarnessDown {
        failures: 1,
        since: 0,
        not_before,
        last: "429".into(),
    })
}

/// Every arm, because the arms are the whole decision: go on while `nunki` has
/// something to do on its own, stop the moment a human is needed.
#[test]
fn after_a_verify_the_monitor_goes_on_or_stops_for_a_human() {
    use nunki::harness::Role::Integrator;
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
        who: "nunki".into(),
        date: "now".into(),
        reason: Some("the cap".into()),
    })));
    assert!(stops(go(Step::Findings {
        report: "one".into(),
        lifted: vec![],
    })));
    // The one arm where stopping is what keeps the flow honest rather than
    // what ends it: the stage has not moved, so going on would play the same
    // gate against the same wall on every tick, for as long as the monitor
    // lives. Nothing but a human changes the answer.
    assert!(stops(go(Step::GateUnplayable {
        role: Integrator,
        why: "gate 7 (every survivor has an outcome) could not be played: no mutation \
              campaign has run on this mission — `nunki mission mutants` starts one"
            .into(),
    })));

    // A human driving the slot is not a failure: the monitor waits its turn.
    let held = VerifyError::Lock(nunki::state::LockError::Held {
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

/// A stand-in for the `nunki` binary: named `nunki`, it sleeps, and its command
/// line carries what `ensure` gave it.
fn fake_nunki(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let nunki = bin.join("nunki");
    std::fs::write(&nunki, "#!/bin/sh\nsleep 30\n").unwrap();
    std::fs::set_permissions(&nunki, std::fs::Permissions::from_mode(0o755)).unwrap();
    nunki
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
    let exe = fake_nunki(dir.path());

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
        nunki::monitor::pidfile(&project.hq_root, "other"),
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

/// Only the `nunki` binary starts a monitor: a test harness calling this would
/// otherwise fork itself into a detached process.
#[test]
fn only_the_nunki_binary_starts_a_monitor() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let exe = std::env::current_exe().unwrap();
    assert!(matches!(
        ensure(&project, "m1", &exe),
        Err(MonitorError::NotNunki(_))
    ));
    assert!(running(&project.hq_root, "m1").is_none());
}

/// A mission at its integration stage with a shared provider.
fn integrating(id: &str, with_run: bool) -> MissionState {
    use nunki::mission::flow::Event;
    let mut framing = header();
    framing.integration = Integration::Services {
        services: vec![nunki::mission::Service {
            name: "stripe".into(),
            reach: vec!["api.stripe.com".into()],
            shared: true,
        }],
        wiring: Vec::new(),
    };
    let mut state = state(with_run);
    state.id = id.into();
    state.slot = format!("slot-{id}");
    state.flow = Flow::new(framing).unwrap();
    state
        .flow
        .advance(Event::RunEnded {
            outcome: nunki::harness::Outcome::Finished(Default::default()),
            lot_done: true,
        })
        .unwrap();
    state.flow.advance(Event::GatesPassed).unwrap();
    state
}

/// Waiting on a provider another mission holds is a wait that ends by
/// itself: the monitor is wanted for it, and no longer once it is free.
#[test]
fn a_monitor_is_wanted_while_another_mission_holds_a_provider() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let store = nunki::state::Store::open(&project.hq_root).unwrap();
    let mine = integrating("m1", false);

    store.save(&integrating("m2", true)).unwrap();
    assert!(wanted(&project, &mine, 1_000), "the provider is held");

    store.save(&integrating("m2", false)).unwrap();
    assert!(!wanted(&project, &mine, 1_000), "free: verify launches");
}

/// A provider another mission holds is a wait: the monitor goes on.
#[test]
fn a_busy_provider_is_waited_for_not_handed_over() {
    assert_eq!(
        after_verify(&Ok(vec![Step::Busy {
            role: nunki::harness::Role::Integrator,
            provider: "stripe".into(),
            by: "mission m2".into(),
        }])),
        Next::Continue
    );
}
