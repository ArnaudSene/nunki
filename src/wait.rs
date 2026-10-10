//! `nunki mission wait`: block until a mission needs someone (SPEC 4.5).
//!
//! A mission runs for hours, driven by its monitor ([`crate::monitor`]).
//! Whoever drives the HQ has to know when it stops and why, without polling
//! `mission status` and reading its `stage` line. This module says, from the
//! state, whether the mission stands at a stop someone must act on, and in
//! one line which stop: `<id> · <stage> · <detail> · awaits <who>: <what>`,
//! with an exit code per kind of stop. The monitor's own exit message is
//! built by the same function, so its log and `wait` say the same thing.
//!
//! `wait` reads; it never drives. It takes no lock, launches nothing, and
//! moving a mission stays the monitor's job: two `wait` on one mission are
//! harmless. (Opening the store creates the HQ's state directories when
//! they are missing; nothing else is written.) Its loop takes its clock and
//! its reader as parameters ([`Clock`], [`Reader`]), so that a test drives
//! time and the state without sleeping and without a monitor.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::mission::flow::{Flow, Stage, Work};
use crate::project::Project;
use crate::state::{MissionState, StateError, Store};
use crate::text::{brief, one_line};

/// How often the state is read again, by default.
pub const EVERY_SECONDS: u64 = 30;

/// How much of a findings report the line carries: the report itself is in
/// the security agent's journal.
const REPORT_CHARS: usize = 160;

#[derive(Debug, thiserror::Error)]
pub enum WaitError {
    #[error(
        "mission {0} has not started, so there is nothing to wait for — \
         `nunki mission start {0} --slot <slot>` starts it"
    )]
    NotStarted(String),
    #[error(transparent)]
    State(#[from] StateError),
}

/// The kinds of stop, each with its exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// Every declared stage is green; the human reads it, then pushes.
    Verified,
    /// Verified and pushed: nothing is owed but closing it.
    Pushed,
    /// Archived: over.
    Archived,
    /// The security agent's findings: the HQ iterates or lifts them.
    Findings,
    /// Handed over to the human, a ruling included.
    Handover,
    /// Held by `nunki mission stop`, or by `nunki` itself.
    Held,
    /// The account's window is past its threshold: nothing is launched
    /// before it resets.
    WindowSpent,
    /// The monitor stopped on something the state does not show: a gate that
    /// could not be played, a `verify` that failed, a campaign lost.
    MonitorFailed,
    /// No monitor runs, and the stage is not a stop: nothing moves the
    /// mission.
    MonitorGone,
    /// `--timeout` came first.
    TimedOut,
}

impl Stop {
    /// The exit code `nunki mission wait` returns for this stop.
    pub fn code(self) -> u8 {
        match self {
            Stop::Verified | Stop::Pushed | Stop::Archived => 0,
            Stop::Findings => 10,
            Stop::Handover => 11,
            Stop::Held | Stop::WindowSpent => 12,
            Stop::MonitorFailed => 13,
            Stop::MonitorGone => 14,
            Stop::TimedOut => 15,
        }
    }
}

/// Who a stop waits on, and what they are expected to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Awaits {
    pub who: String,
    pub what: String,
}

/// A mission's stop, as `wait` prints it and the monitor logs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Status {
    pub id: String,
    pub stage: String,
    pub detail: String,
    pub awaits: Awaits,
    pub code: u8,
    #[serde(skip)]
    pub stop: Stop,
}

impl Status {
    fn new(id: &str, stage: String, detail: String, who: &str, what: String, stop: Stop) -> Self {
        // Every part an agent may have written, and not only the detail: a
        // lot's label reaches the stage as a report reaches the detail. The
        // id is the operator's own argument.
        Self {
            id: id.to_string(),
            stage: one_line(&stage),
            detail: one_line(&detail),
            awaits: Awaits {
                who: who.to_string(),
                what: one_line(&what),
            },
            code: stop.code(),
            stop,
        }
    }

