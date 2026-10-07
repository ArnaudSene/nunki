//! The mission's monitor (SPEC 4.3): when one is wanted, what it does after a
//! `verify`, when it wakes, and how it is started exactly once.

mod common;

use std::path::{Path, PathBuf};

use nunki::harness::{RunHandle, SessionId};
use nunki::mission::flow::{Flow, Handover};
use nunki::mission::{Bounds, Header, Integration, Lot, Security};
use nunki::monitor::{
    Ensured, MonitorError, Next, after_verify, ensure, next_wake, running, wake_after_reading,
    wanted,
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
        rigor: Default::default(),
        mutation_threshold: None,
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
            root: None,
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
            rigor: None,
            mutation_threshold: 80,
            mutation_jobs: 1,
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
        pushed: None,
        updated_at: String::new(),
        revision: 0,
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
    use nunki::harness::Role::{Coder, Integrator};
    let go = |step: Step| after_verify("m1", &Ok(vec![step]));
    let stops = |next: Next| matches!(next, Next::Exit(_));

    assert_eq!(
        go(Step::Launched {
            role: Integrator,
            application: "started".into()
        }),
        Next::Continue
    );
    // A campaign this launch ended is said, not acted on: the launch is the
    // step after it, and that one goes on.
    assert_eq!(
        go(Step::CampaignEnded {
            since: "2026-09-18T22:52:37Z".into()
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

    // A lift nunki made is not a human's decision to wait for: the mission
    // goes on, as after a human's accept. One it withheld is followed by
    // the findings, which stop.
    assert_eq!(
        go(Step::LiftedByNunki {
            findings: vec!["LOW — a: x".into()]
        }),
        Next::Continue
    );
    assert_eq!(
        go(Step::LeftToHuman {
            why: "finding \"a\" is MEDIUM".into()
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
        why: "gate 6 (the battery is green) could not be played: the system profile \
              did not come up"
            .into(),
    })));
    // And the exception to it: gate 7's missing campaign is an obstacle nunki
    // removes itself, so the monitor runs one instead of handing the mission
    // back for a verb a human would have typed.
    assert_eq!(
        go(Step::CampaignOwed {
            role: Coder,
            why: "gate 7 (every survivor has an outcome) could not be played: no mutation \
                  campaign has run on this mission"
                .into(),
        }),
        Next::RunCampaign
    );

    // A human driving the slot is not a failure: the monitor waits its turn.
    let held = VerifyError::Lock(nunki::state::LockError::Held {
        slot: "one".into(),
        verb: "verify".into(),
        pid: 1,
        since: "now".into(),
    });
    assert_eq!(after_verify("m1", &Err(held)), Next::Continue);
    assert_eq!(
        after_verify(
            "m1",
            &Err(VerifyError::RunInProgress {
                mission: "m1".into(),
                slot: "one".into()
            })
        ),
        Next::Continue
    );
    assert_eq!(
        after_verify(
            "m1",
            &Err(VerifyError::Spared {
                mission: "m1".into(),
                account: "main".into(),
                window: "five-hour window".into(),
                percent: "95".into(),
                until: "later".into(),
            })
        ),
        Next::Continue
    );
    // Anything else wants a human to look, not a loop retrying it all night.
    assert!(stops(after_verify(
        "m1",
        &Err(VerifyError::NotStarted("m1".into()))
    )));
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
    held.hold("Alex Martin", false);
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

    // Read back after `verify`: the same wake for a state that was read, a
    // minute for one that could not be.
    waiting.harness_down = down_until(now + 600);
    assert_eq!(wake_after_reading(&project, Some(&waiting), now), now + 600);
    assert_eq!(wake_after_reading(&project, None, now), now + 60);
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
    // What this proves is what `ps` says about a pid, and it ends a monitor
    // with `kill`, so where either cannot be run there is nothing to measure
    // — nunki's own rust image carries no procps, and its battery runs
    // there. Skipped and said, the way a test needing a container engine
    // skips without one; it runs on a developer's machine and on CI, where
    // both are part of the system.
    for (tool, probe) in [("ps", ["-o", "pid="]), ("kill", ["-l", "1"])] {
        if std::process::Command::new(tool)
            .args(probe)
            .output()
            .is_err()
        {
            eprintln!(
                "skipped: no `{tool}` on this machine, and a monitor's liveness is what \
                 `ps` says about a process `kill` can end"
            );
            return;
        }
    }
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
        after_verify(
            "m1",
            &Ok(vec![Step::Busy {
                role: nunki::harness::Role::Integrator,
                provider: "stripe".into(),
                by: "mission m2".into(),
            }])
        ),
        Next::Continue
    );
}

/// A campaign the monitor started, looked at again: it goes on while the
/// campaign does, and stops on the two ends nobody can work through.
#[test]
fn a_campaign_that_overran_or_vanished_stops_the_monitor_and_the_rest_does_not() {
    use nunki::monitor::after_campaign;
    use nunki::mutants::Progress;

    for progress in [
        Progress::Started {
            fingerprint: "abc1234".into(),
            scope: Default::default(),
        },
        Progress::Running {
            started_at: "2026-09-16T05:00:00Z".into(),
            lines: 12,
        },
        Progress::Fresh { survivors: 0 },
        Progress::Finished {
            survivors: 3,
            chain: vec![],
        },
    ] {
        assert_eq!(
            after_campaign(&progress),
            Next::Continue,
            "{progress:?} is a campaign in hand, not a wall"
        );
    }

    // A campaign past its deadline was stopped, and a monitor that started
    // another would spend the same hour again. Both of these are a human's
    // to look at.
    match after_campaign(&Progress::Overrun { minutes: 45 }) {
        Next::Exit(why) => assert!(why.contains("45-minute deadline"), "{why}"),
        other => panic!("{other:?}"),
    }
    // Could not run, and said why: gate 7 goes red on it at the next
    // `verify`, which sends a volet. Nothing here waits on a human.
    assert_eq!(
        after_campaign(&Progress::CouldNotRun("FAILED tests/test_x.py".into())),
        Next::Continue,
        "a campaign that could not run goes back to the branch through gate 7"
    );
    match after_campaign(&Progress::Lost("the container went away".into())) {
        Next::Exit(why) => assert!(why.contains("the container went away"), "{why}"),
        other => panic!("{other:?}"),
    }
}

/// A lot awaiting a ruling stops the monitor once, and its exit line says in
/// one line what the HQ is to rule on and what to type after.
#[test]
fn the_monitor_stops_on_a_ruling_and_names_the_survivors() {
    let next = after_verify(
        "m7",
        &Ok(vec![nunki::verify::Step::AwaitingHuman(
            Handover::AwaitingRuling {
                lot: "L1".into(),
                what: "`m1` and `m2` are equivalent".into(),
                attempt: 2,
                survivors: vec!["m1".into(), "m2".into()],
            },
        )]),
    );
    let Next::Exit(said) = next else {
        panic!("a ruling is the human's: {next:?}");
    };
    assert!(!said.contains('\n'), "{said}");
    assert!(
        !said.contains("<mission>"),
        "the real id, so it can be copied: {said}"
    );
    for part in [
        "lot L1",
        "attempt 2",
        "`m1`, `m2`",
        "nunki mission mutants m7 --equivalent",
        "nunki mission retry m7",
    ] {
        assert!(said.contains(part), "{part:?} in {said}");
    }
}

/// The log is for what changed. Ten ticks on the same state write nothing
/// after the first; a run launched, a stage moved, a new word from `verify`
/// each write one line.
#[test]
fn ten_quiet_ticks_write_nothing_to_the_monitors_log() {
    use nunki::monitor::Changes;
    let mut said = Changes::default();
    let mut log: Vec<u8> = Vec::new();
    let quiet = state(false);

    said.tick(&mut log, 0, &quiet);
    let first = String::from_utf8(log.clone()).unwrap();
    assert!(first.contains("no run under way"), "{first}");
    assert!(first.contains("stage coding lot L1, attempt 1"), "{first}");
    said.note(&mut log, 0, "verify", "verify: Waiting");
    let before = log.len();

    for minute in 1..=10 {
        said.tick(&mut log, minute * 60, &quiet);
        assert!(!said.note(&mut log, minute * 60, "verify", "verify: Waiting"));
    }
    assert_eq!(
        log.len(),
        before,
        "{}",
        String::from_utf8_lossy(&log[before..])
    );

    // A run launched: one line, for the run, and the stage is unchanged.
    said.tick(&mut log, 700, &state(true));
    let launched = String::from_utf8(log[before..].to_vec()).unwrap();
    assert_eq!(launched.lines().count(), 1, "{launched}");
    assert!(launched.contains("run s1 under way"), "{launched}");
    // A new word from `verify` is written, under its own topic.
    assert!(said.note(&mut log, 760, "verify", "verify: Launched"));
}

/// A running campaign is said once, not at every tick its line count grows.
#[test]
fn a_running_campaign_is_said_once_whatever_it_has_written() {
    use nunki::monitor::campaign_line;
    use nunki::mutants::Progress;
    let running = |lines| Progress::Running {
        started_at: "2026-10-05T10:00:00Z".into(),
        lines,
    };
    assert_eq!(campaign_line(&running(3)), campaign_line(&running(300)));
    assert_eq!(
        campaign_line(&Progress::Finished {
            survivors: 2,
            chain: vec![]
        }),
        "mutation campaign ended: 2 survivor(s)"
    );
}

/// The monitor's log and its last line carry what an agent wrote — a
/// `verify` step quoting a report, a campaign's stderr, the reason it stops —
/// escaped and never raw.
#[test]
fn agent_text_reaches_the_monitors_log_escaped_never_raw() {
    use nunki::monitor::{Changes, campaign_line, stop};
    let mut said = Changes::default();
    let mut log: Vec<u8> = Vec::new();
    assert!(said.note(&mut log, 0, "verify", common::HOSTILE));
    // The same hostile text again is the same line: nothing more is written.
    assert!(!said.note(&mut log, 60, "verify", common::HOSTILE));
    said.note(
        &mut log,
        120,
        "campaign",
        &campaign_line(&nunki::mutants::Progress::CouldNotRun(
            common::HOSTILE.into(),
        )),
    );
    let log = String::from_utf8(log).unwrap();
    assert_eq!(log.lines().count(), 2, "{log}");
    common::assert_printable(&log, "the monitor's log");

    let dir = tempfile::tempdir().unwrap();
    // No state: the line says the state cannot be read, and the reason.
    let last = stop(dir.path(), "m1", common::HOSTILE);
    common::assert_printable(&last, "the monitor's last line, no state");
    let store = nunki::state::Store::open(dir.path()).unwrap();
    store.save(&state(false)).unwrap();
    let last = stop(dir.path(), "m1", common::HOSTILE);
    common::assert_printable(&last, "the monitor's last line");
}

/// A clock for the monitor's loop: it never sleeps, remembers each pause, and
/// at each one does what the test says the world did meanwhile. A loop that
/// reads the time over and over without pausing is a loop that spins: it is
/// stopped, rather than left to hang the battery.
struct Ticks<'a> {
    now: u64,
    reads: usize,
    slept: Vec<u64>,
    meanwhile: Box<dyn FnMut() + 'a>,
}

