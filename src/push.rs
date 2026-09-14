//! `nunki mission fetch` and `nunki push` (SPEC 4.2, verb table; 4.4; 4.5).
//!
//! The only verb that touches the forge in write, and the only place a
//! mission's commits leave their slot. Everything before it can be
//! autonomous **because** this one is not: the push happens on an explicit
//! human argument, and the merge is a human's on the forge.
//!
//! What it refuses, and why it can refuse it. A verdict is worth one commit
//! (SPEC 4.5), and `VERDICT.json` holds one verdict at a time — every role
//! overwrites it — so the file cannot answer "did the integrator pass, and on
//! what?" once the security agent has written. `nunki`'s state can, and that is
//! what is read here.
//!
//! The rule for the coder is the one Arnaud decided on 2026-09-09, after the
//! first version asked for something impossible: **the coder's verdict stays
//! valid as long as everything added after it is the integrator's wiring**,
//! judged by the same allowlist its own gate 4 uses. A commit after the
//! coder's that touches business code invalidates it, and the coder goes back
//! out in a volet.

use crate::harness::Role;
use crate::mission::Verdict;
use crate::mission::flow::Stage;
use crate::project::Project;
use crate::state::{MissionState, Store, lock::SlotLock};

#[derive(Debug, thiserror::Error)]
pub enum PushError {
    #[error("mission {0} has not started — there is nothing to push")]
    NotStarted(String),
    #[error(
        "mission {mission} is at {stage:?}, not verified — `nunki verify {mission}` says what \
         it is waiting for. There is no flag to push past this"
    )]
    NotVerified { mission: String, stage: Stage },
    #[error(
        "pushing is the one thing that is not autonomous: say so on the command line \
         (`nunki push {0} --yes`) once you have read the pull request"
    )]
    NotAuthorised(String),
    #[error("the {role:?} has not concluded on {head} — it concluded on {on}")]
    VerdictElsewhere {
        role: Role,
        head: String,
        on: String,
    },
    #[error("the {role:?} has not concluded at all, and the mission declares it")]
    NoVerdict { role: Role },
    #[error("the {role:?} concluded {verdict:?}, and nothing red is ever pushed")]
    Red { role: Role, verdict: Verdict },
    #[error(
        "the security agent concluded FINDINGS and nobody lifted it on {0} — \
         `nunki mission accept` records who took the risk, and why"
    )]
    NotLifted(String),
    #[error(
        "the coder's verdict is on {coder}, and {commits} commit(s) after it touch more \
         than the integrator's wiring: {paths}. A commit after the coder's that touches \
         business code invalidates its verdict (SPEC 4.4)"
    )]
    NotOnlyWiring {
        coder: String,
        commits: usize,
        paths: String,
    },
    #[error("the coder's verdict is on {0}, which is not an ancestor of {1}")]
    NotAnAncestor(String, String),
    #[error(
        "the repository is on {0}, which is the branch being fetched — git refuses to \
         fetch into the branch that is checked out. Move off it first"
    )]
    FetchingIntoCurrent(String),
    #[error("the repository has no remote named {0} — `git remote -v` says what it has")]
    NoRemote(String),
    #[error(transparent)]
    Git(#[from] crate::git::GitError),
    #[error(transparent)]
    State(#[from] crate::state::StateError),
    #[error(transparent)]
    Lock(#[from] crate::state::LockError),
    #[error(transparent)]
    Slot(#[from] crate::slot::SlotError),
    #[error(transparent)]
    Gate(#[from] crate::gate::GateError),
}

/// The remote a mission is pushed to. One name, because a project with two
/// remotes has a decision to make that `nunki` must not make for it.
pub const REMOTE: &str = "origin";

/// Bring a mission's commits from its slot into the main repository.
///
/// This is the only way commits leave a slot (SPEC 4.2, "slots et
/// branches"), and it goes one way: into the repository, never back. The
/// repository is the human's, and `nunki` writes one ref in it and nothing else.
pub fn fetch(project: &Project, id: &str) -> Result<Fetched, PushError> {
    let store = Store::open(&project.hq_root)?;
    let state = store
        .load(id)
        .map_err(|_| PushError::NotStarted(id.to_string()))?;
    let slot = crate::slot::find(project, &state.slot)?;
    let branch = state.flow.header().branch.clone();

    // Git refuses to fetch into the branch that is checked out, and says so
    // in a message about refs. Said here instead, about the repository.
    if crate::git::current_branch(&project.root)? == branch {
        return Err(PushError::FetchingIntoCurrent(branch));
    }

    let before = crate::git::run(
        &project.root,
        &["rev-parse", "--verify", "--quiet", &branch],
    )
    .ok()
    .filter(|s| !s.is_empty());
    // No `+`: a non-fast-forward is refused rather than forced. What is in
    // the repository was put there by a human or by an earlier fetch, and
    // overwriting it is not this verb's to do.
    crate::git::run(
        &project.root,
        &[
            "fetch",
            &slot.tree.display().to_string(),
            &format!("{branch}:{branch}"),
        ],
    )?;
    let after = crate::git::head_of(&project.root, &branch)?;
    Ok(Fetched {
        branch,
        head: after,
        was: before,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    pub branch: String,
    /// Where the branch is in the repository now.
    pub head: String,
    /// Where it was, if it was there at all.
    pub was: Option<String>,
}

/// What a push did, for the caller to print.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pushed {
    pub branch: String,
    pub head: String,
    pub remote: String,
    pub pull_request: PullRequestState,
}

/// Where the pull request stands once the branch is on the forge.
///
/// Not a `Result`: by the time it is decided the push has **already
/// happened**, and it cannot be undone. A forge that refused the pull request
/// is reported next to a push that succeeded, never in place of it — a
/// `nunki push` that exited red after pushing would be retried, and the second
/// `git push` is a no-op that hides the first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullRequestState {
    Opened(crate::forge::Opened),
    /// Left for the human to open, and why — with the exact address when
    /// `nunki` can work one out.
    ByHand {
        compare: Option<String>,
        why: String,
    },
}

/// Push a verified mission's branch, on an explicit human argument, and open
/// its pull request on the forge's real API.
pub fn push(project: &Project, id: &str, authorised: bool) -> Result<Pushed, PushError> {
    push_to(project, id, authorised, crate::forge::API)
}

/// [`push`], against a given forge API — how the tests point the real client
/// at a server they control. The binary calls [`push`] only, so the address
/// the human's token goes to is never read from anything a process inherits.
pub fn push_to(
    project: &Project,
    id: &str,
    authorised: bool,
    api: &str,
) -> Result<Pushed, PushError> {
    if !authorised {
        return Err(PushError::NotAuthorised(id.to_string()));
    }
    let store = Store::open(&project.hq_root)?;
    let mut state = store
        .load(id)
        .map_err(|_| PushError::NotStarted(id.to_string()))?;
    let _lock = SlotLock::acquire(&project.hq_root.join("locks"), &state.slot, "push")?;

    if !matches!(state.flow.stage(), Stage::Verified) {
        return Err(PushError::NotVerified {
            mission: id.to_string(),
            stage: state.flow.stage().clone(),
        });
    }

    let slot = crate::slot::find(project, &state.slot)?;
    let head = crate::git::head(&slot.tree)?;
    let header = state.flow.header().clone();
    verdicts_hold(&slot, &state, &head)?;

    let fetched = fetch(project, id)?;
    let remote = remote_url(project)?;
    crate::git::run(&project.root, &["push", REMOTE, &fetched.branch])?;

    state.updated_at = crate::state::now_rfc3339();
    store.save(&state)?;

    // After the push, and under the same `--yes`: SPEC names one verb that
    // pushes the branch and opens the pull request, on one argument.
    let pr = crate::mission::dir::Paths::of(&project.hq_root, id).pr;
    let pull_request = open_pull_request(project, &remote, &header, &fetched.branch, &pr, api);
    Ok(Pushed {
        branch: fetched.branch.clone(),
        head,
        remote,
        pull_request,
    })
}

/// Open the pull request if everything it needs is there, and say which
/// piece is missing otherwise. Every branch but the last is a reason the
/// forge was never asked; the last is the forge's own answer.
fn open_pull_request(
    project: &Project,
    remote: &str,
    header: &crate::mission::Header,
    branch: &str,
    pr_file: &std::path::Path,
    api: &str,
) -> PullRequestState {
    let compare = pull_request_url(remote, &header.base, branch);
    let by_hand = |why: String| PullRequestState::ByHand {
        compare: compare.clone(),
        why,
    };

    let Some(repo) = crate::forge::Repo::of_remote(remote) else {
        return by_hand(format!(
            "the remote is not on GitHub ({remote}), and GitHub is the only forge nunki opens \
             pull requests on"
        ));
    };
    let Some(token) = crate::forge::token(&project.hq_root) else {
        return by_hand(format!(
            "no forge credential at {} — a GitHub token that can open pull requests on \
             {}/{}, and nunki opens it itself",
            project.hq_root.join(crate::forge::TOKEN_FILE).display(),
            repo.owner,
            repo.name
        ));
    };
    let text = std::fs::read_to_string(pr_file).unwrap_or_default();
    let Some(request) = crate::forge::PullRequest::from_markdown(&text, branch, &header.base)
    else {
        return by_hand(format!(
            "{} says nothing a pull request could be titled with",
            pr_file.display()
        ));
    };
    match crate::forge::open(api, &token, &repo, &request) {
        Ok(opened) => PullRequestState::Opened(opened),
        Err(e) => by_hand(e.to_string()),
    }
}

/// Every precondition of SPEC 4.4, in the order a human would ask them.
fn verdicts_hold(
    slot: &crate::slot::Slot,
    state: &MissionState,
    head: &str,
) -> Result<(), PushError> {
    let header = state.flow.header();

    if header.has_integration() {
        let concluded = state
            .concluded(Role::Integrator)
            .ok_or(PushError::NoVerdict {
                role: Role::Integrator,
            })?;
        on_this_commit(Role::Integrator, concluded, head)?;
        green(Role::Integrator, concluded.verdict)?;
    }

    if header.has_security_agent() {
        let concluded = state
            .concluded(Role::Security)
            .ok_or(PushError::NoVerdict {
                role: Role::Security,
            })?;
        on_this_commit(Role::Security, concluded, head)?;
        // The one verdict a human may overrule, and only by having said so
        // on this commit, with a reason, through `nunki mission accept`.
        if concluded.verdict == Some(Verdict::Findings) {
            if !state.verdict_lifted_on(head) {
                return Err(PushError::NotLifted(head.to_string()));
            }
        } else {
            green(Role::Security, concluded.verdict)?;
        }
    }

    // The coder's, and the rule that makes it survive the integrator's
    // commits (SPEC 4.4, "le verdict et le `HEAD`, quand l'intégrateur
    // commite").
    let coder = state
        .concluded(Role::Coder)
        .ok_or(PushError::NoVerdict { role: Role::Coder })?;
    only_wiring_since(slot, &coder.head, head, header)
}

fn on_this_commit(
    role: Role,
    concluded: &crate::state::Concluded,
    head: &str,
) -> Result<(), PushError> {
    if concluded.head == head {
        Ok(())
    } else {
        Err(PushError::VerdictElsewhere {
            role,
            head: head.to_string(),
            on: concluded.head.clone(),
        })
    }
}

fn green(role: Role, verdict: Option<Verdict>) -> Result<(), PushError> {
    match verdict {
        Some(v) if v.is_green() => Ok(()),
        Some(v) => Err(PushError::Red { role, verdict: v }),
        // Only the coder's is `None`, and it is not asked here.
        None => Err(PushError::NoVerdict { role }),
    }
}

/// Everything added after the coder's commit must be the integrator's wiring,
/// judged by the very allowlist its own gate 4 used.
fn only_wiring_since(
    slot: &crate::slot::Slot,
    coder: &str,
    head: &str,
    header: &crate::mission::Header,
) -> Result<(), PushError> {
    if coder == head {
        return Ok(());
    }
    // An ancestor, or the question means nothing: two commits on different
    // branches have a difference that is not "what was added".
    if crate::git::run(&slot.tree, &["merge-base", "--is-ancestor", coder, head]).is_err() {
        return Err(PushError::NotAnAncestor(
            coder.to_string(),
            head.to_string(),
        ));
    }

    let wiring = match &header.integration {
        crate::mission::Integration::Services { wiring, .. } => wiring.clone(),
        // No integration mission, so nothing had the right to commit after
        // the coder — and something did.
        crate::mission::Integration::None { .. } => Vec::new(),
    };
    // `touched_paths` reads `<base>...HEAD`, and the slot is on `head` — the
    // gates refuse to run at all on a tree that is not (gate 1).
    let touched = crate::gate::touched_paths(&slot.tree, coder)?;
    let allowed = crate::gate::compile(&wiring)?;
    let outside: Vec<String> = touched
        .into_iter()
        .filter(|p| !allowed.is_match(p))
        .collect();
    if outside.is_empty() {
        return Ok(());
    }
    let commits = crate::git::run(
        &slot.tree,
        &["rev-list", "--count", &format!("{coder}..{head}")],
    )?
    .trim()
    .parse()
    .unwrap_or(0);
    Err(PushError::NotOnlyWiring {
        coder: coder.to_string(),
        commits,
        paths: outside
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
    })
}

fn remote_url(project: &Project) -> Result<String, PushError> {
    crate::git::run(&project.root, &["remote", "get-url", REMOTE])
        .map_err(|_| PushError::NoRemote(REMOTE.to_string()))
}

/// The address a human opens the pull request at, worked out from the
/// remote's own URL.
///
/// Used whenever `nunki` does not open it itself — no credential, a remote that
/// is not on GitHub, a forge that refused. A URL nobody has to assemble is
/// the difference between "opened by hand" and "left to figure out".
pub fn pull_request_url(remote: &str, base: &str, branch: &str) -> Option<String> {
    let path = remote
        .trim_end_matches(".git")
        .rsplit_once("github.com")
        .map(|(_, rest)| rest.trim_start_matches([':', '/']).to_string())?;
    if path.is_empty() {
        return None;
    }
    Some(format!(
        "https://github.com/{path}/compare/{base}...{branch}?expand=1"
    ))
}
