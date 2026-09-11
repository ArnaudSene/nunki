//! The lock on a real provider (SPEC 7): derived from hq's state, with a
//! short guard around the claim.

use std::path::PathBuf;

use hq::harness::{Outcome, RunHandle, SessionId};
use hq::mission::flow::{Event, Flow};
use hq::mission::{Bounds, Header, Integration, Lot, Security, Service};
use hq::provider::{Claim, blocked, claim, holder, shared};
use hq::state::{MissionState, Store};

fn header(services: &[(&str, bool)]) -> Header {
    Header {
        branch: "mission/x".into(),
        base: "dev".into(),
        lots: vec![Lot {
            id: "L1".into(),
            title: "one".into(),
        }],
        integration: Integration::Services {
            services: services
                .iter()
                .map(|(name, shared)| Service {
                    name: name.to_string(),
                    reach: vec![format!("{name}.example")],
                    shared: *shared,
                })
                .collect(),
            wiring: Vec::new(),
        },
        security: Security::Gates,
        arbiter: None,
        run: None,
        account: None,
        bounds: Bounds::default(),
    }
}

/// A mission standing on its integration stage, with a run recorded or not.
fn integrating(id: &str, header: Header, with_run: bool) -> MissionState {
    let mut flow = Flow::new(header).unwrap();
    flow.advance(Event::RunEnded {
        outcome: Outcome::Finished(Default::default()),
        lot_done: true,
    })
    .unwrap();
    flow.advance(Event::GatesPassed).unwrap();
    MissionState {
        id: id.into(),
        slot: format!("slot-{id}"),
        flow,
        run: with_run.then(|| RunHandle {
            session: SessionId(format!("s-{id}")),
            container: "cafe1234".into(),
            pid: Some(41),
            log: PathBuf::from("/dev/null"),
        }),
        app: None,
        verdicts: Vec::new(),
        accepted: Vec::new(),
        stopped: None,
        harness_down: None,
        spent: Default::default(),
        spared: None,
        coder_session: None,
        updated_at: String::new(),
    }
}

struct World {
    dir: tempfile::TempDir,
    store: Store,
}

impl World {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("locks")).unwrap();
        let store = Store::open(dir.path()).unwrap();
        Self { dir, store }
    }

    fn claim(&self, services: &[(&str, bool)]) -> Claim {
        claim(
            &self.store,
            self.dir.path(),
            "m1",
            &header(services),
            "verify",
        )
        .unwrap()
    }
}

#[test]
fn only_the_services_declared_shared_are_providers() {
    assert_eq!(
        shared(&header(&[("stripe", true), ("db", false)])),
        vec!["stripe"]
    );
    let mut none = header(&[]);
    none.integration = Integration::None {
        reason: "nothing external".into(),
    };
    assert!(shared(&none).is_empty());
}

/// Held while another mission's integration run is recorded — never by the
/// asking mission itself, never for a provider that mission does not share.
#[test]
fn a_provider_is_held_by_another_missions_integration_run() {
    let world = World::new();
    world
        .store
        .save(&integrating(
            "m2",
            header(&[("stripe", true), ("db", false)]),
            true,
        ))
        .unwrap();

    assert_eq!(
        holder(&world.store, "m1", "stripe").unwrap().as_deref(),
        Some("m2")
    );
    assert_eq!(holder(&world.store, "m2", "stripe").unwrap(), None);
    assert_eq!(holder(&world.store, "m1", "db").unwrap(), None);
    assert_eq!(
        blocked(
            &world.store,
            "m1",
            &header(&[("db", false), ("stripe", true)])
        )
        .unwrap(),
        Some(("stripe".to_string(), "m2".to_string()))
    );
}

/// The lock is the recorded run: read back, or on another stage, the other
/// mission holds nothing — there is no file to forget.
#[test]
fn a_run_read_back_or_another_stage_holds_nothing() {
    let world = World::new();
    world
        .store
        .save(&integrating("m2", header(&[("stripe", true)]), false))
        .unwrap();
    assert_eq!(holder(&world.store, "m1", "stripe").unwrap(), None);

    let mut coding = integrating("m2", header(&[("stripe", true)]), true);
    coding.flow = Flow::new(header(&[("stripe", true)])).unwrap();
    world.store.save(&coding).unwrap();
    assert_eq!(holder(&world.store, "m1", "stripe").unwrap(), None);
}

/// Free when nothing holds it, busy when a mission does — and busy too while
/// another `hq` is claiming it this very moment.
#[test]
fn a_claim_is_refused_while_a_mission_or_another_claim_holds_the_provider() {
    let world = World::new();
    world
        .store
        .save(&integrating("m2", header(&[("stripe", true)]), true))
        .unwrap();
    match world.claim(&[("stripe", true)]) {
        Claim::Busy { provider, by } => {
            assert_eq!(provider, "stripe");
            assert_eq!(by, "mission m2");
        }
        other => panic!("{other:?}"),
    }

    world
        .store
        .save(&integrating("m2", header(&[("stripe", true)]), false))
        .unwrap();
    let guards = match world.claim(&[("stripe", true)]) {
        Claim::Free(guards) => guards,
        other => panic!("{other:?}"),
    };
    assert_eq!(guards.len(), 1);
    match world.claim(&[("stripe", true)]) {
        Claim::Busy { by, .. } => assert!(by.contains("claiming it now"), "{by}"),
        other => panic!("{other:?}"),
    }
    drop(guards);
    assert!(matches!(world.claim(&[("stripe", true)]), Claim::Free(_)));
}

/// Nothing shared, nothing claimed; and a provider's name is the human's to
/// choose, so one that is no file name still claims.
#[test]
fn only_shared_providers_are_claimed_whatever_their_name() {
    let world = World::new();
    match world.claim(&[("db", false)]) {
        Claim::Free(guards) => assert!(guards.is_empty()),
        other => panic!("{other:?}"),
    }
    match world.claim(&[("stripe/test tier", true)]) {
        Claim::Free(guards) => assert_eq!(guards.len(), 1),
        other => panic!("{other:?}"),
    }
}
