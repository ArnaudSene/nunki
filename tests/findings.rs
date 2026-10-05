//! Lifting a security verdict (SPEC 4.5), and what the HQ carries to the
//! coder in `FOLLOWUP_HQ.md`.

use std::path::Path;

use nunki::followup;
use nunki::harness::Role;
use nunki::mission::{LowFinding, Severity, Verdict, VerdictFile, Withheld};

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
        "Alex Martin",
        "open redirect in /auth/callback",
        "the callback is behind the VPN and the host allowlist is closed",
        "abc123def4567",
    )
    .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("Alex Martin"), "{text}");
    assert!(text.contains("behind the VPN"), "{text}");
    assert!(text.contains("FINDINGS"), "{text}");
}

// --- severities, and the lift nunki records itself (SPEC 4.5) --------------

/// A security verdict as the agent writes it, with `findings` spliced in as
/// raw JSON — or left out when `None`.
fn verdict_with(verdict: &str, findings: Option<&str>) -> VerdictFile {
    let findings = findings
        .map(|f| format!(",\"findings\":{f}"))
        .unwrap_or_default();
    serde_json::from_str(&format!(
        "{{\"role\":\"Security\",\"verdict\":\"{verdict}\",\"head\":\"abc\",\
         \"date\":\"2026-10-05T00:00:00Z\",\"report\":\"what was attacked\"{findings}}}"
    ))
    .unwrap_or_else(|e| panic!("a verdict with findings {findings} does not read: {e}"))
}

/// A report whose every finding is LOW or INFO, each with its reason, is
/// lifted, finding by finding, in the agent's words.
#[test]
fn a_findings_verdict_with_only_low_and_info_is_lifted_by_nunki() {
    let file = verdict_with(
        "FINDINGS",
        Some(
            r#"[{"severity":"LOW","title":"verbose error page","why_acceptable":"it names no path"},
                {"severity":"INFO","title":"no security.txt","why_acceptable":"nothing is exposed by its absence"}]"#,
        ),
    );
    let lifted = file.automatic_lift().unwrap();
    assert_eq!(
        lifted,
        vec![
            LowFinding {
                severity: Severity::Low,
                title: "verbose error page".into(),
                why_acceptable: "it names no path".into(),
            },
            LowFinding {
                severity: Severity::Info,
                title: "no security.txt".into(),
                why_acceptable: "nothing is exposed by its absence".into(),
            },
        ]
    );
    assert_eq!(
        lifted[0].line(),
        "LOW — verbose error page: it names no path"
    );
}

/// One MEDIUM, or one HIGH, anywhere in the list, and nothing is lifted:
/// the whole verdict is left to a human, the LOW beside it included.
#[test]
fn one_medium_or_high_finding_leaves_the_whole_verdict_to_a_human() {
    for (severity, list) in [
        (
            Severity::Medium,
            r#"[{"severity":"LOW","title":"a","why_acceptable":"fine"},{"severity":"MEDIUM","title":"b","why_acceptable":"said anyway"}]"#,
        ),
        (
            Severity::High,
            r#"[{"severity":"HIGH","title":"b"},{"severity":"INFO","title":"a","why_acceptable":"fine"}]"#,
        ),
    ] {
        let file = verdict_with("FINDINGS", Some(list));
        assert_eq!(
            file.automatic_lift(),
            Err(Withheld::TooSevere {
                title: "b".into(),
                severity,
            }),
            "{list}"
        );
    }
}

