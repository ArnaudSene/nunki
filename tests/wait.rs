//! `nunki mission wait` (SPEC 4.5): each stop, its exit code and its line;
//! the loop, driven by a clock and a reader the tests hold; and the HQ's own
//! files read the way the monitor leaves them.

mod common;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use nunki::harness::{Outcome, Role, RunHandle, SessionId, Usage};
use nunki::mission::flow::{Event, Flow, Handover, Stage};
use nunki::mission::{Bounds, Header, Integration, Lot, Rigor, Security, Verdict};
use nunki::project::{Config, Project, ProtectedPaths};
use nunki::state::{MissionState, PushedAt, Stopped, Store};
use nunki::wait::{
    Clock, Context, Observed, Reader, Spent, Status, Stop, WaitError, Watcher, parse_duration,
    status, wait,
};

fn header(security: Security) -> Header {
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
        security,
        rigor: Rigor::Standard,
        mutation_threshold: None,
        arbiter: None,
        run: None,
        account: None,
        model: None,
        bounds: Bounds::default(),
    }
}

fn state_of(flow: Flow) -> MissionState {
    MissionState {
        id: "m1".into(),
        slot: "one".into(),
        flow,
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
        updated_at: "2026-10-05T10:00:00Z".into(),
        revision: 0,
    }
}

fn finished(lot_done: bool) -> Event {
    Event::RunEnded {
        outcome: Outcome::Finished(Usage::default()),
        lot_done,
    }
}

/// A flow driven by `events` from the first lot.
fn flow(security: Security, events: Vec<Event>) -> Flow {
    let mut flow = Flow::new(header(security)).unwrap();
    for event in events {
        flow.advance(event).unwrap();
    }
    flow
}

fn coding() -> MissionState {
    state_of(flow(Security::Gates, vec![]))
}

fn verified() -> MissionState {
    state_of(flow(
        Security::Gates,
        vec![finished(true), Event::GatesPassed],
    ))
}

fn findings() -> MissionState {
    state_of(flow(
        Security::Agent,
        vec![
            finished(true),
            Event::GatesPassed,
            Event::Verdict {
                verdict: Verdict::Findings,
                report: "SQL built by hand\nin src/db.rs".into(),
            },
        ],
    ))
}

fn alive() -> Context {
    Context {
        watcher: Watcher::Alive,
        window: None,
        proposals: Ok(Vec::new()),
    }
}

/// The stop `state` stands at, read with a monitor alive.
fn stop_of(state: &MissionState) -> Status {
    status("m1", state, &alive()).expect("a stop")
}

fn assert_says(status: &Status, parts: &[&str]) {
    let line = status.line();
    assert!(!line.contains('\n'), "one line: {line}");
    assert!(
        !line.contains("{ ") && !line.contains("Handover"),
        "no Debug rendering: {line}"
    );
    for part in parts {
        assert!(line.contains(part), "{part:?} in {line}");
    }
}

// --- one test per stop ----------------------------------------------------

#[test]
fn a_verified_mission_returns_0_and_says_the_human_reads_it_then_pushes() {
    let said = stop_of(&verified());
    assert_eq!((said.stop, said.code), (Stop::Verified, 0));
    assert_eq!(
        said.line(),
        "m1 · verified · every declared stage is green · \
         awaits the human: read it, then `nunki push m1 --yes`"
    );
    // With a security agent, the round it concluded on.
    let cleared = state_of(flow(
        Security::Agent,
        vec![
            finished(true),
            Event::GatesPassed,
            Event::Verdict {
                verdict: Verdict::Clear,
                report: "nothing".into(),
            },
        ],
    ));
    assert_says(
        &stop_of(&cleared),
        &["m1 · verified · every declared stage is green, security round 1 of 1 · "],
    );
}

#[test]
fn a_pushed_mission_returns_0_and_names_the_push() {
    let mut state = verified();
    state.pushed = Some(PushedAt {
        head: "0123456789abcdef0123".into(),
        date: "2026-10-05T11:00:00Z".into(),
    });
    let said = stop_of(&state);
    assert_eq!((said.stop, said.code), (Stop::Pushed, 0));
    assert_says(
        &said,
        &[
            "m1 · pushed · ",
            "pushed on 2026-10-05T11:00:00Z at 0123456789ab ",
            "awaits the human: `nunki mission archive m1` closes it",
        ],
    );
}

