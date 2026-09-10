//! The harness boundary (SPEC 4.3).
//!
//! A harness is the coding agent's runtime — Claude Code, Codex, OpenCode.
//! `hq` never talks to one directly: it holds a `Box<dyn Harness>` and asks
//! it to launch a run, read its state, stop it. Everything that is specific
//! to a harness (command line, session resumption, structured output,
//! headless login, optional guards) lives behind this trait and nowhere
//! else. What is NOT here — role prompts, the mission protocol, gates, the
//! flow, the clone, the container — is handed in as arguments.

pub mod claude_code;
pub mod fake;
pub mod spawn;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The three agent roles of SPEC 2. The HQ and the human are not roles a
/// harness runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Role {
    Coder,
    Integrator,
    Security,
}

/// Where a run works: the slot's tree and the mission folder, **as the
/// container sees them** — every path here is a container path, and nothing
/// on the host may be derived from one. The harness only needs paths; how
/// they are mounted is the engine's business (SPEC 4.1, mounts per profile).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    /// The slot's working tree inside the container.
    pub tree: PathBuf,
    /// The mission folder inside the container (`JOURNAL.md`, `PR.md`,
    /// `VERDICT.json` writable; `MISSION.md`, `FOLLOWUP_HQ.md` read-only).
    pub mission_dir: PathBuf,
}

/// What a run is asked to do: one lot, or a retry of one (SPEC 4.3, "un run
/// par lot").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRequest {
    pub role: Role,
    pub workspace: Workspace,
    /// Identifier of the lot this run works on, from the mission header.
    pub lot: String,
    /// Which attempt this is for that lot, starting at 1.
    pub attempt: u32,
    /// Resume this harness session instead of starting a fresh one. The
    /// identifier is chosen by `hq` at the first launch and imposed on the
    /// harness, never read back from it.
    pub session: SessionId,
    /// Whether `session` already exists on the harness side.
    pub resume: bool,
    /// Where the harness's structured output is written, **as `hq` sees it**
    /// — a host path, unlike everything in [`Workspace`]. It cannot be in the
    /// mission folder: that is mounted read-only but for the agent's own
    /// files (SPEC
    /// 4.1), and a run in a container could not write there at all.
    pub runs_dir: PathBuf,
}

/// A harness session identifier, chosen by `hq`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub String);

/// Handle on a launched run. Persistable: the engine writes it to its state
/// so a restarted `hq` can re-derive whether the run is alive (SPEC 4.2,
/// "la reprise re-dérive avant de décider").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunHandle {
    pub session: SessionId,
    /// Container the run lives in, as named by the container engine; empty
    /// for a local run.
    pub container: String,
    /// Process id of the harness, when known.
    pub pid: Option<u32>,
    /// Where the harness's structured output is written, as `hq` sees it.
    pub log: PathBuf,
}

/// Why a run ended. The distinction is what SPEC 4.3 and 4.5 are built on:
/// a harness failure is replayed and consumes neither an attempt nor a
/// volet; a mission failure does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    /// The run ended on its own, having done what it could; whether the lot
    /// is actually done is read from the journal and the verdict, not from
    /// here.
    Finished(Usage),
    /// Quota, expired token, network, harness crash. Not the mission's fault.
    HarnessFailure(String),
    /// The run was stopped by `hq` (stall observed, human `stop`/`kill`) or
    /// left without honouring the run contract. The mission's fault.
    MissionFailure(String),
}

/// What a run consumed, as the harness reports it. Counted against the
/// per-mission cap of SPEC 7, in tokens — never in money (SPEC 4.3,
/// subscription only).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Current state of a launched run (SPEC 4.3: two states, plus the stall
/// checks that the engine derives from [`Progress`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunState {
    Running(Progress),
    /// Frozen by a human, exactly where they froze it. Neither running nor
    /// finished, and saying either would be a lie the human would act on.
    Paused(Progress),
    Finished(Outcome),
}

