//! Lifting a security verdict, and sending it back to the coder (SPEC 4.5).
//!
//! "Rien ne se pousse en rouge. Il n'existe pas de drapeau pour passer
//! outre." A `FINDINGS` verdict closes one of three ways, and two of them are
//! gestures a human makes:
//!
//! - **corrected** — the mission goes back to the coder as a volet, bounded
//!   by `max_volets`. That is [`iterate`], and it is a gesture rather than a
//!   default because a session that iterates by itself spends a volet the
//!   human might have wanted to spend on an acceptance instead;
//! - **lifted as a risk**, by a human, with a reason. That is [`accept`].
//!
//! A mission that came through verified is still read by the HQ before it is
//! pushed, and what the HQ refuses goes back the same way, as a volet:
//! [`iterate`] with the reason.
//!
//! Neither ever touches `VERDICT.json`. The verdict says what the agent
//! found; `nunki`'s state says what the human decided; and `nunki push` reads the
//! decision there. Letting a lift edit the verdict would turn a red answer
//! into a green one, which is the one thing this whole section exists to
//! prevent.

use crate::mission::dir::Paths;
use crate::mission::flow::{Event, Stage};
use crate::project::Project;
use crate::state::{Accepted, MissionState, Store, lock::SlotLock};

/// What a human is lifting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lift {
    /// One finding, named as the report names it. The verdict stays where it
    /// is: itemising is not concluding.
    Finding(String),
    /// What the report still carries. This is the one that moves the mission
    /// to `Verified`.
    Verdict,
}

#[derive(Debug, thiserror::Error)]
pub enum FindingsError {
    #[error("mission {0} has not started — nothing has reported a finding yet")]
    NotStarted(String),
    #[error(
        "mission {mission} is at {stage:?}, neither on a security verdict nor verified — \
         there is nothing to lift or to send back yet"
    )]
    NotOnFindings { mission: String, stage: Stage },
    #[error(
        "a risk accepted without a reason is not accepted, it is forgotten — say why \
         with `--because`"
    )]
    NoReason,
    #[error(
        "a verified mission is sent back with what the review refuses — say it with \
         `--because`, which is what the coder reads"
    )]
    NoReview,
    #[error("nunki does not know who you are: `nunki whoami` says where it looks")]
    NoHuman,
    #[error(transparent)]
    State(#[from] crate::state::StateError),
    #[error(transparent)]
    Lock(#[from] crate::state::LockError),
    #[error(transparent)]
    Slot(#[from] crate::slot::SlotError),
    #[error(transparent)]
    Git(#[from] crate::git::GitError),
    #[error(transparent)]
    Followup(#[from] crate::followup::FollowupError),
    #[error(transparent)]
    Mutants(#[from] crate::mutants::MutantsError),
    #[error(
        "the registry of equivalences could not be changed — {0}. Nothing was lifted, on \
         the mission or in the registry: a ruling lifted from the mission and left in the \
         registry would answer the next mission still"
    )]
    RegistryUnchanged(String),
    #[error(
        "--ratify --all stopped at `{at}`: {why}. The HQ's ruling is written on {}{}; \
         nothing after it was touched — list what is pending again with `nunki mission \
         status`",
        if ruled.is_empty() { "no survivor before it".to_string() } else {
            format!(
                "{} before it",
                ruled.iter().map(|r| format!("`{r}`")).collect::<Vec<_>>().join(", ")
            )
        },
        if *at_written { format!(", and on `{at}` itself, before it failed") } else { String::new() }
    )]
    RatifiedPart {
        at: String,
        ruled: Vec<String>,
        /// The HQ's file already holds the ruling on `at`: what failed came
        /// after it was written.
        at_written: bool,
        why: String,
    },
    #[error(
        "--ratify --all lists two proposals on `{0}` with different sentences, and one ruling \
         answers every survivor of an id: nothing was ratified — rule on it with `nunki \
         mission mutants <id> --ratify {0} --because <why>`"
    )]
    TwoSentences(String),
}

