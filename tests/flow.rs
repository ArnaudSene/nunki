//! The flow of SPEC 4.5, exercised without a container or a model.

use nunki::harness::{Outcome, Role, Usage};
use nunki::mission::flow::{Event, Flow, Handover, SecurityCap, Stage, Work};
use nunki::mission::{Bounds, Header, Integration, Lot, Rigor, Security, Service, Verdict};

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
        rigor: Default::default(),
        mutation_threshold: None,
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
/// Exempting this return from the count is wrong. SPEC 7 says the loop is
/// bounded and what the bound counts: on the third return to the coder on
/// the same mission, HQ does not relaunch. Any return, not only the one a
/// red verdict opens.
///
/// What the exemption costs: gate 7 red on survivors no test can kill, the
/// agent declaring its volet done, the gates played again, the same gate red
/// again, the same volet 0 opened again — dozens of times, millions of
/// tokens, and nothing in the flow that can stop it.
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

/// The HQ reads a verified mission before it is pushed, and may send it back
/// (SPEC 4.5): the coder gets a volet carrying the HQ's reason, and the
/// verification runs again after it, as after any volet.
#[test]
fn a_verified_mission_the_hq_sends_back_is_a_volet_with_its_reason() {
    let mut flow = Flow::new(header(none(), Security::Gates, Bounds::default())).unwrap();
    code_through(&mut flow);
    flow.advance(Event::GatesPassed).unwrap();
    assert_eq!(flow.stage(), &Stage::Verified);

    flow.advance(Event::Reviewed {
        because: "revert the Playwright bump".into(),
    })
    .unwrap();
    match flow.stage() {
        Stage::Coding {
            work: Work::Volet { n: 1, cause },
            attempt: 1,
        } => assert!(cause.contains("revert the Playwright bump"), "{cause}"),
        other => panic!("not a volet: {other:?}"),
    }
    flow.advance(finished(true)).unwrap();
    assert_eq!(flow.stage(), &Stage::Gates);
    flow.advance(Event::GatesPassed).unwrap();
    assert_eq!(flow.stage(), &Stage::Verified);
    assert_eq!(flow.volets(), 1);
}

/// A review is a volet like any other: it spends the same bounded budget,
/// and with none left the mission is handed back to the human rather than
/// sent round again.
#[test]
fn a_review_with_no_volet_left_hands_the_mission_back() {
    let bounds = Bounds {
        max_volets: 0,
        ..Bounds::default()
    };
    let mut flow = Flow::new(header(none(), Security::Gates, bounds)).unwrap();
    code_through(&mut flow);
    flow.advance(Event::GatesPassed).unwrap();
    flow.advance(Event::Reviewed {
        because: "one more thing".into(),
    })
    .unwrap();
    assert!(
        matches!(
            flow.stage(),
            Stage::AwaitingHuman(Handover::VoletsExhausted { .. })
        ),
        "{:?}",
        flow.stage()
    );
}

/// Only a verified mission is reviewed: a mission still being coded or
/// verified has not been handed to the HQ yet.
#[test]
fn a_review_before_verification_is_refused() {
    let mut flow = Flow::new(header(none(), Security::Gates, Bounds::default())).unwrap();
    assert!(
        flow.advance(Event::Reviewed {
            because: "too early".into(),
        })
        .is_err()
    );
}

// ---------------------------------------------------------------------------
// Security rounds, bounded by the mission's rigor (SPEC 4.5).
// ---------------------------------------------------------------------------

/// A flow at `rigor`, with a security agent and the default bounds — the
/// ones a mission gets when its header says nothing, three volets.
fn at(rigor: Rigor, integration: Integration) -> Flow {
    let mut h = header(integration, Security::Agent, Bounds::default());
    h.rigor = rigor;
    Flow::new(h).unwrap()
}

/// A `FINDINGS` whose report says which round it was.
fn findings(round: u32) -> Event {
    Event::Verdict {
        verdict: Verdict::Findings,
        report: format!("round {round}: an open redirect"),
    }
}