impl<'a> Ticks<'a> {
    fn at(now: u64, meanwhile: impl FnMut() + 'a) -> Self {
        Self {
            now,
            reads: 0,
            slept: Vec::new(),
            meanwhile: Box::new(meanwhile),
        }
    }
}

impl nunki::wait::Clock for Ticks<'_> {
    fn now(&mut self) -> u64 {
        self.reads += 1;
        assert!(self.reads < 50, "the loop spins without pausing");
        self.now
    }
    fn sleep(&mut self, seconds: u64) {
        assert!(self.slept.len() < 10, "the loop should have stopped");
        self.slept.push(seconds);
        self.now += seconds;
        (self.meanwhile)();
    }
}

/// A clock on which every reading comes a minute after the one before: each
/// tick of the monitor outlasts its minute. At its second reading, the world
/// does what the test says.
struct Slow<'a> {
    reads: u64,
    slept: Vec<u64>,
    meanwhile: Box<dyn FnMut() + 'a>,
}

impl nunki::wait::Clock for Slow<'_> {
    fn now(&mut self) -> u64 {
        self.reads += 1;
        assert!(self.reads < 50, "the loop spins without pausing");
        if self.reads == 2 {
            (self.meanwhile)();
        }
        1_000 + 60 * (self.reads - 1)
    }
    fn sleep(&mut self, seconds: u64) {
        assert!(self.slept.len() < 10, "the loop should have stopped");
        self.slept.push(seconds);
    }
}