/// Record a human's acceptance, and — for [`Lift::Verdict`] — conclude.
pub fn accept(
    project: &Project,
    id: &str,
    lift: Lift,
    why: &str,
) -> Result<MissionState, FindingsError> {
    if why.trim().is_empty() {
        return Err(FindingsError::NoReason);
    }
    let who = crate::human::me(&project.nunki_home(), Some(&project.root))
        .name
        .ok_or(FindingsError::NoHuman)?;

    let store = Store::open(&project.hq_root)?;
    let mut state = load(&store, id)?;
    let _lock = SlotLock::acquire(
        &project.hq_root.join("locks"),
        &state.slot,
        "mission accept",
    )?;
    on_findings(id, &state)?;

    let slot = crate::slot::find(project, &state.slot)?;
    let head = crate::git::head(&slot.tree)?;
    let paths = Paths::of(&project.hq_root, id);

    match &lift {
        Lift::Finding(finding) => {
            crate::followup::lifted(&paths.followup, &who, finding, why, &head)?;
        }
        Lift::Verdict => {
            crate::followup::lifted_all(&paths.followup, &who, why, &head)?;
        }
    }
    state.accepted.push(Accepted {
        finding: match &lift {
            Lift::Finding(f) => Some(f.clone()),
            Lift::Verdict => None,
        },
        why: why.trim().to_string(),
        who,
        head,
        date: crate::state::now_rfc3339(),
        by_nunki: false,
    });
    // The file is written before the state, and the state before the
    // transition: a session that dies in between leaves a record of what the
    // human said, never a mission that moved on a decision nobody can read.
    store.save(&state)?;
    if lift == Lift::Verdict {
        store.apply(&mut state, Event::HumanAccepted)?;
    }
    Ok(state)
}

/// Record `nunki`'s own lift of a `FINDINGS` whose every finding is `LOW` or
/// `INFO` ([`crate::mission::VerdictFile::automatic_lift`] decides that),
/// and conclude as [`accept`] does for a human's lift of the whole verdict.
///
/// A lift and nothing more: one [`Accepted`] for the verdict as a whole, on
/// `head`, marked as `nunki`'s, its reason the findings line by line — so
/// every rule `nunki push` applies to a human's lift applies to it
/// unchanged. Written in the same order as [`accept`]: the follow-up, then
/// the state, then the transition.
pub fn lift_by_nunki(
    store: &Store,
    state: &mut MissionState,
    followup: &std::path::Path,
    lifted: &[crate::mission::LowFinding],
    head: &str,
) -> Result<(), FindingsError> {
    on_findings(&state.id, state)?;
    if lifted.is_empty() {
        // Nothing ranked is nothing to lift: the caller's decision refuses an
        // empty list, and this refuses it again rather than record a lift
        // with no reason.
        return Err(FindingsError::NoReason);
    }
    let lines: Vec<String> = lifted.iter().map(|f| f.line()).collect();
    crate::followup::lifted_by_nunki(followup, &lines, head)?;
    state.accepted.push(Accepted {
        finding: None,
        why: lines.join("\n"),
        who: crate::state::NUNKI.to_string(),
        head: head.to_string(),
        date: crate::state::now_rfc3339(),
        by_nunki: true,
    });
    store.save(state)?;
    store.apply(state, Event::HumanAccepted)?;
    Ok(())
}

/// The findings a lift `nunki` recorded itself carries, one line each and
/// printable, as `nunki push` and `nunki mission status` print them; nothing
/// for a human's lift.
pub fn lifted_by_nunki(accepted: &Accepted) -> Vec<String> {
    if !accepted.by_nunki {
        return Vec::new();
    }
    accepted.why.lines().map(crate::text::one_line).collect()
}