fn back_on(round: u32) -> Stage {
    Stage::Findings {
        report: format!("round {round}: an open redirect"),
    }
}

/// One security round that ends in findings, an iterate, the coder's volet
/// gated as usual — and then, at `standard`, no second round. The volet
/// that answered the findings was never attacked, so the mission is not
/// verified: it goes back to those findings, where `accept` or `iterate`
/// decide, and the cap is left for the follow-up, once.
#[test]
fn at_standard_findings_then_an_iterate_come_back_to_the_findings_with_no_second_round() {
    let mut flow = at(Rigor::Standard, none());
    assert_eq!(flow.max_security_rounds(), 1);
    code_through(&mut flow);
    flow.advance(Event::GatesPassed).unwrap();
    assert_eq!(flow.stage(), &Stage::SecurityAgent { attempt: 1 });
    flow.advance(findings(1)).unwrap();
    assert_eq!(flow.security_rounds(), 1);

    // The iterate after the last round is still accepted, and its volet is
    // run and gated like any other.
    flow.advance(Event::Iterate).unwrap();
    assert!(matches!(
        flow.stage(),
        Stage::Coding {
            work: Work::Volet { n: 1, .. },
            ..
        }
    ));
    flow.advance(finished(true)).unwrap();
    assert_eq!(flow.stage(), &Stage::Gates);
    // A red gate still sends it back: the cap spares the agent, not the gates.
    flow.advance(Event::GatesFailed {
        reason: "battery".into(),
    })
    .unwrap();
    flow.advance(finished(true)).unwrap();
    assert_eq!(flow.take_security_cap(), None, "nothing was skipped yet");

    flow.advance(Event::GatesPassed).unwrap();
    assert_eq!(
        flow.stage(),
        &back_on(1),
        "no second round, and no Verified"
    );
    assert_eq!(flow.security_rounds(), 1);
    assert_eq!(
        flow.take_security_cap(),
        Some(SecurityCap { rounds: 1, max: 1 })
    );
    assert_eq!(flow.take_security_cap(), None, "said once");

    // The verbs of a FINDINGS apply. Iterate spends a volet, bounded, and
    // comes back to the same findings; accept is what verifies.
    flow.advance(Event::Iterate).unwrap();
    flow.advance(finished(true)).unwrap();
    flow.advance(Event::GatesPassed).unwrap();
    assert_eq!(flow.stage(), &back_on(1));
    assert_eq!(flow.volets(), 3);
    flow.advance(Event::HumanAccepted).unwrap();
    assert_eq!(flow.stage(), &Stage::Verified);
}

/// The integrator still replays after an iterate at the cap; it is only the
/// security stage that is not played, and the mission is back on its
/// findings after the integrator's verdict.
#[test]
fn at_standard_the_integrator_replays_and_the_spent_round_is_skipped_after_it() {
    let mut flow = at(Rigor::Standard, services());
    code_through(&mut flow);
    flow.advance(Event::GatesPassed).unwrap();
    flow.advance(verdict(Verdict::Integrated)).unwrap();
    flow.advance(findings(1)).unwrap();
    flow.advance(Event::Iterate).unwrap();
    flow.advance(finished(true)).unwrap();
    flow.advance(Event::GatesPassed).unwrap();
    assert_eq!(flow.stage(), &Stage::Integration { attempt: 1 });
    flow.advance(verdict(Verdict::Integrated)).unwrap();
    assert_eq!(flow.stage(), &back_on(1));
    assert_eq!(
        flow.take_security_cap(),
        Some(SecurityCap { rounds: 1, max: 1 })
    );
}

