//! What a subscription has left (SPEC 4.3): the two windows, their
//! thresholds, and the measure kept per account.

use nunki::consumption::{Kind, Measure, Window, Windows, over, per_mille, percent, read, record};
use nunki::mission::Bounds;

fn measure(five: Option<(u32, u64)>, week: Option<(u32, u64)>, at: u64) -> Measure {
    let window = |w: Option<(u32, u64)>| {
        w.map(|(per_mille, resets_at)| Window {
            per_mille,
            resets_at,
        })
    };
    Measure {
        windows: Windows {
            five_hour: window(five),
            weekly: window(week),
        },
        measured_at: at,
        harness: "claude-code".into(),
    }
}

/// 90 % of five hours and 80 % of the week by default; at the threshold a
/// launch waits, one thousandth below it does not.
#[test]
fn a_window_at_its_threshold_stops_and_one_below_does_not() {
    let bounds = Bounds::default();
    assert_eq!(
        (bounds.five_hour_stop_percent, bounds.weekly_stop_percent),
        (90, 80)
    );

    let at = over(&measure(Some((900, 2_000)), None, 0), &bounds, 1_000);
    assert_eq!(at.map(|o| (o.kind, o.until)), Some((Kind::FiveHour, 2_000)));
    assert_eq!(
        over(&measure(Some((899, 2_000)), None, 0), &bounds, 1_000),
        None
    );

    let at = over(&measure(None, Some((800, 5_000)), 0), &bounds, 1_000);
    assert_eq!(at.map(|o| (o.kind, o.until)), Some((Kind::Weekly, 5_000)));
    assert_eq!(
        over(&measure(None, Some((799, 5_000)), 0), &bounds, 1_000),
        None
    );
}

/// A window whose reset has passed forbids nothing: that is how `nunki` goes
/// on by itself once it resets.
#[test]
fn a_window_that_has_reset_stops_nothing() {
    let bounds = Bounds::default();
    assert_eq!(
        over(&measure(Some((990, 1_000)), None, 0), &bounds, 1_000),
        None
    );
    assert!(over(&measure(Some((990, 1_001)), None, 0), &bounds, 1_000).is_some());
}

/// Both past their threshold: the launch waits for the later reset, since
/// the earlier one would leave the other window still over.
#[test]
fn both_windows_over_wait_for_the_later_reset() {
    let bounds = Bounds::default();
    let week_later = over(
        &measure(Some((950, 2_000)), Some((850, 9_000)), 0),
        &bounds,
        1_000,
    );
    assert_eq!(
        week_later.map(|o| (o.kind, o.until)),
        Some((Kind::Weekly, 9_000))
    );
    let five_later = over(
        &measure(Some((950, 9_000)), Some((850, 2_000)), 0),
        &bounds,
        1_000,
    );
    assert_eq!(
        five_later.map(|o| (o.kind, o.until)),
        Some((Kind::FiveHour, 9_000))
    );
}

#[test]
fn a_utilization_reads_in_thousandths_and_prints_as_a_percent() {
    assert_eq!(per_mille(0.53), 530);
    assert_eq!(per_mille(0.905), 905);
    assert_eq!(per_mille(-1.0), 0);
    assert_eq!(percent(530), "53");
    assert_eq!(percent(905), "90.5");
}

/// Two missions on one account read the windows at different times; an
/// older reading must not overwrite a newer one.
#[test]
fn a_newer_measure_is_kept_and_an_older_one_is_not() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    record(root, "main", &measure(Some((500, 9)), None, 200)).unwrap();
    record(root, "main", &measure(Some((100, 9)), None, 100)).unwrap();
    assert_eq!(read(root, "main").unwrap().unwrap().measured_at, 200);
    record(root, "main", &measure(Some((600, 9)), None, 300)).unwrap();
    let kept = read(root, "main").unwrap().unwrap();
    assert_eq!(
        (kept.measured_at, kept.windows.five_hour.unwrap().per_mille),
        (300, 600)
    );
    // Per account: another one has a file of its own.
    assert!(read(root, "other").unwrap().is_none());
}

/// Nothing measured is `None`; a file that cannot be read is an error, never
/// "nothing measured" — which a reader would take for "nothing spent".
#[test]
fn nothing_measured_is_none_and_an_unreadable_file_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    assert!(read(dir.path(), "main").unwrap().is_none());
    std::fs::create_dir_all(dir.path().join(nunki::consumption::USAGE_DIR)).unwrap();
    std::fs::write(nunki::consumption::path(dir.path(), "main"), "not json").unwrap();
    assert!(read(dir.path(), "main").is_err());
}