    /// `<id> · <stage> · <detail> · awaits <who>: <what>`, on one line.
    pub fn line(&self) -> String {
        format!(
            "{} · {} · {} · awaits {}: {}",
            self.id, self.stage, self.detail, self.awaits.who, self.awaits.what
        )
    }

    /// The same, as one JSON object: `id`, `stage`, `detail`, `awaits`
    /// (`who`, `what`) and `code`.
    pub fn json(&self) -> String {
        serde_json::to_string(self).expect("a Status is strings and a number")
    }
}

/// What is known of the mission's monitor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Watcher {
    /// A monitor is alive and watching.
    Alive,
    /// None is, and the last one said why it stopped, on the state as it is
    /// now.
    Stopped { why: String },
    /// None is, and none said why on the state as it is now.
    Gone,
}

/// The account's window past its threshold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spent {
    pub account: String,
    /// Which window, as a human reads it.
    pub window: String,
    /// When it resets, in epoch seconds.
    pub until: u64,
}

/// What a stop is decided on, beside the state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    pub watcher: Watcher,
    pub window: Option<Spent>,
    /// The equivalence proposals that await the HQ's ruling — the coder's,
    /// and `nunki`'s from rulings it matched
    /// ([`crate::mutants::awaiting_ruling`]) — or why they could not be read.
    pub proposals: Result<Vec<crate::mutants::Proposal>, String>,
}

/// The stage as the line names it.
pub fn stage_name(flow: &Flow) -> String {
    match flow.stage() {
        Stage::Coding { work, attempt } => match work {
            Work::Lot(_) => format!("coding lot {}, attempt {attempt}", flow.label(work)),
            Work::Volet { .. } => format!("coding {}, attempt {attempt}", flow.label(work)),
        },
        Stage::Gates => "final gates".to_string(),
        Stage::Integration { attempt } => format!("integration, attempt {attempt}"),
        Stage::SecurityAgent { attempt } => format!("security agent, attempt {attempt}"),
        Stage::Findings { .. } => "findings".to_string(),
        Stage::FinalCampaign => "final full campaign".to_string(),
        Stage::AwaitingHuman(_) => "awaiting human".to_string(),
        Stage::Verified => "verified".to_string(),
    }
}

/// The stop the mission stands at, or `None` while it is still being driven.
///
/// Where the flow stands comes first, then a hold, then the monitor, then the
/// account's window: the first is the most a reader can act on, and a
/// monitor that is gone is something to mend whatever the window says.
pub fn status(id: &str, state: &MissionState, context: &Context) -> Option<Status> {
    stopped(id, state, context).map(|stop| with_proposals(id, stop, &context.proposals))
}

/// A stop, saying how many equivalence proposals await the HQ, and from
/// whom.
///
/// A verified mission with one waits on the HQ and not on the human's push:
/// `nunki push` would refuse, so the line names the two verbs that rule.
fn with_proposals(
    id: &str,
    mut stop: Status,
    proposals: &Result<Vec<crate::mutants::Proposal>, String>,
) -> Status {
    let (said, waiting) = match proposals {
        Ok(waiting) => match crate::mutants::proposals_await(waiting) {
            Some(said) => (said, true),
            None => return stop,
        },
        Err(why) => (
            format!("the equivalence proposals could not be read: {why}"),
            false,
        ),
    };
    stop.detail = one_line(&format!("{}; {said}", stop.detail));
    if waiting && stop.stop == Stop::Verified {
        stop.awaits = Awaits {
            who: "the HQ".to_string(),
            what: format!(
                "`nunki mission mutants {id} --ratify <survivor>` (or `--ratify --all`), \
                 or `--refuse <survivor> --because <why>`, for each; then `nunki push {id} \
                 --yes`"
            ),
        };
    }
    stop
}