/// `critical` keeps its three rounds, and a fourth is never launched. With
/// the default bounds, three FINDINGS rounds and three iterates use every
/// volet; the third volet's green gates do not verify the mission — it is
/// back on the third round's findings, and dev never let a mission reach
/// `Verified` without a CLEAR or a human's lift.
#[test]
fn at_critical_the_fourth_round_is_not_launched_and_the_third_findings_stand() {
    let mut flow = at(Rigor::Critical, none());
    assert_eq!(flow.max_security_rounds(), 3);
    code_through(&mut flow);
    for round in 1..=3 {
        flow.advance(Event::GatesPassed).unwrap();
        assert_eq!(
            flow.stage(),
            &Stage::SecurityAgent { attempt: 1 },
            "round {round} is played"
        );
        flow.advance(findings(round)).unwrap();
        assert_eq!(flow.security_rounds(), round);
        flow.advance(Event::Iterate).unwrap();
        flow.advance(finished(true)).unwrap();
    }
    assert_eq!(flow.volets(), 3);
    flow.advance(Event::GatesPassed).unwrap();
    assert_eq!(
        flow.stage(),
        &back_on(3),
        "a fourth round is not played, and nothing is verified"
    );
    assert_eq!(
        flow.take_security_cap(),
        Some(SecurityCap { rounds: 3, max: 3 })
    );

    // Iterate is bounded as any volet: none is left.
    let mut spent = flow.clone();
    spent.advance(Event::Iterate).unwrap();
    assert!(
        matches!(
            spent.stage(),
            Stage::AwaitingHuman(Handover::VoletsExhausted { .. })
        ),
        "{:?}",
        spent.stage()
    );

    // Only the human's lift verifies it.
    flow.advance(Event::HumanAccepted).unwrap();
    assert_eq!(flow.stage(), &Stage::Verified);
}

/// A report a human lifted is not held any more. A finding lifted, then a
/// review that sends the branch back: at the cap, the new volet's green
/// gates verify the mission, the cap said, rather than bringing back a
/// report somebody already lifted and asking for the same lift again on
/// every volet. `nunki push` is what then reads the lift, and names the
/// commits after the last round as not attacked (SPEC 4.5).
#[test]
fn after_an_accept_a_review_and_green_gates_at_the_cap_the_old_report_does_not_come_back() {
    let mut flow = at(Rigor::Standard, none());
    code_through(&mut flow);
    flow.advance(Event::GatesPassed).unwrap();
    flow.advance(findings(1)).unwrap();
    flow.advance(Event::HumanAccepted).unwrap();
    assert_eq!(flow.stage(), &Stage::Verified);
    flow.advance(Event::Reviewed {
        because: "rename it".into(),
    })
    .unwrap();
    flow.advance(finished(true)).unwrap();
    flow.advance(Event::GatesPassed).unwrap();
    assert_eq!(flow.stage(), &Stage::Verified);
    assert_eq!(
        flow.take_security_cap(),
        Some(SecurityCap { rounds: 1, max: 1 })
    );

    // And after a second review too: nothing brings the lifted report back.
    flow.advance(Event::Reviewed {
        because: "and this".into(),
    })
    .unwrap();
    flow.advance(finished(true)).unwrap();
    flow.advance(Event::GatesPassed).unwrap();
    assert_eq!(flow.stage(), &Stage::Verified);
}

/// The lift clears the report it lifted, and only that: findings concluded
/// after it, on a round still left, are held again, and come back at the
/// cap like any other.
#[test]
fn findings_after_an_accept_are_held_again_and_come_back_at_the_cap() {
    let mut flow = at(Rigor::Critical, none());
    code_through(&mut flow);
    flow.advance(Event::GatesPassed).unwrap();
    flow.advance(findings(1)).unwrap();
    flow.advance(Event::HumanAccepted).unwrap();
    flow.advance(Event::Reviewed {
        because: "rename it".into(),
    })
    .unwrap();
    flow.advance(finished(true)).unwrap();
    flow.advance(Event::GatesPassed).unwrap();
    assert_eq!(flow.stage(), &Stage::SecurityAgent { attempt: 1 });
    flow.advance(findings(2)).unwrap();
    flow.advance(Event::Iterate).unwrap();
    flow.advance(finished(true)).unwrap();
    flow.advance(Event::GatesPassed).unwrap();
    flow.advance(findings(3)).unwrap();
    assert_eq!(flow.security_rounds(), 3);
    flow.advance(Event::Iterate).unwrap();
    assert_eq!(flow.volets(), 3, "a review and two iterates");
    flow.advance(finished(true)).unwrap();
    flow.advance(Event::GatesPassed).unwrap();
    assert_eq!(flow.stage(), &back_on(3));
}

