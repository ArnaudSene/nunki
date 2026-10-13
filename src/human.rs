//! Who `nunki` is talking to (SPEC 4.5, the handovers).
//!
//! A mission ends by giving something back to a person: an arbitration, a
//! verdict to accept, a push to authorise. Saying "waiting on the human" is
//! enough when there is one; it stops being enough the moment a second
//! person works on the same project, and "arbitration for Alice" and
//! "arbitration for Bob" are different sentences.
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
    /// `~/.nunki/me.yaml` exists and cannot be read, for the reason given.
    /// Nothing else is asked then: the file is the human's statement, and a
    /// name from git or the account in its place would be somebody else's.
    Unreadable { path: PathBuf, why: String },
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
            Source::Unreadable { path, why } => {
                format!("{}, which cannot be read: {why}", path.display())
            }
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
    /// not pretend to know one — and says why, when the human's own file is
    /// what could not be read.
    pub fn addressed(&self) -> String {
        match (&self.name, &self.source) {
            (Some(name), _) => name.clone(),
            (None, Source::Unreadable { .. }) => {
                format!("the human ({})", self.source.describe())
            }
            (None, _) => "the human".to_string(),
        }
    }

    /// Why there is no name, when it is because the human's own file cannot
    /// be read: the file and the reason, to say where the name was needed.
    pub fn unreadable(&self) -> Option<String> {
        matches!(self.source, Source::Unreadable { .. }).then(|| self.source.describe())
    }

    pub fn is_known(&self) -> bool {
        self.name.is_some()
    }
}

/// Who is running `nunki`. `at` is the repository, for git's answer.
pub fn me(nunki_home: &Path, at: Option<&Path>) -> Human {
    let path = nunki_home.join(ME_FILE);
    match declared(&path) {
        Ok(Some(declared)) => {
            return Human {
                name: Some(declared.name),
                email: declared.email,
                source: Source::Declared(path),
            };
        }
        // The human wrote who they are, and it cannot be read: no other
        // source answers in its place, or a mission would say somebody else
        // ruled. The reason travels with the missing name.
        Err(why) => {
            return Human {
                name: None,
                email: None,
                source: Source::Unreadable { path, why },
            };
        }
        Ok(None) => {}
    }
    if let Some(at) = at
        && let Ok(name) = crate::git::run(at, &["config", "user.name"])
        && !name.trim().is_empty()
    {
        let email = crate::git::run(at, &["config", "user.email"])
            .ok()
            .filter(|e| !e.trim().is_empty());
        return Human {
            name: Some(name.trim().to_string()),
            email,
            source: Source::Git,
        };
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

/// What `me.yaml` declares: `Ok(None)` when the file does not exist, or
/// names nobody; an error, saying why, when it exists and cannot be read.
fn declared(path: &Path) -> Result<Option<Declared>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let declared: Declared = crate::yaml::from_str(&text).map_err(|e| e.to_string())?;
    Ok((!declared.name.trim().is_empty()).then_some(Declared {
        name: declared.name.trim().to_string(),
        email: declared.email,
    }))
}
