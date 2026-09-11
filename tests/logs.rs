//! `hq logs` (SPEC 4.2, verb table): what replaces watching a screen.

use std::path::PathBuf;

use hq::harness::LineKind;
use hq::harness::claude_code::ClaudeCode;
use hq::harness::spawn::LocalSpawner;
use hq::logs::{self, LogsError};
use hq::mission::flow::Flow;
use hq::mission::{Bounds, Header, Integration, Lot, Security};
use hq::project::{Config, Project, ProtectedPaths};
use hq::state::{MissionState, Store};

fn harness() -> ClaudeCode {
    ClaudeCode::new(Default::default(), Box::new(LocalSpawner))
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
        arbiter: None,
        run: None,
        account: None,
        bounds: Bounds::default(),
    }
}

struct World {
    _dir: tempfile::TempDir,
    project: Project,
}

impl World {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let hq_root = dir.path().join("hq");
        for d in ["locks", "missions", "state/missions"] {
            std::fs::create_dir_all(hq_root.join(d)).unwrap();
        }
        let project = Project::at(
            dir.path().join("repo"),
            Config {
                harness: "claude-code".into(),
                forge: vec![],
                stacks: vec!["rust".into()],
                protected_branches: vec!["main".into()],
                protected_paths: ProtectedPaths::default(),
                account: None,
                bounds: Default::default(),
                credentials: None,
                run: None,
                services_file: None,
                forge_protection: Default::default(),
            },
            hq_root.clone(),
        );
        hq::mission::dir::create(&hq_root, "m1", &header(), "do it").unwrap();
        Self { _dir: dir, project }
    }

    fn run_log(&self, session: &str, body: &str) -> PathBuf {
        let runs = self.project.hq_root.join("missions/m1/runs");
        std::fs::create_dir_all(&runs).unwrap();
        let path = runs.join(format!("{session}.jsonl"));
        std::fs::write(&path, body).unwrap();
        path
    }

    fn started(&self, id: &str, when: &str) {
        let store = Store::open(&self.project.hq_root).unwrap();
        store
            .save(&MissionState {
                id: id.into(),
                slot: "one".into(),
                flow: Flow::new(header()).unwrap(),
                run: None,
                app: None,
                verdicts: Vec::new(),
                accepted: Vec::new(),
                stopped: None,
                updated_at: when.into(),
            })
            .unwrap();
    }
}

const STREAM: &str = concat!(
    r#"{"type":"system","subtype":"init","model":"claude-opus-5","session_id":"s1"}"#,
    "\n",
    r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Reading the mission."}]}}"#,
    "\n",
    r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/work/tree/src/lib.rs"}}]}}"#,
    "\n",
    r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"pub fn one() {}"}]}}"#,
    "\n",
    r#"{"type":"result","subtype":"success","is_error":false,"usage":{"input_tokens":20,"output_tokens":7}}"#,
    "\n",
);

/// A run reads as a run: what it was told, what it said, what it did, how it
/// ended — and the tool result coming back is not a line of its own, because
/// it is the other half of a call already shown.
#[test]
fn a_run_reads_as_what_it_was_told_what_it_said_and_what_it_did() {
    let world = World::new();
    world.run_log("s1", STREAM);

    let runs = logs::of(&world.project, "m1", &harness()).unwrap();
    assert_eq!(runs.len(), 1);
    let kinds: Vec<LineKind> = runs[0].lines.iter().map(|l| l.kind).collect();
    assert_eq!(
        kinds,
        vec![
            LineKind::Start,
            LineKind::Said,
            LineKind::Did,
            LineKind::Ended
        ]
    );
    assert!(
        runs[0].lines[0].text.contains("claude-opus-5"),
        "{:?}",
        runs[0].lines
    );
    assert!(runs[0].lines[1].text.contains("Reading the mission"));
    // The call is recognisable by the field a reader recognises it by, not by
    // its whole input object printed back as JSON.
    assert_eq!(runs[0].lines[2].text, "Read /work/tree/src/lib.rs");
    assert!(
        runs[0].lines[3].text.contains("Finished"),
        "{:?}",
        runs[0].lines[3]
    );
}

/// A line `hq` cannot parse is kept and marked. A log rendered by dropping
/// what the reader did not expect hides exactly the run that went wrong.
#[test]
fn a_line_hq_cannot_read_is_kept_and_marked() {
    let world = World::new();
    world.run_log(
        "s1",
        "thread 'main' panicked at src/main.rs:1:1\nnot json either\n",
    );

    let runs = logs::of(&world.project, "m1", &harness()).unwrap();
    assert_eq!(runs[0].lines.len(), 2);
    assert!(runs[0].lines.iter().all(|l| l.kind == LineKind::Unread));
    assert!(
        runs[0].lines[0].text.contains("panicked"),
        "{:?}",
        runs[0].lines
    );
}