/// Same rule after a retry: the volet the human took back from a handover
/// is gated, and the findings it was answering come back.
#[test]
fn a_retry_after_findings_at_the_cap_comes_back_to_them() {
    let mut flow = at(Rigor::Standard, none());
    code_through(&mut flow);
    flow.advance(Event::GatesPassed).unwrap();
    flow.advance(findings(1)).unwrap();
    flow.advance(Event::Iterate).unwrap();
    for _ in 0..3 {
        flow.advance(finished(false)).unwrap();
    }
    assert!(
        matches!(
            flow.stage(),
            Stage::AwaitingHuman(Handover::LotAttemptsExhausted { .. })
        ),
        "{:?}",
        flow.stage()
    );
    flow.advance(Event::Retried {
        because: "the harness is back".into(),
    })
    .unwrap();
    flow.advance(finished(true)).unwrap();
    flow.advance(Event::GatesPassed).unwrap();
    assert_eq!(flow.stage(), &back_on(1));
}

/// A CLEAR is a round too, and it clears the findings before it. After a
/// CLEAR the mission was verified by the agent; a review that sends it back
/// is the HQ's own request, and at `standard` the next green gates verify
/// it without a second round, the cap said.
#[test]
fn a_review_after_a_clear_at_the_cap_is_verified_with_the_cap_said() {
    let mut flow = at(Rigor::Critical, none());
    code_through(&mut flow);
    flow.advance(Event::GatesPassed).unwrap();
    flow.advance(findings(1)).unwrap();
    flow.advance(Event::Iterate).unwrap();
    flow.advance(finished(true)).unwrap();
    flow.advance(Event::GatesPassed).unwrap();
    flow.advance(findings(2)).unwrap();
    flow.advance(Event::Iterate).unwrap();
    flow.advance(finished(true)).unwrap();
    flow.advance(Event::GatesPassed).unwrap();
    flow.advance(verdict(Verdict::Clear)).unwrap();
    assert_eq!(flow.stage(), &Stage::Verified);
    assert_eq!(flow.security_rounds(), 3);
    assert_eq!(flow.take_security_cap(), None);
    flow.advance(Event::Reviewed {
        because: "rename it".into(),
    })
    .unwrap();
    flow.advance(finished(true)).unwrap();
    flow.advance(Event::GatesPassed).unwrap();
    assert_eq!(flow.stage(), &Stage::Verified);
    assert_eq!(
        flow.take_security_cap(),
        Some(SecurityCap { rounds: 3, max: 3 })
    );
}

/// A prototype runs the coder and the mechanical gates only. `mission new`
/// says so, and the flow says it again where a header is frozen — at the
/// start and at a reframe — because a header can be written by hand.
#[test]
fn a_prototype_with_a_security_agent_or_services_is_refused_where_it_is_frozen() {
    use nunki::mission::RigorError;
    use nunki::mission::flow::FlowError;

    let mut agent = header(none(), Security::Agent, Bounds::default());
    agent.rigor = Rigor::Prototype;
    assert_eq!(
        Flow::new(agent.clone()),
        Err(FlowError::Rigor(RigorError::PrototypeWithSecurityAgent))
    );
    let mut wired = header(services(), Security::Gates, Bounds::default());
    wired.rigor = Rigor::Prototype;
    assert_eq!(
        Flow::new(wired.clone()),
        Err(FlowError::Rigor(RigorError::PrototypeWithServices))
    );
    let err = Flow::new(wired.clone()).unwrap_err().to_string();
    assert!(err.contains("a prototype runs the coder"), "{err}");

    let mut flow = Flow::new(header(none(), Security::Agent, Bounds::default())).unwrap();
    let before = flow.clone();
    assert_eq!(
        flow.reframe(agent),
        Err(FlowError::Rigor(RigorError::PrototypeWithSecurityAgent))
    );
    assert_eq!(
        flow.reframe(wired),
        Err(FlowError::Rigor(RigorError::PrototypeWithServices))
    );
    assert_eq!(flow, before, "a refused reframe changes nothing");

    // A prototype that declares neither is framed, and reframed, as before.
    let mut bare = header(none(), Security::Gates, Bounds::default());
    bare.rigor = Rigor::Prototype;
    assert!(Flow::new(bare.clone()).is_ok());
    assert_eq!(flow.reframe(bare), Ok(()));
}

