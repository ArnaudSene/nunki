//! The mission's monitor (SPEC 4.3, decided by Arnaud on 2026-09-11).
//!
//! `hq` has no system service, and a verb returns: `verify` launches a run
//! and gives the terminal back. So what watches a run at night, and what
//! calls `verify` again once a wait is over, is a process of its own — one
//! per mission, started by the verbs that launch or resume, detached from
//! the terminal (`nohup`, a process group of its own) so it outlives it.
//!
//! Every minute while a run goes, it measures the account's windows and ends
//! the run's turn past the threshold ([`crate::gesture::spare`]). When no run
//! goes, it calls `verify` — which reads the run back, waits, or launches the
//! next one — and sleeps to the next deadline it knows: the harness wait, or
//! the window's reset. It stops when the mission needs a human or has
//! nothing left that `hq` can launch: verified, handed over, held, findings
//! to lift, or a coder run owed, which `hq` does not launch yet.
//!
//! It takes the slot's lock as `hq mission monitor`, so a human who types
//! `verify` meanwhile is told what holds it; and a lock already held is a
//! human driving, not a failure — the monitor waits its turn.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use crate::engine::Engine;
use crate::harness::{Harness, RunState};
use crate::mission::flow::Stage;
use crate::project::Project;
use crate::state::{MissionState, Store};
use crate::verify::{Step, VerifyError};

/// Where monitors keep their pid and their log, under the HQ.
pub const MONITORS_DIR: &str = "monitors";
/// How often a run under way is looked at.
pub const TICK_SECONDS: u64 = 60;
/// The name the monitor takes the slot's lock under.
pub const LOCK_VERB: &str = "mission monitor";

#[derive(Debug, thiserror::Error)]
pub enum MonitorError {
    #[error(
        "{0} is not hq: a monitor is only ever started from the hq binary, never from \
         another program — a test harness would fork itself into a detached process"
    )]
    NotHq(PathBuf),
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
}

/// What [`ensure`] found or did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ensured {
    /// A monitor was already watching, with this pid.
    Running(u32),
    /// One was started, with this pid.
    Started(u32),
}

pub fn pidfile(hq_root: &Path, id: &str) -> PathBuf {
    hq_root.join(MONITORS_DIR).join(format!("{id}.pid"))
}

pub fn logfile(hq_root: &Path, id: &str) -> PathBuf {
    hq_root.join(MONITORS_DIR).join(format!("{id}.log"))
}

/// Whether `pid` is a live monitor of mission `id`: alive, and its command
/// line says so. A pid alone would do for a while, until the system hands it
/// to something else.
pub fn is_monitor(pid: u32, id: &str) -> bool {
    let Ok(out) = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "command="])
        .output()
    else {
        return false;
    };
    let line = String::from_utf8_lossy(&out.stdout);
    line.contains("mission monitor") && line.split_whitespace().any(|word| word == id)
}

/// The mission's monitor, if one is alive.
pub fn running(hq_root: &Path, id: &str) -> Option<u32> {
    let text = std::fs::read_to_string(pidfile(hq_root, id)).ok()?;
    let pid: u32 = text.trim().parse().ok()?;
    is_monitor(pid, id).then_some(pid)
}

/// Whether the mission has anything a monitor would watch or wait for: a run
/// recorded, or a wait that ends by itself. A mission that needs a human
/// does not: a monitor started there would stop at once.
pub fn wanted(project: &Project, state: &MissionState, now: u64) -> bool {
    if state.held() {
        return false;
    }
    if matches!(
        state.flow.stage(),
        Stage::Verified | Stage::AwaitingHuman(_) | Stage::Findings { .. }
    ) {
        return false;
    }
    if state.run.is_some() {
        return true;
    }
    let harness_wait = state
        .harness_down
        .as_ref()
        .is_some_and(|down| down.not_before > now);
    harness_wait || window_reset(project, state, now).is_some()
}