/// Signals a liveness check reads without asking the agent (SPEC 4.3, the
/// 15-minute check). All counters are monotonic since launch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Progress {
    /// Structured events the harness emitted so far.
    pub events: u64,
    /// Tool calls made so far.
    pub tool_calls: u64,
    /// Length of the longest run of identical consecutive tool calls seen.
    pub longest_repeat: u32,
}

/// What a harness needs on the container side (SPEC 4.3, `provision()`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Provisioning {
    /// Shell commands that put the harness in the image, run **as the agent**
    /// in a layer `hq` generates on top of the stack's image. They belong to
    /// the adapter and not to the project: a stack fragment describes a
    /// stack, and changing harness must not mean editing every project's
    /// Dockerfile.
    pub install: Vec<String>,
    /// What must be on the agent's `PATH` afterwards. `hq check` looks for
    /// it rather than assuming the install worked.
    pub binary: String,
    /// Directories to prepend to `PATH` for the agent, when the install puts
    /// the binary somewhere a login shell would not look.
    pub path: Vec<String>,
    /// Directory the harness keeps its config and sessions in; mounted as a
    /// named volume per slot so session resumption survives a rebuild.
    pub config_dir: Option<PathBuf>,
    /// Domains the harness itself must reach (the model API). Added to the
    /// role's allowlist by the engine (SPEC 4.1 bis, rule 6).
    pub domains: Vec<String>,
}

/// Harness-side guards that double the container/git restrictions (SPEC
/// 3.2). Passed at invocation, never written into the slot (SPEC 3.3).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GuardSetup {
    /// Extra command-line arguments to pass at launch.
    pub args: Vec<String>,
}

/// How a role's prompt is handed to the harness (SPEC 4.3, `expose(role)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exposure {
    /// Appended to the system prompt from a file.
    SystemPromptFile(PathBuf),
    /// Given as the first user message.
    UserMessage(String),
}

#[derive(Debug, thiserror::Error)]
pub enum HarnessError {
    #[error("launch failed: {0}")]
    Launch(String),
    #[error("unknown run: {0:?}")]
    UnknownRun(SessionId),
    /// The run could not be reached, and that is not a verdict on it. A
    /// container taken down, an engine that did not answer: `hq` says it
    /// does not know rather than call the agent dead (SPEC 4.2).
    #[error("the run cannot be reached: {0}")]
    Unreachable(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// The contract every harness adapter implements. See the module docs.
pub trait Harness: Send + Sync {
    /// The environment variable this harness reads its long-lived token
    /// from. Harness-specific knowledge, so it lives here and nowhere else:
    /// an Anthropic subscription and an OpenAI one are not passed the same
    /// way (SPEC 4.3).
    fn token_env(&self) -> &'static str;

    /// Stable name, used in `hq.yaml` and in the state.
    fn name(&self) -> &'static str;

    /// What the container must carry for this harness to run.
    fn provision(&self) -> Provisioning;

    /// Start a headless run for `request`. Returns a persistable handle.
    fn launch(
        &self,
        request: &RunRequest,
        guards: &GuardSetup,
        exposure: &Exposure,
    ) -> Result<RunHandle, HarnessError>;

    /// Read the run's state from what the container exposes: structured
    /// output and exit code first, a terminal pane only as a last resort.
    fn state(&self, handle: &RunHandle) -> Result<RunState, HarnessError>;

    /// End the current turn properly (for Claude Code, `SIGINT`, not
    /// `SIGTERM`), so the agent can write its resume block.
    fn stop(&self, handle: &RunHandle) -> Result<(), HarnessError>;

    /// Optional: guards this harness can add on top of the container and
    /// git ones. The default is none, and that is a complete answer.
    fn guards(&self, _role: Role) -> GuardSetup {
        GuardSetup::default()
    }

    /// How this harness prefers to receive a role prompt.
    fn expose(&self, role: Role, prompt_file: PathBuf) -> Exposure;
}
