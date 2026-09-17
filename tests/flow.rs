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

/// A red gate at the final verification sends the coder back, and **that is
/// a return to the coder**: it counts, and the count is what ends a mission
/// the coder cannot fix.
///
/// This test asserted the opposite until 2026-09-17 — it was named
/// `a_failed_gate_sends_the_coder_a_fix_run_without_counting_a_volet` and
/// required `volets() == 0` — and it was wrong. SPEC 7 says the loop is
/// bounded and what the bound counts: "au troisième retour au codeur sur une
/// même mission, le HQ ne relance pas". Any return, not only the one a red
/// verdict opens.
///
/// What the exemption cost, measured on `notes-3` on 2026-09-16: gate 7 red
/// on survivors no test could kill, the agent declaring its volet done, the
/// gates played again, the same gate red again, the same volet 0 opened
/// again — **twenty-eight times in half an hour**, 34 million tokens, and
/// nothing in the flow that could have stopped it.
#[test]
fn a_failed_gate_opens_a_volet_that_counts_and_the_third_hands_over() {
    let mut flow = Flow::new(header(none(), Security::Gates, Bounds::default())).unwrap();
    code_through(&mut flow);

    for n in 1..=3 {
        flow.advance(Event::GatesFailed {
            reason: format!("2 surviving mutants, run {n}"),
        })
        .unwrap();
        assert!(
            matches!(
                flow.stage(),
                Stage::Coding {
                    work: Work::Volet { n: got, .. },
                    attempt: 1
                } if *got == n
            ),
            "{:?}",
            flow.stage()
        );
        assert_eq!(flow.volets(), n);
        // The coder does its run and says the volet is done; the flow plays
        // the gates again, which is where the loop used to close.
        flow.advance(finished(true)).unwrap();
        assert_eq!(flow.stage(), &Stage::Gates);
    }

    // The fourth time, nothing is relaunched: three returns are the bound,
    // and the three causes go to the human side by side.
    flow.advance(Event::GatesFailed {
        reason: "2 surviving mutants, run 4".into(),
    })
    .unwrap();
    match flow.stage() {
        Stage::AwaitingHuman(Handover::VoletsExhausted { causes }) => {
            assert_eq!(causes.len(), 4, "{causes:?}");
            assert!(causes.iter().all(|c| c.starts_with("gate: ")), "{causes:?}");
        }
        other => panic!("{other:?}"),
    }
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

/// The whole point of the verb, and the reason it had to exist: before it,
/// `AwaitingHuman` was terminal. `#30` made that reachable — a red gate now
/// opens a volet that counts, and the third hands over — so the only way the
/// bounds could stop a mission was into a stage nothing could leave.
///
/// A retry hands the bounds back **whole**. Resetting the counter is the
/// verb: without it the very next red gate lands in the same handover, and
/// the human is back where they started having spent a run to learn it.
#[test]
fn a_mission_whose_volets_ran_out_is_taken_back_with_its_budget_whole() {
    let mut flow = Flow::new(header(none(), Security::Gates, Bounds::default())).unwrap();
    code_through(&mut flow);

    for n in 1..=4 {
        flow.advance(Event::GatesFailed {
            reason: format!("run {n}"),
        })
        .unwrap();
        if n < 4 {
            flow.advance(finished(true)).unwrap();
        }
    }
    assert!(
        matches!(
            flow.stage(),
            Stage::AwaitingHuman(Handover::VoletsExhausted { .. })
        ),
        "{:?}",
        flow.stage()
    );

    flow.advance(Event::Retried {
        because: "the campaign was reading a stale log; it is fixed".into(),
    })
    .unwrap();

    // On the cause nobody ever got to work on — the fourth, which arrived
    // with no budget left to open a volet for it.
    match flow.stage() {
        Stage::Coding {
            work: Work::Volet { n, cause },
            attempt: 1,
        } => {
            assert_eq!(*n, 1, "the budget was not handed back whole");
            assert_eq!(cause, "gate: run 4", "not the cause nobody worked on");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(flow.volets(), 1);

    // And the budget really is whole: three more returns before it stops
    // again, not zero.
    for n in 1..=3 {
        flow.advance(finished(true)).unwrap();
        assert_eq!(flow.stage(), &Stage::Gates);
        flow.advance(Event::GatesFailed {
            reason: format!("after the retry, {n}"),
        })
        .unwrap();
    }
    assert!(
        matches!(
            flow.stage(),
            Stage::AwaitingHuman(Handover::VoletsExhausted { .. })
        ),
        "{:?}",
        flow.stage()
    );
}

/// A lot that used its attempts comes back on **that** lot. The handover
/// carries a label, and `Flow::label` is not invertible — it folds a volet's
/// cause away, and nothing stops a human naming a lot `volet-2`. So the work
/// is kept whole from the moment the flow hands over, and this is what says
/// the label is not what gets resumed.
#[test]
fn a_lot_that_used_its_attempts_comes_back_on_that_lot() {
    let bounds = Bounds {
        attempts_per_lot: 2,
        ..Bounds::default()
    };
    let mut flow = Flow::new(header(none(), Security::Gates, bounds)).unwrap();
    // First lot done, so the mission is on the second when it stops.
    flow.advance(finished(true)).unwrap();
    for _ in 0..2 {
        flow.advance(finished(false)).unwrap();
    }
    match flow.stage() {
        Stage::AwaitingHuman(Handover::LotAttemptsExhausted { lot, attempts }) => {
            assert_eq!(lot, "L2");
            assert_eq!(*attempts, 2);
        }
        other => panic!("{other:?}"),
    }

    flow.advance(Event::Retried {
        because: "the fixture it needed is in place now".into(),
    })
    .unwrap();

    assert_eq!(
        flow.stage(),
        &Stage::Coding {
            work: Work::Lot(1),
            attempt: 1
        }
    );
    // Whole, not one left: it fails twice again before handing over.
    flow.advance(finished(false)).unwrap();
    assert!(
        matches!(flow.stage(), Stage::Coding { attempt: 2, .. }),
        "{:?}",
        flow.stage()
    );
}

/// A role's attempts are read straight from the handover: `Role` is typed, so
/// there is no label to invert.
#[test]
fn a_role_that_used_its_attempts_comes_back_on_that_role() {
    let bounds = Bounds {
        attempts_per_lot: 2,
        ..Bounds::default()
    };
    let mut flow = Flow::new(header(services(), Security::Gates, bounds)).unwrap();
    code_through(&mut flow);
    flow.advance(Event::GatesPassed).unwrap();
    assert_eq!(flow.stage(), &Stage::Integration { attempt: 1 });
    for _ in 0..2 {
        flow.advance(Event::Stalled {
            reason: "the container went away".into(),
        })
        .unwrap();
    }
    assert!(
        matches!(
            flow.stage(),
            Stage::AwaitingHuman(Handover::RoleAttemptsExhausted {
                role: Role::Integrator,
                ..
            })
        ),
        "{:?}",
        flow.stage()
    );

    flow.advance(Event::Retried {
        because: "the engine was out of disk; it has room now".into(),
    })
    .unwrap();

    assert_eq!(flow.stage(), &Stage::Integration { attempt: 1 });
}

/// A mission the human called off is not a bound that ran out. `end` says
/// "stop asking me about this", and a verb that undid it would make `end`
/// something they could not rely on.
#[test]
fn a_mission_called_off_is_not_taken_back() {
    let mut flow = Flow::new(header(none(), Security::Gates, Bounds::default())).unwrap();
    flow.advance(Event::Ended {
        reason: "the feature was dropped".into(),
    })
    .unwrap();

    let error = flow
        .advance(Event::Retried {
            because: "I changed my mind".into(),
        })
        .unwrap_err();

    assert!(
        matches!(error, nunki::mission::flow::FlowError::CalledOff(_)),
        "{error:?}"
    );
    let said = error.to_string();
    assert!(said.contains("the feature was dropped"), "{said}");
    // And it is still where it was: a refused verb changes nothing.
    assert!(
        matches!(
            flow.stage(),
            Stage::AwaitingHuman(Handover::Abandoned { .. })
        ),
        "{:?}",
        flow.stage()
    );
}

/// A mission that is running fine has nothing to take back, and is told that
/// rather than moved.
#[test]
fn a_mission_that_never_stopped_is_not_taken_back() {
    let mut flow = Flow::new(header(none(), Security::Gates, Bounds::default())).unwrap();

    let error = flow
        .advance(Event::Retried {
            because: "why not".into(),
        })
        .unwrap_err();

    assert!(
        matches!(
            error,
            nunki::mission::flow::FlowError::InvalidTransition { .. }
        ),
        "{error:?}"
    );
}
