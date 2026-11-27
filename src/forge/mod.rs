//! The forge: opening the pull request, and asking what it protects
//! (SPEC 4.2, `nunki push`).
//!
//! The one outward-facing write in the product, and it happens under the same
//! `--yes` as the push: SPEC names one verb that "pousse la branche … et ouvre
//! la pull request", on one explicit argument. There is no second gesture,
//! because there is no second decision — the human read the pull request
//! before saying yes.
//!
//! It talks to the forge with **the project's** token, kept in the project's
//! own home at `~/.nunki/<id>/forge-token` ([`TOKEN_FILE`]) and read from
//! there and nowhere else ([`credential`]). One file per project, because on
//! GitHub what grants rights is the token, not the account: a fine-grained
//! token can be limited to one repository and to the permissions nunki uses
//! ([`PERMISSIONS`]), and a file per project lets each project hold only its
//! own. The file `~/.nunki/forge-token` that every project used to share is
//! no longer read, not even as a fallback; it is only named, so that the move
//! is said rather than silent.
//!
//! The token is **required**: without it nothing reaches the forge. `nunki
//! push` refuses before it fetches or pushes anything, and `nunki check` is
//! red, both with the one message [`Absent::refusal`] writes — the project's
//! path, the permissions the token needs, and that its file is readable by
//! its owner only.
//!
//! Its owner only, and nothing less: on unix, a token file whose mode grants
//! anything to its group or to others is refused as ssh refuses such a
//! private key ([`judge`]), and so is one whose mode cannot be read. The mode
//! judged is that of the file opened, before a byte of it is read.
//!
//! The token never reaches a container. No bind mount nunki declares may
//! expose the project's home or the token file — [`crate::compose`] refuses a
//! plan whose mounts would ([`exposes`]) — and what a container does see under
//! the home is its own mission folder, the stack's scripts one file at a
//! time, and the secrets file the human ruled on.
//!
//! **One adapter per forge**, as there is one per harness (SPEC 4.3) and one
//! per container engine (4.2). This module holds what every forge means — a
//! repository, a pull request, a protected branch — and each adapter holds how
//! that forge is asked. GitHub is the one implemented, because it is the one
//! the projects here use; a remote elsewhere is **said to be elsewhere and
//! never guessed at**: it is not pushed, since the project's token is a GitHub
//! token and nothing else may carry the push ([`git`]).

pub mod git;
pub mod github;

use std::path::{Path, PathBuf};

/// The token's file, in the project's home (`~/.nunki/<id>/forge-token`),
/// the only place it is read.
///
/// Per project rather than once per machine: a fine-grained token is limited
/// to the repositories it names, so one file per project lets each project
/// hold a token for its own repository and nothing more. One file shared by
/// every project would push towards one token that can do everything
/// everywhere. The same name directly under `~/.nunki/` is the retired file,
/// which [`credential`] names and never reads.
pub const TOKEN_FILE: &str = "forge-token";

/// What the project's token must be allowed to do on GitHub, and nothing
/// more: push the branch, open its pull request, and read what the
/// repository is — its branch protection included, for `nunki check`.
pub const PERMISSIONS: &str =
    "Contents: read and write, Pull requests: read and write, and Metadata: read";

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

/// The project's forge token: a secret that does not print itself.
///
/// Its `Debug` says nothing of what it holds, so a token carried in a value
/// that is logged, asserted on or put in an error never reaches the output.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    /// The secret itself, for the one use it has: the forge's
    /// `Authorization` header.
    pub fn secret(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(..)")
    }
}

/// What the project's home holds for the forge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Credential {
    /// A token, read from the project's own file.
    Token(Token),
    /// No token: the file is missing or says nothing.
    Absent(Absent),
    /// A file is there, and nunki will not use it: what it holds was not
    /// read, or not kept.
    Refused(Refused),
}

/// A token file nunki refuses, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    /// The project's token file.
    pub at: PathBuf,
    pub why: Why,
}

/// Why a token file is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Why {
    /// Its mode grants something to its group or to others: the permission
    /// bits, `0o777` at most.
    OpenToOthers(u32),
    /// Its mode could not be read, and an unknown mode is not an allowed one.
    ModeUnknown(String),
    /// It is there, and could not be opened or read.
    Unopenable(String),
}

impl Refused {
    /// Why nothing reaches the forge, said the same way by `nunki push` and
    /// `nunki check`: the file's path, what is wrong with it, and what fixes
    /// it. Never anything the file holds.
    pub fn refusal(&self) -> String {
        let at = self.at.display();
        let wrong = match &self.why {
            Why::OpenToOthers(mode) => format!(
                "the forge token at {at} has mode {mode:04o}, which opens it to others than \
                 its owner: nunki refuses a token others can read, as ssh refuses such a private \
                 key. `chmod 600 {at}` fixes it"
            ),
            Why::ModeUnknown(e) => format!(
                "the mode of the forge token at {at} could not be read ({e}), and a token \
                 nunki cannot tell others are kept from is refused. Check that `ls -l {at}` \
                 shows `-rw-------`, or run `chmod 600 {at}`"
            ),
            Why::Unopenable(e) => format!(
                "the forge token at {at} is there and could not be read ({e}). It must be a \
                 file its owner can read and nobody else can: `chmod 600 {at}`"
            ),
        };
        format!("{wrong} — nothing is pushed and no pull request is opened until it is")
    }
}

/// Why there is no token, and where it would go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Absent {
    /// The project's token file, where a token goes.
    pub at: PathBuf,
    /// The machine-wide file a token used to live in, when it is still there.
    /// Named so the move is said; never read.
    pub retired: Option<PathBuf>,
}

