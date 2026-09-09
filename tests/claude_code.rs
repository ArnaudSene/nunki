//! The Claude Code adapter (SPEC 4.3), against captured output and the
//! command line it builds. The live test needs the installed CLI and spends
//! the subscription: `cargo test -- --ignored live_` runs it on purpose.

use std::fs;
use std::path::PathBuf;

use hq::harness::claude_code::{ClaudeCode, Config, FORBIDDEN_ARGS, parse_stream};
use hq::harness::spawn::{CommandSpec, LocalSpawner, Spawned, Spawner};
use hq::harness::{
    Exposure, GuardSetup, Harness, Outcome, Progress, Role, RunHandle, RunRequest, RunState,
    SessionId, Workspace,
};

fn request(resume: bool) -> RunRequest {
    RunRequest {
        role: Role::Coder,
        workspace: Workspace {
            tree: PathBuf::from("/work/tree"),
            mission_dir: PathBuf::from("/work/missions/m1"),
        },
        lot: "L2".into(),
        attempt: 1,
        session: SessionId("11111111-2222-4333-8444-555555555555".into()),
        resume,
    }
}

fn adapter() -> ClaudeCode {
    ClaudeCode::new(Config::default(), Box::new(LocalSpawner))
}

fn fixture() -> String {
    fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/claude-code-v2.1.266-ok.jsonl"
    ))
    .unwrap()
}

#[test]
fn the_command_line_is_headless_refuses_prompts_and_never_bare() {
    let cmd = adapter().command(
        &request(false),
        &GuardSetup::default(),
        &Exposure::SystemPromptFile("/roles/coder.md".into()),
    );
    let line = cmd.display();
    for must in [
        "-p",
        "--output-format stream-json",
        "--permission-mode dontAsk",
        "--permission-prompts none",
        "--session-id 11111111-2222-4333-8444-555555555555",
        "--append-system-prompt-file /roles/coder.md",
    ] {
        assert!(line.contains(must), "missing `{must}` in `{line}`");
    }
    for never in FORBIDDEN_ARGS {
        assert!(
            !cmd.args.iter().any(|a| a == never),
            "`{never}` must never be passed"
        );
    }
    assert!(!line.contains("--resume"));
    assert_eq!(cmd.cwd, PathBuf::from("/work/tree"));
    // Without a config dir, the CLI keeps its own (the human's login).
    assert!(!cmd.env.contains_key("CLAUDE_CONFIG_DIR"));
    // The lot and the mission folder reach the agent in its first message.
    let last = cmd.args.last().unwrap();
    assert!(last.contains("lot `L2`") && last.contains("/work/missions/m1"));
}

#[test]
fn resuming_uses_the_same_session_id_with_resume() {
    let cmd = adapter().command(
        &request(true),
        &GuardSetup::default(),
        &Exposure::SystemPromptFile("/r.md".into()),
    );
    let line = cmd.display();
    assert!(line.contains("--resume 11111111-2222-4333-8444-555555555555"));
    assert!(!line.contains("--session-id"));
}

#[test]
fn guards_and_config_are_passed_at_invocation() {
    let config = Config {
        model: Some("claude-opus-5".into()),
        max_turns: Some(40),
        config_dir: Some("/home/agent/.claude".into()),
        ..Config::default()
    };
    let hq = ClaudeCode::new(config, Box::new(LocalSpawner));
    let guards = GuardSetup {
        args: vec!["--settings".into(), "{\"hooks\":{}}".into()],
    };
    let cmd = hq.command(
        &request(false),
        &guards,
        &Exposure::SystemPromptFile("/r.md".into()),
    );
    let line = cmd.display();
    assert_eq!(
        cmd.env.get("CLAUDE_CONFIG_DIR").map(String::as_str),
        Some("/home/agent/.claude")
    );
    assert!(line.contains("--model claude-opus-5"));
    assert!(line.contains("--max-turns 40"));
    assert!(line.contains("--settings {\"hooks\":{}}"));
}