fn stopped(id: &str, state: &MissionState, context: &Context) -> Option<Status> {
    if let Some(settled) = settled(id, state) {
        return Some(settled);
    }
    match &context.watcher {
        Watcher::Stopped { why } => return Some(monitor_failed(id, state, why)),
        Watcher::Gone => {
            return Some(Status::new(
                id,
                stage_name(&state.flow),
                "no monitor is watching, and nothing moves the mission".to_string(),
                "the human",
                format!("`nunki verify {id}` picks it up and starts one"),
                Stop::MonitorGone,
            ));
        }
        Watcher::Alive => {}
    }
    context.window.as_ref().map(|spent| {
        Status::new(
            id,
            stage_name(&state.flow),
            format!(
                "account {}'s {} is past its threshold",
                spent.account, spent.window
            ),
            "the account",
            format!(
                "nothing is launched before {}; the monitor goes on then",
                crate::state::rfc3339(spent.until)
            ),
            Stop::WindowSpent,
        )
    })
}

/// The line the monitor ends on, having stopped for `why`: what the state
/// shows when it shows a stop, and `why` when it does not. The same function
/// as [`status`] with the monitor gone and having said `why`, so that the
/// monitor's log and `wait` say the same thing about the same state.
pub fn monitor_exit(id: &str, state: &MissionState, why: &str) -> Status {
    settled(id, state).unwrap_or_else(|| monitor_failed(id, state, why))
}

/// The stops the state shows by itself: the flow's, then a hold.
fn settled(id: &str, state: &MissionState) -> Option<Status> {
    let flow = &state.flow;
    let stage = stage_name(flow);
    match flow.stage() {
        Stage::Verified => {
            return Some(match &state.pushed {
                Some(pushed) => Status::new(
                    id,
                    "pushed".to_string(),
                    format!(
                        "verified, and pushed on {} at {}",
                        pushed.date,
                        pushed.head.chars().take(12).collect::<String>()
                    ),
                    "the human",
                    format!("`nunki mission archive {id}` closes it"),
                    Stop::Pushed,
                ),
                None => Status::new(
                    id,
                    stage,
                    if flow.header().has_security_agent() {
                        format!(
                            "every declared stage is green, security round {} of {}",
                            flow.security_rounds(),
                            flow.max_security_rounds()
                        )
                    } else {
                        "every declared stage is green".to_string()
                    },
                    "the human",
                    format!("read it, then `nunki push {id} --yes`"),
                    Stop::Verified,
                ),
            });
        }
        Stage::Findings { report } => {
            let found = format!(
                "security round {} of {} found: {}",
                flow.security_rounds(),
                flow.max_security_rounds(),
                brief(report, REPORT_CHARS)
            );
            // A report the rigor does not count as blocking says so, and
            // names `accept` first: `iterate` alone would be refused.
            let (detail, what) = match flow.rigor_refuses() {
                Some(departure) => (
                    format!("{found} — it does not block: {departure}"),
                    format!(
                        "`nunki mission accept {id} --because <why>` lifts it, or \
                         `nunki mission iterate {id} --override --because <why>` sends it \
                         back as a departure from the rigor"
                    ),
                ),
                None => (
                    found,
                    format!(
                        "`nunki mission iterate {id}` sends it back to the coder, or \
                         `nunki mission accept {id} --because <why>` lifts it"
                    ),
                ),
            };
            return Some(Status::new(
                id,
                stage,
                detail,
                "the HQ",
                what,
                Stop::Findings,
            ));
        }
        Stage::AwaitingHuman(handover) => {
            let (who, what) = handover.awaits(id);
            // The handover lists the causes; the count against the cap is
            // the flow's, which knows the cap and what the HQ granted.
            let detail = match handover {
                crate::mission::flow::Handover::VoletsExhausted { .. } => {
                    format!("volets {} — {}", flow.volets_said(), handover.detail())
                }
                _ => handover.detail(),
            };
            return Some(Status::new(id, stage, detail, who, what, Stop::Handover));
        }
        _ => {}
    }
    state.stopped.as_ref().map(|hold| {
        Status::new(
            id,
            stage,
            match &hold.reason {
                Some(reason) => format!("held by {} on {}: {reason}", hold.who, hold.date),
                None => format!("held by {} on {}", hold.who, hold.date),
            },
            "the human",
            format!("`nunki mission resume {id}` lifts the hold"),
            Stop::Held,
        )
    })
}

