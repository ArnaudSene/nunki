//! What a subscription has left, as its harness reports it (SPEC 4.3, 7).
//!
//! A subscription is limited in two windows at once — five hours, and a
//! week — and a harness that knows says how far each is used. Claude Code
//! writes it into the run's own stream (`rate_limit_event`, measured on
//! v2.1.266); another harness says it its own way, which is why reading it
//! is the adapter's job ([`crate::harness::Harness::windows`]) and this
//! module only keeps and judges what an adapter read.
//!
//! Kept **per account**, at `~/.hq/usage/<account>.json`: the windows are
//! the subscription's, shared by every mission and every project that spends
//! it, and a supervisor on another account — or another harness — has a file
//! of its own. Beside the account index and never in the tokens directory,
//! which is for secrets.
//!
//! Measured whenever `hq` reads a run — `hq verify` reading one back, `hq
//! mission watch` while one runs — and judged before every launch: past a
//! threshold, `hq` launches nothing until that window resets, then goes on by
//! itself (decided 2026-09-11: 90 % of five hours, 80 % of the week). Between
//! two readings the last measure stands, so what another session spends on
//! the same account shows at the next reading, not before.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::mission::Bounds;

/// Where the measures live, under `~/.hq/`.
pub const USAGE_DIR: &str = "usage";

/// One window: how much of it is used, in thousandths, and when it resets
/// (epoch seconds). Thousandths and not a float: a threshold compared on a
/// float read back from JSON is a coin flip at the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Window {
    pub per_mille: u32,
    pub resets_at: u64,
}

/// Both windows, each absent when the harness did not report it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Windows {
    #[serde(default)]
    pub five_hour: Option<Window>,
    #[serde(default)]
    pub weekly: Option<Window>,
}

/// A measure, as kept for one account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Measure {
    pub windows: Windows,
    /// Epoch seconds of the reading.
    pub measured_at: u64,
    /// The harness that reported it.
    pub harness: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    FiveHour,
    Weekly,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::FiveHour => "five-hour window",
            Kind::Weekly => "weekly window",
        }
    }
}

/// A window past its threshold, and when it resets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Over {
    pub kind: Kind,
    pub per_mille: u32,
    pub stop_at_percent: u32,
    pub until: u64,
}

/// A utilization as the harness reports it (0.53) in thousandths (530).
pub fn per_mille(utilization: f64) -> u32 {
    (utilization.clamp(0.0, 10.0) * 1000.0).round() as u32
}

pub fn path(hq_home: &Path, account: &str) -> PathBuf {
    hq_home.join(USAGE_DIR).join(format!("{account}.json"))
}

/// The measure kept for `account`. `None` when nothing has been measured
/// yet; an unreadable file is an error, not an absence, because reading it
/// as "nothing measured" would read as "nothing spent".
pub fn read(hq_home: &Path, account: &str) -> Result<Option<Measure>, String> {
    let path = path(hq_home, account);
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|e| format!("{} is not a measure hq can read: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Keep a measure — unless the one on file is newer: two missions on one
/// account read it at different times, and an older reading must not
/// overwrite a newer one. Written atomically, so a reader never sees half.
pub fn record(hq_home: &Path, account: &str, measure: &Measure) -> Result<(), String> {
    if let Ok(Some(kept)) = read(hq_home, account)
        && kept.measured_at > measure.measured_at
    {
        return Ok(());
    }
    let path = path(hq_home, account);
    let dir = hq_home.join(USAGE_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_vec_pretty(measure).expect("a measure serialises");
    std::fs::write(&tmp, body).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))
}

/// The window that forbids a launch at `now`, if one does.
///
/// A window whose reset has passed forbids nothing: the measure is about a
/// window that is over, and that is how `hq` goes on by itself once it
/// resets. When both are past their threshold, the later reset is the one
/// to wait for.
pub fn over(measure: &Measure, bounds: &Bounds, now: u64) -> Option<Over> {
    let judge = |kind: Kind, window: Option<Window>, percent: u32| {
        window
            .filter(|w| w.resets_at > now && w.per_mille >= percent.saturating_mul(10))
            .map(|w| Over {
                kind,
                per_mille: w.per_mille,
                stop_at_percent: percent,
                until: w.resets_at,
            })
    };
    let five = judge(
        Kind::FiveHour,
        measure.windows.five_hour,
        bounds.five_hour_stop_percent,
    );
    let week = judge(
        Kind::Weekly,
        measure.windows.weekly,
        bounds.weekly_stop_percent,
    );
    match (five, week) {
        (Some(five), Some(week)) => Some(if week.until >= five.until { week } else { five }),
        (five, week) => five.or(week),
    }
}

/// Keep what a run's stream said of the windows, under the account the
/// mission spends. Nothing to keep when the harness said nothing.
pub fn note(
    project: &crate::project::Project,
    from_mission: Option<&str>,
    windows: Option<Windows>,
    now: u64,
) -> Result<(), String> {
    let Some(windows) = windows else {
        return Ok(());
    };
    let account = account_of(project, from_mission)?;
    record(
        &project.hq_home(),
        &account,
        &Measure {
            windows,
            measured_at: now,
            harness: project.config.harness.clone(),
        },
    )
}

/// Thousandths as a percent a human reads: 530 is "53", 905 is "90.5".
pub fn percent(per_mille: u32) -> String {
    if per_mille % 10 == 0 {
        format!("{}", per_mille / 10)
    } else {
        format!("{}.{}", per_mille / 10, per_mille % 10)
    }
}

/// The account a mission spends, by name, without reading its token: the
/// measure is kept under that name.
pub fn account_of(
    project: &crate::project::Project,
    from_mission: Option<&str>,
) -> Result<String, String> {
    let hq_home = project.hq_home();
    let accounts = crate::account::Accounts::load(&hq_home).map_err(|e| e.to_string())?;
    accounts
        .choose(&hq_home, from_mission, project.config.account.as_deref())
        .map(|(name, _)| name)
        .map_err(|e| e.to_string())
}