/// Send the mission back to the coder, bounded by `max_volets`.
///
/// From a security verdict, as a correction of its findings. From a verified
/// mission, as the HQ's review of the branch before it is pushed (SPEC 4.5:
/// the HQ reads the code, and its answer is the iteration loop): `because`
/// is then required, is written to `FOLLOWUP_HQ.md` where the coder reads it,
/// and becomes the volet's cause.
pub fn iterate(
    project: &Project,
    id: &str,
    because: Option<&str>,
) -> Result<MissionState, FindingsError> {
    let store = Store::open(&project.hq_root)?;
    let mut state = load(&store, id)?;
    let _lock = SlotLock::acquire(
        &project.hq_root.join("locks"),
        &state.slot,
        "mission iterate",
    )?;
    let because = because.map(str::trim).filter(|b| !b.is_empty());
    match state.flow.stage() {
        Stage::Findings { .. } => {
            store.apply(&mut state, Event::Iterate)?;
        }
        // The HQ read the verified branch and refuses part of it. The reason
        // is what the coder is sent back with, so a review without one is
        // refused rather than turned into a volet nobody can act on.
        Stage::Verified => {
            let because = because.ok_or(FindingsError::NoReview)?;
            let paths = Paths::of(&project.hq_root, id);
            let who = crate::human::me(&project.nunki_home(), Some(&project.root)).addressed();
            crate::followup::reviewed(&paths.followup, &who, because)?;
            store.apply(
                &mut state,
                Event::Reviewed {
                    because: because.to_string(),
                },
            )?;
        }
        stage => {
            return Err(FindingsError::NotOnFindings {
                mission: id.to_string(),
                stage: stage.clone(),
            });
        }
    }
    Ok(state)
}

fn load(store: &Store, id: &str) -> Result<MissionState, FindingsError> {
    store
        .load(id)
        .map_err(|_| FindingsError::NotStarted(id.to_string()))
}

fn on_findings(id: &str, state: &MissionState) -> Result<(), FindingsError> {
    match state.flow.stage() {
        Stage::Findings { .. } => Ok(()),
        stage => Err(FindingsError::NotOnFindings {
            mission: id.to_string(),
            stage: stage.clone(),
        }),
    }
}

/// What [`refuse_proposal`] did beyond the refusal itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    /// The mission was verified, and the refusal sent it back to the coder:
    /// the stage it is at now.
    pub sent_back: Option<Stage>,
}

/// The HQ refuses the equivalence the coder proposed on `survivor`, because
/// of `because` ([`crate::mutants::refuse_and_say`]) — and on a verified
/// mission, sends it back to the coder as a volet, the path an HQ review
/// takes, with the refusal as its cause.
///
/// A refusal reopens its survivor, and a verified mission has no coder run
/// left to answer it: without the volet it would wait on a `nunki push`
/// that refuses it (security round 1, MEDIUM). On any other stage the flow
/// is still going, and gate 7 reads the reopened survivor when the final
/// gates are played. A mission that has not started is only refused on file.
pub fn refuse_proposal(
    project: &Project,
    id: &str,
    survivor: &str,
    because: &str,
) -> Result<Refused, FindingsError> {
    let paths = Paths::of(&project.hq_root, id);
    let who = crate::human::me(&project.nunki_home(), Some(&project.root)).addressed();
    let store = Store::open(&project.hq_root)?;
    let Some(mut state) = started(&store, id)? else {
        crate::mutants::refuse_and_say(&paths, &who, survivor, because)?;
        return Ok(Refused { sent_back: None });
    };
    let _lock = SlotLock::acquire(
        &project.hq_root.join("locks"),
        &state.slot,
        "mission mutants --refuse",
    )?;
    crate::mutants::refuse_and_say(&paths, &who, survivor, because)?;
    if !matches!(state.flow.stage(), Stage::Verified) {
        return Ok(Refused { sent_back: None });
    }
    store.apply(
        &mut state,
        Event::Reviewed {
            because: format!(
                "the HQ refused the equivalence proposed on `{}`: {}",
                crate::text::one_line(survivor),
                crate::text::one_line(because)
            ),
        },
    )?;
    Ok(Refused {
        sent_back: Some(state.flow.stage().clone()),
    })
}