fn save(project: &Project, state: &MissionState) {
    nunki::state::Store::open(&project.hq_root)
        .unwrap()
        .save(state)
        .unwrap();
}

/// The monitor stops on a `verify` that fails — here, the slot is gone —
/// and the line it ends on is `wait`'s for the same state: the code 13 stop,
/// carrying the monitor's own word.
#[test]
fn a_monitor_that_cannot_verify_stops_on_the_line_wait_prints() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    save(&project, &state(false));
    let engine: std::sync::Arc<dyn nunki::engine::Engine> =
        std::sync::Arc::new(nunki::engine::fake::FakeEngine::default());

    let line = nunki::monitor::run(&project, "m1", engine, "docker");
    assert!(
        line.contains("the monitor stopped: verify failed, and a human should look"),
        "{line}"
    );
    let said = nunki::wait::wait(
        "m1",
        &mut nunki::wait::Hq::new(&project),
        &mut nunki::wait::SystemClock,
        30,
        Some(0),
    )
    .unwrap();
    assert_eq!(said.code, 13, "{}", said.line());
    assert_eq!(said.line(), line);
    assert!(
        !nunki::monitor::pidfile(&project.hq_root, "m1").exists(),
        "the pid is forgotten once the monitor stops"
    );
}

/// While the run goes, the monitor watches it and does not call `verify`: a
/// tick on a live run measures and sleeps a minute. Once the run is read back
/// (here, by the test, during that minute), the next tick calls `verify`.
#[test]
fn a_running_run_is_watched_and_verify_waits_for_it_to_end() {
    use nunki::engine::{ExecOutput, Liveness, fake::FakeEngine};
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    save(&project, &state(true));
    let engine: std::sync::Arc<dyn nunki::engine::Engine> = std::sync::Arc::new(
        FakeEngine::default()
            .with_liveness("cafe1234", Liveness::Running)
            .with_exec(ExecOutput {
                status: 0,
                stdout: "nunki-run-running\n".into(),
                stderr: String::new(),
            }),
    );
    let mut clock = Ticks::at(1_000, || save(&project, &state(false)));
    let mut log: Vec<u8> = Vec::new();

    let why = nunki::monitor::watch(&project, "m1", engine, "docker", &mut clock, &mut log);

    let log = String::from_utf8(log).unwrap();
    assert!(why.starts_with("verify failed"), "{why}\n{log}");
    assert_eq!(clock.slept, vec![60], "{log}");
    let verifies: Vec<&str> = log.lines().filter(|l| l.contains("verify:")).collect();
    assert_eq!(
        verifies.len(),
        1,
        "verify is called once, after the run: {log}"
    );
    assert!(!log.contains("a run is still going"), "{log}");
    assert!(log.contains("run s1 under way"), "{log}");
}