/// A prototype plays no round. A state frozen before the flow refused one
/// with a security agent can still carry both: it is verified on its gates,
/// and the cap — 0 / 0 — is said.
#[test]
fn a_prototype_plays_no_security_round() {
    let flow = at(Rigor::Critical, none());
    let mut json = serde_json::to_value(&flow).unwrap();
    json["header"]["rigor"] = serde_json::json!("prototype");
    let mut flow: Flow = serde_json::from_value(json).unwrap();
    assert_eq!(flow.max_security_rounds(), 0);
    code_through(&mut flow);
    flow.advance(Event::GatesPassed).unwrap();
    assert_eq!(flow.stage(), &Stage::Verified);
    assert_eq!(
        flow.take_security_cap(),
        Some(SecurityCap { rounds: 0, max: 0 })
    );
}

/// A mission with no security agent was never going to call it: no cap is
/// said for a round nobody asked for.
#[test]
fn a_mission_without_a_security_agent_records_no_cap() {
    let mut h = header(none(), Security::Gates, Bounds::default());
    h.rigor = Rigor::Prototype;
    let mut flow = Flow::new(h).unwrap();
    code_through(&mut flow);
    flow.advance(Event::GatesPassed).unwrap();
    assert_eq!(flow.stage(), &Stage::Verified);
    assert_eq!(flow.take_security_cap(), None);
}

/// State written before the count reads zero rounds and no findings held,
/// and a flow that was not skipping anything carries no cap on disk.
#[test]
fn an_old_state_reads_zero_security_rounds() {
    let mut flow = at(Rigor::Standard, none());
    code_through(&mut flow);
    let mut json: serde_json::Value = serde_json::to_value(&flow).unwrap();
    assert!(json.get("security_cap").is_none(), "{json}");
    assert!(json.get("last_findings").is_none(), "{json}");
    let object = json.as_object_mut().unwrap();
    assert_eq!(object.remove("security_rounds"), Some(serde_json::json!(0)));
    let old: Flow = serde_json::from_value(json).unwrap();
    assert_eq!(old.security_rounds(), 0);
    assert_eq!(old, flow);
}

// --- a lot that awaits a ruling (SPEC 4.4 gate 7, 4.5) -----------------------

fn ruling(ids: &[&str]) -> Event {
    Event::RulingAwaited {
        what: ids
            .iter()
            .map(|id| format!("`{id}`"))
            .collect::<Vec<_>>()
            .join(" and ")
            + " change nothing observable",
        survivors: ids.iter().map(|id| id.to_string()).collect(),
    }
}

fn bounded(attempts_per_lot: u32) -> Flow {
    Flow::new(header(
        none(),
        Security::Gates,
        Bounds {
            attempts_per_lot,
            ..Bounds::default()
        },
    ))
    .unwrap()
}

/// A ruling the coder may not give is not worth another attempt: the next
/// one would meet the same survivor and say the same thing. The flow hands
/// over at once, on the attempt that asked, naming what is to be ruled.
#[test]
fn a_lot_that_awaits_a_ruling_hands_over_at_once_on_the_attempt_that_asked() {
    let mut flow = bounded(3);
    flow.advance(finished(false)).unwrap();
    assert!(matches!(flow.stage(), Stage::Coding { attempt: 2, .. }));

    flow.advance(ruling(&["m1", "m2"])).unwrap();
    assert_eq!(
        flow.stage(),
        &Stage::AwaitingHuman(Handover::AwaitingRuling {
            lot: "L1".into(),
            what: "`m1` and `m2` change nothing observable".into(),
            attempt: 2,
            survivors: vec!["m1".into(), "m2".into()],
        })
    );
}

