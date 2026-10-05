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

/// What a reader cannot see is shown, each character at both ends of its
/// range: a zero-width space, the word joiner and the invisible operators,
/// the byte-order mark, the tag block, the line and paragraph separators.
#[test]
fn every_invisible_character_is_shown_escaped() {
    for (c, escape) in [
        ('\u{200b}', "\\u{200b}"),
        ('\u{2060}', "\\u{2060}"),
        ('\u{2064}', "\\u{2064}"),
        ('\u{feff}', "\\u{feff}"),
        ('\u{e0000}', "\\u{e0000}"),
        ('\u{e0041}', "\\u{e0041}"),
        ('\u{e007f}', "\\u{e007f}"),
        ('\u{2028}', "\\u{2028}"),
        ('\u{2029}', "\\u{2029}"),
    ] {
        assert_eq!(printable(&format!("a{c}b")), format!("a{escape}b"));
    }
    // Hidden text in tags: "hi" spelled where nothing shows.
    assert_eq!(
        printable("ok\u{e0001}\u{e0068}\u{e0069}\u{e007f}"),
        "ok\\u{e0001}\\u{e0068}\\u{e0069}\\u{e007f}"
    );
    // A separator in a line folds like any whitespace.
    assert_eq!(one_line("a\u{2028}b\u{2029}c"), "a b c");
}

/// And what a reader does see stays as it was written: letters and accents
/// of any script, emoji, the zero-width joiner that holds an emoji sequence
/// together, and the characters just outside each range escaped.
#[test]
fn every_visible_letter_accent_and_emoji_is_kept() {
    for text in [
        "déjà vu, Ærøskøbing, Straße, ﬁ",
        "Ελληνικά, кириллица, 日本語, 한국어, עברית, العربية",
        "e\u{301} n\u{303}",
        "👍 🇫🇷 👍🏽",
        "👩\u{200d}💻 👨\u{200d}👩\u{200d}👧",
    ] {
        assert_eq!(printable(text), text);
        assert_eq!(brief(text, 200), text);
    }
    // Next to each range, and kept (two of them are spaces, which `brief`
    // folds as it folds any whitespace).
    let edges = "\u{200a}\u{200c}\u{200d}\u{2027}\u{205f}\u{2065}\u{fefe}\u{e0080}";
    assert_eq!(printable(edges), edges);
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
