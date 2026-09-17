//! `FOLLOWUP_HQ.md`, the file every role is told to read before anything else.
//!
//! Its whole job is to be read, so what it holds is prose. A run of four
//! spaces at the start of a line renders as a Markdown code block, and the
//! sentences that matter arrive monospaced and unread — measured on
//! 2026-09-17, `ended` put the reason a mission was called off inside one.

use nunki::followup;
use nunki::harness::Role;
use nunki::mission::Verdict;

fn head() -> String {
    "a".repeat(40)
}

/// Every line of it, whoever wrote it.
fn assert_prose(text: &str, who: &str) {
    for line in text.lines() {
        assert!(
            !line.starts_with("    "),
            "{who} renders as a code block: {line:?}\n{text}"
        );
        assert!(
            !line.contains("  "),
            "{who} has a run of spaces inside a line: {line:?}\n{text}"
        );
    }
}

/// Driven by writing every one of them into the same file: a writer added
/// later is covered here the day it is called, not the day someone remembers
/// this test exists.
#[test]
fn every_record_the_hq_leaves_is_prose_and_not_a_code_block() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("FOLLOWUP_HQ.md");
    std::fs::write(&file, "").unwrap();

    followup::carry(
        &file,
        Role::Security,
        Verdict::Findings,
        "a token in the log",
        &head(),
    )
    .unwrap();
    followup::lifted(
        &file,
        "Someone",
        "a token in the log",
        "the log is not shipped",
        &head(),
    )
    .unwrap();
    followup::lifted_all(&file, "Someone", "the risk is accepted", &head()).unwrap();
    followup::ended(&file, "Someone", "the approach was wrong").unwrap();
    followup::said(&file, "Someone", "read the migration before the store").unwrap();

    let text = std::fs::read_to_string(&file).unwrap();
    assert_prose(&text, "a record");
    // And it really did write all of them, so a green run is not an empty one.
    for needle in [
        "carried",
        "lifted a security finding",
        "lifted the security verdict",
        "called this mission off",
        "left an instruction",
    ] {
        assert!(text.contains(needle), "{needle} is missing:\n{text}");
    }
}

/// The reason a mission was called off is the one sentence whoever finds it
/// six months from now has. It was inside a code block.
#[test]
fn the_reason_a_mission_was_called_off_is_readable() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("FOLLOWUP_HQ.md");
    std::fs::write(&file, "").unwrap();

    followup::ended(&file, "Someone", "the approach was wrong").unwrap();

    let text = std::fs::read_to_string(&file).unwrap();
    assert_prose(&text, "ended");
    assert!(
        text.contains("**Because:** the approach was wrong"),
        "{text}"
    );
}