#[test]
fn a_captured_successful_run_parses_as_finished_with_its_usage() {
    let parsed = parse_stream(&fixture());
    assert_eq!(
        parsed.session_id.as_deref(),
        Some("e57525dc-e97f-4b71-b094-3adb7dce5a49")
    );
    assert_eq!(parsed.progress.events, 5);
    assert_eq!(parsed.progress.tool_calls, 0);
    match parsed.outcome {
        Some(Outcome::Finished(usage)) => {
            assert_eq!((usage.input_tokens, usage.output_tokens), (2, 4))
        }
        other => panic!("expected Finished, got {other:?}"),
    }
}

fn tool_use(name: &str, input: &str) -> String {
    format!(
        r#"{{"type":"assistant","session_id":"s","message":{{"content":[{{"type":"tool_use","name":"{name}","input":{input}}}]}}}}"#
    )
}

#[test]
fn a_run_in_progress_reports_its_tool_calls_and_repeats() {
    let mut log = String::from(r#"{"type":"system","subtype":"init","session_id":"s"}"#);
    log.push('\n');
    for _ in 0..3 {
        log.push_str(&tool_use("Bash", r#"{"command":"cargo build"}"#));
        log.push('\n');
    }
    log.push_str(&tool_use("Read", r#"{"file_path":"x.rs"}"#));
    log.push_str("\n{\"type\":\"assistant\",\"truncated");
    let parsed = parse_stream(&log);
    assert!(parsed.outcome.is_none());
    assert_eq!(
        parsed.progress,
        Progress {
            events: 5,
            tool_calls: 4,
            longest_repeat: 3
        }
    );
}

#[test]
fn an_error_result_is_classified_by_its_cause() {
    let rate = r#"{"type":"result","subtype":"error","is_error":true,"result":"API Error: 429 rate limit reached","usage":{}}"#;
    let auth = r#"{"type":"result","subtype":"error","is_error":true,"result":"boom","api_error_status":401}"#;
    let login = r#"{"type":"result","subtype":"error","is_error":true,"result":"Not logged in · Please run /login"}"#;
    let mission = r#"{"type":"result","subtype":"error_max_turns","is_error":true,"result":"Reached max turns"}"#;
    assert!(matches!(
        parse_stream(rate).outcome,
        Some(Outcome::HarnessFailure(_))
    ));
    assert!(matches!(
        parse_stream(auth).outcome,
        Some(Outcome::HarnessFailure(_))
    ));
    assert!(matches!(
        parse_stream(login).outcome,
        Some(Outcome::HarnessFailure(_))
    ));
    assert!(matches!(
        parse_stream(mission).outcome,
        Some(Outcome::MissionFailure(_))
    ));
}

#[test]
fn state_reads_the_log_and_the_process() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("runs/s.jsonl");
    fs::create_dir_all(log.parent().unwrap()).unwrap();
    let hq = adapter();
    let session = SessionId("s".into());

    // No result yet, our own pid is alive: running.
    fs::write(
        &log,
        r#"{"type":"system","subtype":"init","session_id":"s"}"#,
    )
    .unwrap();
    let alive = RunHandle {
        session: session.clone(),
        container: String::new(),
        pid: Some(std::process::id()),
        log: log.clone(),
    };
    assert!(matches!(hq.state(&alive).unwrap(), RunState::Running(p) if p.events == 1));

    // No result, and the process is gone: a harness failure, replayed later.
    let child = std::process::Command::new("true").spawn().unwrap();
    let dead_pid = child.id();
    let _ = child.wait_with_output();
    let dead = RunHandle {
        pid: Some(dead_pid),
        ..alive.clone()
    };
    assert!(matches!(
        hq.state(&dead).unwrap(),
        RunState::Finished(Outcome::HarnessFailure(_))
    ));

    // A result: finished, whatever the process.
    fs::write(&log, fixture()).unwrap();
    assert!(matches!(
        hq.state(&dead).unwrap(),
        RunState::Finished(Outcome::Finished(_))
    ));

    // No log at all yet, process alive: running with nothing.
    let none = RunHandle {
        log: dir.path().join("nope.jsonl"),
        ..alive
    };
    assert!(matches!(hq.state(&none).unwrap(), RunState::Running(p) if p.events == 0));
}

/// A spawner that records what it was asked and starts nothing.
struct Recording(std::sync::Mutex<Vec<(CommandSpec, PathBuf)>>);
impl Spawner for Recording {
    fn spawn(&self, cmd: &CommandSpec, log: &std::path::Path) -> std::io::Result<Spawned> {
        self.0
            .lock()
            .unwrap()
            .push((cmd.clone(), log.to_path_buf()));
        Ok(Spawned {
            pid: None,
            container: "c1".into(),
        })
    }
}

#[test]
fn launch_writes_the_log_under_the_mission_folder_and_keeps_the_handle_persistable() {
    let rec = std::sync::Arc::new(Recording(Default::default()));
    let hq = ClaudeCode::new(Config::default(), Box::new(RecordingRef(rec.clone())));
    let handle = hq
        .launch(
            &request(false),
            &GuardSetup::default(),
            &Exposure::SystemPromptFile("/r.md".into()),
        )
        .unwrap();
    assert_eq!(
        handle.log,
        PathBuf::from("/work/missions/m1/runs/11111111-2222-4333-8444-555555555555.jsonl")
    );
    assert_eq!(handle.container, "c1");
    let calls = rec.0.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1, handle.log);
    let json = serde_json::to_string(&handle).unwrap();
    assert_eq!(serde_json::from_str::<RunHandle>(&json).unwrap(), handle);
}

struct RecordingRef(std::sync::Arc<Recording>);
impl Spawner for RecordingRef {
    fn spawn(&self, cmd: &CommandSpec, log: &std::path::Path) -> std::io::Result<Spawned> {
        self.0.spawn(cmd, log)
    }
}

/// Against the installed CLI, on the subscription. Run on purpose only.
#[test]
#[ignore = "spends the subscription; run with --ignored"]
fn live_a_real_headless_run_finishes_with_a_result() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/live");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    struct Dir(PathBuf);
    impl Dir {
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }
    let dir = Dir(root);
    let hq = ClaudeCode::new(
        Config {
            max_turns: Some(1),
            ..Config::default()
        },
        Box::new(LocalSpawner),
    );
    let req = RunRequest {
        workspace: Workspace {
            tree: dir.path().to_path_buf(),
            mission_dir: dir.path().join("m"),
        },
        session: SessionId(uuid_v4()),
        ..request(false)
    };
    let exposure = Exposure::UserMessage("Reply with exactly the word OK and nothing else.".into());
    let handle = hq.launch(&req, &GuardSetup::default(), &exposure).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        match hq.state(&handle).unwrap() {
            RunState::Finished(Outcome::Finished(usage)) => {
                assert!(usage.output_tokens > 0);
                break;
            }
            RunState::Finished(other) => {
                let log = fs::read_to_string(&handle.log).unwrap_or_default();
                let err = fs::read_to_string(handle.log.with_extension("err")).unwrap_or_default();
                panic!(
                    "unexpected outcome {other:?}
--- log ---
{log}
--- err ---
{err}"
                );
            }
            RunState::Running(_) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            RunState::Running(p) => panic!("still running after 120 s: {p:?}"),
        }
    }
}

fn uuid_v4() -> String {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let b = n.to_le_bytes();
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-4{:01x}{:02x}-8{:01x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0],
        b[1],
        b[2],
        b[3],
        b[4],
        b[5],
        b[6] & 0xf,
        b[7],
        b[8] & 0xf,
        b[9],
        b[10],
        b[11],
        b[12],
        b[13],
        b[14],
        b[15]
    )
}