/// Run `ruling` on mission `id`'s files under its slot's lock, the lock
/// `--refuse` takes: a ruling rewrites `MUTANTS.json` and the coder's
/// triage, which `verify` and a campaign's read-back write too, and two
/// writers on one file lose one of them (HQ review of the pull request,
/// item 4). A mission that has not started has no slot and nothing else
/// writing its files: the ruling runs as it is. A state that cannot be read
/// is not a mission that has not started: it is an error, and no ruling is
/// written ([`started`]).
///
/// `ruling` is also handed a repository holding the campaign's commit, for
/// the project's registry of equivalences to read the survivor's line in:
/// the mission's slot, where the campaign ran, or the project's repository
/// for a mission with no slot — where a line that cannot be read enters
/// nothing.
fn under_slot_lock<T>(
    project: &Project,
    id: &str,
    verb: &str,
    ruling: impl FnOnce(&Paths, &std::path::Path) -> Result<T, FindingsError>,
) -> Result<T, FindingsError> {
    let paths = Paths::of(&project.hq_root, id);
    let store = Store::open(&project.hq_root)?;
    let (_lock, tree) = match started(&store, id)? {
        Some(state) => (
            Some(SlotLock::acquire(
                &project.hq_root.join("locks"),
                &state.slot,
                verb,
            )?),
            crate::slot::find(project, &state.slot)
                .map(|s| s.tree)
                .unwrap_or_else(|_| project.root.clone()),
        ),
        None => (None, project.root.clone()),
    };
    ruling(&paths, &tree)
}

/// Mission `id`'s state, or `None` when it has not started — and only then.
///
/// The HQ's rulings run without the slot's lock on a mission that has not
/// started, since nothing else writes its files. Any other reason the state
/// cannot be read — a file that does not parse, one that cannot be opened —
/// says nothing about whether a run, a `verify` or a campaign's read-back is
/// writing `MUTANTS.json` right now. Read as "not started", it would let a
/// ruling be written without the lock beside such a writer, and one of the
/// two writes would be lost: so it is an error, and nothing is written.
fn started(store: &Store, id: &str) -> Result<Option<MissionState>, FindingsError> {
    match store.load(id) {
        Ok(state) => Ok(Some(state)),
        Err(crate::state::StateError::Missing(_)) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// The registry's view of a ruling the HQ gives on mission `id`.
fn registry_ruling<'a>(
    project: &'a Project,
    id: &'a str,
    tree: &'a std::path::Path,
    by: &'a str,
) -> crate::equivalences::Ruling<'a> {
    crate::equivalences::Ruling {
        hq_root: &project.hq_root,
        tree,
        mission: id,
        by,
    }
}

/// Who the HQ's verbs say ruled.
fn who(project: &Project) -> String {
    crate::human::me(&project.nunki_home(), Some(&project.root)).addressed()
}

/// [`crate::mutants::read`], for a ruling that has just written it.
fn campaign_of(paths: &Paths) -> Result<crate::mutants::Campaign, crate::mutants::MutantsError> {
    crate::mutants::read(&paths.dir)?.ok_or_else(|| {
        crate::mutants::MutantsError::Unreadable(
            paths.mutants.clone(),
            "there is no campaign to rule on — `nunki mission mutants` runs one".to_string(),
        )
    })
}

/// `nunki mission mutants --ratify`: [`crate::mutants::ratify`], under the
/// slot's lock, and the ruling entered in the project's registry of
/// equivalences. Returns the reason the ruling was written with, and what
/// became of the registry.
pub fn ratify_proposal(
    project: &Project,
    id: &str,
    survivor: &str,
    because: Option<&str>,
) -> Result<(String, crate::equivalences::Registered), FindingsError> {
    let by = who(project);
    under_slot_lock(project, id, "mission mutants --ratify", |paths, tree| {
        let origin = registry_origin(paths, survivor)?;
        let why = crate::mutants::ratify(&paths.dir, survivor, because)?;
        let registered = register_ratified(
            &registry_ruling(project, id, tree, &by),
            &campaign_of(paths)?,
            survivor,
            origin.as_ref(),
        );
        Ok((why, registered))
    })
}

/// The mission and commit of the registry entry behind the proposal
/// survivor `id` holds, when it holds one from the registry.
fn registry_origin(paths: &Paths, id: &str) -> Result<Option<(String, String)>, FindingsError> {
    Ok(crate::mutants::read(&paths.dir)?.and_then(|c| {
        c.survivors
            .into_iter()
            .find(|s| s.id == id)
            .and_then(|s| match s.outcome {
                Some(crate::mutants::Triage::ProposedByNunki {
                    from:
                        crate::mutants::ProposedFrom::Registry {
                            mission, commit, ..
                        },
                    ..
                }) => Some((mission, commit)),
                _ => None,
            })
    }))
}