#[test]
fn findings_return_10_with_the_round_and_what_the_hq_may_do() {
    let said = stop_of(&findings());
    assert_eq!((said.stop, said.code), (Stop::Findings, 10));
    assert_says(
        &said,
        &[
            "m1 · findings · security round 1 of 1 found: SQL built by hand in src/db.rs",
            "awaits the HQ: `nunki mission iterate m1`",
            "`nunki mission accept m1 --because <why>`",
        ],
    );
}

#[test]
fn a_lot_out_of_attempts_returns_11_naming_the_lot_and_its_attempts() {
    let stalled = || Event::Stalled {
        reason: "quiet".into(),
    };
    let state = state_of(flow(Security::Gates, vec![stalled(), stalled(), stalled()]));
    let said = stop_of(&state);
    assert_eq!((said.stop, said.code), (Stop::Handover, 11));
    assert_says(
        &said,
        &[
            "m1 · awaiting human · lot L1 failed 3 attempt(s) · awaits the human: ",
            "read the journals, then `nunki mission retry m1 --because <what changed>`",
        ],
    );
}

#[test]
fn a_role_out_of_attempts_returns_11_naming_the_role() {
    let stalled = || Event::Stalled {
        reason: "quiet".into(),
    };
    let state = state_of(flow(
        Security::Agent,
        vec![
            finished(true),
            Event::GatesPassed,
            stalled(),
            stalled(),
            stalled(),
        ],
    ));
    let said = stop_of(&state);
    assert_eq!((said.stop, said.code), (Stop::Handover, 11));
    assert_says(
        &said,
        &[
            "the Security failed 3 attempt(s)",
            "awaits the human: `nunki mission retry m1",
        ],
    );
    assert_eq!(
        state.flow.stage(),
        &Stage::AwaitingHuman(Handover::RoleAttemptsExhausted {
            role: Role::Security,
            attempts: 3
        })
    );
}

#[test]
fn volets_used_up_return_11_with_their_count_their_causes_and_the_grant() {
    let failed = |n: u32| Event::GatesFailed {
        reason: format!("gate 7 red, round {n}\nsee the report"),
    };
    let mut events = vec![finished(true)];
    for n in 1..=3 {
        events.push(failed(n));
        events.push(finished(true));
    }
    events.push(failed(4));
    let state = state_of(flow(Security::Gates, events));
    let said = stop_of(&state);
    assert_eq!((said.stop, said.code), (Stop::Handover, 11));
    assert_says(
        &said,
        &[
            "volets 3 / 3 — 3 return(s) to the coder were taken, and the last cause found \
             none left: gate: gate 7 red, round 1 see the report; ",
            "gate: gate 7 red, round 4 see the report",
            "awaits the human: `nunki mission retry m1 --because <why>` grants one more volet",
        ],
    );

    // Past the cap, the count says how many the HQ granted.
    let mut flow = state.flow.clone();
    flow.advance(Event::Retried {
        because: "granted".into(),
    })
    .unwrap();
    flow.advance(finished(true)).unwrap();
    flow.advance(failed(5)).unwrap();
    let said = stop_of(&state_of(flow));
    assert_says(&said, &["volets 4 / 3 (1 granted by the HQ) — 4 return(s)"]);
}

#[test]
fn a_mission_called_off_returns_11_with_the_reason() {
    let state = state_of(flow(
        Security::Gates,
        vec![Event::Ended {
            reason: "the client dropped it".into(),
        }],
    ));
    let said = stop_of(&state);
    assert_eq!((said.stop, said.code), (Stop::Handover, 11));
    assert_says(
        &said,
        &[
            "called off: the client dropped it",
            "awaits the human: `nunki mission archive m1` closes it",
        ],
    );
}