/// A run a human froze is theirs to thaw: the monitor neither measures it nor
/// calls `verify` on it, and looks again a minute later. Once the run is read
/// back (here, by the test, during that minute), the next tick calls
/// `verify`.
#[test]
fn a_paused_run_is_left_alone_and_looked_at_again_a_minute_later() {
    use nunki::engine::{Liveness, fake::FakeEngine};
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    save(&project, &state(true));
    let engine: std::sync::Arc<dyn nunki::engine::Engine> =
        std::sync::Arc::new(FakeEngine::default().with_liveness("cafe1234", Liveness::Paused));
    let mut clock = Ticks::at(1_000, || save(&project, &state(false)));
    let mut log: Vec<u8> = Vec::new();

    let why = nunki::monitor::watch(&project, "m1", engine, "docker", &mut clock, &mut log);

    let log = String::from_utf8(log).unwrap();
    assert!(why.starts_with("verify failed"), "{why}\n{log}");
    assert_eq!(clock.slept, vec![60], "{log}");
    let verifies = log.lines().filter(|l| l.contains("verify:")).count();
    assert_eq!(verifies, 1, "verify is called once, after the run: {log}");
}

/// A tick that outlasts its minute goes straight on to the next: the
/// monitor pauses only for time still to wait, never for none.
#[test]
fn a_tick_that_outlasts_its_minute_goes_on_without_a_pause() {
    use nunki::engine::{Liveness, fake::FakeEngine};
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    save(&project, &state(true));
    let engine: std::sync::Arc<dyn nunki::engine::Engine> =
        std::sync::Arc::new(FakeEngine::default().with_liveness("cafe1234", Liveness::Paused));
    let mut clock = Slow {
        reads: 0,
        slept: Vec::new(),
        meanwhile: Box::new(|| save(&project, &state(false))),
    };
    let mut log: Vec<u8> = Vec::new();

    let why = nunki::monitor::watch(&project, "m1", engine, "docker", &mut clock, &mut log);

    let log = String::from_utf8(log).unwrap();
    assert!(why.starts_with("verify failed"), "{why}\n{log}");
    assert_eq!(clock.slept, Vec::<u64>::new(), "{log}");
}