impl Absent {
    /// Why nothing reaches the forge, said the same way by `nunki push` and
    /// `nunki check`: where the project's token goes, what it must be allowed
    /// to do, and that its file is readable by its owner only — then, when
    /// the retired machine-wide file is still there, that it is no longer
    /// read. Never anything a file holds.
    pub fn refusal(&self) -> String {
        let mut why = format!(
            "no forge token at {at}: nunki reaches the forge with this project's own token and \
             with nothing else, so without it nothing is pushed and no pull request is opened. \
             Put there a fine-grained GitHub token limited to this project's repository, with \
             {PERMISSIONS}, in a file readable by its owner only (`chmod 600 {at}`)",
            at = self.at.display()
        );
        if let Some(note) = self.retired_note() {
            why.push_str(". ");
            why.push_str(&note);
        }
        why
    }

    /// The sentence that says the retired file is not read and where the
    /// project's token goes, or nothing when there is no retired file.
    pub fn retired_note(&self) -> Option<String> {
        self.retired.as_ref().map(|old| {
            format!(
                "{} is no longer read, not even as a fallback: nunki reads one token per \
                 project, from {}, and no other",
                old.display(),
                self.at.display()
            )
        })
    }
}

/// The project's forge credential, read from `home` — the project's own
/// home, `~/.nunki/<id>/` ([`crate::project::Project::home`]) — and from
/// nowhere else. An empty file is no token.
///
/// The file is opened first, and its mode is read **from what was opened**
/// — the file the read would read, through a link if it is one — then
/// judged ([`judge`]) before a byte of it is read: a token others can read
/// is refused, as ssh refuses such a private key, and so is one whose mode
/// cannot be read. A file that exists and cannot be opened or read is
/// refused too, with the reason, rather than taken for no file.
///
/// The machine-wide `~/.nunki/forge-token` (the file of that name in
/// `home`'s parent) is **never read**: when the project has no token and that
/// file exists, its path is returned in [`Absent::retired`], from its
/// metadata alone, so that the caller can say the token moved.
pub fn credential(home: &Path) -> Credential {
    use std::io::Read;

    let at = home.join(TOKEN_FILE);
    let absent = |at: PathBuf| {
        Credential::Absent(Absent {
            retired: retired(home),
            at,
        })
    };
    let refused = |at: &Path, why: Why| {
        Credential::Refused(Refused {
            at: at.to_path_buf(),
            why,
        })
    };
    let mut file = match std::fs::File::open(&at) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return absent(at),
        Err(e) => return refused(&at, Why::Unopenable(e.to_string())),
    };
    if let Err(why) = judge(mode_of(&file)) {
        return refused(&at, why);
    }
    let mut text = String::new();
    if let Err(e) = file.read_to_string(&mut text) {
        return refused(&at, Why::Unopenable(e.to_string()));
    }
    let token = text.trim();
    if token.is_empty() {
        return absent(at);
    }
    Credential::Token(Token(token.to_string()))
}

/// The permission bits of an opened file, or why they could not be read.
#[cfg(unix)]
fn mode_of(file: &std::fs::File) -> Result<Option<u32>, String> {
    use std::os::unix::fs::PermissionsExt;
    file.metadata()
        .map(|m| Some(m.permissions().mode()))
        .map_err(|e| e.to_string())
}

/// Elsewhere there are no unix permission bits to judge.
#[cfg(not(unix))]
fn mode_of(_file: &std::fs::File) -> Result<Option<u32>, String> {
    Ok(None)
}

/// Whether a token file's mode lets it be used: allowed only when it grants
/// nothing to its group or to others (`mode & 0o077 == 0`), as ssh asks of a
/// private key. A mode that could not be read is refused with its own
/// reason, never taken as allowed. `None` is a platform with no such bits.
pub fn judge(mode: Result<Option<u32>, String>) -> Result<(), Why> {
    match mode {
        Err(e) => Err(Why::ModeUnknown(e)),
        Ok(Some(mode)) if mode & 0o077 != 0 => Err(Why::OpenToOthers(mode & 0o777)),
        Ok(_) => Ok(()),
    }
}

/// The retired machine-wide file, if it is still there. Its metadata only:
/// what it holds is never opened.
fn retired(home: &Path) -> Option<PathBuf> {
    let old = home.parent()?.join(TOKEN_FILE);
    std::fs::symlink_metadata(&old).is_ok().then_some(old)
}

/// The paths no bind mount may expose for a project whose home is `home`:
/// the home itself, and the token file. Where a link makes either land is
/// [`exposes`]'s to resolve, at the time of the mount, so a mount of a
/// linked token's target is caught too. The home comes first, so a mount
/// of it or above it is refused as exposing the home.
pub fn kept_out(home: &Path) -> Vec<PathBuf> {
    vec![home.to_path_buf(), home.join(TOKEN_FILE)]
}

/// Whether a bind mount of `source` would put `kept` inside a container: the
/// source is `kept` itself or a directory above it. Compared as written and
/// as the filesystem resolves both, so a link to the home — or to a
/// directory above it — exposes it as plainly as its own path does.
pub fn exposes(source: &Path, kept: &Path) -> bool {
    if kept.starts_with(source) {
        return true;
    }
    match (std::fs::canonicalize(source), std::fs::canonicalize(kept)) {
        (Ok(source), Ok(kept)) => kept.starts_with(source),
        _ => false,
    }
}
