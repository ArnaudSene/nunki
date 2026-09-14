//! The flow of SPEC 4.5, exercised without a container or a model.

use nunki::harness::{Outcome, Role, Usage};
use nunki::mission::flow::{Event, Flow, Handover, Stage, Work};
use nunki::mission::{Bounds, Header, Integration, Lot, Security, Service, Verdict};

fn lots(n: usize) -> Vec<Lot> {
    (1..=n)
        .map(|i| Lot {
            id: format!("L{i}"),
            title: format!("lot {i}"),
        })
        .collect()
}

fn header(integration: Integration, security: Security, bounds: Bounds) -> Header {
    Header {
        branch: "feat/x".into(),
        base: "dev".into(),
        lots: lots(2),
        integration,
        security,
        arbiter: None,
        run: None,
        account: None,
        model: None,
        bounds,
    }
}

fn none() -> Integration {
    Integration::None {
        reason: "pure domain".into(),
    }
}

fn services() -> Integration {
    Integration::Services {
        wiring: Vec::new(),
        services: vec![Service {
            name: "postgres".into(),
            reach: vec!["db:5432".into()],
            shared: false,
        }],
    }
}

fn finished(lot_done: bool) -> Event {
    Event::RunEnded {
        outcome: Outcome::Finished(Usage::default()),
        lot_done,
    }
}

fn verdict(v: Verdict) -> Event {
    Event::Verdict {
        verdict: v,
        report: format!("{v:?}"),
    }
}

/// Drive the coder through its planned lots and the final gates.
fn code_through(flow: &mut Flow) {
    let n = flow.header().lots.len();
    for _ in 0..n {
        flow.advance(finished(true)).unwrap();
    }
    assert_eq!(flow.stage(), &Stage::Gates);
}

#[test]
fn code_only_ends_verified_after_the_gates() {
    let mut flow = Flow::new(header(none(), Security::Gates, Bounds::default())).unwrap();
    assert_eq!(
        flow.stage(),
        &Stage::Coding {
            work: Work::Lot(0),
            attempt: 1
        }
    );
    code_through(&mut flow);
    flow.advance(Event::GatesPassed).unwrap();
    assert_eq!(flow.stage(), &Stage::Verified);
    assert_eq!(flow.volets(), 0);
}

#[test]
fn code_and_security_skips_the_integrator() {
    let mut flow = Flow::new(header(none(), Security::Agent, Bounds::default())).unwrap();
    code_through(&mut flow);
    flow.advance(Event::GatesPassed).unwrap();
    assert_eq!(flow.stage(), &Stage::SecurityAgent { attempt: 1 });
    flow.advance(verdict(Verdict::Clear)).unwrap();
    assert_eq!(flow.stage(), &Stage::Verified);
}

#[test]
fn full_shape_iterates_and_replays_integration_after_findings() {
    let mut flow = Flow::new(header(services(), Security::Agent, Bounds::default())).unwrap();
    code_through(&mut flow);
    flow.advance(Event::GatesPassed).unwrap();
    assert_eq!(flow.stage(), &Stage::Integration { attempt: 1 });

    // BROKEN: back to the coder as volet 1, then gates, then integration again.
    flow.advance(verdict(Verdict::Broken)).unwrap();
    assert!(matches!(
        flow.stage(),
        Stage::Coding {
            work: Work::Volet { n: 1, .. },
            attempt: 1
        }
    ));
    flow.advance(finished(true)).unwrap();
    assert_eq!(flow.stage(), &Stage::Gates);
    flow.advance(Event::GatesPassed).unwrap();
    assert_eq!(flow.stage(), &Stage::Integration { attempt: 1 });

    // INTEGRATED, then FINDINGS: the HQ iterates, and the integrator REPLAYS
    // before security because the code changed.
    flow.advance(verdict(Verdict::Integrated)).unwrap();
    assert_eq!(flow.stage(), &Stage::SecurityAgent { attempt: 1 });
    flow.advance(verdict(Verdict::Findings)).unwrap();
    assert!(matches!(flow.stage(), Stage::Findings { .. }));
    flow.advance(Event::Iterate).unwrap();
    assert!(matches!(
        flow.stage(),
        Stage::Coding {
            work: Work::Volet { n: 2, .. },
            ..
        }
    ));
    flow.advance(finished(true)).unwrap();
    flow.advance(Event::GatesPassed).unwrap();
    assert_eq!(flow.stage(), &Stage::Integration { attempt: 1 });
    flow.advance(verdict(Verdict::Integrated)).unwrap();
    flow.advance(verdict(Verdict::Clear)).unwrap();
    assert_eq!(flow.stage(), &Stage::Verified);
    assert_eq!(flow.volets(), 2);
}

#[test]
fn the_human_can_lift_findings_instead_of_iterating() {
    let mut flow = Flow::new(header(none(), Security::Agent, Bounds::default())).unwrap();
    code_through(&mut flow);
    flow.advance(Event::GatesPassed).unwrap();
    flow.advance(verdict(Verdict::Findings)).unwrap();
    flow.advance(Event::HumanAccepted).unwrap();
    assert_eq!(flow.stage(), &Stage::Verified);
}