/// The monitor's log says whether a campaign started full or partial, and
/// from which commit; and once it ended, each campaign of the chain with
/// its counts.
#[test]
fn the_log_says_each_campaign_full_or_partial_with_its_counts() {
    use nunki::monitor::campaign_line;
    use nunki::mutants::{Progress, Scope};
    let started = campaign_line(&Progress::Started {
        fingerprint: "abc1234".into(),
        scope: Scope::Partial {
            since: "0123456789abcdef".into(),
        },
    });
    assert_eq!(
        started,
        "mutation campaign started on abc1234, partial since 0123456789ab"
    );
    let started = campaign_line(&Progress::Started {
        fingerprint: "abc1234".into(),
        scope: Scope::Full {
            why: "asked `--again`, which is a full campaign".into(),
        },
    });
    assert_eq!(
        started,
        "mutation campaign started on abc1234, full (asked `--again`, which is a full campaign)"
    );
    let ended = campaign_line(&Progress::Finished {
        survivors: 1,
        chain: vec![
            "full at aaa — tried 9".into(),
            "partial since aaa at bbb — tried 2".into(),
        ],
    });
    assert_eq!(
        ended,
        "mutation campaign ended: 1 survivor(s) — campaign 1 of 2: full at aaa — tried 9; \
         campaign 2 of 2: partial since aaa at bbb — tried 2"
    );
}

/// A full campaign with no reason recorded — the first of a mission — is
/// logged without an empty pair of parentheses.
#[test]
fn a_full_campaign_without_a_reason_is_logged_without_one() {
    use nunki::monitor::campaign_line;
    use nunki::mutants::{Progress, Scope};
    let started = campaign_line(&Progress::Started {
        fingerprint: "abc1234".into(),
        scope: Scope::Full { why: " ".into() },
    });
    assert_eq!(started, "mutation campaign started on abc1234, full");
}
