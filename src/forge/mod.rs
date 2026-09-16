//! The forge: opening the pull request, and asking what it protects
//! (SPEC 4.2, `nunki push`).
//!
//! The one outward-facing write in the product, and it happens under the same
//! `--yes` as the push: SPEC names one verb that "pousse la branche … et ouvre
//! la pull request", on one explicit argument. There is no second gesture,
//! because there is no second decision — the human read the pull request
//! before saying yes.
//!
//! It talks to the forge with **the human's** credential, kept beside the
//! accounts at `~/.nunki/forge-token` and never mounted in a container: a
//! forge account belongs to the human, not to a project, and the only part of
//! a project's home a container ever sees is its own mission folder. Without
//! that file `nunki push` still pushes, and hands over the exact address to
//! open the pull request at by hand.
//!
//! **One adapter per forge**, as there is one per harness (SPEC 4.3) and one
//! per container engine (4.2). This module holds what every forge means — a
//! repository, a pull request, a protected branch — and each adapter holds how
//! that forge is asked. GitHub is the one implemented, because it is the one
//! the projects here use; a remote elsewhere is **said to be elsewhere and
//! never guessed at**, and the push still happens.

pub mod github;

/// The credential's file, under `~/.nunki/` and beside the accounts.
///
/// A forge credential belongs to the human and not to a project — two
/// projects on the same forge are the same account — so it is kept once
/// rather than copied into every project's HQ (decided by Arnaud on
/// 2026-09-16, as the accounts were on 2026-09-10).
pub const TOKEN_FILE: &str = "forge-token";

#[derive(Debug, thiserror::Error)]
pub enum ForgeError {
    #[error("the forge refused ({status}): {message}")]
    Refused { status: u16, message: String },
    #[error("the forge could not be reached: {0}")]
    Unreachable(String),
    #[error("the forge answered something nunki cannot read: {0}")]
    Unreadable(String),
}

/// A repository on a forge, as every forge names one: an owner and a name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    pub owner: String,
    pub name: String,
}

/// What is asked of the forge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    pub title: String,
    pub body: String,
    pub head: String,
    pub base: String,
}

impl PullRequest {
    /// Title and body from the mission's `PR.md`: the first line that says
    /// anything is the title, a markdown heading's hashes taken off, and the
    /// rest is the body. `None` when the file says nothing — gate 5 refuses
    /// that already, and a pull request titled with nothing is not one.
    pub fn from_markdown(text: &str, head: &str, base: &str) -> Option<PullRequest> {
        let mut lines = text.lines();
        let title = lines.by_ref().map(str::trim).find(|l| !l.is_empty())?;
        let title = title.trim_start_matches('#').trim().to_string();
        if title.is_empty() {
            return None;
        }
        Some(PullRequest {
            title,
            body: lines.collect::<Vec<_>>().join("\n").trim().to_string(),
            head: head.to_string(),
            base: base.to_string(),
        })
    }
}

/// What the forge did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opened {
    Created(String),
    /// A pull request for this branch was already open — the second push of a
    /// mission after a volet. The forge refuses a duplicate, and the one that
    /// exists is the answer, not a failure.
    AlreadyOpen(String),
}

impl Opened {
    pub fn url(&self) -> &str {
        match self {
            Opened::Created(u) | Opened::AlreadyOpen(u) => u,
        }
    }
}

/// What the forge says about one branch (SPEC 4.1 bis, "branche protégée").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protection {
    Protected,
    Unprotected,
    /// The forge has no such branch, so there is nothing to protect yet.
    NoSuchBranch,
}

/// What `nunki` asks of a forge, and nothing else: it opens one pull request,
/// it asks whether a branch is protected, it checks the credential can read
/// the repository, and it can say where a human would open the pull request
/// themselves. Four questions, because those are the four `nunki` has.
pub trait Forge {
    /// The forge's name, for a message a human reads.
    fn name(&self) -> &'static str;

    /// Open the pull request, or find the one already open for this branch.
    fn open(&self, token: &str, repo: &Repo, pr: &PullRequest) -> Result<Opened, ForgeError>;

    /// Whether `branch` is protected on the forge.
    fn protection(&self, token: &str, repo: &Repo, branch: &str) -> Result<Protection, ForgeError>;

    /// Whether the token can read the repository — the one read-only call,
    /// which proves the address, the certificate chain and the credential
    /// together.
    fn can_read(&self, token: &str, repo: &Repo) -> Result<(), ForgeError>;

    /// The address a human opens to write the pull request themselves, when
    /// `nunki` could not.
    fn compare(&self, repo: &Repo, base: &str, head: &str) -> String;
}

/// The forge a remote is on, and the repository it points at — or nothing,
/// for a remote on a forge `nunki` has no adapter for.
pub fn of_remote(remote: &str) -> Option<(Box<dyn Forge>, Repo)> {
    of_remote_at(remote, github::API)
}

/// The same, against a given API address: how a test points the real client at
/// a server it controls. The binary calls [`of_remote`] only, so the address
/// the human's token goes to is never read from anything a process inherits.
pub fn of_remote_at(remote: &str, api: &str) -> Option<(Box<dyn Forge>, Repo)> {
    github::repo_of_remote(remote)
        .map(|repo| (Box::new(github::GitHub::at(api)) as Box<dyn Forge>, repo))
}

/// The human's token, if they put one beside the accounts. An empty file is
/// no token.
pub fn token(nunki_home: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(nunki_home.join(TOKEN_FILE))
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}
