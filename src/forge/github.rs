//! GitHub, the one forge adapter there is (SPEC 4.2, `nunki push`).
//!
//! Everything GitHub-shaped lives here: the API's address, how a remote URL
//! names a repository, the two calls `nunki` makes and the headers they carry.
//! Another forge is another file beside this one, implementing the same
//! [`super::Forge`] trait, and nothing above this module changes.

use std::time::Duration;

use serde::Deserialize;

use super::{Forge, ForgeError, Opened, Protection, PullRequest, Repo};

/// GitHub's API — the only host the human's credential is ever sent to.
///
/// Deliberately not overridable from the environment: a variable that could
/// redirect this address would send the token to whatever host it named, and
/// nothing in SPEC lets a credential travel to a host nobody declared. Tests
/// reach a server of their own through [`super::of_remote_at`], which the
/// binary never calls with anything but this.
pub const API: &str = "https://api.github.com";

/// The host a remote must name for this adapter to answer for it.
const HOST: &str = "github.com";

pub struct GitHub {
    api: String,
}

impl GitHub {
    pub fn at(api: &str) -> Self {
        Self {
            api: api.to_string(),
        }
    }
}

/// The repository a remote URL points at, if it is on GitHub — SSH
/// (`git@github.com:o/r.git`) and HTTPS alike.
pub fn repo_of_remote(remote: &str) -> Option<Repo> {
    let path = remote
        .trim()
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .rsplit_once(HOST)
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

impl Forge for GitHub {
    fn name(&self) -> &'static str {
        "GitHub"
    }

    fn open(&self, token: &str, repo: &Repo, pr: &PullRequest) -> Result<Opened, ForgeError> {
        let agent = agent();
        let url = format!("{}/repos/{}/{}/pulls", self.api, repo.owner, repo.name);
        let body = serde_json::json!({
            "title": pr.title,
            "body": pr.body,
            "head": pr.head,
            "base": pr.base,
        });
        // Serialised here rather than through `ureq`'s `json` feature: that
        // feature is what puts `cookie_store`, `cookie` and `time` in the
        // lockfile, and `time` carried RUSTSEC-2026-0009 at the only version
        // our minimum Rust allows. Nothing here needs cookies.
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
                let created: Created = serde_json::from_str(&text)
                    .map_err(|e| ForgeError::Unreadable(e.to_string()))?;
                Ok(Opened::Created(created.html_url))
            }
            // GitHub answers a duplicate with 422 and says so in `errors`.
            422 if text.contains("already exists") => self
                .existing(&agent, token, repo, pr)
                .map(Opened::AlreadyOpen),
            _ => Err(refused(status, &text)),
        }
    }

    /// Read from the branch itself (`protected`), not from the protection
    /// endpoint: measured on 2026-09-10, a private repository on GitHub's free
    /// plan refuses the latter with 403 ("Upgrade to GitHub Pro") while still
    /// answering the former — and the question asked here is the one a push
    /// would meet, not how the rules are written.
    fn protection(&self, token: &str, repo: &Repo, branch: &str) -> Result<Protection, ForgeError> {
        #[derive(Deserialize)]
        struct Branch {
            protected: bool,
        }
        let url = format!(
            "{}/repos/{}/{}/branches/{branch}",
            self.api, repo.owner, repo.name
        );
        let mut response = authorised(agent().get(&url), token)
            .call()
            .map_err(|e| ForgeError::Unreachable(e.to_string()))?;
        let status = response.status().as_u16();
        let text = response
            .body_mut()
            .read_to_string()
            .map_err(|e| ForgeError::Unreadable(e.to_string()))?;
        match status {
            200 => {
                let branch: Branch = serde_json::from_str(&text)
                    .map_err(|e| ForgeError::Unreadable(e.to_string()))?;
                Ok(if branch.protected {
                    Protection::Protected
                } else {
                    Protection::Unprotected
                })
            }
            // Measured: a missing branch answers 404 "Branch not found", and a
            // repository the token cannot see answers 404 too, with "Not
            // Found". Only the first is an answer about the branch; the second
            // is the forge declining to say, and is reported as a refusal.
            404 if text.contains("Branch not found") => Ok(Protection::NoSuchBranch),
            _ => Err(refused(status, &text)),
        }
    }

    fn can_read(&self, token: &str, repo: &Repo) -> Result<(), ForgeError> {
        let url = format!("{}/repos/{}/{}", self.api, repo.owner, repo.name);
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

    fn compare(&self, repo: &Repo, base: &str, head: &str) -> String {
        format!(
            "https://{HOST}/{}/{}/compare/{base}...{head}?expand=1",
            repo.owner, repo.name
        )
    }
}

impl GitHub {
    /// The open pull request from `head` into `base`, which the forge has just
    /// said exists.
    fn existing(
        &self,
        agent: &ureq::Agent,
        token: &str,
        repo: &Repo,
        pr: &PullRequest,
    ) -> Result<String, ForgeError> {
        let url = format!("{}/repos/{}/{}/pulls", self.api, repo.owner, repo.name);
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

/// A client that returns a 4xx as data rather than as an error: the forge's
/// refusal carries the reason, and throwing it away would leave "status 403"
/// where "Resource not accessible by personal access token" was said.
fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        // A forge that does not answer should say so quickly: `nunki check`
        // asks once per protected branch, and a silent minute reads as a
        // hung verb.
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into()
}

fn authorised<B>(request: ureq::RequestBuilder<B>, token: &str) -> ureq::RequestBuilder<B> {
    request
        .header("Authorization", &format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", concat!("nunki/", env!("CARGO_PKG_VERSION")))
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
