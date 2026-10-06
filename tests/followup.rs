//! `FOLLOWUP_HQ.md`, the file every role is told to read before anything else.
//!
//! Its whole job is to be read, so what it holds is prose. A run of four
//! spaces at the start of a line renders as a Markdown code block, and the
//! sentences that matter arrive monospaced and unread — `ended` putting the
//! reason a mission was called off inside one is exactly that failure.

mod common;

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
    followup::lifted_by_nunki(
        &file,
        &["LOW — a verbose error page: it names no path".to_string()],
        &head(),
    )
    .unwrap();
    followup::ended(&file, "Someone", "the approach was wrong").unwrap();
    followup::said(&file, "Someone", "read the migration before the store").unwrap();
    followup::security_capped(&file, 1, 1, &["0123456789ab the volet".to_string()]).unwrap();
    followup::security_capped_on_findings(&file, 3, 3, &["0123456789ab the fix".to_string()])
        .unwrap();
    followup::proposal_refused(
        &file,
        "Someone",
        "src/lib.rs:3: replace + with -",
        &nunki::mutants::Refusal {
            proposed: "only a log line reads it".into(),
            because: "the CLI prints that line".into(),
        },
    )
    .unwrap();
    followup::proposals_await(&file, "m1", &[proposal("s1", "only a log line reads it")]).unwrap();

    let text = std::fs::read_to_string(&file).unwrap();
    assert_prose(&text, "a record");
    // And it really did write all of them, so a green run is not an empty one.
    for needle in [
        "carried",
        "lifted a security finding",
        "lifted the security verdict",
        "nunki lifted the security verdict",
        "called this mission off",
        "left an instruction",
        "did not call the security agent again",
        "Not attacked by the security agent",
        "refused an equivalence the coder proposed",
        "equivalence(s) the coder proposed await the HQ",
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

/// A spent round cap names the commits after the last round, oldest first,
/// one per line — and when there are none to name, says nothing about them
/// rather than a heading over an empty list (SPEC 4.5).
#[test]
fn a_spent_round_cap_names_the_commits_not_attacked_and_only_when_there_are_some() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("FOLLOWUP_HQ.md");
    let commits = [
        "0123456789ab the volet begins".to_string(),
        "ba9876543210 the volet ends".to_string(),
    ];
    followup::security_capped(&file, 1, 1, &commits).unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(
        text.contains(
            "Not attacked by the security agent:\n\n\
             - 0123456789ab the volet begins\n\
             - ba9876543210 the volet ends\n"
        ),
        "{text}"
    );

    for capped in [
        followup::security_capped,
        followup::security_capped_on_findings,
    ] {
        let empty = dir.path().join("EMPTY.md");
        let _ = std::fs::remove_file(&empty);
        capped(&empty, 1, 1, &[]).unwrap();
        let text = std::fs::read_to_string(&empty).unwrap();
        assert!(text.contains("security round cap was reached"), "{text}");
        assert!(!text.contains("Not attacked"), "{text}");
    }
}

/// What agents and authors write here — a verdict's report, the commits not
/// attacked — is read by a human, often with `cat`: every record carries
/// it escaped, never raw, and keeps its line breaks.
#[test]
fn agent_text_is_written_into_the_follow_up_escaped_never_raw() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("FOLLOWUP_HQ.md");
    std::fs::write(&file, "").unwrap();
    let report = format!("first line\n{}", common::HOSTILE);

    followup::carry(&file, Role::Security, Verdict::Findings, &report, &head()).unwrap();
    followup::security_capped(&file, 1, 1, &[common::HOSTILE.to_string()]).unwrap();
    followup::security_capped_on_findings(&file, 1, 1, &[common::HOSTILE.to_string()]).unwrap();

    let text = std::fs::read_to_string(&file).unwrap();
    common::assert_printable(&text, "FOLLOWUP_HQ.md");
    assert!(text.contains("first line\n"), "{text}");
    assert_eq!(text.matches("\\u{1b}[2K").count(), 3, "{text}");
}

fn proposal(id: &str, why: &str) -> nunki::mutants::Proposal {
    nunki::mutants::Proposal {
        id: id.into(),
        file: "src/lib.rs".into(),
        line: 3,
        why: why.into(),
    }
}

/// When the final gates pass on proposals nobody ruled on, the HQ reads
/// each with its reason, and the two verbs that rule — and nothing at all
/// is written when there is none.
#[test]
fn the_proposals_awaiting_the_hq_are_listed_with_their_reasons() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("FOLLOWUP_HQ.md");
    std::fs::write(&file, "").unwrap();

    followup::proposals_await(&file, "m1", &[]).unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "");

    followup::proposals_await(
        &file,
        "m1",
        &[
            proposal("s1", "only a log line reads it"),
            proposal("s2", "both arms return the same constant"),
        ],
    )
    .unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    for part in [
        "2 equivalence(s) the coder proposed await the HQ",
        "- `s1` (src/lib.rs:3) — only a log line reads it",
        "- `s2` (src/lib.rs:3) — both arms return the same constant",
        "nunki mission mutants m1 --ratify <survivor>",
        "--refuse <survivor> --because <why>",
        "`nunki push` refuses",
    ] {
        assert!(text.contains(part), "{part:?} in:\n{text}");
    }
}
