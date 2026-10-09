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
//! The rule for the coder, since requiring a fresh coder verdict after every
//! commit would ask for something impossible: **the coder's verdict stays
//! valid as long as everything added after it is the integrator's wiring**,
//! judged by the same allowlist its own gate 4 uses. A commit after the
//! coder's that touches business code invalidates it, and the coder goes back
//! out in a volet.
//!
//! The rule for the security agent once its rounds are spent (SPEC 4.5):
//! the rigor caps its rounds, and once the cap is reached it is never
//! launched again, so a verdict on a later `HEAD` can never come. Its last
//! concluded verdict then stands for the commits after it — a `CLEAR` as it
//! is, a `FINDINGS` only once a human lifted it after it was concluded — and
//! those commits are named, in push's answer, as not attacked by the security
//! agent. While a round is left, nothing changes: a verdict on `HEAD`.

use crate::harness::Role;
use crate::mission::Verdict;
use crate::mission::flow::Stage;
use crate::project::Project;
use crate::state::{MissionState, Store, lock::SlotLock};

#[derive(Debug, thiserror::Error)]
pub enum PushError {
    /// What would be published is not the commit the gates and verdicts
    /// judged (security round 3).
    #[error(
        "{at}, the branch {branch} is at {tip}, but the commit the gates and verdicts judged \
         is {judged}: nunki publishes only the judged commit, and {done}"
    )]
    NotJudged {
        branch: String,
        at: &'static str,
        tip: String,
        judged: String,
        done: &'static str,
    },
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
        "the security rounds are spent and the last security verdict is on {on}, which is \
         not an ancestor of {head} — what follows it on another branch is not what came \
         after the last round"
    )]
    SecurityNotAnAncestor { on: String, head: String },
    #[error(
        "the repository is on {0}, which is the branch being fetched — git refuses to \
         fetch into the branch that is checked out. Move off it first"
    )]
    FetchingIntoCurrent(String),
    #[error("the repository has no remote named {0} — `git remote -v` says what it has")]
    NoRemote(String),
    #[error(
        "{count} equivalence proposal(s) await the HQ's ruling: {listed}. A proposal — \
         the coder's, or `nunki`'s from a ruling it matched — is not a ruling, and \
         nothing rests on one that nobody ruled: `nunki mission mutants {mission} --ratify \
         <survivor>` (or `--ratify --all`) makes it the HQ's equivalence, `nunki mission \
         mutants {mission} --refuse <survivor> --because <why>` sends it back to the coder"
    )]
    ProposalsAwait {
        mission: String,
        count: usize,
        listed: String,
    },
    #[error(
        "gate 7's rule no longer holds on the campaign as it stands — {owed}. A survivor \
         reopened after the gates needs a coder run: `nunki mission iterate {mission} \
         --because <why>` sends one"
    )]
    MutantsOwed { mission: String, owed: String },
    #[error(
        "mission {mission} is at `critical` rigor, and {why} — a `critical` mission is \
         pushed only on a full campaign at the pushed commit that passes: `nunki mission \
         mutants {mission} --again` runs one"
    )]
    NotFinal { mission: String, why: String },
    #[error(
        "mission {mission} is at `{rigor}` rigor and {file} holds no mutation campaign — \
         no campaign, no push: `nunki mission mutants {mission}` runs one, and the same \
         verb, once the campaign has finished, writes it; `nunki push` reads it then"
    )]
    NoCampaign {
        mission: String,
        rigor: crate::mission::Rigor,
        file: &'static str,
    },
    #[error(transparent)]
    Mutants(#[from] crate::mutants::MutantsError),
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
    let judged = crate::git::slot_head(&slot.tree)?;
    fetch_judged(project, id, &judged)
}