/// What a ratification leaves in the registry: a proposal from the registry
/// is recorded on its own entry, which keeps its origin
/// ([`crate::equivalences::ratified`]); any other is entered as the HQ's
/// ruling on this mission ([`crate::equivalences::enter`]).
fn register_ratified(
    ruling: &crate::equivalences::Ruling,
    campaign: &crate::mutants::Campaign,
    survivor: &str,
    origin: Option<&(String, String)>,
) -> crate::equivalences::Registered {
    match origin {
        Some((mission, commit)) => {
            crate::equivalences::ratified(ruling, campaign, survivor, (mission, commit))
        }
        None => crate::equivalences::enter(ruling, campaign, survivor),
    }
}

/// What `--ratify --all` did with one proposal: the proposal as it stood,
/// and what became of the registry once it was the HQ's ruling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ratified {
    pub proposal: crate::mutants::Proposal,
    pub registered: crate::equivalences::Registered,
}

/// The proposals pending on mission `id` — the coder's, and `nunki`'s —
/// that `--ratify --all` would ratify, for the verb to print before it does.
pub fn pending(
    project: &Project,
    id: &str,
) -> Result<Vec<crate::mutants::Proposal>, FindingsError> {
    Ok(crate::mutants::awaiting_ruling(
        &Paths::of(&project.hq_root, id).dir,
    )?)
}

/// `nunki mission mutants --ratify --all`: every one of `listed` — the
/// proposals the verb printed, read by [`pending`] — ratified as
/// [`ratify_proposal`] ratifies one, under one hold of the slot's lock, each
/// with **the sentence it printed**, passed explicitly. A proposal that is
/// not pending any more as it was listed — refused, ratified, or its
/// sentence changed in between — fails the verb before anything is
/// written, so what was printed is what was ruled. One that fails midway
/// stops it, and the error says what was written before it
/// ([`ratify_in_turn`]).
pub fn ratify_all(
    project: &Project,
    id: &str,
    listed: &[crate::mutants::Proposal],
) -> Result<Vec<Ratified>, FindingsError> {
    let by = who(project);
    under_slot_lock(
        project,
        id,
        "mission mutants --ratify --all",
        |paths, tree| {
            let now = crate::mutants::awaiting_ruling(&paths.dir)?;
            if let Some(gone) = listed.iter().find(|p| !now.contains(p)) {
                return Err(crate::mutants::MutantsError::NoProposal(format!(
                    "the proposal on {:?} changed since it was listed, so nothing was ratified \
                 — list them again",
                    gone.id
                ))
                .into());
            }
            ratify_in_turn(
                listed,
                |proposal| {
                    let origin = registry_origin(paths, &proposal.id)?;
                    crate::mutants::ratify(&paths.dir, &proposal.id, Some(&proposal.why))?;
                    Ok(register_ratified(
                        &registry_ruling(project, id, tree, &by),
                        &campaign_of(paths)?,
                        &proposal.id,
                        origin.as_ref(),
                    ))
                },
                |proposal| holds_ruling(paths, proposal),
            )
        },
    )
}

/// Whether the HQ's file holds the ruling `proposal` would have written: an
/// `equivalent` on its survivor, with its sentence.
fn holds_ruling(paths: &Paths, proposal: &crate::mutants::Proposal) -> bool {
    crate::mutants::read(&paths.dir)
        .ok()
        .flatten()
        .is_some_and(|c| {
            c.survivors.iter().any(|s| {
                s.id == proposal.id
                    && matches!(&s.outcome, Some(crate::mutants::Triage::Equivalent { why, .. }) if *why == proposal.why)
            })
        })
}

