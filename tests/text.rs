//! Text an agent or a commit author wrote, made safe to print: the one
//! function every outlet goes through. The outlets themselves are tested
//! where they live (`wait`, `monitor`, `flow`, `followup`, `push`, `logs`).

mod common;

use nunki::text::{brief, one_line, printable};

#[test]
fn every_control_an_agent_could_send_a_terminal_is_shown_escaped() {
    let out = printable(common::HOSTILE);
    common::assert_printable(&out, "printable");
    for escape in [
        "\\u{1b}[2K",
        "\\u{1b}]52;c;",
        "\\u{7}",
        "\\u{9d}",
        "\\u{9c}",
        "\\u{9b}8m",
        "\\u{d}over",
        "\\u{7f}",
        "\\u{202e}",
    ] {
        assert!(out.contains(escape), "{escape} in {out}");
    }
    // What was text stays text, untouched.
    assert!(out.contains("m1 · verified · awaits the human: `nunki push m1 --yes`"));
}

#[test]
fn line_breaks_and_tabs_are_kept_and_a_crlf_is_a_line_break() {
    assert_eq!(printable("one\n\ttwo\r\nthree"), "one\n\ttwo\nthree");
    assert_eq!(printable("déjà — vu · ok"), "déjà — vu · ok");
    assert_eq!(printable(""), "");
}

#[test]
fn one_line_folds_whitespace_and_escapes_the_rest() {
    assert_eq!(one_line("  a\n b\r\n\tc  "), "a b c");
    assert_eq!(one_line("a\u{1b}[2Kb"), "a\\u{1b}[2Kb");
    let out = one_line(common::HOSTILE);
    assert!(!out.contains('\n'));
    common::assert_printable(&out, "one_line");
}

/// The cut is made on the text and before the escapes are spelled: an
/// escape is never split, and a short text keeps no ellipsis.
#[test]
fn brief_cuts_the_text_and_never_an_escape() {
    assert_eq!(brief("ab\u{1b}cd", 3), "ab\\u{1b}…");
    assert_eq!(brief("ab\u{1b}", 3), "ab\\u{1b}");
    assert_eq!(brief("é".repeat(5).as_str(), 2), "éé…");
    common::assert_printable(&brief(common::HOSTILE, 40), "brief");
}