/// The runs come back oldest first: a mission is read forwards, and the run
/// that explains the one before it is the one after it.
#[test]
fn the_runs_come_back_oldest_first() {
    let world = World::new();
    for session in ["s3", "s1", "s2"] {
        world.run_log(session, STREAM);
    }
    let runs = logs::of(&world.project, "m1", &harness()).unwrap();
    let sessions: Vec<&str> = runs.iter().map(|r| r.session.as_str()).collect();
    assert_eq!(sessions, vec!["s1", "s2", "s3"]);
}

/// A run writes its stderr beside its stream. Rendering that as a run would
/// put the wrapper's words in the agent's mouth.
#[test]
fn only_the_structured_stream_is_read_and_not_what_sits_beside_it() {
    let world = World::new();
    world.run_log("s1", STREAM);
    let runs_dir = world.project.hq_root.join("missions/m1/runs");
    std::fs::write(runs_dir.join("s1.err"), "warning: something\n").unwrap();
    std::fs::write(runs_dir.join("s1.pid"), "41\n").unwrap();

    let runs = logs::of(&world.project, "m1", &harness()).unwrap();
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(runs[0].session, "s1");
}

/// A mission that has never run says so, in a sentence about the mission —
/// not about a directory that does not exist.
#[test]
fn a_mission_with_no_run_says_so_and_an_unknown_one_says_something_else() {
    let world = World::new();
    let err = logs::of(&world.project, "m1", &harness()).unwrap_err();
    assert!(matches!(err, LogsError::NoRuns(_)), "{err}");

    let err = logs::of(&world.project, "nope", &harness()).unwrap_err();
    assert!(matches!(err, LogsError::NoMission(_)), "{err}");
}

/// `hq logs` with no argument means the mission this HQ touched last, and
/// choosing for the reader without telling them which is a guess dressed as a
/// convenience — so the caller is given the name to print.
#[test]
fn the_default_mission_is_the_one_this_hq_touched_last() {
    let world = World::new();
    assert_eq!(logs::most_recent(&world.project).unwrap(), None);

    world.started("m1", "2026-09-10T09:00:00Z");
    world.started("m2", "2026-09-10T11:00:00Z");
    world.started("m3", "2026-09-10T10:00:00Z");
    assert_eq!(
        logs::most_recent(&world.project).unwrap().as_deref(),
        Some("m2")
    );
}

/// A run's own heartbeat is not the run. Measured on a real five-minute run:
/// 127 of its events were progress telemetry — thinking tokens, task
/// notifications — and one line each buried what the agent actually did. They
/// are counted and said once instead, and `Noted` says so: "I read this and
/// it is not worth a line" is a different fact from "I could not read this",
/// and a renderer that says neither is one the reader cannot calibrate.
#[test]
fn a_runs_own_heartbeat_is_counted_and_said_once() {
    let world = World::new();
    let mut stream = String::from(r#"{"type":"system","subtype":"init","model":"claude-opus-5"}"#);
    stream.push('\n');
    for _ in 0..5 {
        stream.push_str(r#"{"type":"system","subtype":"thinking_tokens"}"#);
        stream.push('\n');
    }
    stream.push_str(
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/a"}}]}}"#,
    );
    stream.push('\n');
    world.run_log("s1", &stream);

    let runs = logs::of(&world.project, "m1", &harness()).unwrap();
    let kinds: Vec<LineKind> = runs[0].lines.iter().map(|l| l.kind).collect();
    assert_eq!(
        kinds,
        vec![LineKind::Start, LineKind::Did, LineKind::Noted],
        "{:?}",
        runs[0].lines
    );
    let noted = runs[0].lines.last().unwrap();
    assert!(noted.text.starts_with("5 further"), "{noted:?}");
    // Only `init` carries the model, so only `init` may claim one.
    assert_eq!(runs[0].lines[0].text, "init — model claude-opus-5");
    assert!(
        !runs[0].lines.iter().any(|l| l.text.contains("model ?")),
        "{:?}",
        runs[0].lines
    );
}

/// And a run with no telemetry at all gets no line about it: a renderer that
/// always says "0 further events" is noise of its own.
#[test]
fn a_run_with_nothing_hidden_says_nothing_about_it() {
    let world = World::new();
    world.run_log("s1", STREAM);
    let runs = logs::of(&world.project, "m1", &harness()).unwrap();
    assert!(
        !runs[0].lines.iter().any(|l| l.kind == LineKind::Noted),
        "{:?}",
        runs[0].lines
    );
}