#[test]
fn a_ruling_awaited_returns_11_and_awaits_the_hq() {
    let state = state_of(flow(
        Security::Gates,
        vec![
            finished(false),
            Event::RulingAwaited {
                what: "`a` is equivalent".into(),
                survivors: vec!["a".into(), "b".into()],
            },
        ],
    ));
    let said = stop_of(&state);
    assert_eq!((said.stop, said.code), (Stop::Handover, 11));
    assert_says(
        &said,
        &[
            "lot L1, attempt 2, awaits the HQ's ruling on `a`, `b`",
            "awaits the HQ: rule each (`nunki mission mutants m1 --equivalent",
            "`nunki mission retry m1 --because <what was ruled>`",
        ],
    );
}

#[test]
fn a_hold_returns_12_with_who_when_and_why() {
    let mut state = coding();
    state.stopped = Some(Stopped {
        who: "nunki".into(),
        date: "2026-10-05T09:00:00Z".into(),
        interrupted: false,
        reason: Some("the harness failed 6 times".into()),
    });
    let said = stop_of(&state);
    assert_eq!((said.stop, said.code), (Stop::Held, 12));
    assert_says(
        &said,
        &[
            "m1 · coding lot L1, attempt 1 · held by nunki on 2026-10-05T09:00:00Z: the harness \
             failed 6 times",
            "awaits the human: `nunki mission resume m1` lifts the hold",
        ],
    );
    // A hold without a reason says who and when, and no dangling colon.
    state.stopped.as_mut().unwrap().reason = None;
    assert_says(
        &stop_of(&state),
        &["held by nunki on 2026-10-05T09:00:00Z · awaits"],
    );
}

#[test]
fn a_spent_window_returns_12_with_its_reset() {
    let context = Context {
        watcher: Watcher::Alive,
        window: Some(Spent {
            account: "main".into(),
            window: "five-hour window".into(),
            until: 0,
        }),
        proposals: Ok(Vec::new()),
    };
    let said = status("m1", &coding(), &context).unwrap();
    assert_eq!((said.stop, said.code), (Stop::WindowSpent, 12));
    assert_says(
        &said,
        &[
            "account main's five-hour window is past its threshold",
            "awaits the account: nothing is launched before 1970-01-01T00:00:00Z",
        ],
    );
}

#[test]
fn a_gate_that_could_not_be_played_returns_13_naming_the_gate_and_the_reason() {
    let context = Context {
        watcher: Watcher::Stopped {
            why: "a gate could not be played: gate 6 (the battery is green) could not be \
                  played: the system profile did not come up"
                .into(),
        },
        window: None,
        proposals: Ok(Vec::new()),
    };
    let state = state_of(flow(Security::Gates, vec![finished(true)]));
    let said = status("m1", &state, &context).unwrap();
    assert_eq!((said.stop, said.code), (Stop::MonitorFailed, 13));
    assert_says(
        &said,
        &[
            "m1 · final gates · the monitor stopped: a gate could not be played: gate 6 (the \
             battery is green)",
            "the system profile did not come up",
            "awaits the human: act on it, then `nunki verify m1`",
        ],
    );
}

#[test]
fn a_dead_monitor_during_a_running_stage_returns_14() {
    let context = Context {
        watcher: Watcher::Gone,
        window: None,
        proposals: Ok(Vec::new()),
    };
    let said = status("m1", &coding(), &context).unwrap();
    assert_eq!((said.stop, said.code), (Stop::MonitorGone, 14));
    assert_says(
        &said,
        &[
            "m1 · coding lot L1, attempt 1 · no monitor is watching",
            "awaits the human: `nunki verify m1`",
        ],
    );
}

#[test]
fn a_mission_still_driven_by_its_monitor_is_not_a_stop() {
    assert_eq!(status("m1", &coding(), &alive()), None);
}

/// The flow's stop is said whatever the monitor does: a monitor stops on
/// purpose when the mission is verified, and that is not a failure.
#[test]
fn where_the_flow_stands_comes_before_the_monitor() {
    for watcher in [
        Watcher::Gone,
        Watcher::Stopped {
            why: "anything".into(),
        },
    ] {
        let context = Context {
            watcher,
            window: None,
            proposals: Ok(Vec::new()),
        };
        assert_eq!(
            status("m1", &verified(), &context).map(|s| s.stop),
            Some(Stop::Verified)
        );
    }
}