/// [`fetch`], of the commit `judged` and of nothing else: refused, with
/// nothing fetched, when the slot's mission branch is not that commit — a
/// branch moved on past what the gates saw, or a `HEAD` detached from it.
pub fn fetch_judged(project: &Project, id: &str, judged: &str) -> Result<Fetched, PushError> {
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

    // Named in full everywhere: a tag of the same name would win over the
    // branch for a short name.
    let full = format!("refs/heads/{branch}");
    let before = crate::git::run(
        &project.root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{full}^{{commit}}"),
        ],
    )
    .ok()
    .filter(|s| !s.is_empty());
    // No `+`: a non-fast-forward is refused rather than forced. What is in
    // the repository was put there by a human or by an earlier fetch, and
    // overwriting it is not this verb's to do.
    // From the slot's host mirror, never from the slot: a fetch from a
    // path runs `upload-pack` there, under that repository's configuration,
    // and the slot's is its agent's (`git::SlotGit`).
    //
    // Exactly the mission's branch crosses, and nothing else: `--no-tags`,
    // or git would follow every tag of the mirror that points into the
    // fetched history, and a tag the agent named `dev` would shadow the
    // human's `dev` (security round 2).
    let mirror = crate::git::SlotGit::open(&slot.tree)?;
    // The branch the mirror holds — what the fetch would bring — is the
    // judged commit, or nothing crosses.
    let tip = mirror
        .run(&["rev-parse", "--verify", &format!("{full}^{{commit}}")])
        .unwrap_or_else(|_| "nothing".into());
    if tip != judged {
        return Err(PushError::NotJudged {
            branch,
            at: "in the slot",
            tip,
            judged: judged.to_string(),
            done: "nothing was fetched",
        });
    }
    crate::git::run(
        &project.root,
        &[
            "fetch",
            "--no-tags",
            &mirror.mirror().display().to_string(),
            &format!("{full}:{full}"),
        ],
    )?;
    let after = crate::git::head_of(&project.root, &full)?;
    Ok(Fetched {
        branch,
        head: after,
        was: before,
    })
}

