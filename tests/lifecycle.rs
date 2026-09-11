//! Re-framing a mission, and closing one (SPEC 4.1 rule 3, 4.2 verb table).

use std::path::PathBuf;

use hq::lifecycle::{self, LifecycleError};
use hq::mission::flow::{Event, Flow, Stage, Work};
use hq::mission::{Bounds, Header, Integration, Lot, Security, Service};
use hq::project::{Config, Project, ProtectedPaths};
use hq::state::{MissionState, Store};

fn header(lots: usize) -> Header {
    Header {
        branch: "mission/x".into(),
        base: "dev".into(),
        lots: (1..=lots)
            .map(|n| Lot {
                id: format!("L{n}"),
                title: format!("lot {n}"),
            })
            .collect(),
        integration: Integration::None {
            reason: "none".into(),
        },
        security: Security::Gates,
        arbiter: None,
        run: None,
        account: None,
        bounds: Bounds::default(),
    }
}

struct World {
    _dir: tempfile::TempDir,
    project: Project,
}

impl World {
    fn new(lots: usize) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let hq_root = dir.path().join("hq");
        for d in ["locks", "missions", "state/missions"] {
            std::fs::create_dir_all(hq_root.join(d)).unwrap();
        }
        let project = Project::at(
            dir.path().join("repo"),
            Config {
                harness: "claude-code".into(),
                forge: vec![],
                stacks: vec!["rust".into()],
                protected_branches: vec!["main".into()],
                protected_paths: ProtectedPaths::default(),
                account: None,
                bounds: Default::default(),
                credentials: None,
                run: None,
                services_file: None,
                forge_protection: Default::default(),
            },
            hq_root.clone(),
        );
        hq::mission::dir::create(&hq_root, "m1", &header(lots), "do it").unwrap();
        Store::open(&hq_root)
            .unwrap()
            .save(&MissionState {
                id: "m1".into(),
                slot: "one".into(),
                flow: Flow::new(header(lots)).unwrap(),
                run: None,
                app: None,
                verdicts: Vec::new(),
                accepted: Vec::new(),
                stopped: None,
                harness_down: None,
                spent: Default::default(),
                spared: None,
                coder_session: None,
                updated_at: "2026-09-10T00:00:00Z".into(),
            })
            .unwrap();
        Self { _dir: dir, project }
    }

    /// Rewrite `MISSION.md` with a different framing, the way a human edits it.
    fn reframe_file(&self, header: &Header) {
        hq::mission::dir::create(&self.project.hq_root, "m2", header, "x").unwrap();
        let from = self.project.hq_root.join("missions/m2/MISSION.md");
        let to = self.project.hq_root.join("missions/m1/MISSION.md");
        let fresh = std::fs::read_to_string(&from).unwrap();
        // Keep the human's prose, replace the framing: that is what editing
        // the file means.
        std::fs::write(to, fresh).unwrap();
        std::fs::remove_dir_all(self.project.hq_root.join("missions/m2")).unwrap();
    }

    fn state(&self) -> MissionState {
        Store::open(&self.project.hq_root)
            .unwrap()
            .load("m1")
            .unwrap()
    }

    fn running(&self) {
        let store = Store::open(&self.project.hq_root).unwrap();
        let mut state = store.load("m1").unwrap();
        state.run = Some(hq::harness::RunHandle {
            session: hq::harness::SessionId("s1".into()),
            container: "cafe".into(),
            pid: Some(41),
            log: PathBuf::from("/dev/null"),
        });
        store.save(&state).unwrap();
    }

    fn at(&self, events: &[Event]) {
        let store = Store::open(&self.project.hq_root).unwrap();
        let mut state = store.load("m1").unwrap();
        for event in events {
            store.apply(&mut state, event.clone()).unwrap();
        }
    }
}

