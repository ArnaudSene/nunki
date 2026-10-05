//! The line a coder run leaves to say how its lot ended (SPEC 4.1, 4.3).

use nunki::mission::journal::{Judgement, LotLine, awaitable, judge, lot_line, parse};

fn done(lot: &str) -> Option<LotLine> {
    Some(LotLine::Done { lot: lot.into() })
}

fn failed(lot: &str, reason: &str) -> Option<LotLine> {
    Some(LotLine::Failed {
        lot: lot.into(),
        reason: reason.into(),
    })
}

/// A resume block with `block` in it, and an older lot's line further down
/// that must never be read as this run's.
fn journal(block: &str) -> String {
    format!("# Journal\n\n## ÉTAT DE REPRISE\n\n{block}\n\n## Earlier\n\nLot: L0 — done\n")
}

#[test]
fn done_and_failed_read_as_the_grammar_says() {
    assert_eq!(parse("Lot: L2 — done"), done("L2"));
    assert_eq!(
        parse("Lot: L2 — failed: the test X does not pass"),
        failed("L2", "the test X does not pass")
    );
    assert_eq!(parse("Lot: L2 — failed"), failed("L2", ""));
}

#[test]
fn the_forms_an_agent_writes_are_read_too() {
    assert_eq!(parse("Lot: L2 – done"), done("L2"), "en dash");
    assert_eq!(parse("Lot: L2 - done"), done("L2"), "hyphen between spaces");
    assert_eq!(parse("- Lot: `L2` — done."), done("L2"), "a list item");
    assert_eq!(parse("* lot: L2 — Done"), done("L2"), "case");
    assert_eq!(
        parse("Lot: volet-1 — failed:  still red "),
        failed("volet-1", "still red"),
        "an identifier with a hyphen"
    );
}

#[test]
fn what_is_not_the_grammar_is_not_read() {
    for line in [
        "Lot: L2 — failedx",
        "Lot: L2 — in progress",
        "Lot: — done",
        "Lot:L2—done",
        "The lot: L2 — done",
        "> Lot: L2 — done",
        "Lot: L2",
    ] {
        assert_eq!(parse(line), None, "{line}");
    }
}

#[test]
fn only_the_resume_block_is_read_and_its_last_line_wins() {
    assert_eq!(
        lot_line(&journal("Lot: L1 — failed: first try\nLot: L1 — done")),
        done("L1")
    );
    assert_eq!(lot_line(&journal("Nothing said.")), None);
    assert_eq!(lot_line("# Journal\n\nLot: L1 — done\n"), None, "no block");
}

#[test]
fn the_judgement_says_why_a_lot_is_not_done() {
    assert_eq!(judge(&journal("Lot: L1 — done"), "L1"), Judgement::Done);
    let why = |block: &str| match judge(&journal(block), "L1") {
        Judgement::Failed(why) => why,
        other => panic!("{block}: {other:?}"),
    };
    assert!(why("Lot: L1 — failed: X is red").contains("failed: X is red"));
    assert!(why("Lot: L1 — failed").contains("without saying why"));
    assert!(why("Lot: L2 — done").contains("reports lot L2"));
    assert!(why("Lot: L2 — failed: no").contains("reports lot L2"));
    assert!(why("Lot: L2 — awaits ruling: `a`").contains("reports lot L2"));
    assert!(why("Nothing said.").contains("`Lot: L1 — done`"));
}

// --- the third line: a ruling the coder may not give -------------------------

fn awaits(lot: &str, what: &str) -> Option<LotLine> {
    Some(LotLine::AwaitsRuling {
        lot: lot.into(),
        what: what.into(),
    })
}

#[test]
fn a_ruling_line_reads_with_the_tolerance_of_the_other_two() {
    assert_eq!(
        parse("Lot: L1 — awaits ruling: `m1` is equivalent"),
        awaits("L1", "`m1` is equivalent")
    );
    assert_eq!(
        parse("- Lot: `L1` – Awaits Ruling:  `m1` "),
        awaits("L1", "`m1`"),
        "a list item, an en dash, case, spaces"
    );
    assert_eq!(
        parse("* lot: volet-2 - awaits ruling `m1`"),
        awaits("volet-2", "`m1`"),
        "a hyphen between spaces, no colon"
    );
}

#[test]
fn awaits_ruling_with_nothing_after_it_is_not_a_ruling() {
    for line in [
        "Lot: L1 — awaits ruling",
        "Lot: L1 — awaits ruling:",
        "Lot: L1 — awaits ruling:   ",
        "Lot: L1 — awaits ruling.",
        "Lot: L1 — awaits rulingx: `m1`",
    ] {
        assert_eq!(parse(line), None, "{line}");
    }
    // And so the judgement is today's: no valid line, a failed attempt.
    match judge(&journal("Lot: L1 — awaits ruling"), "L1") {
        Judgement::Failed(why) => assert!(why.contains("`Lot: L1 — done`"), "{why}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_ruling_names_the_survivors_between_its_backquotes() {
    match judge(
        &journal(
            "Lot: L1 — awaits ruling: `src/lib.rs:3:1: replace + with -` and ` m2 ` and \
             `m2` again, both equivalent",
        ),
        "L1",
    ) {
        Judgement::AwaitsRuling { what, survivors } => {
            assert!(what.starts_with("`src/lib.rs:3:1"), "{what}");
            assert_eq!(survivors, ["src/lib.rs:3:1: replace + with -", "m2"]);
        }
        other => panic!("{other:?}"),
    }
    match judge(&journal("Lot: L1 — awaits ruling: nothing quoted"), "L1") {
        Judgement::AwaitsRuling { survivors, .. } => assert!(survivors.is_empty()),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_ruling_is_awaited_only_on_open_survivors() {
    let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let open = ids(&["m1", "m2"]);
    assert_eq!(awaitable("L1", &ids(&["m1"]), &open), Ok(()));
    assert_eq!(awaitable("L1", &ids(&["m2", "m1"]), &open), Ok(()));

    let none = awaitable("L1", &[], &open).unwrap_err();
    assert!(none.contains("named no survivor"), "{none}");

    let stranger = awaitable("L1", &ids(&["m1", "m9"]), &open).unwrap_err();
    assert!(stranger.contains("`m9`"), "{stranger}");
    assert!(
        !stranger.contains("`m1`"),
        "only the one at fault: {stranger}"
    );
    assert!(stranger.contains("that is not a survivor"), "{stranger}");

    let two = awaitable("L1", &ids(&["m8", "m9"]), &open).unwrap_err();
    assert!(two.contains("`m8`, `m9`, and those are"), "{two}");

    let nothing_open = awaitable("L1", &ids(&["m1"]), &[]).unwrap_err();
    assert!(nothing_open.contains("`m1`"), "{nothing_open}");
}
