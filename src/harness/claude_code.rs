//! The Claude Code adapter (SPEC 4.3), against the CLI as installed:
//! `claude -p` with `--output-format stream-json`, sessions imposed by `hq`
//! with `--session-id` and resumed with `--resume`, permissions refused
//! rather than asked (`--permission-mode dontAsk --permission-prompts none`),
//! and **never `--bare`**: it skips `CLAUDE.md` and ignores the
//! subscription token, and the subscription is the only login (SPEC 4.3,
//! "L'abonnement, et rien d'autre").
//!
//! The shapes parsed here were captured from Claude Code 2.1.266 and kept
//! under `tests/fixtures/`; the live test against the installed CLI is
//! `#[ignore]`d because it spends the human's subscription.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;

use serde_json::Value;

use super::spawn::{CommandSpec, Signal, Spawned, Spawner};
use super::{
    Exposure, GuardSetup, Harness, HarnessError, Outcome, Progress, Provisioning, Role, RunHandle,
    RunRequest, RunState, Usage,
};

/// What is fixed per project or per mission, not per run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The binary, `claude` unless the image puts it elsewhere.
    pub program: String,
    /// Model to request, or the CLI's default.
    pub model: Option<String>,
    /// Upper bound on agentic turns per run, or none.
    pub max_turns: Option<u32>,
    /// Where the harness keeps config and sessions (`CLAUDE_CONFIG_DIR`).
    /// In a container: a named volume per slot, set by the engine. `None`
    /// leaves the CLI to its own default — the human's login on this machine,
    /// which is what a local run and the live test want.
    pub config_dir: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            program: "claude".into(),
            model: None,
            max_turns: None,
            config_dir: None,
        }
    }
}

pub struct ClaudeCode {
    config: Config,
    spawner: Box<dyn Spawner>,
}

impl ClaudeCode {
    pub fn new(config: Config, spawner: Box<dyn Spawner>) -> Self {
        Self { config, spawner }
    }

    /// The command line for a run. Pure: the tests assert on it without
    /// running anything.
    pub fn command(
        &self,
        request: &RunRequest,
        guards: &GuardSetup,
        exposure: &Exposure,
    ) -> CommandSpec {
        let mut args: Vec<String> = vec![
            "-p".into(),
            "--output-format".into(),
            "stream-json".into(),
            "--verbose".into(),
            "--permission-mode".into(),
            "dontAsk".into(),
            "--permission-prompts".into(),
            "none".into(),
        ];
        if request.resume {
            args.extend(["--resume".into(), request.session.0.clone()]);
        } else {
            args.extend(["--session-id".into(), request.session.0.clone()]);
        }
        if let Some(model) = &self.config.model {
            args.extend(["--model".into(), model.clone()]);
        }
        if let Some(n) = self.config.max_turns {
            args.extend(["--max-turns".into(), n.to_string()]);
        }
        match exposure {
            Exposure::SystemPromptFile(path) => {
                args.extend([
                    "--append-system-prompt-file".into(),
                    path.display().to_string(),
                ]);
            }
            Exposure::UserMessage(_) => {}
        }
        args.extend(guards.args.iter().cloned());
        args.push(lot_prompt(request, exposure));
        let mut env = BTreeMap::new();
        if let Some(dir) = &self.config.config_dir {
            env.insert("CLAUDE_CONFIG_DIR".into(), dir.display().to_string());
        }
        CommandSpec {
            program: self.config.program.clone(),
            args,
            cwd: request.workspace.tree.clone(),
            env,
        }
    }

    /// On the host, never in the container: `mission_dir` is a container
    /// path, and the folder it names is mounted read-only but for three
    /// files.
    fn log_path(request: &RunRequest) -> PathBuf {
        request
            .runs_dir
            .join(format!("{}.jsonl", request.session.0))
    }
}

/// The user message that starts a run: which lot, which attempt, where the
/// mission files are. The role prompt itself travels as a system prompt.
fn lot_prompt(request: &RunRequest, exposure: &Exposure) -> String {
    let base = format!(
        "Mission folder: {}. Work on lot `{}` (attempt {}). Read MISSION.md and JOURNAL.md first; \
         write your ÉTAT DE REPRISE block in JOURNAL.md at every checkpoint and before you stop.",
        request.workspace.mission_dir.display(),
        request.lot,
        request.attempt
    );
    match exposure {
        Exposure::UserMessage(m) => format!("{m}\n\n{base}"),
        Exposure::SystemPromptFile(_) => base,
    }
}

