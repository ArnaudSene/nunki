//! The Claude Code adapter (SPEC 4.3), against captured output and the
//! command line it builds. The live test needs the installed CLI and spends
//! the subscription: `cargo test -- --ignored live_` runs it on purpose.

use std::fs;
use std::path::PathBuf;

use nunki::harness::claude_code::{ClaudeCode, Config, FORBIDDEN_ARGS, parse_stream};
use nunki::harness::spawn::{CommandSpec, LocalSpawner, Presence, Spawned, Spawner};
use nunki::harness::{
    Exposure, GuardSetup, Harness, Outcome, Progress, Role, RunHandle, RunRequest, RunState,
    SessionId, Workspace,
};

fn request(resume: bool) -> RunRequest {
    RunRequest {
        role: Role::Coder,
        // Container paths, all of them.
        workspace: Workspace {
            tree: PathBuf::from("/work/tree"),
            mission_dir: PathBuf::from("/work/mission"),
        },
        lot: "L2".into(),
        attempt: 1,
        session: SessionId("11111111-2222-4333-8444-555555555555".into()),
        resume,
        // A host path, and deliberately not under the mission folder.
        runs_dir: PathBuf::from("/nunki/demo/missions/m1/runs"),
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
        "--permission-mode auto",
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
    // The agent is told the container's path, not the host's: what it
    // reads is what it is handed.
    assert!(last.contains("lot `L2`") && last.contains("/work/mission"));
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
        // Not the default: what is asserted below is that the project's
        // choice reaches the command line, not that the default does.
        permission_mode: "dontAsk".into(),
        config_dir: Some("/home/agent/.claude".into()),
        ..Config::default()
    };
    let nunki = ClaudeCode::new(config, Box::new(LocalSpawner));
    let guards = GuardSetup {
        args: vec!["--settings".into(), "{\"hooks\":{}}".into()],
    };
    let cmd = nunki.command(
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
    assert!(line.contains("--permission-mode dontAsk"));
    // Refused whatever the mode: it is what makes a prompt impossible.
    assert!(line.contains("--permission-prompts none"));
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
    // The four kinds, as v2.1.266 reports them: the cache read is nearly all
    // of it, and a count of input and output alone would have said six.
    let spent = nunki::harness::Usage {
        input_tokens: 2,
        output_tokens: 4,
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: 16_964,
    };
    match parsed.outcome {
        Some(Outcome::Finished(usage)) => assert_eq!(usage, spent),
        other => panic!("expected Finished, got {other:?}"),
    }
    assert_eq!(parsed.usage.as_ref().map(|u| u.total()), Some(16_970));
}

/// A run that fell for a quota spent tokens too, and a cap that counted only
/// the runs that succeeded would undercount the ones that cost the most.
#[test]
fn a_run_that_failed_still_says_what_it_spent() {
    let quota = r#"{"type":"result","subtype":"error","is_error":true,"result":"API Error: 429 rate limit reached","usage":{"input_tokens":3,"output_tokens":1,"cache_read_input_tokens":500}}"#;
    let parsed = parse_stream(quota);
    assert!(
        matches!(parsed.outcome, Some(Outcome::HarnessFailure(_))),
        "{parsed:?}"
    );
    assert_eq!(parsed.usage.map(|u| u.total()), Some(504));
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
    // A quota may pass by itself; a 401 or a logged-out session will not,
    // and the difference is read here, where the status is still in hand —
    // the 401 below says only "boom".
    let authentication = |log: &str| match parse_stream(log).outcome {
        Some(Outcome::HarnessFailure(fault)) => fault.authentication,
        other => panic!("{other:?}"),
    };
    assert!(!authentication(rate));
    assert!(authentication(auth));
    assert!(authentication(login));
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
    let nunki = adapter();
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
    assert!(matches!(nunki.state(&alive).unwrap(), RunState::Running(p) if p.events == 1));

    // No result, and the process is gone: a harness failure, replayed later.
    let child = std::process::Command::new("true").spawn().unwrap();
    let dead_pid = child.id();
    let _ = child.wait_with_output();
    let dead = RunHandle {
        pid: Some(dead_pid),
        ..alive.clone()
    };
    assert!(matches!(
        nunki.state(&dead).unwrap(),
        RunState::Finished(Outcome::HarnessFailure(_))
    ));

    // A result: finished, whatever the process.
    fs::write(&log, fixture()).unwrap();
    assert!(matches!(
        nunki.state(&dead).unwrap(),
        RunState::Finished(Outcome::Finished(_))
    ));

    // No log at all yet, process alive: running with nothing.
    let none = RunHandle {
        log: dir.path().join("nope.jsonl"),
        ..alive
    };
    assert!(matches!(nunki.state(&none).unwrap(), RunState::Running(p) if p.events == 0));
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

    fn alive(&self, _spawned: &Spawned) -> std::io::Result<Presence> {
        Ok(Presence::Ended)
    }

    fn signal(
        &self,
        _spawned: &Spawned,
        _signal: nunki::harness::spawn::Signal,
    ) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn launch_writes_the_log_where_hq_can_read_it_not_where_the_agent_runs() {
    let rec = std::sync::Arc::new(Recording(Default::default()));
    let nunki = ClaudeCode::new(Config::default(), Box::new(RecordingRef(rec.clone())));
    let handle = nunki
        .launch(
            &request(false),
            &GuardSetup::default(),
            &Exposure::SystemPromptFile("/r.md".into()),
        )
        .unwrap();
    // The host's runs directory, never the container's mission folder: that
    // one is mounted read-only but for the agent's own files, and its path
    // means
    // nothing on this side of the mount.
    assert_eq!(
        handle.log,
        PathBuf::from("/nunki/demo/missions/m1/runs/11111111-2222-4333-8444-555555555555.jsonl")
    );
    assert!(!handle.log.starts_with("/work"), "{:?}", handle.log);
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

    fn alive(&self, spawned: &Spawned) -> std::io::Result<Presence> {
        self.0.alive(spawned)
    }

    fn signal(
        &self,
        spawned: &Spawned,
        signal: nunki::harness::spawn::Signal,
    ) -> std::io::Result<()> {
        self.0.signal(spawned, signal)
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
    let nunki = ClaudeCode::new(
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
        // A run on the host: the container view and the host view coincide
        // here, and both have to be said rather than derived from each other.
        runs_dir: dir.path().join("runs"),
        ..request(false)
    };
    let exposure = Exposure::UserMessage("Reply with exactly the word OK and nothing else.".into());
    let handle = nunki
        .launch(&req, &GuardSetup::default(), &exposure)
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        match nunki.state(&handle).unwrap() {
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
            RunState::Paused(p) => panic!("nothing froze this run: {p:?}"),
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

#[test]
fn a_role_is_allowed_the_tools_its_work_needs_and_the_prompt_survives() {
    let nunki = adapter();
    let guards = nunki.guards(Role::Coder);
    let cmd = nunki.command(
        &request(false),
        &guards,
        &Exposure::UserMessage("do the thing".into()),
    );

    // Refusing everything is not a guard, it is a container that cannot
    // work: with nothing allowed, the first real run had Bash and Write
    // refused and ended without a result.
    let allowed = guards.args.join(" ");
    for tool in ["Read", "Write", "Bash", "Edit", "Grep"] {
        assert!(allowed.contains(tool), "{allowed}");
    }

    // One token, not two. `--allowedTools` is variadic, so the separated
    // form swallows the prompt that follows it — measured, and the run died
    // on "Input must be provided either through stdin or as a prompt
    // argument".
    // Asserted as the property rather than by counting the arguments: the
    // guards carry the invocation's settings too since 2026-09-16, and a
    // count would go red for a reason that has nothing to do with the trap
    // it was written for.
    assert_eq!(
        guards
            .args
            .iter()
            .filter(|a| a.starts_with("--allowedTools"))
            .count(),
        1,
        "{:?}",
        guards.args
    );
    assert!(
        guards.args.iter().any(|a| a.starts_with("--allowedTools=")),
        "the separated form would swallow the prompt: {:?}",
        guards.args
    );
    assert_eq!(
        cmd.args
            .last()
            .map(String::as_str)
            .map(|a| a.contains("do the thing")),
        Some(true),
        "the prompt must still be the last argument: {:?}",
        cmd.args
    );
}

/// A spawner that answers `answer`, and — the point of it — writes `writes`
/// into the log at the moment it is asked. That is the race, made
/// deterministic: the harness emits its `result` and exits between `nunki`'s
/// two reads.
struct EndsWhileAsked {
    answer: Presence,
    writes: Option<(PathBuf, String)>,
}

impl Spawner for EndsWhileAsked {
    fn spawn(&self, _cmd: &CommandSpec, _log: &std::path::Path) -> std::io::Result<Spawned> {
        unreachable!("this spawner is only ever asked about liveness")
    }

    fn alive(&self, _spawned: &Spawned) -> std::io::Result<Presence> {
        if let Some((log, text)) = &self.writes {
            fs::write(log, text)?;
        }
        Ok(self.answer.clone())
    }

    fn signal(
        &self,
        _spawned: &Spawned,
        _signal: nunki::harness::spawn::Signal,
    ) -> std::io::Result<()> {
        Ok(())
    }
}

fn handle_for(log: PathBuf) -> RunHandle {
    RunHandle {
        session: SessionId("s".into()),
        container: "c1".into(),
        pid: Some(4242),
        log,
    }
}

/// The ordering, and it is the whole test: liveness is asked **before** the
/// log is read.
///
/// A run that emits its `result` and exits in between is a run that said how
/// it ended. Read the log first and the process second, and `nunki` sees an
/// empty log and a dead process, and files a harness failure against an
/// agent that had just succeeded. A process that has ended writes nothing
/// more, so the log read after the answer is complete — which is why this
/// order and not the other.
#[test]
fn a_run_that_ends_between_the_two_reads_is_not_a_harness_failure() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("s.jsonl");
    fs::write(
        &log,
        r#"{"type":"system","subtype":"init","session_id":"s"}"#,
    )
    .unwrap();

    let nunki = ClaudeCode::new(
        Config::default(),
        Box::new(EndsWhileAsked {
            answer: Presence::Ended,
            writes: Some((log.clone(), fixture())),
        }),
    );
    match nunki.state(&handle_for(log)).unwrap() {
        RunState::Finished(Outcome::Finished(_)) => {}
        other => panic!("the run said how it ended and nunki must read it, got {other:?}"),
    }
}

/// A run `nunki` cannot reach is not a run that died.
///
/// Everything that goes wrong between here and the process — the container
/// taken down, the engine not running, the profile recycled — used to come
/// back as "not alive", and "not alive" plus a log without a `result` is a
/// harness failure recorded against the agent. `nunki` does not know, and the
/// only honest answer is to say so.
#[test]
fn a_run_that_cannot_be_reached_is_not_a_run_that_died() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("s.jsonl");
    fs::write(
        &log,
        r#"{"type":"system","subtype":"init","session_id":"s"}"#,
    )
    .unwrap();

    let nunki = ClaudeCode::new(
        Config::default(),
        Box::new(EndsWhileAsked {
            answer: Presence::Unknown("the engine did not answer: no such daemon".into()),
            writes: None,
        }),
    );
    let err = nunki.state(&handle_for(log.clone())).unwrap_err();
    let said = err.to_string();
    assert!(
        matches!(err, nunki::harness::HarnessError::Unreachable(_)),
        "{said}"
    );
    // And it carries the reason, so the human knows where to look.
    assert!(said.contains("no such daemon"), "{said}");
    assert!(
        !said.contains("without a result event"),
        "an unreachable run must not be described as a harness that died: {said}"
    );
}

/// A container that went away is a harness failure — SPEC 4.2 says the run
/// is interrupted without costing an attempt — but it is **not** the same
/// harness failure as a process that quit on its own, and the record has to
/// tell them apart. One is about the container, the other about the agent.
#[test]
fn a_container_that_went_away_says_so_and_does_not_blame_the_agent() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("s.jsonl");
    fs::write(
        &log,
        r#"{"type":"system","subtype":"init","session_id":"s"}"#,
    )
    .unwrap();

    let nunki = ClaudeCode::new(
        Config::default(),
        Box::new(EndsWhileAsked {
            answer: Presence::Vanished("the engine no longer knows container abc123def456".into()),
            writes: None,
        }),
    );
    match nunki.state(&handle_for(log)).unwrap() {
        RunState::Finished(Outcome::HarnessFailure(nunki::harness::Fault { why, .. })) => {
            assert!(why.contains("abc123def456"), "{why}");
            assert!(
                !why.contains("without a result event"),
                "that sentence is about the agent, and this was about the container: {why}"
            );
        }
        other => panic!("expected a harness failure naming the container, got {other:?}"),
    }
}

/// A spawner with one answer, for the states a real process cannot be put in
/// from a test.
struct Answering(Presence);
impl Spawner for Answering {
    fn spawn(&self, _cmd: &CommandSpec, _log: &std::path::Path) -> std::io::Result<Spawned> {
        unimplemented!("this spawner only answers about a run")
    }
    fn alive(&self, _spawned: &Spawned) -> std::io::Result<Presence> {
        Ok(self.0.clone())
    }
    fn signal(
        &self,
        _spawned: &Spawned,
        _signal: nunki::harness::spawn::Signal,
    ) -> std::io::Result<()> {
        Ok(())
    }
}

/// A run a human froze is neither running nor finished, and saying either is
/// a lie they would act on: "running" tells them to wait for progress that
/// cannot come, "finished" tells them their run died. The adapter carries the
/// distinction through instead of flattening it.
#[test]
fn a_paused_run_is_neither_running_nor_finished() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("run.jsonl");
    fs::write(
        &log,
        r#"{"type":"system","subtype":"init","session_id":"s"}"#,
    )
    .unwrap();
    let handle = RunHandle {
        session: SessionId("s".into()),
        container: "cafe1234".into(),
        pid: Some(41),
        log: log.clone(),
    };

    let frozen = ClaudeCode::new(Default::default(), Box::new(Answering(Presence::Paused)));
    assert!(
        matches!(frozen.state(&handle).unwrap(), RunState::Paused(p) if p.events == 1),
        "{:?}",
        frozen.state(&handle)
    );

    // And the same run, unfrozen, reads as running: it is the freeze that is
    // being reported, not the log.
    let live = ClaudeCode::new(Default::default(), Box::new(Answering(Presence::Running)));
    assert!(matches!(live.state(&handle).unwrap(), RunState::Running(_)));

    // A result outranks the freeze: the run said how it ended, and no reading
    // of the container contradicts that.
    fs::write(&log, fixture()).unwrap();
    assert!(matches!(
        frozen.state(&handle).unwrap(),
        RunState::Finished(Outcome::Finished(_))
    ));
}

/// The subscription's two windows, as v2.1.266 writes them into the run's
/// own stream (`rate_limit_event`), in thousandths.
#[test]
fn the_subscription_windows_are_read_from_the_stream() {
    let windows = parse_stream(&fixture())
        .windows
        .expect("v2.1.266 reports them");
    assert_eq!(
        windows.five_hour,
        Some(nunki::consumption::Window {
            per_mille: 530,
            resets_at: 1_789_003_800
        })
    );
    assert_eq!(
        windows.weekly,
        Some(nunki::consumption::Window {
            per_mille: 410,
            resets_at: 1_789_351_200
        })
    );
}

/// Emitted when a window moves, several times a run: the last is kept.
#[test]
fn the_last_rate_limit_event_is_the_one_kept() {
    let event = |u: f64| {
        format!(
            r#"{{"type":"rate_limit_event","rate_limit_info":{{"unifiedWindows":{{"five_hour":{{"utilization":{u},"resetsAt":10}}}}}}}}"#
        )
    };
    let log = format!("{}\n{}\n", event(0.37), event(0.40));
    let windows = parse_stream(&log).windows.unwrap();
    assert_eq!(windows.five_hour.map(|w| w.per_mille), Some(400));
    assert_eq!(
        windows.weekly, None,
        "a window not reported is absent, not zero"
    );
}

/// A resumed session keeps its id, and a log is appended to: the second run
/// gets a file of its own, and the first one's is left as it was.
#[test]
fn a_resumed_session_writes_its_run_to_a_log_of_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let rec = std::sync::Arc::new(Recording(Default::default()));
    let nunki = ClaudeCode::new(Config::default(), Box::new(RecordingRef(rec.clone())));
    let mut req = request(true);
    req.runs_dir = dir.path().to_path_buf();
    let first = dir
        .path()
        .join("11111111-2222-4333-8444-555555555555.jsonl");
    fs::write(&first, "the first run\n").unwrap();
    let launch = || {
        nunki
            .launch(
                &req,
                &GuardSetup::default(),
                &Exposure::SystemPromptFile("/r.md".into()),
            )
            .unwrap()
    };

    let second = launch();
    assert_eq!(
        second.log,
        dir.path()
            .join("11111111-2222-4333-8444-555555555555-2.jsonl")
    );
    fs::write(&second.log, "the second run\n").unwrap();
    assert_eq!(
        launch().log,
        dir.path()
            .join("11111111-2222-4333-8444-555555555555-3.jsonl")
    );
    assert_eq!(fs::read_to_string(&first).unwrap(), "the first run\n");
}