#[test]
fn the_json_carries_the_same_five_fields() {
    let said = stop_of(&findings());
    let json: serde_json::Value = serde_json::from_str(&said.json()).unwrap();
    assert_eq!(json["id"], "m1");
    assert_eq!(json["stage"], "findings");
    assert_eq!(json["detail"], said.detail.as_str());
    assert_eq!(json["awaits"]["who"], "the HQ");
    assert_eq!(json["awaits"]["what"], said.awaits.what.as_str());
    assert_eq!(json["code"], 10);
    assert_eq!(json.as_object().unwrap().len(), 5, "{json}");
}

// --- the loop ------------------------------------------------------------

/// Hands out readings in turn, and the last one for ever.
struct Script(VecDeque<Observed>);

impl Reader for Script {
    fn observe(&mut self, _: &str) -> Result<Observed, WaitError> {
        Ok(if self.0.len() > 1 {
            self.0.pop_front().unwrap()
        } else {
            self.0[0].clone()
        })
    }
}

/// A clock that moves only when slept on, and remembers each sleep.
struct Hands {
    now: u64,
    slept: Vec<u64>,
}

impl Clock for Hands {
    fn now(&mut self) -> u64 {
        self.now
    }
    fn sleep(&mut self, seconds: u64) {
        // A pause of nothing is a loop that reads the state as fast as it
        // can, and with this clock one that never ends.
        assert!(seconds > 0, "a zero-second sleep: {:?}", self.slept);
        self.slept.push(seconds);
        self.now += seconds;
    }
}

fn live(state: MissionState) -> Observed {
    Observed::Live {
        state: Box::new(state),
        context: alive(),
    }
}

fn clock() -> Hands {
    Hands {
        now: 1_000,
        slept: Vec::new(),
    }
}

#[test]
fn an_already_stopped_mission_returns_without_waiting() {
    let mut clock = clock();
    let said = wait(
        "m1",
        &mut Script(VecDeque::from([live(findings())])),
        &mut clock,
        30,
        None,
    )
    .unwrap();
    assert_eq!(said.code, 10);
    assert!(clock.slept.is_empty(), "{:?}", clock.slept);
}

#[test]
fn a_running_mission_is_read_again_every_interval_until_it_stops() {
    let mut clock = clock();
    let said = wait(
        "m1",
        &mut Script(VecDeque::from([
            live(coding()),
            live(coding()),
            live(verified()),
        ])),
        &mut clock,
        30,
        None,
    )
    .unwrap();
    assert_eq!(said.stop, Stop::Verified);
    assert_eq!(clock.slept, vec![30, 30]);
}

#[test]
fn the_timeout_returns_15_and_never_sleeps_past_it() {
    let mut clock = clock();
    let said = wait(
        "m1",
        &mut Script(VecDeque::from([live(coding())])),
        &mut clock,
        30,
        Some(75),
    )
    .unwrap();
    assert_eq!((said.stop, said.code), (Stop::TimedOut, 15));
    assert_eq!(clock.slept, vec![30, 30, 15]);
    assert_says(
        &said,
        &[
            "m1 · coding lot L1, attempt 1 · no stop after 75 second(s) of waiting",
            "awaits the monitor: it is still driving the mission",
        ],
    );
}

#[test]
fn a_zero_timeout_reads_once_and_returns() {
    let mut clock = clock();
    let said = wait(
        "m1",
        &mut Script(VecDeque::from([live(coding())])),
        &mut clock,
        30,
        Some(0),
    )
    .unwrap();
    assert_eq!(said.code, 15);
    assert!(clock.slept.is_empty());
}

#[test]
fn durations_are_seconds_minutes_or_hours() {
    assert_eq!(parse_duration("90"), Ok(90));
    assert_eq!(parse_duration("90s"), Ok(90));
    assert_eq!(parse_duration("5m"), Ok(300));
    assert_eq!(parse_duration("2h"), Ok(7_200));
    // `u64` parsing takes a leading `+`; a duration does not.
    for wrong in [
        "",
        "m",
        "-5",
        "+5",
        "+5m",
        "5d",
        "1.5h",
        "h5",
        "99999999999999999999h",
    ] {
        assert!(
            parse_duration(wrong).is_err(),
            "{wrong:?} is not a duration"
        );
    }
}

// --- the HQ's files ------------------------------------------------------

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
            mutation_jobs: None,
            forge_protection: Default::default(),
        },
        dir.join("nunki"),
    )
}