/// Editing `MISSION.md` changes nothing until the verb says so: `hq` never
/// re-reads the header during a mission. The verb puts the framing back in
/// front of the human first — it says what would change and does nothing.
#[test]
fn reframing_says_what_would_change_and_freezes_nothing_until_told() {
    let world = World::new(2);
    let mut fresh = header(2);
    fresh.security = Security::Agent;
    fresh.integration = Integration::Services {
        services: vec![Service {
            name: "db".into(),
            reach: vec!["db".into()],
            shared: false,
        }],
        wiring: vec!["compose.yaml".into()],
    };
    world.reframe_file(&fresh);

    let seen = lifecycle::reframe(&world.project, "m1", false).unwrap();
    assert!(!seen.applied);
    let what: Vec<&str> = seen.changes.iter().map(|c| c.what).collect();
    assert_eq!(what, vec!["integration", "security"], "{:?}", seen.changes);
    // And the frozen header has not moved.
    assert!(!world.state().flow.header().has_security_agent());

    let done = lifecycle::reframe(&world.project, "m1", true).unwrap();
    assert!(done.applied);
    assert!(world.state().flow.header().has_security_agent());
    assert!(world.state().flow.header().has_integration());
}

/// A framing that says the same thing is not a change, and saying so is
/// better than a diff of a file that was reformatted.
#[test]
fn an_unchanged_framing_is_reported_as_unchanged() {
    let world = World::new(2);
    let seen = lifecycle::reframe(&world.project, "m1", true).unwrap();
    assert!(seen.changes.is_empty(), "{:?}", seen.changes);
    assert!(!seen.applied);
}

/// Reframing under a running agent would change the perimeter the agent is
/// inside. Refused, and the message names the verb that ends its turn.
#[test]
fn reframing_under_a_running_agent_is_refused() {
    let world = World::new(2);
    world.running();
    let err = lifecycle::reframe(&world.project, "m1", true).unwrap_err();
    assert!(matches!(err, LifecycleError::RunInProgress { .. }), "{err}");
    assert!(err.to_string().contains("hq mission stop"), "{err}");
}

/// The flow may be standing on a lot the new framing does not have.
/// Re-numbering under it would leave the state pointing at work nobody
/// described, and there is no honest guess about which lot was meant.
#[test]
fn a_framing_that_removes_the_lot_being_worked_on_is_refused() {
    let world = World::new(3);
    // Finish the first lot, so the flow stands on the second.
    world.at(&[Event::RunEnded {
        outcome: hq::harness::Outcome::Finished(Default::default()),
        lot_done: true,
    }]);
    assert!(matches!(
        world.state().flow.stage(),
        Stage::Coding {
            work: Work::Lot(1),
            ..
        }
    ));

    world.reframe_file(&header(1));
    let err = lifecycle::reframe(&world.project, "m1", true).unwrap_err();
    assert!(
        matches!(
            err,
            LifecycleError::Flow(hq::mission::flow::FlowError::LotGone { .. })
        ),
        "{err}"
    );
    assert!(err.to_string().contains("L2"), "{err}");
    // Refused means refused: the frozen header still has three lots.
    assert_eq!(world.state().flow.header().lots.len(), 3);
}

/// Archiving moves; it never deletes. The journals, the pull request text and
/// the verdicts are the record of what was done.
#[test]
fn archiving_moves_a_finished_mission_and_keeps_everything_in_it() {
    let world = World::new(1);
    world.at(&[
        Event::RunEnded {
            outcome: hq::harness::Outcome::Finished(Default::default()),
            lot_done: true,
        },
        Event::GatesPassed,
    ]);
    assert!(matches!(world.state().flow.stage(), Stage::Verified));
    std::fs::write(
        world.project.hq_root.join("missions/m1/JOURNAL.md"),
        "what was done\n",
    )
    .unwrap();

    let archived = lifecycle::archive(&world.project, "m1").unwrap();
    assert_eq!(
        archived.at,
        lifecycle::archive_dir(&world.project).join("m1")
    );
    assert!(!world.project.hq_root.join("missions/m1").exists());
    assert_eq!(
        std::fs::read_to_string(archived.at.join("JOURNAL.md")).unwrap(),
        "what was done\n"
    );
    // The state goes with the folder, in the folder: one thing to keep or to
    // move, not two that have to be found again.
    assert!(archived.at.join("state.json").is_file());
    assert!(
        !world
            .project
            .hq_root
            .join("state/missions/m1.json")
            .exists()
    );
}

