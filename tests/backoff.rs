//! Waiting out the harness (SPEC 4.3): the schedule, the ceiling, and the
//! one failure no wait mends.

use nunki::backoff::{HarnessDown, Next, after, due, wait_minutes};
use nunki::harness::Fault;

fn down(failures: u32, since: u64) -> HarnessDown {
    HarnessDown {
        failures,
        since,
        not_before: 0,
        last: "429".into(),
    }
}

#[test]
fn each_failure_in_a_row_doubles_the_wait_up_to_an_hour() {
    let schedule: Vec<u64> = (1..=8).map(wait_minutes).collect();
    assert_eq!(schedule, vec![2, 4, 8, 16, 32, 60, 60, 60]);
    // However long the harness stays down, the arithmetic does not overflow.
    assert_eq!(wait_minutes(u32::MAX), 60);
}

#[test]
fn the_first_failure_waits_two_minutes_and_the_count_keeps_its_start() {
    let (record, next) = after(None, &Fault::transient("429"), 1_000, 6);
    assert_eq!(
        record,
        HarnessDown {
            failures: 1,
            since: 1_000,
            not_before: 1_120,
            last: "429".into()
        }
    );
    assert_eq!(next, Next::Wait { until: 1_120 });

    let (record, _) = after(Some(&record), &Fault::transient("503"), 1_200, 6);
    assert_eq!((record.failures, record.since), (2, 1_000));
    assert_eq!(record.not_before, 1_200 + 4 * 60);
    assert_eq!(record.last, "503");
}

/// A wait that would end exactly at the ceiling is still waited; one second
/// past it is not — the boundary is where a flipped comparison hides.
#[test]
fn a_wait_that_would_pass_the_ceiling_goes_to_the_human() {
    // The sixth failure waits an hour; with a one-hour ceiling counted from
    // `since = 0`, a failure at 0 ends its wait exactly on the ceiling.
    let (_, next) = after(Some(&down(5, 0)), &Fault::transient("429"), 0, 1);
    assert_eq!(next, Next::Wait { until: 3_600 });

    let (_, next) = after(Some(&down(5, 0)), &Fault::transient("429"), 1, 1);
    match next {
        Next::Human { why } => {
            assert!(why.contains("1-hour ceiling"), "{why}");
            assert!(why.contains("6 time(s)"), "{why}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_ceiling_of_zero_sends_the_first_failure_to_the_human() {
    let (_, next) = after(None, &Fault::transient("429"), 1_000, 0);
    assert!(matches!(next, Next::Human { .. }), "{next:?}");
}

#[test]
fn an_authentication_failure_goes_to_the_human_at_once() {
    let (record, next) = after(None, &Fault::authentication("401"), 1_000, 6);
    assert_eq!(record.failures, 1);
    match next {
        Next::Human { why } => assert!(why.contains("could not authenticate"), "{why}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_run_is_due_once_its_not_before_is_reached() {
    assert!(due(None, 0));
    let record = HarnessDown {
        not_before: 100,
        ..down(1, 0)
    };
    assert!(!due(Some(&record), 99));
    assert!(due(Some(&record), 100));
}