impl Harness for ClaudeCode {
    fn name(&self) -> &'static str {
        "claude-code"
    }

    fn token_env(&self) -> &'static str {
        // The subscription's long-lived token, never an API key (SPEC 4.3).
        "CLAUDE_CODE_OAUTH_TOKEN"
    }

    fn provision(&self) -> Provisioning {
        Provisioning {
            install: vec!["npm install -g @anthropic-ai/claude-code".into()],
            config_dir: self.config.config_dir.clone(),
            // The model API, and the feature-flag endpoint the CLI calls at
            // start; both were in the inherited allowlist.
            domains: vec!["api.anthropic.com".into(), "statsig.anthropic.com".into()],
        }
    }

    fn launch(
        &self,
        request: &RunRequest,
        guards: &GuardSetup,
        exposure: &Exposure,
    ) -> Result<RunHandle, HarnessError> {
        let cmd = self.command(request, guards, exposure);
        let log = Self::log_path(request);
        let spawned = self
            .spawner
            .spawn(&cmd, &log)
            .map_err(|e| HarnessError::Launch(e.to_string()))?;
        Ok(RunHandle {
            session: request.session.clone(),
            container: spawned.container,
            pid: spawned.pid,
            log,
        })
    }

    fn state(&self, handle: &RunHandle) -> Result<RunState, HarnessError> {
        let text = match fs::read_to_string(&handle.log) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(HarnessError::Io(e)),
        };
        let parsed = parse_stream(&text);
        if let Some(outcome) = parsed.outcome {
            return Ok(RunState::Finished(outcome));
        }
        // Asked of the spawner, because where the process lives decides
        // how the question is put: a pid on this machine, or a pid inside a
        // container that only the engine can reach.
        let alive = self
            .spawner
            .alive(&Spawned {
                pid: handle.pid,
                container: handle.container.clone(),
            })
            .map_err(HarnessError::Io)?;
        if alive {
            Ok(RunState::Running(parsed.progress))
        } else {
            Ok(RunState::Finished(Outcome::HarnessFailure(
                "the harness process ended without a result event".into(),
            )))
        }
    }

    fn stop(&self, handle: &RunHandle) -> Result<(), HarnessError> {
        if handle.pid.is_none() {
            return Err(HarnessError::UnknownRun(handle.session.clone()));
        }
        // SIGINT ends the turn properly; SIGTERM would leave it unfinished.
        self.spawner
            .signal(
                &Spawned {
                    pid: handle.pid,
                    container: handle.container.clone(),
                },
                Signal::Interrupt,
            )
            .map_err(HarnessError::Io)
    }

    fn expose(&self, _role: Role, prompt_file: PathBuf) -> Exposure {
        Exposure::SystemPromptFile(prompt_file)
    }
}

/// What a stream-json log says so far.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    pub progress: Progress,
    pub outcome: Option<Outcome>,
    pub session_id: Option<String>,
}

/// Parse the stream-json lines Claude Code writes. Tolerant of a truncated
/// last line (the process may be mid-write).
pub fn parse_stream(text: &str) -> Parsed {
    let mut parsed = Parsed::default();
    let mut last_call: Option<String> = None;
    let mut repeat: u32 = 0;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        parsed.progress.events += 1;
        if let Some(id) = event.get("session_id").and_then(Value::as_str) {
            parsed.session_id = Some(id.to_string());
        }
        match event.get("type").and_then(Value::as_str) {
            Some("assistant") => {
                let blocks = event
                    .pointer("/message/content")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                for block in blocks {
                    if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                        parsed.progress.tool_calls += 1;
                        let key = format!(
                            "{}:{}",
                            block.get("name").and_then(Value::as_str).unwrap_or(""),
                            block.get("input").map(Value::to_string).unwrap_or_default()
                        );
                        if last_call.as_deref() == Some(key.as_str()) {
                            repeat += 1;
                        } else {
                            repeat = 1;
                            last_call = Some(key);
                        }
                        parsed.progress.longest_repeat = parsed.progress.longest_repeat.max(repeat);
                    }
                }
            }
            Some("result") => {
                parsed.outcome = Some(classify_result(&event));
            }
            _ => {}
        }
    }
    parsed
}

/// A `result` event into an [`Outcome`]. An error whose text names the
/// harness's own conditions — quota, authentication, network — is a harness
/// failure, replayed without counting; any other error is the mission's.
fn classify_result(event: &Value) -> Outcome {
    let is_error = event
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let usage = Usage {
        input_tokens: event
            .pointer("/usage/input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        output_tokens: event
            .pointer("/usage/output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    };
    if !is_error {
        return Outcome::Finished(usage);
    }
    // The text to classify: `result` when it is a string, else the whole
    // event — an error may sit in an `errors` array or a nested field.
    let text = match event.get("result").and_then(Value::as_str) {
        Some(r) => r.to_lowercase(),
        None => event.to_string().to_lowercase(),
    };
    let status = event.get("api_error_status").and_then(Value::as_u64);
    let harness_cause = matches!(status, Some(401 | 403 | 408 | 429 | 500..=599))
        || [
            "rate limit",
            "rate_limit",
            "overloaded",
            "quota",
            "authentication",
            "unauthorized",
            "not logged in",
            "login",
            "credential",
            "network",
            "econn",
            "timed out",
            "timeout",
            "api error",
        ]
        .iter()
        .any(|needle| text.contains(needle));
    let message = match event.get("result").and_then(Value::as_str) {
        Some(r) => r.to_string(),
        None => event.to_string().chars().take(400).collect(),
    };
    if harness_cause {
        Outcome::HarnessFailure(message)
    } else {
        Outcome::MissionFailure(message)
    }
}

/// Arguments that must never appear (SPEC 4.3): `--bare` skips `CLAUDE.md`
/// and ignores the subscription token. Checked by the tests, and by
/// `hq check` against the adapter's own command.
pub const FORBIDDEN_ARGS: &[&str] = &["--bare"];

impl std::fmt::Debug for ClaudeCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeCode")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}