/// A mission still being worked on is not archived: that would hide it rather
/// than close it.
#[test]
fn a_mission_still_being_worked_on_is_not_archived() {
    let world = World::new(1);
    let err = lifecycle::archive(&world.project, "m1").unwrap_err();
    assert!(matches!(err, LifecycleError::NotFinished { .. }), "{err}");
    assert!(err.to_string().contains("Coding"), "{err}");
    assert!(world.project.hq_root.join("missions/m1").is_dir());
}

/// A mission handed back to the human and left there is finished too — it
/// ended, badly, and hiding it behind "not verified" would leave the HQ with
/// missions nobody can close.
#[test]
fn a_mission_handed_back_to_the_human_can_be_archived() {
    let world = World::new(1);
    let store = Store::open(&world.project.hq_root).unwrap();
    let mut state = store.load("m1").unwrap();
    for _ in 0..3 {
        store
            .apply(
                &mut state,
                Event::Stalled {
                    reason: "no progress".into(),
                },
            )
            .unwrap();
    }
    assert!(matches!(state.flow.stage(), Stage::AwaitingHuman(_)));
    lifecycle::archive(&world.project, "m1").unwrap();
}

/// Archiving twice is a mistake worth naming rather than a second folder.
#[test]
fn archiving_twice_says_so() {
    let world = World::new(1);
    world.at(&[
        Event::RunEnded {
            outcome: hq::harness::Outcome::Finished(Default::default()),
            lot_done: true,
        },
        Event::GatesPassed,
    ]);
    lifecycle::archive(&world.project, "m1").unwrap();

    hq::mission::dir::create(&world.project.hq_root, "m1", &header(1), "again").unwrap();
    Store::open(&world.project.hq_root)
        .unwrap()
        .save(&MissionState {
            id: "m1".into(),
            slot: "one".into(),
            flow: Flow::new(header(1)).unwrap(),
            run: None,
            app: None,
            verdicts: Vec::new(),
            accepted: Vec::new(),
            stopped: None,
            harness_down: None,
            spent: Default::default(),
            spared: None,
            coder_session: None,
            updated_at: String::new(),
        })
        .unwrap();
    world.at(&[
        Event::RunEnded {
            outcome: hq::harness::Outcome::Finished(Default::default()),
            lot_done: true,
        },
        Event::GatesPassed,
    ]);

    let err = lifecycle::archive(&world.project, "m1").unwrap_err();
    assert!(matches!(err, LifecycleError::AlreadyArchived(_)), "{err}");
}

/// Every decision in the header is compared, and the comparison is field by
/// field rather than a diff of the serialised text: what matters is which
/// **decision** moved, and a text diff would report a reordered list as a
/// change and a renumbered lot as two.
#[test]
fn every_decision_in_the_framing_is_compared() {
    let world = World::new(2);
    let mut fresh = header(2);
    fresh.branch = "mission/y".into();
    fresh.base = "main".into();
    fresh.lots[1].title = "something else entirely".into();
    fresh.security = Security::Agent;
    fresh.arbiter = Some("Igor".into());
    fresh.account = Some("pro".into());
    fresh.run = Some("none".into());
    fresh.bounds.max_volets = 7;
    world.reframe_file(&fresh);

    let seen = lifecycle::reframe(&world.project, "m1", false).unwrap();
    let what: Vec<&str> = seen.changes.iter().map(|c| c.what).collect();
    assert_eq!(
        what,
        vec![
            "branch", "base", "lots", "security", "arbiter", "account", "run", "bounds"
        ],
        "{:?}",
        seen.changes
    );
    // And each says what it was and what it becomes, so the reader decides on
    // the change rather than on the fact that there is one.
    let lots = seen.changes.iter().find(|c| c.what == "lots").unwrap();
    assert!(lots.from.contains("lot 2"), "{lots:?}");
    assert!(lots.to.contains("something else entirely"), "{lots:?}");
}