/// The coder's first message names the line that ends its lot; the other
/// roles conclude with a verdict and are not told it.
#[test]
fn the_coder_is_told_the_line_that_ends_its_lot() {
    let exposure = Exposure::SystemPromptFile("/r.md".into());
    let cmd = adapter().command(&request(false), &GuardSetup::default(), &exposure);
    let last = cmd.args.last().unwrap();
    assert!(
        last.contains("`Lot: L2 — done`") && last.contains("`Lot: L2 — failed: <why>`"),
        "{last}"
    );
    let mut integrator = request(false);
    integrator.role = Role::Integrator;
    let cmd = adapter().command(&integrator, &GuardSetup::default(), &exposure);
    assert!(!cmd.args.last().unwrap().contains("Lot:"));
}

/// The harness adds no attribution of its own: nunki turns it off at
/// invocation, where the agent cannot get it wrong.
///
/// `role.rs` asks for the same thing, and asking is worth what the agent's
/// care is worth. This is the other half — and it was the missing half:
/// `notes-api`'s integrator signed `Co-Authored-By: Claude Sonnet 5` on
/// 2026-09-16 while doing everything else it was told.
#[test]
fn the_harness_is_told_to_sign_nothing_and_says_it_once() {
    use nunki::harness::Harness;

    let nunki = ClaudeCode::new(Config::default(), Box::new(LocalSpawner));
    for role in [Role::Coder, Role::Integrator, Role::Security] {
        let guards = nunki.guards(role);
        let line = guards.args.join(" ");

        // One `--settings`, and one only: two of them and the CLI reads the
        // last, measured in the agent image on 2026-09-16 — a second flag
        // added elsewhere would silently take this one's place.
        assert_eq!(
            guards.args.iter().filter(|a| *a == "--settings").count(),
            1,
            "{line}"
        );

        let json = guards
            .args
            .iter()
            .skip_while(|a| *a != "--settings")
            .nth(1)
            .unwrap_or_else(|| panic!("{line}"));
        let settings: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(settings["attribution"]["commitTrailers"], false, "{json}");
        assert_eq!(settings["attribution"]["sessionUrl"], false, "{json}");
        // Its deprecated ancestor, for an older CLI in an older image.
        assert_eq!(settings["includeCoAuthoredBy"], false, "{json}");

        // And it reaches the command line the run is launched with.
        let cmd = nunki.command(
            &request(false),
            &guards,
            &Exposure::SystemPromptFile("/r.md".into()),
        );
        assert!(cmd.display().contains(json), "{}", cmd.display());
    }
}
