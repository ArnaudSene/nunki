//! Who `nunki` is talking to (SPEC 4.5, the handovers).
//!
//! A mission ends by giving something back to a person: an arbitration, a
//! verdict to accept, a push to authorise. Saying "waiting on the human" is
//! enough when there is one; it stops being enough the moment a second
//! person works on the same project, and "arbitration for Arnaud" and
//! "arbitration for Igor" are different sentences.
//!
//! `nunki` therefore knows a name, and it takes it from the least surprising
//! place that has one: a file the human wrote, then git, then the account on
//! the machine. It is never guessed silently — [`Human::source`] says where
//! the name came from, and `nunki check` says when there is none.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The file a human writes to say who they are, under `~/.nunki/`.
pub const ME_FILE: &str = "me.yaml";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Declared {
    pub name: String,
    #[serde(default)]
    pub email: Option<String>,
}

/// Where a name came from. Kept because a name from `$USER` is a guess and a
/// name from a file is a statement, and a report that treats them alike is
/// lying by omission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// `~/.nunki/me.yaml`.
    Declared(PathBuf),
    /// `git config user.name`, in the project.
    Git,
    /// The machine's account name.
    Environment,
    /// Nothing said who this is.
    Unknown,
}

impl Source {
    pub fn describe(&self) -> String {
        match self {
            Source::Declared(p) => p.display().to_string(),
            Source::Git => "git config user.name".to_string(),
            Source::Environment => "the machine's account name".to_string(),
            Source::Unknown => "nowhere — nothing says who you are".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Human {
    pub name: Option<String>,
    pub email: Option<String>,
    pub source: Source,
}

impl Human {
    /// The name to put in front of a person, or a plain fallback that does
    /// not pretend to know one.
    pub fn addressed(&self) -> String {
        self.name.clone().unwrap_or_else(|| "the human".to_string())
    }

    pub fn is_known(&self) -> bool {
        self.name.is_some()
    }
}

/// Who is running `nunki`. `at` is the repository, for git's answer.
pub fn me(nunki_home: &Path, at: Option<&Path>) -> Human {
    if let Some(declared) = declared(nunki_home) {
        return Human {
            name: Some(declared.name),
            email: declared.email,
            source: Source::Declared(nunki_home.join(ME_FILE)),
        };
    }
    if let Some(at) = at {
        if let Ok(name) = crate::git::run(at, &["config", "user.name"]) {
            if !name.trim().is_empty() {
                let email = crate::git::run(at, &["config", "user.email"])
                    .ok()
                    .filter(|e| !e.trim().is_empty());
                return Human {
                    name: Some(name.trim().to_string()),
                    email,
                    source: Source::Git,
                };
            }
        }
    }
    match std::env::var("USER").ok().filter(|u| !u.is_empty()) {
        Some(user) => Human {
            name: Some(user),
            email: None,
            source: Source::Environment,
        },
        None => Human {
            name: None,
            email: None,
            source: Source::Unknown,
        },
    }
}

fn declared(nunki_home: &Path) -> Option<Declared> {
    let text = std::fs::read_to_string(nunki_home.join(ME_FILE)).ok()?;
    let declared: Declared = serde_yaml_ng::from_str(&text).ok()?;
    (!declared.name.trim().is_empty()).then_some(Declared {
        name: declared.name.trim().to_string(),
        email: declared.email,
    })
}
