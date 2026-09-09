//! One lock per slot (SPEC 4.2).
//!
//! Only the verbs that change a slot's state take it — `start`, `verify`,
//! `reset`, `rebuild`, `rm`, `push`. Readers (`status`, `logs`, `watch`,
//! `check`) and the human's gestures (`say`, `pause`, `resume`, `stop`,
//! `kill`) never do. `hq exec` launched by `verify` runs under `verify`'s
//! own guard and does not ask twice: reentrancy is in-process, by passing
//! the guard, not on disk.
//!
//! An orphan lock lifts itself: the file carries the pid and the time of its
//! taker, and a pid that no longer exists means the lock is free — with a
//! line said about it, never silently.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::now_rfc3339;

#[derive(Debug, thiserror::Error)]
pub enum LockError {
    #[error("slot {slot} is locked by `hq {verb}` (pid {pid}, since {since})")]
    Held {
        slot: String,
        verb: String,
        pid: u32,
        since: String,
    },
    #[error("io on {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
}

/// What the lock file says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockInfo {
    pub slot: String,
    pub verb: String,
    pub pid: u32,
    pub since: String,
}

/// A held lock. Released on drop.
#[derive(Debug)]
pub struct SlotLock {
    path: PathBuf,
    info: LockInfo,
    /// What was found and taken over, if an orphan lock was lifted.
    lifted: Option<LockInfo>,
}

impl SlotLock {
    /// Take the lock for `slot` on behalf of `verb`. Refuses if a live
    /// process holds it; takes over an orphan.
    pub fn acquire(locks_dir: &Path, slot: &str, verb: &str) -> Result<Self, LockError> {
        let path = locks_dir.join(format!("{slot}.lock"));
        let info = LockInfo {
            slot: slot.to_string(),
            verb: verb.to_string(),
            pid: std::process::id(),
            since: now_rfc3339(),
        };
        let mut lifted = None;
        loop {
            // O_EXCL: creating the file IS taking the lock; two takers race
            // on the kernel, not on a read-then-write.
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut f) => {
                    let bytes = serde_json::to_vec_pretty(&info).expect("LockInfo serializes");
                    f.write_all(&bytes).map_err(|source| LockError::Io {
                        path: path.clone(),
                        source,
                    })?;
                    return Ok(Self { path, info, lifted });
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    let holder = read_info(&path)?;
                    match holder {
                        Some(h) if process_alive(h.pid) => {
                            return Err(LockError::Held {
                                slot: h.slot,
                                verb: h.verb,
                                pid: h.pid,
                                since: h.since,
                            });
                        }
                        // Dead holder, or an unreadable file: lift it and retry.
                        other => {
                            lifted = other.or(lifted);
                            fs::remove_file(&path).map_err(|source| LockError::Io {
                                path: path.clone(),
                                source,
                            })?;
                        }
                    }
                }
                Err(source) => return Err(LockError::Io { path, source }),
            }
        }
    }

    pub fn info(&self) -> &LockInfo {
        &self.info
    }

    /// The orphan lock this acquisition lifted, if any — so the caller can
    /// say so in its journal.
    pub fn lifted(&self) -> Option<&LockInfo> {
        self.lifted.as_ref()
    }

    /// Who holds the lock on `slot`, if anyone alive.
    pub fn holder(locks_dir: &Path, slot: &str) -> Result<Option<LockInfo>, LockError> {
        let path = locks_dir.join(format!("{slot}.lock"));
        match read_info(&path)? {
            Some(h) if process_alive(h.pid) => Ok(Some(h)),
            _ => Ok(None),
        }
    }
}

impl Drop for SlotLock {
    fn drop(&mut self) {
        // Only remove the file if it is still ours: a lifted-and-retaken
        // lock by another process must not be deleted from under it.
        if let Ok(Some(h)) = read_info(&self.path)
            && h.pid == self.info.pid
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn read_info(path: &Path) -> Result<Option<LockInfo>, LockError> {
    match fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes).ok()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(LockError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Does a process with this pid exist? `kill(pid, 0)` sends nothing and
/// answers: success or EPERM means alive, ESRCH means gone.
#[cfg(unix)]
pub fn process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    // SAFETY: signal 0 performs no action; the call only checks existence.
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if rc == 0 {
        return true;
    }
    io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
pub fn process_alive(_pid: u32) -> bool {
    // Windows native is not a target (SPEC 4.2 bis); be conservative.
    true
}