/// The second check of [`push_to`]: the branch the fetch left in the
/// project is the judged commit, or nothing is pushed. The first, before the
/// fetch, is [`fetch_judged`]'s; this one holds even if that one is wrong.
fn is_judged(fetched: &Fetched, judged: &str) -> Result<(), PushError> {
    if fetched.head == judged {
        return Ok(());
    }
    Err(PushError::NotJudged {
        branch: fetched.branch.clone(),
        at: "in the project, after the fetch",
        tip: fetched.head.clone(),
        judged: judged.to_string(),
        done: "nothing was pushed",
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
    /// The commits after the last security round, oldest first, as
    /// `<short sha> <subject>`: pushed because the rounds were spent, and
    /// not attacked by the security agent. Empty when its verdict is on
    /// `HEAD`, or when the mission has no security agent.
    pub not_attacked: Vec<String>,
    /// The findings `nunki` lifted itself, every one `LOW` or `INFO`, in the
    /// lift this push stands on, one line each (SPEC 4.5). Empty when the
    /// security verdict was a `CLEAR`, or a human lifted it.
    pub lifted_by_nunki: Vec<String>,
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
    push_to(project, id, authorised, crate::forge::github::API)
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
    let head = crate::git::slot_head(&slot.tree)?;
    let header = state.flow.header().clone();
    let Held {
        not_attacked,
        lifted_by_nunki,
    } = verdicts_hold(&slot, &state, &head)?;
    proposals_ruled(project, id)?;
    nothing_owed(project, id, &header, &slot.tree, &head)?;

    // Exactly the judged commit, by id: refused before the fetch when the
    // slot's branch is not it, and checked again on the project's side before
    // anything is pushed.
    let fetched = fetch_judged(project, id, &head)?;
    is_judged(&fetched, &head)?;
    let remote = remote_url(project)?;
    crate::git::run(&project.root, &["push", REMOTE, &fetched.branch])?;

    state.updated_at = crate::state::now_rfc3339();
    state.pushed = Some(crate::state::PushedAt {
        head: head.clone(),
        date: state.updated_at.clone(),
    });
    store.save(&state)?;

    // After the push, and under the same `--yes`: SPEC names one verb that
    // pushes the branch and opens the pull request, on one argument.
    let pr = crate::mission::dir::Paths::of(&project.hq_root, id).pr;
    let pull_request = open_pull_request(
        project,
        &remote,
        &header,
        &fetched.branch,
        &pr,
        state.flow.overrides(),
        api,
    );
    Ok(Pushed {
        branch: fetched.branch.clone(),
        head,
        remote,
        pull_request,
        not_attacked,
        lifted_by_nunki,
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
    overrides: &[crate::mission::flow::Override],
    api: &str,
) -> PullRequestState {
    // The forge the remote is on, if `nunki` has an adapter for it. The
    // address a human would open the pull request at is that adapter's too:
    // a remote elsewhere gets no address rather than a GitHub one.
    let found = crate::forge::of_remote_at(remote, api);
    let compare = found
        .as_ref()
        .map(|(forge, repo)| forge.compare(repo, &header.base, branch));
    let by_hand = |why: String| PullRequestState::ByHand {
        compare: compare.clone(),
        why,
    };

    let Some((forge, repo)) = found else {
        return by_hand(format!(
            "the remote is on a forge nunki has no adapter for ({remote}): GitHub is the one \
             it opens pull requests on"
        ));
    };
    let Some(token) = crate::forge::token(&project.nunki_home()) else {
        return by_hand(format!(
            "no forge credential at {} — a {} token that can open pull requests on {}/{}, and \
             nunki opens it itself",
            project
                .nunki_home()
                .join(crate::forge::TOKEN_FILE)
                .display(),
            forge.name(),
            repo.owner,
            repo.name
        ));
    };
    let text = std::fs::read_to_string(pr_file).unwrap_or_default();
    let Some(mut request) = crate::forge::PullRequest::from_markdown(&text, branch, &header.base)
    else {
        return by_hand(format!(
            "{} says nothing a pull request could be titled with",
            pr_file.display()
        ));
    };
    request.body = with_overrides(&request.body, overrides);
    match forge.open(&token, &repo, &request) {
        Ok(opened) => PullRequestState::Opened(opened),
        Err(e) => by_hand(e.to_string()),
    }
}

/// Every equivalence proposed on the current campaign — by the coder, or by
/// `nunki` from a ruling it matched — has been ratified or refused by the HQ
/// (SPEC 4.4, gate 7).
///
/// Gate 7 counts a proposal as an outcome so that the mission is not stopped
/// for it; this is where it waits instead. The decision on an equivalence is
/// the HQ's, and a branch whose triage rests on a sentence nobody ruled on is
/// not pushed.
fn proposals_ruled(project: &Project, id: &str) -> Result<(), PushError> {
    let dir = crate::mission::dir::Paths::of(&project.hq_root, id).dir;
    let waiting = crate::mutants::awaiting_ruling(&dir)?;
    if waiting.is_empty() {
        return Ok(());
    }
    Err(PushError::ProposalsAwait {
        mission: id.to_string(),
        count: waiting.len(),
        listed: waiting
            .iter()
            .map(|p| {
                format!(
                    "`{}` ({}: {})",
                    crate::text::one_line(&p.id),
                    p.source(),
                    crate::text::brief(&p.why, 120)
                )
            })
            .collect::<Vec<_>>()
            .join("; "),
    })
}

/// Gate 7's rule still holds on the campaign as it stands
/// ([`crate::mutants::owed`]), with the threshold the gate used: the one
/// frozen in the header, or the project's for a header framed before it.
///
/// Asked again here because `Verified` is not proof of it any more once the
/// HQ has ruled after the gates: a refused proposal reopens its survivor,
/// and a branch carrying a mutant nobody answered is not pushed (security
/// round 1, MEDIUM).
///
/// And it fails closed: a mission whose rigor owes a campaign and whose
/// `MUTANTS.json` is missing or empty is not pushed. Gate 7 never lets such
/// a mission reach `Verified`, so the only way here is a file taken away
/// after the gates — and "nothing on file" must not read as "nothing owed"
/// (HQ review of the pull request, item 3).
///
/// It reads the coder's file as gate 7 does — only the outcomes the coder
/// may give, and every test one names must exist, as a whole word, in the
/// tree of `head`, the commit being pushed — and
/// its refusal names the entries it did not read (HQ review 3): a triage
/// rewritten after the gates is judged as the gate would have judged it.
fn nothing_owed(
    project: &Project,
    id: &str,
    header: &crate::mission::Header,
    tree: &std::path::Path,
    head: &str,
) -> Result<(), PushError> {
    let dir = crate::mission::dir::Paths::of(&project.hq_root, id).dir;
    if header.rigor != crate::mission::Rigor::Prototype && crate::mutants::read(&dir)?.is_none() {
        return Err(PushError::NoCampaign {
            mission: id.to_string(),
            rigor: header.rigor,
            file: crate::mutants::FILE,
        });
    }
    if header.rigor == crate::mission::Rigor::Critical
        && let Some(campaign) = crate::mutants::read(&dir)?
        && let Some(why) = not_final_at(tree, &header.base, &campaign)?
    {
        return Err(PushError::NotFinal {
            mission: id.to_string(),
            why,
        });
    }
    let threshold = header
        .mutation_threshold
        .unwrap_or(project.config.mutation_threshold);
    let missing = match crate::mutants::read(&dir)? {
        Some(campaign) if header.rigor != crate::mission::Rigor::Prototype => {
            crate::gate::named_test_missing(
                tree,
                Some(head),
                &campaign,
                &crate::mutants::read_triage(&dir)?,
            )
        }
        _ => None,
    };
    let owed = crate::mutants::owed_on_file(&dir, header.rigor, threshold)?.or(missing);
    match owed {
        None => Ok(()),
        Some(owed) => Err(PushError::MutantsOwed {
            mission: id.to_string(),
            owed: match crate::mutants::foreign_said(&crate::mutants::foreign(&dir)?) {
                Some(refused) => format!("{owed}; {refused}"),
                None => owed,
            },
        }),
    }
}

/// Why `campaign` is not the final one a `critical` mission is pushed on
/// (SPEC 4.4, 4.5), or `None` when it is: it must have run on the content
/// of `HEAD`, the commit being pushed — the same fingerprint gate 7 asks —
/// and be full ([`crate::mutants::not_final`]).
fn not_final_at(
    tree: &std::path::Path,
    base: &str,
    campaign: &crate::mutants::Campaign,
) -> Result<Option<String>, PushError> {
    let touched = crate::gate::touched_since_base(tree, base)?;
    let want = crate::mutants::fingerprint(tree, &touched)?;
    if campaign.fingerprint != want {
        return Ok(Some(format!(
            "the campaign on file ran on other content than the pushed commit ({} against {})",
            &campaign.fingerprint[..7.min(campaign.fingerprint.len())],
            &want[..7.min(want.len())]
        )));
    }
    Ok(crate::mutants::not_final(campaign))
}

/// What the verdicts a push stands on leave to say.
struct Held {
    /// The commits the security agent did not attack because its rounds
    /// were spent — empty whenever its verdict is on `HEAD`.
    not_attacked: Vec<String>,
    /// The findings `nunki` lifted itself in the lift the push stands on.
    lifted_by_nunki: Vec<String>,
}

/// Every precondition of SPEC 4.4, in the order a human would ask them.
fn verdicts_hold(
    slot: &crate::slot::Slot,
    state: &MissionState,
    head: &str,
) -> Result<Held, PushError> {
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

    let held = if header.has_security_agent() {
        security_holds(slot, state, head)?
    } else {
        Held {
            not_attacked: Vec::new(),
            lifted_by_nunki: Vec::new(),
        }
    };

    // The coder's, and the rule that makes it survive the integrator's
    // commits (SPEC 4.4, "le verdict et le `HEAD`, quand l'intégrateur
    // commite").
    let coder = state
        .concluded(Role::Coder)
        .ok_or(PushError::NoVerdict { role: Role::Coder })?;
    only_wiring_since(slot, &coder.head, head, header)?;
    Ok(held)
}

/// The security agent's precondition (SPEC 4.5).
///
/// While the mission has a round left, a verdict on `HEAD`: a later commit
/// can still be attacked, so it must be. Once the rounds are spent, no
/// verdict on a later commit can ever come, and asking for one would refuse
/// every mission with a volet after its last round. The last verdict then
/// stands for what came after it, and only that way: a `CLEAR` as it is, a
/// `FINDINGS` lifted after it was concluded — never one nobody lifted. What
/// came after is returned, to be named as not attacked.
///
/// A lift `nunki` recorded itself, on a report whose every finding is `LOW`
/// or `INFO`, is read by exactly these rules, and its findings are returned
/// so the push names them.
///
/// A prototype plays no round: there was never a security agent to
/// conclude, and none is asked for, as for `security: gates`. A header
/// framed today cannot carry both ([`crate::mission::Rigor::admits`]), but
/// a state frozen before that refusal can.
fn security_holds(
    slot: &crate::slot::Slot,
    state: &MissionState,
    head: &str,
) -> Result<Held, PushError> {
    let mut held = Held {
        not_attacked: Vec::new(),
        lifted_by_nunki: Vec::new(),
    };
    if state.flow.max_security_rounds() == 0 {
        return Ok(held);
    }
    let concluded = state
        .concluded(Role::Security)
        .ok_or(PushError::NoVerdict {
            role: Role::Security,
        })?;
    let spent = state.flow.security_rounds() >= state.flow.max_security_rounds();
    if !spent {
        on_this_commit(Role::Security, concluded, head)?;
        // The one verdict that may be overruled, and only by a lift on this
        // commit, with a reason: a human's through `nunki mission accept`,
        // or `nunki`'s own on a report below MEDIUM.
        if concluded.verdict == Some(Verdict::Findings) {
            let lifts: Vec<&crate::state::Accepted> = state
                .accepted_on(head)
                .into_iter()
                .filter(|a| a.finding.is_none())
                .collect();
            if lifts.is_empty() {
                return Err(PushError::NotLifted(head.to_string()));
            }
            held.lifted_by_nunki = by_nunki(&lifts);
        } else {
            green(Role::Security, concluded.verdict)?;
        }
        return Ok(held);
    }

    // "What came after the last round" means something only on its branch.
    if !is_ancestor(&slot.tree, &concluded.head, head) {
        return Err(PushError::SecurityNotAnAncestor {
            on: concluded.head.clone(),
            head: head.to_string(),
        });
    }
    if concluded.verdict == Some(Verdict::Findings) {
        let lifts = lifted_after(&slot.tree, state, concluded, head);
        if lifts.is_empty() {
            return Err(PushError::NotLifted(head.to_string()));
        }
        held.lifted_by_nunki = by_nunki(&lifts);
    } else {
        green(Role::Security, concluded.verdict)?;
    }
    held.not_attacked = not_attacked(&slot.tree, &concluded.head, head)?;
    Ok(held)
}

/// The findings `nunki` lifted itself among `lifts`, one line each.
fn by_nunki(lifts: &[&crate::state::Accepted]) -> Vec<String> {
    lifts
        .iter()
        .flat_map(|a| crate::findings::lifted_by_nunki(a))
        .collect()
}

/// The lifts of the verdict as a whole given after `concluded` was
/// concluded — on its commit or a later one of the branch being pushed, and
/// no earlier in time. A lift given on an earlier report is a decision about
/// that report, not about this one.
fn lifted_after<'s>(
    tree: &std::path::Path,
    state: &'s MissionState,
    concluded: &crate::state::Concluded,
    head: &str,
) -> Vec<&'s crate::state::Accepted> {
    state
        .accepted
        .iter()
        .filter(|a| {
            a.finding.is_none()
                && a.date >= concluded.date
                && is_ancestor(tree, &concluded.head, &a.head)
                && is_ancestor(tree, &a.head, head)
        })
        .collect()
}

/// Whether `older` is `newer` or one of its ancestors. Anything git cannot
/// answer — an unknown commit — is a no.
fn is_ancestor(tree: &std::path::Path, older: &str, newer: &str) -> bool {
    crate::git::on_slot(tree, &["merge-base", "--is-ancestor", older, newer]).is_ok()
}

/// The commits after `since` up to `head`, oldest first, each as
/// `<short sha> <subject>`: what the security agent did not attack once its
/// rounds were spent on `since` (SPEC 4.5). `nunki push` and the follow-up
/// note name them the same way.
pub fn not_attacked(
    tree: &std::path::Path,
    since: &str,
    head: &str,
) -> Result<Vec<String>, crate::git::GitError> {
    let out = crate::git::on_slot(
        tree,
        &[
            "log",
            "--reverse",
            "--abbrev=12",
            "--format=%h %s",
            &format!("{since}..{head}"),
        ],
    )?;
    // A subject is its author's text, printed by `push` and written into
    // `FOLLOWUP_HQ.md`: made printable here, once, for both.
    Ok(out.lines().map(crate::text::printable).collect())
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
    if crate::git::on_slot(&slot.tree, &["merge-base", "--is-ancestor", coder, head]).is_err() {
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
    // A commit, and said so: what the integrator added after the coder's
    // gates were green, never a base by name.
    let touched = crate::gate::touched_paths(&slot.tree, &crate::gate::Rev::commit(coder))?;
    let allowed = crate::gate::compile(&wiring)?;
    let outside: Vec<String> = touched
        .into_iter()
        .filter(|p| !allowed.is_match(p))
        .collect();
    if outside.is_empty() {
        return Ok(());
    }
    let commits = crate::git::on_slot(
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

/// The remote a mission is pushed to, as git holds it.
///
/// The address a human would open the pull request at is not worked out here
/// any more: it is the forge adapter's ([`crate::forge::Forge::compare`]), so
/// a remote on a forge `nunki` has no adapter for gets no address rather than
/// one shaped like somebody else's.
fn remote_url(project: &Project) -> Result<String, PushError> {
    crate::git::run(&project.root, &["remote", "get-url", REMOTE])
        .map_err(|_| PushError::NoRemote(REMOTE.to_string()))
}

/// Each departure from the rigor in one printable line — its date, what
/// the rigor said, and the HQ's reason — as `mission status` and the pull
/// request's note list them (SPEC 4.5).
pub fn overrides_said(overrides: &[crate::mission::flow::Override]) -> Vec<String> {
    overrides
        .iter()
        .map(|o| {
            crate::text::one_line(&format!(
                "{} — {}; set aside because: {}",
                o.date.split('T').next().unwrap_or_default(),
                o.departure,
                o.because
            ))
        })
        .collect()
}

/// The pull request's body, followed by the note on the rigor when the HQ
/// set it aside: every override, with its reason, so a reviewer on the
/// forge sees how often the rule gave way. `PR.md` is the agent's and is
/// left as it is; the note is added on the way to the forge.
pub fn with_overrides(body: &str, overrides: &[crate::mission::flow::Override]) -> String {
    if overrides.is_empty() {
        return body.to_string();
    }
    let lines: Vec<String> = overrides_said(overrides)
        .into_iter()
        .map(|said| format!("- {said}"))
        .collect();
    let note = format!(
        "## Departures from the rigor\n\nThe HQ set the mission's rigor aside {} time(s):\n\n{}",
        overrides.len(),
        lines.join("\n")
    );
    if body.trim().is_empty() {
        note
    } else {
        format!("{}\n\n{note}", body.trim_end())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A project-side tip that is not the judged commit is refused, naming
    /// both, whatever the check before the fetch let through.
    #[test]
    fn a_fetched_tip_that_is_not_the_judged_commit_is_never_pushed() {
        let fetched = |head: &str| Fetched {
            branch: "mission/x".into(),
            head: head.into(),
            was: None,
        };
        assert!(is_judged(&fetched("aaa"), "aaa").is_ok());
        let err = is_judged(&fetched("bbb"), "aaa").unwrap_err().to_string();
        assert!(err.contains("at bbb"), "{err}");
        assert!(err.contains("judged is aaa"), "{err}");
        assert!(err.contains("nothing was pushed"), "{err}");
    }
}
