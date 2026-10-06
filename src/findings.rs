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
    let Ok(mut state) = store.load(id) else {
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
/// writing its files: the ruling runs as it is.
fn under_slot_lock<T>(
    project: &Project,
    id: &str,
    verb: &str,
    ruling: impl FnOnce(&Paths) -> Result<T, crate::mutants::MutantsError>,
) -> Result<T, FindingsError> {
    let paths = Paths::of(&project.hq_root, id);
    let store = Store::open(&project.hq_root)?;
    let _lock = match store.load(id) {
        Ok(state) => Some(SlotLock::acquire(
            &project.hq_root.join("locks"),
            &state.slot,
            verb,
        )?),
        Err(_) => None,
    };
    Ok(ruling(&paths)?)
}

/// `nunki mission mutants --ratify`: [`crate::mutants::ratify`], under the
/// slot's lock. Returns the reason the ruling was written with.
pub fn ratify_proposal(
    project: &Project,
    id: &str,
    survivor: &str,
    because: Option<&str>,
) -> Result<String, FindingsError> {
    under_slot_lock(project, id, "mission mutants --ratify", |paths| {
        crate::mutants::ratify(&paths.dir, survivor, because)
    })
}

/// `nunki mission mutants --equivalent`: [`crate::mutants::rule_equivalent`],
/// under the slot's lock.
pub fn rule_equivalent(
    project: &Project,
    id: &str,
    survivor: &str,
    why: &str,
) -> Result<(), FindingsError> {
    under_slot_lock(project, id, "mission mutants --equivalent", |paths| {
        crate::mutants::rule_equivalent(&paths.dir, survivor, why)
    })
}

/// `nunki mission mutants --lift`: [`crate::mutants::lift_equivalent`],
/// under the slot's lock.
pub fn lift_equivalent(project: &Project, id: &str, survivor: &str) -> Result<(), FindingsError> {
    under_slot_lock(project, id, "mission mutants --lift", |paths| {
        crate::mutants::lift_equivalent(&paths.dir, survivor)
    })
}