fn monitor_failed(id: &str, state: &MissionState, why: &str) -> Status {
    Status::new(
        id,
        stage_name(&state.flow),
        format!("the monitor stopped: {why}"),
        "the human",
        format!("act on it, then `nunki verify {id}` picks the mission up"),
        Stop::MonitorFailed,
    )
}

/// A mission that has been archived: over, whatever stage it ended at.
pub fn archived(id: &str, at: &Path, ended: Option<&MissionState>) -> Status {
    Status::new(
        id,
        "archived".to_string(),
        match ended {
            Some(state) => format!(
                "closed at {} under {}",
                stage_name(&state.flow),
                at.display()
            ),
            None => format!("closed under {}", at.display()),
        },
        "nobody",
        "it is over".to_string(),
        Stop::Archived,
    )
}

/// `--timeout` came first: the mission was still being driven.
pub fn timed_out(id: &str, state: &MissionState, after: u64) -> Status {
    Status::new(
        id,
        stage_name(&state.flow),
        format!("no stop after {after} second(s) of waiting"),
        "the monitor",
        "it is still driving the mission".to_string(),
        Stop::TimedOut,
    )
}

/// What one reading of a mission found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observed {
    /// The mission's state, and what is known beside it.
    Live {
        state: Box<MissionState>,
        context: Context,
    },
    /// The mission was archived, under `at`, and ended as `ended` says when
    /// its state could be read there.
    Archived {
        at: PathBuf,
        ended: Option<Box<MissionState>>,
    },
}

/// Where `wait` reads a mission from.
pub trait Reader {
    fn observe(&mut self, id: &str) -> Result<Observed, WaitError>;
}

/// The time `wait` keeps, and how it waits.
pub trait Clock {
    /// Now, in epoch seconds.
    fn now(&mut self) -> u64;
    fn sleep(&mut self, seconds: u64);
}

/// Wait on mission `id` until it stops, reading it every `every` seconds,
/// and for at most `timeout` seconds when one is given. A mission already
/// stopped returns at the first reading, without waiting.
pub fn wait(
    id: &str,
    reader: &mut dyn Reader,
    clock: &mut dyn Clock,
    every: u64,
    timeout: Option<u64>,
) -> Result<Status, WaitError> {
    let start = clock.now();
    loop {
        let state = match reader.observe(id)? {
            Observed::Archived { at, ended } => return Ok(archived(id, &at, ended.as_deref())),
            Observed::Live { state, context } => match status(id, &state, &context) {
                Some(stop) => return Ok(stop),
                None => state,
            },
        };
        let now = clock.now();
        let mut pause = every.max(1);
        if let Some(timeout) = timeout {
            let deadline = start.saturating_add(timeout);
            if now >= deadline {
                return Ok(timed_out(id, &state, timeout));
            }
            pause = pause.min(deadline - now);
        }
        clock.sleep(pause);
    }
}

/// The clock of the machine.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&mut self) -> u64 {
        crate::state::now_secs()
    }

    fn sleep(&mut self, seconds: u64) {
        std::thread::sleep(std::time::Duration::from_secs(seconds));
    }
}

/// Whether a mission's monitor is alive, asked with the HQ root and the
/// mission's id.
type Liveness<'a> = dyn FnMut(&Path, &str) -> bool + 'a;

