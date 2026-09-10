//! Opening the pull request on the forge (SPEC 4.2, `hq push`).
//!
//! The one outward-facing write in the product, and it happens under the same
//! `--yes` as the push: SPEC names one verb that "pousse la branche … et ouvre
//! la pull request", on one explicit argument. There is no second gesture,
//! because there is no second decision — the human read the pull request
//! before saying yes.
//!
//! It talks to GitHub with **the human's** credential, kept at the HQ and
//! never mounted in a container: `~/.hq/<project>/forge-token`. The only
//! part of the HQ a container ever sees is its own mission folder, one level
//! below. Without that file `hq push` still pushes, and hands over the exact
//! address to open the pull request at by hand.
//!
//! Only GitHub, because it is the only forge any project here uses; a remote
//! elsewhere is said to be elsewhere, not guessed at.

use std::time::Duration;

use serde::Deserialize;

/// GitHub's API — the only host the human's credential is ever sent to.
///
/// Deliberately not overridable from the environment: a variable that could
/// redirect this address would send the token to whatever host it named, and
/// nothing in SPEC lets a credential travel to a host nobody declared. Tests
/// reach a server of their own through [`crate::push::push_to`]'s parameter,
/// which the binary never calls with anything but this.
pub const API: &str = "https://api.github.com";

/// The credential's file, under the project's HQ.
pub const TOKEN_FILE: &str = "forge-token";

#[derive(Debug, thiserror::Error)]
pub enum ForgeError {
    #[error("the forge refused ({status}): {message}")]
    Refused { status: u16, message: String },
    #[error("the forge could not be reached: {0}")]
    Unreachable(String),
    #[error("the forge answered something hq cannot read: {0}")]
    Unreadable(String),
}

/// `owner/name` on GitHub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    pub owner: String,
    pub name: String,
}

impl Repo {
    /// The repository a remote URL points at, if it is on GitHub — SSH
    /// (`git@github.com:o/r.git`) and HTTPS alike.
    pub fn of_remote(remote: &str) -> Option<Repo> {
        let path = remote
            .trim()
            .trim_end_matches('/')
            .trim_end_matches(".git")
            .rsplit_once("github.com")
            .map(|(_, rest)| rest.trim_start_matches([':', '/']))?;
        let (owner, name) = path.split_once('/')?;
        if owner.is_empty() || name.is_empty() || name.contains('/') {
            return None;
        }
        Some(Repo {
            owner: owner.to_string(),
            name: name.to_string(),
        })
    }
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

/// The human's token, if they put one at the HQ. An empty file is no token.
pub fn token(hq_root: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(hq_root.join(TOKEN_FILE))
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

#[derive(Deserialize)]
struct Created {
    html_url: String,
}

#[derive(Deserialize)]
struct Refusal {
    message: Option<String>,
    #[serde(default)]
    errors: Vec<serde_json::Value>,
}

/// Open the pull request, or find the one already open for this branch.
pub fn open(api: &str, token: &str, repo: &Repo, pr: &PullRequest) -> Result<Opened, ForgeError> {
    let agent = agent();
    let url = format!("{api}/repos/{}/{}/pulls", repo.owner, repo.name);
    let body = serde_json::json!({
        "title": pr.title,
        "body": pr.body,
        "head": pr.head,
        "base": pr.base,
    });
    // Serialised here rather than through `ureq`'s `json` feature: that
    // feature is what puts `cookie_store`, `cookie` and `time` in the
    // lockfile, and `time` carried RUSTSEC-2026-0009 at the only version our
    // minimum Rust allows. Nothing here needs cookies.
    let mut response = authorised(agent.post(&url), token)
        .header("Content-Type", "application/json")
        .send(body.to_string())
        .map_err(|e| ForgeError::Unreachable(e.to_string()))?;
    let status = response.status().as_u16();
    let text = response
        .body_mut()
        .read_to_string()
        .map_err(|e| ForgeError::Unreadable(e.to_string()))?;

    match status {
        201 => {
            let created: Created =
                serde_json::from_str(&text).map_err(|e| ForgeError::Unreadable(e.to_string()))?;
            Ok(Opened::Created(created.html_url))
        }
        // GitHub answers a duplicate with 422 and says so in `errors`.
        422 if text.contains("already exists") => {
            existing(&agent, api, token, repo, pr).map(Opened::AlreadyOpen)
        }
        _ => Err(refused(status, &text)),
    }
}

/// The open pull request from `head` into `base`, which the forge has just
/// said exists.
fn existing(
    agent: &ureq::Agent,
    api: &str,
    token: &str,
    repo: &Repo,
    pr: &PullRequest,
) -> Result<String, ForgeError> {
    let url = format!("{api}/repos/{}/{}/pulls", repo.owner, repo.name);
    let head = format!("{}:{}", repo.owner, pr.head);
    let mut response = authorised(agent.get(&url), token)
        .query("head", &head)
        .query("base", &pr.base)
        .query("state", "open")
        .call()
        .map_err(|e| ForgeError::Unreachable(e.to_string()))?;
    let status = response.status().as_u16();
    let text = response
        .body_mut()
        .read_to_string()
        .map_err(|e| ForgeError::Unreadable(e.to_string()))?;
    if status != 200 {
        return Err(refused(status, &text));
    }
    let open: Vec<Created> =
        serde_json::from_str(&text).map_err(|e| ForgeError::Unreadable(e.to_string()))?;
    open.into_iter()
        .next()
        .map(|p| p.html_url)
        .ok_or_else(|| ForgeError::Refused {
            status: 422,
            message: format!(
                "a pull request from {head} into {} already exists, and none is open",
                pr.base
            ),
        })
}

/// Whether the token can read the repository — the one read-only call, which
/// proves the address, the certificate chain and the credential together.
pub fn can_read(api: &str, token: &str, repo: &Repo) -> Result<(), ForgeError> {
    let url = format!("{api}/repos/{}/{}", repo.owner, repo.name);
    let mut response = authorised(agent().get(&url), token)
        .call()
        .map_err(|e| ForgeError::Unreachable(e.to_string()))?;
    let status = response.status().as_u16();
    if status == 200 {
        return Ok(());
    }
    let text = response.body_mut().read_to_string().unwrap_or_default();
    Err(refused(status, &text))
}

/// A client that returns a 4xx as data rather than as an error: the forge's
/// refusal carries the reason, and throwing it away would leave "status 403"
/// where "Resource not accessible by personal access token" was said.
fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into()
}

fn authorised<B>(request: ureq::RequestBuilder<B>, token: &str) -> ureq::RequestBuilder<B> {
    request
        .header("Authorization", &format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", concat!("hq/", env!("CARGO_PKG_VERSION")))
}

fn refused(status: u16, text: &str) -> ForgeError {
    let message = match serde_json::from_str::<Refusal>(text) {
        Ok(r) => {
            let mut m = r.message.unwrap_or_else(|| "no reason given".to_string());
            if !r.errors.is_empty() {
                m.push_str(&format!(" — {}", serde_json::Value::from(r.errors)));
            }
            m
        }
        Err(_) if text.trim().is_empty() => "no reason given".to_string(),
        Err(_) => text.trim().chars().take(300).collect(),
    };
    ForgeError::Refused { status, message }
}
