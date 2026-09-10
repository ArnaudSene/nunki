//! Accounts: which subscription a mission spends (SPEC 4.3).
//!
//! One human can hold several — two Anthropic subscriptions, an OpenAI one —
//! and a mission says which it runs on. That choice is made when the mission
//! is framed and **frozen with the rest of the header**, so a mission cannot
//! change the account it spends halfway through, any more than it can change
//! its perimeter.
//!
//! Accounts live at `~/.hq/accounts.yaml`, beside the projects' HQs and
//! outside every repository: an account is a property of the human, not of a
//! project, and two projects share the same subscriptions. The tokens
//! themselves are in files of their own, one per account, so the index can be
//! read without reading the secrets.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The file that names the accounts, under `~/.hq/`.
pub const INDEX_FILE: &str = "accounts.yaml";
/// Where the tokens themselves live, one file per account.
pub const TOKENS_DIR: &str = "accounts";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    /// Which harness this account authenticates. An Anthropic subscription
    /// is not an OpenAI one, and they are not passed the same way.
    pub harness: String,
    /// Where the token is. Relative paths are taken from `~/.hq/`.
    pub token_file: PathBuf,
    /// What this account is, for a human reading the list.
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Accounts {
    #[serde(default)]
    pub accounts: BTreeMap<String, Account>,
    /// Used when neither the mission nor `hq.yaml` names one.
    #[serde(default)]
    pub default: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum AccountError {
    #[error("no account {name:?}; {known}")]
    Unknown { name: String, known: String },
    #[error(
        "no account named: put one in the mission's header (`hq mission new --account <name>`), \
         in hq.yaml, or as `default:` in {0}"
    )]
    NotNamed(PathBuf),
    #[error("account {name:?} has no token at {path} — put the output of `{command}` there")]
    NoToken {
        name: String,
        path: PathBuf,
        command: String,
    },
    #[error("{0} is not valid: {1}")]
    Invalid(PathBuf, String),
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
}

impl Accounts {
    /// Read the index. A missing file is an empty index, not an error: a
    /// project that has never named an account has nothing to read.
    pub fn load(hq_home: &Path) -> Result<Self, AccountError> {
        let path = hq_home.join(INDEX_FILE);
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_yaml_ng::from_str(&text)
                .map_err(|e| AccountError::Invalid(path, e.to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(AccountError::Io(path, e)),
        }
    }

    pub fn names(&self) -> Vec<&str> {
        self.accounts.keys().map(String::as_str).collect()
    }

    /// The account to use, given what the mission asks and what the project
    /// declares. The order is the order of specificity: the mission wins,
    /// then the project, then the index's own default — and when a single
    /// account exists, it is the answer rather than a question.
    pub fn choose(
        &self,
        hq_home: &Path,
        from_mission: Option<&str>,
        from_project: Option<&str>,
    ) -> Result<(String, Account), AccountError> {
        let name = from_mission
            .or(from_project)
            .map(str::to_string)
            .or_else(|| self.default.clone())
            .or_else(|| match self.accounts.len() {
                1 => self.accounts.keys().next().cloned(),
                _ => None,
            })
            .ok_or_else(|| AccountError::NotNamed(hq_home.join(INDEX_FILE)))?;

        let account = self
            .accounts
            .get(&name)
            .cloned()
            .ok_or_else(|| AccountError::Unknown {
                name: name.clone(),
                known: if self.accounts.is_empty() {
                    format!(
                        "none are declared in {}",
                        hq_home.join(INDEX_FILE).display()
                    )
                } else {
                    format!("known: {}", self.names().join(", "))
                },
            })?;
        Ok((name, account))
    }
}

impl Account {
    /// Where the token is, resolved: a relative path is taken from
    /// `~/.hq/`, so an index can be written without absolute paths.
    pub fn token_path(&self, hq_home: &Path) -> PathBuf {
        if self.token_file.is_absolute() {
            self.token_file.clone()
        } else {
            hq_home.join(&self.token_file)
        }
    }

    /// The token itself. Read at launch and passed to the container as an
    /// environment variable; never written into a slot (SPEC 3.3).
    pub fn token(&self, hq_home: &Path, name: &str) -> Result<String, AccountError> {
        let path = self.token_path(hq_home);
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let token = text.trim().to_string();
        if token.is_empty() {
            return Err(AccountError::NoToken {
                name: name.to_string(),
                path,
                command: setup_command(&self.harness),
            });
        }
        Ok(token)
    }
}

/// How a human obtains a token for a harness. Named per harness because the
/// answer differs, and saying "run the right command" helps nobody.
pub fn setup_command(harness: &str) -> String {
    match harness {
        "claude-code" => "claude setup-token".to_string(),
        "codex" => "codex login --api-key, or your provider's console".to_string(),
        other => format!("whatever {other} uses to issue a long-lived token"),
    }
}

/// `~/.hq/`, the root every project's HQ sits under.
pub fn hq_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".hq"))
}
