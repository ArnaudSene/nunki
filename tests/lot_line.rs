//! The line a coder run leaves to say how its lot ended (SPEC 4.1, 4.3).

use hq::mission::journal::{LotLine, judge, lot_line, parse};

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
    assert_eq!(judge(&journal("Lot: L1 — done"), "L1"), Ok(()));
    let why = |block: &str| judge(&journal(block), "L1").unwrap_err();
    assert!(why("Lot: L1 — failed: X is red").contains("failed: X is red"));
    assert!(why("Lot: L1 — failed").contains("without saying why"));
    assert!(why("Lot: L2 — done").contains("reports lot L2"));
    assert!(why("Lot: L2 — failed: no").contains("reports lot L2"));
    assert!(why("Nothing said.").contains("`Lot: L1 — done`"));
}

/// The coder is told the line in its role prompt; the roles that conclude
/// with a verdict are not.
#[test]
fn the_coder_is_told_the_line_and_the_other_roles_are_not() {
    use hq::harness::Role;
    let coder = hq::role::prompt(Role::Coder);
    assert!(coder.contains("`Lot: <lot> — done`"), "{coder}");
    assert!(coder.contains("`Lot: <lot> — failed: <why>`"), "{coder}");
    for role in [Role::Integrator, Role::Security] {
        assert!(!hq::role::prompt(role).contains("Lot:"), "{role:?}");
    }
}