fn save(project: &Project, state: &MissionState) {
    Store::open(&project.hq_root).unwrap().save(state).unwrap();
}

fn read(project: &Project) -> Status {
    wait(
        "m1",
        &mut nunki::wait::Hq::new(project),
        &mut clock(),
        30,
        Some(0),
    )
    .unwrap()
}

#[test]
fn with_no_monitor_and_no_word_from_one_the_hq_reads_14() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let mut state = coding();
    state.run = Some(RunHandle {
        session: SessionId("s1".into()),
        container: "cafe1234".into(),
        pid: Some(41),
        log: PathBuf::from("/dev/null"),
    });
    save(&project, &state);
    assert_eq!(read(&project).stop, Stop::MonitorGone);
}

/// The monitor stops, says why, and `wait` prints the very line the monitor
/// logged — for a stop the state shows, and for one only the monitor knows.
#[test]
fn the_monitors_exit_line_equals_waits_line_for_the_same_state() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let gates = state_of(flow(Security::Gates, vec![finished(true)]));
    for (state, why, code) in [
        (
            gates,
            "a gate could not be played: gate 6 could not be played: the profile is down",
            13,
        ),
        (findings(), "the findings are the human's", 10),
        (verified(), "verified — the human reads it", 0),
    ] {
        save(&project, &state);
        let logged = nunki::monitor::stop(&project.hq_root, "m1", why);
        let said = read(&project);
        assert_eq!(logged, said.line());
        assert_eq!(said.code, code, "{logged}");
    }
}

/// A word the monitor left on a state the mission has since moved past says
/// nothing about it now.
#[test]
fn a_monitors_word_on_an_older_state_is_not_read_as_its_exit() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let mut state = coding();
    save(&project, &state);
    nunki::monitor::stop(
        &project.hq_root,
        "m1",
        "verify failed, and a human should look",
    );
    assert_eq!(read(&project).stop, Stop::MonitorFailed);

    state.updated_at = "2026-10-05T12:00:00Z".into();
    save(&project, &state);
    assert_eq!(read(&project).stop, Stop::MonitorGone);
}

/// A write that leaves `updated_at` as it was — a reframe sets nothing but
/// the header — still moves the mission past what its monitor said.
#[test]
fn a_reframe_after_the_monitors_exit_moves_the_mission_past_its_word() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let gates = state_of(flow(Security::Gates, vec![finished(true)]));
    nunki::mission::dir::create(&project.hq_root, "m1", gates.flow.header(), "x").unwrap();
    save(&project, &gates);
    nunki::monitor::stop(
        &project.hq_root,
        "m1",
        "a gate could not be played: gate 6 could not be played: the profile is down",
    );
    assert_eq!(read(&project).stop, Stop::MonitorFailed);

    // The human edits the framing and re-freezes it.
    let mut fresh = gates.flow.header().clone();
    fresh.mutation_threshold = Some(90);
    nunki::mission::dir::create(&project.hq_root, "m2", &fresh, "x").unwrap();
    std::fs::copy(
        project.hq_root.join("missions/m2/MISSION.md"),
        project.hq_root.join("missions/m1/MISSION.md"),
    )
    .unwrap();
    let before = Store::open(&project.hq_root).unwrap().load("m1").unwrap();
    assert!(
        nunki::lifecycle::reframe(&project, "m1", true)
            .unwrap()
            .applied
    );
    let after = Store::open(&project.hq_root).unwrap().load("m1").unwrap();
    assert_eq!(
        after.updated_at, before.updated_at,
        "the reframe kept the time"
    );
    assert_eq!(read(&project).stop, Stop::MonitorGone);
}