/// The HQ's own files: the state, the monitor's pidfile and the record of
/// its exit, and the account's last measure.
pub struct Hq<'a> {
    project: &'a Project,
    /// Whether the mission's monitor is alive: its pidfile, and whether that
    /// pid is this mission's monitor. A parameter so that a test can stop the
    /// monitor at the moment `wait` asks.
    alive: Box<Liveness<'a>>,
}

impl<'a> Hq<'a> {
    /// Liveness read exactly as `mission status` reads it.
    pub fn new(project: &'a Project) -> Self {
        Self::watched_by(project, |hq_root, id| {
            crate::monitor::running(hq_root, id).is_some()
        })
    }

    /// The HQ's files, with the monitor's liveness answered by `alive`.
    pub fn watched_by(project: &'a Project, alive: impl FnMut(&Path, &str) -> bool + 'a) -> Self {
        Self {
            project,
            alive: Box::new(alive),
        }
    }

    /// The account's window past its threshold now, if it is.
    fn window(&self, state: &MissionState) -> Option<Spent> {
        let header = state.flow.header();
        let account =
            crate::consumption::account_of(self.project, header.account.as_deref()).ok()?;
        let measure = crate::consumption::read(&self.project.nunki_home(), &account).ok()??;
        let over = crate::consumption::over(&measure, &header.bounds, crate::state::now_secs())?;
        Some(Spent {
            account,
            window: over.kind.name().to_string(),
            until: over.until,
        })
    }
}

impl Reader for Hq<'_> {
    fn observe(&mut self, id: &str) -> Result<Observed, WaitError> {
        let hq_root = &self.project.hq_root;
        // The monitor first, the state after. A monitor writes the state it
        // stops on, then its exit, then removes its pidfile: read in this
        // order, a monitor that stops between the two readings is seen alive
        // or its stop is in the state read after. Read the other way, the
        // state from before the stop meets a monitor already gone, and a
        // mission that reached its findings reads as one nobody watches.
        let alive = (self.alive)(hq_root, id);
        let exited = if alive {
            None
        } else {
            crate::monitor::last_exit(hq_root, id)
        };
        let state = match Store::open(hq_root)?.load(id) {
            Ok(state) => state,
            Err(StateError::Missing(_)) => {
                let at = crate::lifecycle::archive_dir(self.project).join(id);
                if !at.is_dir() {
                    return Err(WaitError::NotStarted(id.to_string()));
                }
                let ended = std::fs::read(at.join("state.json"))
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<MissionState>(&bytes).ok())
                    .map(Box::new);
                return Ok(Observed::Archived { at, ended });
            }
            Err(e) => return Err(e.into()),
        };
        let watcher = if alive {
            Watcher::Alive
        } else {
            match exited {
                // Said on this very state: any write since has moved on from
                // what the monitor saw.
                Some(exited) if exited.revision == state.revision => {
                    Watcher::Stopped { why: exited.why }
                }
                _ => Watcher::Gone,
            }
        };
        let window = self.window(&state);
        let proposals =
            crate::mutants::awaiting_ruling(&crate::mission::dir::Paths::of(hq_root, id).dir)
                .map_err(|e| e.to_string());
        Ok(Observed::Live {
            state: Box::new(state),
            context: Context {
                watcher,
                window,
                proposals,
            },
        })
    }
}

/// A duration as `--timeout` takes it: seconds, or `<n>s`, `<n>m`, `<n>h`.
pub fn parse_duration(text: &str) -> Result<u64, String> {
    let text = text.trim();
    let (digits, unit) = match text.char_indices().last() {
        Some((at, 's')) => (&text[..at], 1),
        Some((at, 'm')) => (&text[..at], 60),
        Some((at, 'h')) => (&text[..at], 3_600),
        _ => (text, 1),
    };
    let wrong = || format!("a duration is seconds, or `<n>m`, or `<n>h`, and {text:?} is not");
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(wrong());
    }
    digits
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(unit))
        .ok_or_else(wrong)
}
