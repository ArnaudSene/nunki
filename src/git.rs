//! Just enough git, run as a subprocess.
//!
//! `hq` never links a git library: what it needs is what the command line
//! does — clone, branch, log — and shelling out keeps the behaviour identical
//! to what a human sees in the same repository.

use std::path::Path;
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("git {verb} failed in {at}: {stderr}")]
    Failed {
        verb: String,
        at: String,
        stderr: String,
    },
    #[error("git is not on the path: {0}")]
    Missing(String),
}

/// Run git in `at` and return its stdout, trimmed.
pub fn run(at: &Path, args: &[&str]) -> Result<String, GitError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(at)
        .args(args)
        .output()
        .map_err(|e| GitError::Missing(e.to_string()))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(GitError::Failed {
            verb: args.first().unwrap_or(&"?").to_string(),
            at: at.display().to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        })
    }
}

/// Run git outside any repository — `clone`, essentially.
pub fn run_anywhere(args: &[&str]) -> Result<String, GitError> {
    let out = Command::new("git")
        .args(args)
        .output()
        .map_err(|e| GitError::Missing(e.to_string()))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(GitError::Failed {
            verb: args.first().unwrap_or(&"?").to_string(),
            at: ".".to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        })
    }
}

pub fn current_branch(at: &Path) -> Result<String, GitError> {
    run(at, &["rev-parse", "--abbrev-ref", "HEAD"])
}

pub fn head(at: &Path) -> Result<String, GitError> {
    run(at, &["rev-parse", "HEAD"])
}

/// Whether the working tree has anything uncommitted.
pub fn is_clean(at: &Path) -> Result<bool, GitError> {
    Ok(run(at, &["status", "--porcelain"])?.is_empty())
}

/// The commits reachable from `HEAD` in `at` that `elsewhere` does not have.
/// This is what makes `hq slot rm` refuse rather than lose work (SPEC 3.3).
pub fn commits_not_in(at: &Path, elsewhere: &Path) -> Result<Vec<String>, GitError> {
    // Ask the other repository what it knows, then ask this one what it has
    // that the other does not. No fetch, no network, no writing anywhere.
    let branch = current_branch(at)?;
    let here = run(at, &["rev-list", "HEAD"])?;
    let there: std::collections::BTreeSet<String> = run(elsewhere, &["rev-list", "--all"])
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    let missing: Vec<String> = here
        .lines()
        .filter(|c| !there.contains(*c))
        .map(str::to_string)
        .collect();
    let _ = branch;
    Ok(missing)
}

/// Where a named branch is in `at`, whatever `HEAD` is on.
pub fn head_of(at: &Path, branch: &str) -> Result<String, GitError> {
    run(at, &["rev-parse", &format!("{branch}^{{commit}}")])
}
