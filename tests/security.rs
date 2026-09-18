//! The contract gate 8 reads (SPEC 4.4): seven fields, and `nunki` knows no
//! tool that produces them.

use nunki::security::{Finding, Kind, read};

/// The lines `security.sh` actually printed, measured on 2026-09-17 against
/// cargo-deny 0.20.2 and trufflehog 3.97.5. A reader tested on invented input
/// is a reader tested against its author's idea of the format.
const MEASURED: &str = r#"
some progress the script printed on stdout
{"id":"RUSTSEC-2021-0139","kind":"unmaintained","where":"ansi_term 0.12.1","via":"","fix":"","accepted":"","was_at_base":true}
{"id":"RUSTSEC-2020-0159","kind":"vulnerability","where":"chrono 0.4.19","via":"","fix":">=0.4.20","accepted":"localtime_r: pas atteignable ici","was_at_base":true}
{"id":"RUSTSEC-2020-0071","kind":"vulnerability","where":"time 0.1.45","via":"chrono","fix":">=0.2.23","accepted":"","was_at_base":true}
{"id":"1d334847:leaked.py:Github:1","kind":"secret","where":"leaked.py:1","via":"","fix":"","accepted":"","was_at_base":false}
"#;

#[test]
fn the_reader_takes_what_the_script_measured() {
    let findings = read(MEASURED).unwrap();

    assert_eq!(findings.len(), 4, "{findings:?}");
    // Progress on stdout is not a finding, and not an error either: a script
    // that says what it is doing must not fail a gate for it.
    assert_eq!(findings[0].kind, Kind::Unmaintained);
    assert_eq!(
        findings[2].via, "chrono",
        "a transitive finding names its parent"
    );
    assert_eq!(findings[1].at, "chrono 0.4.19", "the wire name is `where`");
    assert_eq!(findings[3].kind, Kind::Secret);
}

/// What the branch brought and nobody accepted. Everything else is a constat:
/// what the repository already carried is not this branch's to answer.
#[test]
fn only_what_the_branch_brought_and_nobody_accepted_blocks() {
    let findings = read(MEASURED).unwrap();

    let blocking: Vec<&str> = findings
        .iter()
        .filter(|f| f.blocks())
        .map(|f| f.id.as_str())
        .collect();

    assert_eq!(
        blocking,
        vec!["1d334847:leaked.py:Github:1"],
        "{findings:?}"
    );
}

/// An acceptance a fix has overtaken. This is what replaces an expiry date:
/// the condition is mechanical rather than guessed.
#[test]
fn an_acceptance_a_fix_has_overtaken_is_stale() {
    let findings = read(MEASURED).unwrap();

    let stale: Vec<&str> = findings
        .iter()
        .filter(|f| f.stale())
        .map(|f| f.id.as_str())
        .collect();

    assert_eq!(stale, vec!["RUSTSEC-2020-0159"], "{findings:?}");
    // And it does not block: it was accepted, so the mission goes on — what
    // it owes is applying the fix, not stopping.
    assert!(!findings[1].blocks());
}

/// An advisory that is not a vulnerability, and a secret ruled a false
/// positive, have no `fix` **by nature**. Their acceptance is permanent, and
/// saying so beats pretending it will expire.
#[test]
fn an_acceptance_no_fix_can_overtake_is_permanent() {
    let one = &read(
        r#"{"id":"RUSTSEC-2024-0436","kind":"unmaintained","where":"paste 1.0.15","via":"alloy","fix":"","accepted":"archivé en amont","was_at_base":true}"#,
    )
    .unwrap()[0];

    assert!(one.permanent());
    assert!(!one.stale());
    assert!(!one.blocks());
    assert_eq!(one.via, "alloy", "the parent is what could be replaced");
}

/// A class this version has no name for is read rather than refused. A gate
/// that rejected a line it did not understand would make a new stack's first
/// run a parse error.
#[test]
fn a_class_this_version_does_not_know_is_read_and_not_refused() {
    let one = &read(
        r#"{"id":"x","kind":"licence-incompatible","where":"somewhere","fix":"","accepted":"","was_at_base":false}"#,
    )
    .unwrap()[0];

    assert_eq!(one.kind, Kind::Other);
    assert!(one.blocks(), "an unknown class still counts");
}

/// A fragment writing a broken contract is told, not silently dropped —
/// unlike prose, which is ignored on purpose.
#[test]
fn an_object_that_cannot_be_read_is_an_error_and_prose_is_not() {
    assert!(read("building…\nrunning the audit\n").unwrap().is_empty());

    let error = read(r#"{"id":"x","kind":"vulnerability""#).unwrap_err();

    assert!(error.to_string().contains(r#"{"id":"x""#), "{error}");
}

/// Absent fields are the common case: a secret carries no `via` and no `fix`,
/// and a stack may write neither.
#[test]
fn what_a_finding_does_not_say_is_empty_and_not_a_failure() {
    let one = &read(r#"{"id":"x","kind":"secret"}"#).unwrap()[0];

    assert_eq!(one.at, "");
    assert_eq!(one.via, "");
    assert_eq!(one.fix, "");
    assert!(!one.was_at_base);
    assert!(
        one.blocks(),
        "unknown provenance is the branch's until said"
    );
}

/// The wire names are the contract's, so a finding round-trips through the
/// file a human reads.
#[test]
fn a_finding_round_trips_through_its_own_spelling() {
    let one: Finding = read(MEASURED).unwrap().remove(2);

    let text = serde_json::to_string(&one).unwrap();

    assert!(text.contains(r#""where":"time 0.1.45""#), "{text}");
    assert!(text.contains(r#""kind":"vulnerability""#), "{text}");
    assert_eq!(read(&text).unwrap()[0], one);
}