#[test]
fn volets_are_bounded_and_the_causes_are_handed_over() {
    let bounds = Bounds {
        max_volets: 3,
        ..Bounds::default()
    };
    let mut flow = Flow::new(header(services(), Security::Gates, bounds)).unwrap();
    code_through(&mut flow);
    flow.advance(Event::GatesPassed).unwrap();
    for n in 1..=3 {
        flow.advance(verdict(Verdict::Broken)).unwrap();
        assert!(
            matches!(flow.stage(), Stage::Coding { work: Work::Volet { n: got, .. }, .. } if *got == n)
        );
        flow.advance(finished(true)).unwrap();
        flow.advance(Event::GatesPassed).unwrap();
    }
    flow.advance(verdict(Verdict::Broken)).unwrap();
    match flow.stage() {
        Stage::AwaitingHuman(Handover::VoletsExhausted { causes }) => assert_eq!(causes.len(), 4),
        other => panic!("expected handover, got {other:?}"),
    }
}

#[test]
fn zero_volets_means_the_first_red_goes_to_the_human() {
    let bounds = Bounds {
        max_volets: 0,
        ..Bounds::default()
    };
    let mut flow = Flow::new(header(services(), Security::Gates, bounds)).unwrap();
    code_through(&mut flow);
    flow.advance(Event::GatesPassed).unwrap();
    flow.advance(verdict(Verdict::Broken)).unwrap();
    assert!(matches!(
        flow.stage(),
        Stage::AwaitingHuman(Handover::VoletsExhausted { .. })
    ));
}

#[test]
fn a_harness_failure_replays_the_same_attempt() {
    let mut flow = Flow::new(header(none(), Security::Gates, Bounds::default())).unwrap();
    for _ in 0..10 {
        flow.advance(Event::RunEnded {
            outcome: Outcome::HarnessFailure(nunki::harness::Fault::transient("rate limit")),
            lot_done: false,
        })
        .unwrap();
    }
    assert_eq!(
        flow.stage(),
        &Stage::Coding {
            work: Work::Lot(0),
            attempt: 1
        }
    );
}

#[test]
fn stalls_and_mission_failures_consume_attempts_up_to_the_bound() {
    let bounds = Bounds {
        attempts_per_lot: 3,
        ..Bounds::default()
    };
    let mut flow = Flow::new(header(none(), Security::Gates, bounds)).unwrap();
    flow.advance(Event::Stalled {
        reason: "no change over 3 checks".into(),
    })
    .unwrap();
    assert_eq!(
        flow.stage(),
        &Stage::Coding {
            work: Work::Lot(0),
            attempt: 2
        }
    );
    flow.advance(Event::RunEnded {
        outcome: Outcome::MissionFailure("run contract not honoured".into()),
        lot_done: false,
    })
    .unwrap();
    assert_eq!(
        flow.stage(),
        &Stage::Coding {
            work: Work::Lot(0),
            attempt: 3
        }
    );
    flow.advance(finished(false)).unwrap();
    assert_eq!(
        flow.stage(),
        &Stage::AwaitingHuman(Handover::LotAttemptsExhausted {
            lot: "L1".into(),
            attempts: 3
        })
    );
}

#[test]
fn a_failed_gate_sends_the_coder_a_fix_run_without_counting_a_volet() {
    let mut flow = Flow::new(header(none(), Security::Gates, Bounds::default())).unwrap();
    code_through(&mut flow);
    flow.advance(Event::GatesFailed {
        reason: "2 surviving mutants".into(),
    })
    .unwrap();
    assert!(matches!(
        flow.stage(),
        Stage::Coding {
            work: Work::Volet { .. },
            attempt: 1
        }
    ));
    assert_eq!(flow.volets(), 0);
}

#[test]
fn an_event_out_of_place_is_an_error_not_a_silent_no_op() {
    let mut flow = Flow::new(header(none(), Security::Gates, Bounds::default())).unwrap();
    let err = flow.advance(verdict(Verdict::Clear)).unwrap_err();
    assert!(matches!(
        err,
        nunki::mission::flow::FlowError::InvalidTransition { .. }
    ));
}

#[test]
fn a_role_that_keeps_failing_its_own_run_is_handed_over() {
    let bounds = Bounds {
        attempts_per_lot: 2,
        ..Bounds::default()
    };
    let mut flow = Flow::new(header(services(), Security::Gates, bounds)).unwrap();
    code_through(&mut flow);
    flow.advance(Event::GatesPassed).unwrap();
    flow.advance(Event::Stalled {
        reason: "stuck".into(),
    })
    .unwrap();
    assert_eq!(flow.stage(), &Stage::Integration { attempt: 2 });
    flow.advance(Event::Stalled {
        reason: "stuck again".into(),
    })
    .unwrap();
    assert_eq!(
        flow.stage(),
        &Stage::AwaitingHuman(Handover::RoleAttemptsExhausted {
            role: Role::Integrator,
            attempts: 2
        })
    );
}

#[test]
fn the_flow_survives_a_round_trip_through_json() {
    let mut flow = Flow::new(header(services(), Security::Agent, Bounds::default())).unwrap();
    code_through(&mut flow);
    let json = serde_json::to_string(&flow).unwrap();
    let back: Flow = serde_json::from_str(&json).unwrap();
    assert_eq!(back, flow);
}