/// `end` is the one verb in SPEC's list that the list never explains, and the
/// definition here is derived rather than quoted: `stop` ends a run,
/// `archive` closes a finished mission, and between them sat a mission a
/// human has given up on — still `Coding`, never to be verified, impossible
/// to close. `end` closes it, and after it `archive` will.
#[test]
fn ending_a_mission_makes_it_closable() {
    let world = World::new(2);
    assert!(matches!(world.state().flow.stage(), Stage::Coding { .. }));
    // Before: it cannot be archived, because it is still being worked on.
    assert!(matches!(
        lifecycle::archive(&world.project, "m1").unwrap_err(),
        LifecycleError::NotFinished { .. }
    ));

    let state = lifecycle::end(&world.project, "m1", "the approach was wrong").unwrap();
    assert!(
        matches!(
            state.flow.stage(),
            Stage::AwaitingHuman(hq::mission::flow::Handover::Abandoned { reason })
                if reason == "the approach was wrong"
        ),
        "{:?}",
        state.flow.stage()
    );

    // The reason is where a human reads it, not only in the state.
    let followup =
        std::fs::read_to_string(world.project.hq_root.join("missions/m1/FOLLOWUP_HQ.md")).unwrap();
    assert!(followup.contains("the approach was wrong"), "{followup}");

    lifecycle::archive(&world.project, "m1").unwrap();
}

/// A mission called off without a reason is a puzzle for whoever finds it six
/// months from now.
#[test]
fn ending_a_mission_without_a_reason_is_refused() {
    let world = World::new(1);
    let err = lifecycle::end(&world.project, "m1", " \n\t").unwrap_err();
    assert!(matches!(err, LifecycleError::NoReason), "{err}");
    assert!(matches!(world.state().flow.stage(), Stage::Coding { .. }));
}

/// "Call it off" is not a thing to say twice, and a mission that is verified
/// is not one to call off at all — it is one to push.
#[test]
fn a_mission_that_is_already_over_is_not_ended_again() {
    let world = World::new(1);
    world.at(&[
        Event::RunEnded {
            outcome: hq::harness::Outcome::Finished(Default::default()),
            lot_done: true,
        },
        Event::GatesPassed,
    ]);
    assert!(matches!(world.state().flow.stage(), Stage::Verified));

    let err = lifecycle::end(&world.project, "m1", "changed my mind").unwrap_err();
    assert!(
        matches!(
            err,
            LifecycleError::State(hq::state::StateError::Flow(
                hq::mission::flow::FlowError::AlreadyOver
            ))
        ),
        "{err}"
    );
    assert!(err.to_string().contains("hq mission archive"), "{err}");
    assert!(matches!(world.state().flow.stage(), Stage::Verified));
}

/// It works from anywhere a mission can still be worked on. A verb that
/// worked in five stages out of seven is one the human cannot rely on when
/// they want out.
#[test]
fn ending_works_wherever_a_mission_can_still_be_worked_on() {
    for reach in [
        vec![],
        vec![Event::RunEnded {
            outcome: hq::harness::Outcome::Finished(Default::default()),
            lot_done: true,
        }],
    ] {
        let world = World::new(2);
        world.at(&reach);
        lifecycle::end(&world.project, "m1", "not worth finishing").unwrap();
        assert!(matches!(
            world.state().flow.stage(),
            Stage::AwaitingHuman(hq::mission::flow::Handover::Abandoned { .. })
        ));
    }
}

/// And not under a running agent: the agent is still writing, and a mission
/// declared over while its run is going is a tree nobody chose the moment of.
#[test]
fn ending_under_a_running_agent_is_refused() {
    let world = World::new(1);
    world.running();
    let err = lifecycle::end(&world.project, "m1", "enough").unwrap_err();
    assert!(matches!(err, LifecycleError::RunInProgress { .. }), "{err}");
}