/// The monitor stops between `wait`'s two readings: it writes the state it
/// stops on, keeps its word, then forgets its pid. Read monitor first and
/// state after, `wait` sees the stop the state now shows; read the other way
/// round, it would pair the state from before the stop with a monitor already
/// gone, and say nobody watches a mission that reached its findings.
#[test]
fn a_monitor_that_stops_while_wait_reads_is_read_on_its_stop_never_as_gone() {
    for (stop_on, why, stop) in [
        (findings(), "the findings are the human's", Stop::Findings),
        (verified(), "verified — the human reads it", Stop::Verified),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let project = project(dir.path());
        save(&project, &coding());
        let mut stopped = false;
        let mut hq = nunki::wait::Hq::watched_by(&project, |hq_root, id| {
            if !stopped {
                stopped = true;
                Store::open(hq_root).unwrap().save(&stop_on).unwrap();
                nunki::monitor::stop(hq_root, id, why);
            }
            false
        });
        let said = wait("m1", &mut hq, &mut clock(), 30, Some(0)).unwrap();
        assert_eq!(said.stop, stop, "{}", said.line());
    }
}

#[test]
fn an_archived_mission_returns_0_and_names_where_it_ended() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let at = nunki::lifecycle::archive_dir(&project).join("m1");
    std::fs::create_dir_all(&at).unwrap();
    std::fs::write(
        at.join("state.json"),
        serde_json::to_vec(&verified()).unwrap(),
    )
    .unwrap();
    let said = read(&project);
    assert_eq!((said.stop, said.code), (Stop::Archived, 0));
    assert_says(
        &said,
        &[
            "m1 · archived · closed at verified under ",
            "awaits nobody: it is over",
        ],
    );
}

#[test]
fn a_mission_never_started_is_an_error_not_a_wait() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let mut clock = clock();
    let err = wait(
        "m1",
        &mut nunki::wait::Hq::new(&project),
        &mut clock,
        30,
        None,
    )
    .unwrap_err();
    assert!(matches!(err, WaitError::NotStarted(_)), "{err}");
    assert!(clock.slept.is_empty());
}

/// A push describes the stage it was made at: the flow leaving it forgets it.
#[test]
fn a_transition_after_a_push_forgets_the_push() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let store = Store::open(&project.hq_root).unwrap();
    let mut state = verified();
    state.pushed = Some(PushedAt {
        head: "abc".into(),
        date: "now".into(),
    });
    store
        .apply(
            &mut state,
            Event::Reviewed {
                because: "off topic".into(),
            },
        )
        .unwrap();
    assert_eq!(state.pushed, None);
    assert_eq!(store.load("m1").unwrap().pushed, None);
}

/// A long report or cause is cut, on a character and not a byte, and says
/// it was; one that fits is left whole.
#[test]
fn a_long_report_is_cut_with_an_ellipsis_and_a_short_one_is_whole() {
    use nunki::text::brief;
    let long = "é".repeat(200);
    assert_eq!(brief(&long, 120), format!("{}…", "é".repeat(120)));
    assert_eq!(brief(&"a".repeat(120), 120), "a".repeat(120));
    assert_eq!(brief("two\n  lines", 120), "two lines");

    let state = state_of(flow(
        Security::Agent,
        vec![
            finished(true),
            Event::GatesPassed,
            Event::Verdict {
                verdict: Verdict::Findings,
                report: "x".repeat(400),
            },
        ],
    ));
    let said = stop_of(&state);
    assert!(
        said.detail.ends_with(&format!("{}…", "x".repeat(160))),
        "{}",
        said.detail
    );
}