/// When the account's window resets, if it is past its threshold now.
fn window_reset(project: &Project, state: &MissionState, now: u64) -> Option<u64> {
    let header = state.flow.header();
    let account = crate::consumption::account_of(project, header.account.as_deref()).ok()?;
    let measure = crate::consumption::read(&project.hq_home(), &account).ok()??;
    crate::consumption::over(&measure, &header.bounds, now).map(|over| over.until)
}

/// The next moment worth looking again: a minute while a run goes;
/// otherwise the harness wait or the window's reset, when one is further off
/// than that.
pub fn next_wake(project: &Project, state: &MissionState, now: u64) -> u64 {
    let tick = now + TICK_SECONDS;
    if state.run.is_some() {
        return tick;
    }
    let harness_wait = state.harness_down.as_ref().map(|down| down.not_before);
    let reset = window_reset(project, state, now);
    [Some(tick), harness_wait, reset]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or(tick)
}

/// Start the mission's monitor, unless one is alive.
///
/// `exe` is the binary it runs as, and anything not named `hq` is refused:
/// the call sites are the CLI's, and a future one added in the library would
/// otherwise fork whatever test binary happened to call it.
pub fn ensure(project: &Project, id: &str, exe: &Path) -> Result<Ensured, MonitorError> {
    use std::os::unix::process::CommandExt;

    if let Some(pid) = running(&project.hq_root, id) {
        return Ok(Ensured::Running(pid));
    }
    if exe.file_name().and_then(|name| name.to_str()) != Some("hq") {
        return Err(MonitorError::NotHq(exe.to_path_buf()));
    }
    let dir = project.hq_root.join(MONITORS_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| MonitorError::Io(dir.clone(), e))?;
    let log_path = logfile(&project.hq_root, id);
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| MonitorError::Io(log_path.clone(), e))?;
    let err = log
        .try_clone()
        .map_err(|e| MonitorError::Io(log_path.clone(), e))?;
    // `nohup` ignores the hangup a closed terminal sends, and execs the
    // monitor in place, so the pid it reports is the monitor's. A process
    // group of its own keeps it out of the terminal's job control.
    let child = Command::new("nohup")
        .arg(exe)
        .arg("-C")
        .arg(&project.root)
        .args(["mission", "monitor", id])
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(err)
        .process_group(0)
        .spawn()
        .map_err(|e| MonitorError::Io(PathBuf::from("nohup"), e))?;
    let pid = child.id();
    let path = pidfile(&project.hq_root, id);
    std::fs::write(&path, format!("{pid}\n")).map_err(|e| MonitorError::Io(path, e))?;
    Ok(Ensured::Started(pid))
}

/// What the monitor does after a `verify`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// Look again at the next wake.
    Continue,
    /// Stop, and say why.
    Exit(String),
}

/// After a `verify`: go on, or stop because nothing is left that `hq` may do
/// without a human. Pure, so every arm is tested.
pub fn after_verify(result: &Result<Vec<Step>, VerifyError>) -> Next {
    match result {
        Ok(steps) => match steps.last() {
            Some(Step::Verified) => {
                Next::Exit("verified — the human reads it, then `hq push`".to_string())
            }
            Some(Step::AwaitingHuman(handover)) => {
                Next::Exit(format!("handed over to the human: {handover:?}"))
            }
            Some(Step::Held { who, reason, .. }) => Next::Exit(match reason {
                Some(reason) => format!("held by {who}: {reason}"),
                None => format!("held by {who}"),
            }),
            Some(Step::Findings { .. }) => Next::Exit(
                "the security agent's findings are the human's to iterate or lift".to_string(),
            ),
            Some(Step::NeedsRun { role, why }) => Next::Exit(format!(
                "a {role:?} run is owed and hq does not launch it yet: {why}"
            )),
            Some(
                Step::Launched { .. }
                | Step::Saving { .. }
                | Step::Waiting { .. }
                | Step::Unreachable { .. }
                | Step::Gates { .. }
                | Step::Moved { .. },
            )
            | None => Next::Continue,
        },
        // A human driving the slot, a run still going, or one just told to
        // end its turn: all things that pass.
        Err(VerifyError::Lock(crate::state::LockError::Held { .. }))
        | Err(VerifyError::RunInProgress { .. })
        | Err(VerifyError::Spared { .. }) => Next::Continue,
        Err(e) => Next::Exit(format!("verify failed, and a human should look: {e}")),
    }
}