/// Only the coder ends a run on a lot line; a ruling anywhere else is out of
/// place, and said.
#[test]
fn a_ruling_outside_a_coder_run_is_refused() {
    let mut flow = bounded(3);
    code_through(&mut flow);
    assert!(matches!(
        flow.advance(ruling(&["m1"])),
        Err(nunki::mission::flow::FlowError::InvalidTransition { .. })
    ));
}

/// A ruling is not a bound running out, so `retry` hands no budget back: the
/// lot resumes at the attempt after the one that asked, and a coder that
/// keeps asking still meets the bound.
#[test]
fn a_retry_after_a_ruling_resumes_at_the_next_attempt_and_the_bound_holds() {
    let mut flow = bounded(3);
    flow.advance(finished(true)).unwrap();
    flow.advance(ruling(&["m1"])).unwrap();
    flow.advance(Event::Retried {
        because: "m1 ruled equivalent".into(),
    })
    .unwrap();
    assert_eq!(
        flow.stage(),
        &Stage::Coding {
            work: Work::Lot(1),
            attempt: 2
        },
        "the same lot, the next attempt — not the first"
    );

    flow.advance(ruling(&["m2"])).unwrap();
    flow.advance(Event::Retried {
        because: "m2 ruled equivalent".into(),
    })
    .unwrap();
    assert_eq!(
        flow.stage(),
        &Stage::Coding {
            work: Work::Lot(1),
            attempt: 3
        }
    );

    // The last attempt: a failure hands over as exhausted, at once.
    flow.advance(finished(false)).unwrap();
    assert_eq!(
        flow.stage(),
        &Stage::AwaitingHuman(Handover::LotAttemptsExhausted {
            lot: "L2".into(),
            attempts: 3
        })
    );
}

/// Asked on the last attempt, a retry does not go past the bound: it hands
/// over as exhausted, and only the retry after that — from a bound — hands
/// the attempts back whole.
#[test]
fn a_ruling_on_the_last_attempt_is_retried_into_exhaustion_not_past_the_bound() {
    let mut flow = bounded(1);
    flow.advance(ruling(&["m1"])).unwrap();
    flow.advance(Event::Retried {
        because: "ruled".into(),
    })
    .unwrap();
    assert_eq!(
        flow.stage(),
        &Stage::AwaitingHuman(Handover::LotAttemptsExhausted {
            lot: "L1".into(),
            attempts: 1
        })
    );
    flow.advance(Event::Retried {
        because: "a fresh budget".into(),
    })
    .unwrap();
    assert_eq!(
        flow.stage(),
        &Stage::Coding {
            work: Work::Lot(0),
            attempt: 1
        },
        "the work is kept through both handovers"
    );
}

/// The HQ's probe: a coder that asks for a ruling on every attempt never
/// runs an attempt past `attempts_per_lot`, however many times it is
/// taken back.
#[test]
fn a_coder_that_always_asks_never_runs_past_the_bound() {
    let mut flow = bounded(1);
    for round in 0..3 {
        if let Stage::Coding { attempt, .. } = flow.stage() {
            assert!(*attempt <= 1, "round {round}: attempt {attempt}");
            flow.advance(ruling(&["m1"])).unwrap();
        }
        flow.advance(Event::Retried {
            because: format!("round {round}"),
        })
        .unwrap();
        if let Stage::Coding { attempt, .. } = flow.stage() {
            assert!(*attempt <= 1, "round {round}: attempt {attempt}");
        }
    }
}