/// Ratify each of `listed` in turn with `one`, each with the sentence it
/// printed.
///
/// One ruling answers every survivor of an id, so a second proposal under
/// an id is ruled with the first — when it says the same; two proposals
/// under one id with different sentences refuse the pair, and nothing is
/// written (HQ review 4).
///
/// When `one` fails, it stops there, and the error says it truthfully: the
/// survivors ruled before it, whether the HQ's file already holds the ruling
/// on the one it was on (`holds`), and that nothing after it was touched
/// (HQ reviews 3 and 4).
pub fn ratify_in_turn(
    listed: &[crate::mutants::Proposal],
    mut one: impl FnMut(
        &crate::mutants::Proposal,
    ) -> Result<crate::equivalences::Registered, FindingsError>,
    holds: impl Fn(&crate::mutants::Proposal) -> bool,
) -> Result<Vec<Ratified>, FindingsError> {
    for (i, a) in listed.iter().enumerate() {
        if let Some(b) = listed[i + 1..]
            .iter()
            .find(|b| b.id == a.id && b.why != a.why)
        {
            return Err(FindingsError::TwoSentences(b.id.clone()));
        }
    }
    let mut done: Vec<Ratified> = Vec::new();
    for proposal in listed {
        if done.iter().any(|d| d.proposal.id == proposal.id) {
            continue;
        }
        match one(proposal) {
            Ok(registered) => done.push(Ratified {
                proposal: proposal.clone(),
                registered,
            }),
            Err(e) => {
                return Err(FindingsError::RatifiedPart {
                    at: proposal.id.clone(),
                    ruled: done.iter().map(|d| d.proposal.id.clone()).collect(),
                    at_written: holds(proposal),
                    why: e.to_string(),
                });
            }
        }
    }
    Ok(done)
}

/// `nunki mission mutants --equivalent`: [`crate::mutants::rule_equivalent`],
/// under the slot's lock, and the ruling entered in the project's registry
/// of equivalences. Returns what became of the registry.
pub fn rule_equivalent(
    project: &Project,
    id: &str,
    survivor: &str,
    why: &str,
) -> Result<crate::equivalences::Registered, FindingsError> {
    let by = who(project);
    under_slot_lock(
        project,
        id,
        "mission mutants --equivalent",
        |paths, tree| {
            crate::mutants::rule_equivalent(&paths.dir, survivor, why)?;
            Ok(crate::equivalences::enter(
                &registry_ruling(project, id, tree, &by),
                &campaign_of(paths)?,
                survivor,
            ))
        },
    )
}

/// `nunki mission mutants --lift`: the ruling taken out of the project's
/// registry of equivalences — whichever mission gave it — so that the next
/// mission is asked again, and then out of the mission's file
/// ([`crate::mutants::lift_equivalent`]), under the slot's lock. Returns how
/// many registry entries went.
///
/// **The registry first, and failing closed** (HQ review, item 4): when it
/// cannot be locked, read or written, the verb fails and the mission's file
/// is left as it was. The other order lifted the ruling from the mission and
/// left it in the registry, answering every later mission still, behind a
/// verb that had said "lifted". And a registry entry is taken out even when
/// the mission's file no longer holds the ruling — lifted there before, by a
/// verb whose registry half failed — so that it can always be cleaned.
pub fn lift_equivalent(
    project: &Project,
    id: &str,
    survivor: &str,
) -> Result<crate::equivalences::Registered, FindingsError> {
    let by = who(project);
    under_slot_lock(project, id, "mission mutants --lift", |paths, tree| {
        // No campaign: the mission's own verb says so, and writes nothing.
        // A campaign without that survivor gives `remove` nothing to take,
        // and the mission's verb names it below.
        let Some(campaign) = crate::mutants::read(&paths.dir)? else {
            crate::mutants::lift_equivalent(&paths.dir, survivor)?;
            return Ok(crate::equivalences::Registered::Done(0));
        };
        let removed = match crate::equivalences::remove(
            &registry_ruling(project, id, tree, &by),
            &campaign,
            survivor,
        ) {
            crate::equivalences::Registered::Done(n) => n,
            crate::equivalences::Registered::Not(why) => {
                return Err(FindingsError::RegistryUnchanged(why));
            }
        };
        let holds = campaign
            .survivors
            .iter()
            .any(|s| s.id == survivor && crate::mutants::liftable(s.outcome.as_ref()));
        // Nothing held here and nothing in the registry: the mission's verb
        // refuses, naming the survivor.
        if holds || removed == 0 {
            crate::mutants::lift_equivalent(&paths.dir, survivor)?;
        }
        Ok(crate::equivalences::Registered::Done(removed))
    })
}