/// Fail closed: whatever nunki cannot read for certain stops at `Findings`
/// as it did before severities existed — never a guess, and never a verdict
/// file refused for it, which would cost the agent an attempt instead.
#[test]
fn a_ranking_nunki_cannot_read_for_certain_lifts_nothing() {
    let cases: [(Option<&str>, Withheld); 10] = [
        (None, Withheld::NoList),
        (Some("null"), Withheld::NoList),
        (Some("[]"), Withheld::Empty),
        (Some(r#""LOW""#), Withheld::Unreadable),
        (
            Some(r#"[{"severity":1,"title":"a"}]"#),
            Withheld::Unreadable,
        ),
        (
            Some(r#"[{"severity":"CRITICAL","title":"a","why_acceptable":"x"}]"#),
            Withheld::UnknownSeverity {
                title: "a".into(),
                severity: "CRITICAL".into(),
            },
        ),
        (
            Some(r#"[{"severity":"low","title":"a","why_acceptable":"x"}]"#),
            Withheld::UnknownSeverity {
                title: "a".into(),
                severity: "low".into(),
            },
        ),
        (
            Some(r#"[{"severity":"LOW","title":"a"}]"#),
            Withheld::NoReason {
                title: "a".into(),
                severity: Severity::Low,
            },
        ),
        (
            Some(r#"[{"severity":"INFO","title":"a","why_acceptable":"  "}]"#),
            Withheld::NoReason {
                title: "a".into(),
                severity: Severity::Info,
            },
        ),
        (
            Some(r#"[{"severity":"LOW","title":" ","why_acceptable":"x"}]"#),
            Withheld::Untitled { index: 1 },
        ),
    ];
    for (findings, why) in cases {
        let file = verdict_with("FINDINGS", findings);
        assert_eq!(file.automatic_lift(), Err(why), "{findings:?}");
    }
}

/// A `CLEAR` is not lifted, whatever its list says: there is nothing to
/// lift, and a lift recorded on it would be a decision about nothing.
#[test]
fn only_a_findings_verdict_is_ever_lifted() {
    let file = verdict_with(
        "CLEAR",
        Some(r#"[{"severity":"LOW","title":"a","why_acceptable":"x"}]"#),
    );
    assert_eq!(
        file.automatic_lift(),
        Err(Withheld::NotFindings(Verdict::Clear))
    );
}

/// A verdict written before severities existed still reads, as one whose
/// findings are left to a human.
#[test]
fn an_old_verdict_file_still_reads() {
    let file: VerdictFile = serde_json::from_str(
        r#"{"role":"Security","verdict":"FINDINGS","head":"abc","date":"2026-09-10T00:00:00Z","report":"an open redirect"}"#,
    )
    .unwrap();
    assert_eq!(file.findings, None);
    assert_eq!(file.automatic_lift(), Err(Withheld::NoList));
}

/// The severities, and the line between them: LOW and INFO below it,
/// MEDIUM and HIGH above.
#[test]
fn severities_are_spelled_exactly_and_only_low_and_info_are_lifted() {
    for (text, severity, lifted) in [
        ("HIGH", Severity::High, false),
        ("MEDIUM", Severity::Medium, false),
        ("LOW", Severity::Low, true),
        ("INFO", Severity::Info, true),
    ] {
        assert_eq!(Severity::parse(text), Some(severity));
        assert_eq!(severity.to_string(), text);
        assert_eq!(severity.lifted_by_nunki(), lifted, "{text}");
    }
    for unknown in ["", "Low", "CRITICAL", "LOW "] {
        assert_eq!(Severity::parse(unknown), None, "{unknown:?}");
    }
}

/// The follow-up says the lift is nunki's, on which commit, and lists each
/// finding with the agent's reason — and that `VERDICT.json` has not moved.
#[test]
fn a_lift_by_nunki_is_said_as_nunkis_with_every_finding() {
    let dir = tempfile::tempdir().unwrap();
    let path = file(dir.path());
    followup::lifted_by_nunki(
        &path,
        &[
            "LOW — verbose error page: it names no path".to_string(),
            "INFO — no security.txt: nothing is exposed".to_string(),
        ],
        "0123456789abcdef0123456789abcdef01234567",
    )
    .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("nunki lifted the security verdict on 0123456789ab (LOW/INFO)"),
        "{text}"
    );
    assert!(
        text.contains("- LOW — verbose error page: it names no path"),
        "{text}"
    );
    assert!(
        text.contains("- INFO — no security.txt: nothing is exposed"),
        "{text}"
    );
    assert!(text.contains("`VERDICT.json` stays `FINDINGS`"), "{text}");
}

/// What `push` and `mission status` print of a lift: its findings when
/// nunki made it, nothing when a human did — a human's reason is not a
/// list of findings nunki accepted.
#[test]
fn only_a_lift_by_nunki_is_listed_as_one() {
    let mut lift = nunki::state::Accepted {
        finding: None,
        why: "LOW — a: x\nINFO — b: y".into(),
        who: nunki::state::NUNKI.into(),
        head: "abc".into(),
        date: "2026-10-05T00:00:00Z".into(),
        by_nunki: true,
    };
    assert_eq!(
        nunki::findings::lifted_by_nunki(&lift),
        vec!["LOW — a: x".to_string(), "INFO — b: y".to_string()]
    );
    lift.by_nunki = false;
    assert!(nunki::findings::lifted_by_nunki(&lift).is_empty());
}

/// A title or a reason made only of characters a reader cannot see is no
/// title and no reason: `str::trim` leaves a zero-width space standing, and
/// the check reads what shows (SPEC 4.5, "nothing is accepted on a guess").
#[test]
fn a_title_or_reason_made_of_invisible_characters_is_none() {
    let one = |severity: &str, title: &str, why: &str| serde_json::json!({"severity": severity, "title": title, "why_acceptable": why});
    let listed = |findings: Vec<serde_json::Value>| serde_json::Value::Array(findings).to_string();
    for (findings, why) in [
        (
            listed(vec![one("LOW", "\u{200b}", "x")]),
            Withheld::Untitled { index: 1 },
        ),
        (
            listed(vec![
                one("LOW", "a", "x"),
                one("INFO", " \u{2060}\u{feff}\u{200d} ", "x"),
            ]),
            Withheld::Untitled { index: 2 },
        ),
        (
            listed(vec![one("LOW", "a", "\u{200b}")]),
            Withheld::NoReason {
                title: "a".into(),
                severity: Severity::Low,
            },
        ),
        (
            listed(vec![one("INFO", "a", "\u{e0041}\u{2028}\u{200c}")]),
            Withheld::NoReason {
                title: "a".into(),
                severity: Severity::Info,
            },
        ),
    ] {
        let file = verdict_with("FINDINGS", Some(&findings));
        assert_eq!(file.automatic_lift(), Err(why), "{findings}");
    }

    // And a visible character beside the invisible ones is a title.
    let findings = listed(vec![one("LOW", "\u{200b}a", "\u{200b}x")]);
    let file = verdict_with("FINDINGS", Some(&findings));
    assert!(file.automatic_lift().is_ok(), "{findings}");
}