/// Whatever an agent wrote — a report, a volet's cause, a survivor's id, the
/// reason a monitor or a hold gives, a lot's label — reaches `wait`'s line,
/// and so the monitor's log, escaped and never raw; `--json` likewise.
#[test]
fn agent_text_reaches_waits_line_escaped_never_raw() {
    let hostile = common::HOSTILE.to_string();
    let report = |verdict| Event::Verdict {
        verdict,
        report: hostile.clone(),
    };
    let mut lines: Vec<(&str, Status)> = Vec::new();

    let found = state_of(flow(
        Security::Agent,
        vec![
            finished(true),
            Event::GatesPassed,
            report(Verdict::Findings),
        ],
    ));
    lines.push(("a findings report", stop_of(&found)));

    let mut events = vec![finished(true)];
    for _ in 0..3 {
        events.push(Event::GatesFailed {
            reason: hostile.clone(),
        });
        events.push(finished(true));
    }
    events.push(Event::GatesFailed {
        reason: hostile.clone(),
    });
    lines.push((
        "the volets' causes",
        stop_of(&state_of(flow(Security::Gates, events))),
    ));

    lines.push((
        "a survivor's id",
        stop_of(&state_of(flow(
            Security::Gates,
            vec![Event::RulingAwaited {
                what: hostile.clone(),
                survivors: vec![hostile.clone()],
            }],
        ))),
    ));
    lines.push((
        "the reason a mission was called off",
        stop_of(&state_of(flow(
            Security::Gates,
            vec![Event::Ended {
                reason: hostile.clone(),
            }],
        ))),
    ));

    let mut held = coding();
    held.stopped = Some(Stopped {
        who: hostile.clone(),
        date: "now".into(),
        interrupted: false,
        reason: Some(hostile.clone()),
    });
    lines.push(("a hold's reason", stop_of(&held)));

    let stopped = Context {
        watcher: Watcher::Stopped {
            why: hostile.clone(),
        },
        window: None,
        proposals: Ok(Vec::new()),
    };
    lines.push((
        "the monitor's word",
        status("m1", &coding(), &stopped).unwrap(),
    ));

    let mut header = header(Security::Gates);
    header.lots[0].id = hostile.clone();
    let gone = Context {
        watcher: Watcher::Gone,
        window: None,
        proposals: Ok(Vec::new()),
    };
    lines.push((
        "a lot's label",
        status("m1", &state_of(Flow::new(header).unwrap()), &gone).unwrap(),
    ));

    for (outlet, said) in lines {
        common::assert_printable(&said.line(), outlet);
        assert!(!said.line().contains('\n'), "{outlet}: {}", said.line());
        let json = said.json();
        assert!(
            !json.chars().any(common::raw_control),
            "{outlet} in JSON: {json}"
        );
        // Not by JSON's escaping alone: decoded, the fields are the line's,
        // escapes included.
        let decoded: serde_json::Value = serde_json::from_str(&json).unwrap();
        let fields = format!(
            "{} · {} · {} · awaits {}: {}",
            decoded["id"].as_str().unwrap(),
            decoded["stage"].as_str().unwrap(),
            decoded["detail"].as_str().unwrap(),
            decoded["awaits"]["who"].as_str().unwrap(),
            decoded["awaits"]["what"].as_str().unwrap(),
        );
        assert_eq!(fields, said.line(), "{outlet} in JSON");
        common::assert_printable(&fields, outlet);
    }
}

/// The real clock reads the machine's time; how it sleeps is the one thing
/// no test here exercises, by the mission's rule that no test sleeps.
#[test]
fn the_system_clock_reads_the_machines_time() {
    use nunki::wait::SystemClock;
    let before = nunki::state::now_secs();
    let read = SystemClock.now();
    let after = nunki::state::now_secs();
    assert!(
        before <= read && read <= after,
        "{before} ≤ {read} ≤ {after}"
    );
}

/// The HQ reads the account's last measure: past its threshold, a mission
/// still being driven stops on 12 and names the window and its reset; below
/// it, nothing stops it.
#[test]
fn the_hq_reads_a_spent_window_from_the_accounts_measure() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let home = project.nunki_home();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join("accounts.yaml"),
        "accounts:\n  main:\n    harness: claude-code\n    token_file: accounts/main.token\n",
    )
    .unwrap();
    let resets = nunki::state::now_secs() + 3_600;
    let measure = |per_mille| nunki::consumption::Measure {
        windows: nunki::consumption::Windows {
            five_hour: Some(nunki::consumption::Window {
                per_mille,
                resets_at: resets,
            }),
            weekly: None,
        },
        measured_at: nunki::state::now_secs(),
        harness: "claude-code".into(),
    };
    save(&project, &coding());
    let observe = || match nunki::wait::Hq::new(&project).observe("m1").unwrap() {
        Observed::Live { context, .. } => context.window,
        other => panic!("{other:?}"),
    };

    nunki::consumption::record(&home, "main", &measure(500)).unwrap();
    assert_eq!(observe(), None, "below the threshold");

    nunki::consumption::record(&home, "main", &measure(950)).unwrap();
    assert_eq!(
        observe(),
        Some(Spent {
            account: "main".into(),
            window: "five-hour window".into(),
            until: resets,
        })
    );
}

