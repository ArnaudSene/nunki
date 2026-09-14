//! Lifting a security verdict (SPEC 4.5), and what the HQ carries to the
//! coder in `FOLLOWUP_HQ.md`.

use std::path::Path;

use nunki::followup;
use nunki::harness::Role;
use nunki::mission::Verdict;

fn file(dir: &Path) -> std::path::PathBuf {
    dir.join("FOLLOWUP_HQ.md")
}

/// The follow-up file is the one thing an agent is told to read before
/// anything else. A block appended without a blank line before it is a
/// heading glued to the previous paragraph — a heading the reader may not
/// see at all.
#[test]
fn blocks_are_appended_one_blank_line_apart_whatever_the_file_ended_with() {
    let dir = tempfile::tempdir().unwrap();

    for ending in ["", "# Follow-up\n", "# Follow-up", "# Follow-up\n\n"] {
        let path = file(dir.path());
        let _ = std::fs::remove_file(&path);
        if !ending.is_empty() {
            std::fs::write(&path, ending).unwrap();
        }
        followup::carry(
            &path,
            Role::Integrator,
            Verdict::Broken,
            "it does not wire",
            "abc123def4567",
        )
        .unwrap();
        followup::carry(
            &path,
            Role::Security,
            Verdict::Findings,
            "an open redirect",
            "abc123def4567",
        )
        .unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        for (before, heading) in text.lines().zip(text.lines().skip(1)) {
            if heading.starts_with("## ") {
                assert!(
                    before.is_empty(),
                    "a heading glued to {before:?} in:\n{text}"
                );
            }
        }
        assert!(!text.contains("\n\n\n"), "two blank lines in:\n{text}");
        // And the head of the file is never rewritten: it belongs to the
        // human who framed the mission.
        if !ending.trim().is_empty() {
            assert!(text.starts_with("# Follow-up"), "{text}");
        }
    }
}

/// A block says who concluded, what, and on which commit — because a verdict
/// is worth one commit and no other, and the coder reading this must be able
/// to tell whether it still applies.
#[test]
fn a_carried_verdict_names_the_role_the_verdict_and_the_commit() {
    let dir = tempfile::tempdir().unwrap();
    let path = file(dir.path());
    followup::carry(
        &path,
        Role::Security,
        Verdict::Findings,
        "an open redirect in /auth/callback",
        "0123456789abcdef0123456789abcdef01234567",
    )
    .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("security"), "{text}");
    assert!(text.contains("Findings"), "{text}");
    assert!(text.contains("0123456789ab"), "{text}");
    assert!(
        !text.contains("0123456789abc"),
        "the short form, as everywhere else: {text}"
    );
    assert!(text.contains("open redirect in /auth/callback"), "{text}");
}

/// A verdict that carried no report still leaves a trace: silence is a fact
/// about the run, and a block that says nothing is better than no block.
#[test]
fn a_verdict_with_no_report_still_leaves_a_block() {
    let dir = tempfile::tempdir().unwrap();
    let path = file(dir.path());
    followup::carry(
        &path,
        Role::Integrator,
        Verdict::Broken,
        "   \n ",
        "abc123def4567",
    )
    .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("carried no report"), "{text}");
}

/// A lift says who took the risk and why, and says in the file itself that
/// `VERDICT.json` has not moved: the verdict is the agent's answer, the lift
/// is the human's decision, and reading one as the other is the failure this
/// section exists to prevent.
#[test]
fn a_lift_names_the_human_the_reason_and_leaves_the_verdict_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = file(dir.path());
    followup::lifted(
        &path,
        "Arnaud",
        "open redirect in /auth/callback",
        "the callback is behind the VPN and the host allowlist is closed",
        "abc123def4567",
    )
    .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("Arnaud"), "{text}");
    assert!(text.contains("behind the VPN"), "{text}");
    assert!(text.contains("FINDINGS"), "{text}");
}