/// Watch mission `id` until nothing is left for `hq` to do on its own, then
/// say why. Its pid is kept while it runs and forgotten when it stops.
pub fn run(project: &Project, id: &str, engine: Arc<dyn Engine>, engine_bin: &str) -> String {
    let me = std::process::id();
    if let Some(other) = running(&project.hq_root, id)
        && other != me
    {
        return format!("monitor {other} already watches mission {id}");
    }
    let path = pidfile(&project.hq_root, id);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, format!("{me}\n"));

    let why = watch(project, id, engine, engine_bin);

    // The last act: forget the pid, if it is still this monitor's.
    let ours = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| text.trim().parse::<u32>().ok())
        == Some(me);
    if ours {
        let _ = std::fs::remove_file(&path);
    }
    why
}

fn say(what: &str) {
    println!(
        "{}  {what}",
        crate::state::rfc3339(crate::state::now_secs())
    );
}

fn watch(project: &Project, id: &str, engine: Arc<dyn Engine>, engine_bin: &str) -> String {
    loop {
        let now = crate::state::now_secs();
        let state = match Store::open(&project.hq_root).and_then(|store| store.load(id)) {
            Ok(state) => state,
            Err(e) => return format!("the mission's state cannot be read: {e}"),
        };

        if let Some(handle) = &state.run {
            let harness = crate::harness::claude_code::ClaudeCode::new(
                Default::default(),
                Box::new(crate::verify::harness_spawner(
                    project,
                    engine.clone(),
                    &state.slot,
                    &handle.session.0,
                )),
            );
            match harness.state(handle) {
                Ok(RunState::Running(_)) => {
                    if let Err(e) = crate::consumption::note(
                        project,
                        state.flow.header().account.as_deref(),
                        harness.windows(handle),
                        now,
                    ) {
                        say(&format!("the measure could not be kept: {e}"));
                    }
                    match crate::gesture::spare(project, id, &harness, now, LOCK_VERB) {
                        Ok(Some(spared)) => say(&format!(
                            "account {}'s {} at {}% — the run was told to end its turn",
                            spared.account,
                            spared.window,
                            crate::consumption::percent(spared.per_mille)
                        )),
                        Ok(None) => {}
                        Err(e) => say(&format!("the run could not be spared: {e}")),
                    }
                    sleep_until(now + TICK_SECONDS);
                    continue;
                }
                // A human froze it, and a frozen run is theirs to thaw.
                Ok(RunState::Paused(_)) => {
                    sleep_until(now + TICK_SECONDS);
                    continue;
                }
                // Ended, or unreachable: `verify` reads it back, or says it
                // cannot.
                _ => {}
            }
        }

        let result = crate::verify::verify_as(project, id, engine.clone(), engine_bin, LOCK_VERB);
        match &result {
            Ok(steps) => {
                if let Some(last) = steps.last() {
                    say(&format!("verify: {last:?}"));
                }
            }
            Err(e) => say(&format!("verify: {e}")),
        }
        if let Next::Exit(why) = after_verify(&result) {
            return why;
        }
        let wake = match Store::open(&project.hq_root).and_then(|store| store.load(id)) {
            Ok(state) => next_wake(project, &state, crate::state::now_secs()),
            Err(_) => crate::state::now_secs() + TICK_SECONDS,
        };
        sleep_until(wake);
    }
}

fn sleep_until(epoch: u64) {
    let now = crate::state::now_secs();
    if epoch > now {
        std::thread::sleep(std::time::Duration::from_secs(epoch - now));
    }
}