/// `n` pending proposals: the first the coder's, the others `nunki`'s from
/// the registry.
fn proposed(n: usize) -> Vec<nunki::mutants::Proposal> {
    (0..n)
        .map(|i| nunki::mutants::Proposal {
            id: format!("s{i}"),
            file: "src/lib.rs".into(),
            line: 3,
            why: "nothing reads it".into(),
            from: (i > 0).then(|| nunki::mutants::ProposedFrom::Registry {
                mission: "earlier".into(),
                commit: "abc".into(),
                date: String::new(),
            }),
        })
        .collect()
}

/// A verified mission with equivalence proposals nobody ruled on waits on
/// the HQ, not on the human's push — `nunki push` would refuse — and the
/// line says how many, from whom, and the verbs that rule.
#[test]
fn a_verified_mission_with_proposals_awaits_the_hq_and_says_how_many() {
    let context = Context {
        proposals: Ok(proposed(2)),
        ..alive()
    };
    let said = status("m1", &verified(), &context).unwrap();
    assert_eq!((said.stop, said.code), (Stop::Verified, 0));
    assert_says(
        &said,
        &[
            "every declared stage is green; 2 equivalence proposal(s) await the HQ's ruling \
             (1 from the coder, 1 from the registry)",
            "awaits the HQ: `nunki mission mutants m1 --ratify <survivor>` (or `--ratify --all`)",
            "--refuse <survivor> --because <why>",
            "then `nunki push m1 --yes`",
        ],
    );

    // None: the line is the one it always was.
    let said = status("m1", &verified(), &alive()).unwrap();
    assert!(!said.line().contains("proposal"), "{}", said.line());
    assert!(said.line().contains("awaits the human"), "{}", said.line());
}

/// Any other stop still counts them, and leaves who it awaits alone; and a
/// count that could not be read is said, never taken for zero.
#[test]
fn the_proposals_are_counted_on_every_stop_and_an_unreadable_count_is_said() {
    let context = Context {
        proposals: Ok(proposed(1)),
        watcher: Watcher::Gone,
        ..alive()
    };
    let said = status("m1", &coding(), &context).unwrap();
    assert_eq!(said.stop, Stop::MonitorGone);
    assert_says(
        &said,
        &[
            "1 equivalence proposal(s) await the HQ's ruling",
            "awaits the human",
        ],
    );

    let context = Context {
        proposals: Err("MUTANTS.triage.json: expected value".into()),
        ..alive()
    };
    let said = status("m1", &verified(), &context).unwrap();
    assert_says(
        &said,
        &[
            "the equivalence proposals could not be read: MUTANTS.triage.json",
            "awaits the human",
        ],
    );
}

/// The HQ's reader counts the proposals from the mission's own two files,
/// exactly as `nunki push` does — and says so when it cannot read them.
#[test]
fn the_hq_reads_the_proposals_from_the_mission_folder() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    save(&project, &coding());
    let mission = nunki::mission::dir::Paths::of(&project.hq_root, "m1").dir;
    std::fs::create_dir_all(&mission).unwrap();
    let observe = || match nunki::wait::Hq::new(&project).observe("m1").unwrap() {
        Observed::Live { context, .. } => context.proposals.map(|p| p.len()),
        other => panic!("{other:?}"),
    };
    assert_eq!(observe(), Ok(0), "no campaign, no proposal");

    std::fs::write(
        mission.join(nunki::mutants::FILE),
        r#"{"fingerprint":"f","head":"h","date":"d","survivors":[
            {"id":"s1","file":"a","line":1},{"id":"s2","file":"a","line":2}]}"#,
    )
    .unwrap();
    std::fs::write(
        mission.join(nunki::mutants::TRIAGE_FILE),
        r#"{"s1": {"kind": "equivalent_proposed", "why": "only a log reads it"},
            "s2": {"kind": "equivalent_proposed", "why": "same constant"}}"#,
    )
    .unwrap();
    assert_eq!(observe(), Ok(2));

    std::fs::write(mission.join(nunki::mutants::TRIAGE_FILE), "{not json").unwrap();
    match observe() {
        Err(why) => assert!(why.contains(nunki::mutants::TRIAGE_FILE), "{why}"),
        Ok(n) => panic!("an unreadable file is not {n} proposals"),
    }
}