/// A volet awaits a ruling like a lot, and comes back as the same volet.
#[test]
fn a_volet_that_awaits_a_ruling_comes_back_as_that_volet() {
    let mut flow = bounded(3);
    code_through(&mut flow);
    flow.advance(Event::GatesFailed {
        reason: "gate 7".into(),
    })
    .unwrap();
    flow.advance(ruling(&["m1"])).unwrap();
    match flow.stage() {
        Stage::AwaitingHuman(Handover::AwaitingRuling { lot, attempt, .. }) => {
            assert_eq!(lot, "volet-1");
            assert_eq!(*attempt, 1);
        }
        other => panic!("{other:?}"),
    }
    flow.advance(Event::Retried {
        because: "ruled".into(),
    })
    .unwrap();
    match flow.stage() {
        Stage::Coding {
            work: Work::Volet { n, cause },
            attempt,
        } => {
            assert_eq!((*n, cause.as_str(), *attempt), (1, "gate: gate 7", 2));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(flow.volets(), 1, "a ruling spends no volet");
}

/// The handover read back, and the old ones with it: a state file written
/// before the variant existed still reads. Frozen from the code before
/// this change, serialised by it.
#[test]
fn an_old_state_file_handed_over_on_attempts_still_reads() {
    let old = r#"{"header":{"branch":"feat/x","base":"dev","lots":[{"id":"L1","title":"lot 1"}],"integration":{"kind":"none","reason":"pure domain"},"security":"gates","rigor":"critical","arbiter":null,"account":null,"model":null,"run":null,"bounds":{"max_volets":3,"attempts_per_lot":1,"checkpoint_minutes":45,"check_minutes":15,"stall_checks":3,"long_lot_hours":8,"mutation_minutes":45,"harness_wait_hours":6,"five_hour_stop_percent":90,"weekly_stop_percent":80}},"stage":{"AwaitingHuman":{"LotAttemptsExhausted":{"lot":"L1","attempts":1}}},"volets":0,"volet_causes":[],"attempts":{"L1":1},"resume_with":{"Lot":0},"security_rounds":0}"#;
    let mut flow: Flow = serde_json::from_str(old).unwrap();
    assert_eq!(
        flow.stage(),
        &Stage::AwaitingHuman(Handover::LotAttemptsExhausted {
            lot: "L1".into(),
            attempts: 1
        })
    );
    flow.advance(Event::Retried {
        because: "fixed".into(),
    })
    .unwrap();
    assert_eq!(
        flow.stage(),
        &Stage::Coding {
            work: Work::Lot(0),
            attempt: 1
        },
        "and a retry from it still hands the budget back whole"
    );

    let mut asked = bounded(3);
    asked.advance(ruling(&["m1"])).unwrap();
    let back: Flow = serde_json::from_str(&serde_json::to_string(&asked).unwrap()).unwrap();
    assert_eq!(back, asked);
}

/// One line says it all: the lot, the attempt, the ids, and the verbs the HQ
/// is expected to type, spelled with the mission's id.
#[test]
fn the_ruling_handover_names_the_lot_the_attempt_and_the_ids_in_one_line() {
    let said = Handover::AwaitingRuling {
        lot: "L2".into(),
        what: "`m1` and `src/lib.rs:3: replace + with -` are equivalent".into(),
        attempt: 2,
        survivors: vec!["m1".into(), "src/lib.rs:3: replace + with -".into()],
    }
    .line("m7");
    assert!(!said.contains('\n'), "{said}");
    for part in [
        "lot L2",
        "attempt 2",
        "`m1`, `src/lib.rs:3: replace + with -`",
        "nunki mission mutants m7 --equivalent",
        "--because",
        "nunki mission retry m7",
    ] {
        assert!(said.contains(part), "{part:?} in {said}");
    }
    // The other handovers keep saying what stopped them.
    let exhausted = Handover::LotAttemptsExhausted {
        lot: "L2".into(),
        attempts: 3,
    }
    .line("m7");
    assert!(
        exhausted.contains("lot L2 failed 3 attempt(s)"),
        "{exhausted}"
    );
    assert!(exhausted.contains("nunki mission retry m7"), "{exhausted}");
}
