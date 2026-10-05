//! Text an agent or a commit author wrote, made safe to print: the one
//! function every outlet goes through. The outlets themselves are tested
//! where they live (`wait`, `monitor`, `flow`, `followup`, `push`, `logs`).

mod common;

use nunki::text::{blank, brief, one_line, printable};

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

/// Blank is what a reader sees: nothing. Whitespace, controls, every
/// invisible character `printable` escapes, and the two joiners on their
/// own are nothing; one visible character, an emoji held together by a
/// joiner included, is something.
#[test]
fn blank_text_is_text_a_reader_sees_nothing_of() {
    for nothing in [
        "",
        " \t\n",
        "\u{200b}",
        "\u{200b} \u{2060}\u{feff}",
        "\u{e0041}\u{e0042}",
        "\u{202e}\u{2028}",
        "\u{200d}",
        "\u{200c}",
        "\u{1b}\u{7}",
    ] {
        assert!(blank(nothing), "{nothing:?} shows nothing");
    }
    for something in ["a", "\u{200b}a", "é", "👩\u{200d}💻", "-"] {
        assert!(!blank(something), "{something:?} shows something");
    }
}

/// The characters that show nothing of their own are shown escaped, each at
/// both ends of its range, and a text of nothing but them is blank: the soft
/// hyphen, the Mongolian vowel separator, the combining grapheme joiner, the
/// variation selectors, the Hangul fillers and the blank braille pattern.
#[test]
fn every_character_that_looks_blank_is_shown_escaped_and_is_blank() {
    for (c, escape) in [
        ('\u{00ad}', "\\u{ad}"),
        ('\u{180e}', "\\u{180e}"),
        ('\u{034f}', "\\u{34f}"),
        ('\u{fe00}', "\\u{fe00}"),
        ('\u{fe0f}', "\\u{fe0f}"),
        ('\u{3164}', "\\u{3164}"),
        ('\u{115f}', "\\u{115f}"),
        ('\u{1160}', "\\u{1160}"),
        ('\u{ffa0}', "\\u{ffa0}"),
        ('\u{2800}', "\\u{2800}"),
    ] {
        assert_eq!(printable(&format!("a{c}b")), format!("a{escape}b"));
        assert!(blank(&format!(" {c}{c} ")), "{escape} shows nothing");
    }
    // Hidden bytes in variation selectors after a visible letter.
    assert_eq!(printable("ok\u{fe01}\u{fe0e}"), "ok\\u{fe01}\\u{fe0e}");

    // And beside each range, what does show stays as it was written, and is
    // not blank — the joiner inside an emoji sequence included.
    for text in [
        "\u{00ac}\u{00ae}",
        "\u{fe10}",
        "\u{3163}\u{3165}",
        "\u{115e}",
        "\u{ff9f}\u{ffa1}",
        "\u{27ff}\u{2801}",
        "👩\u{200d}💻",
        "a",
    ] {
        assert_eq!(printable(text), text);
        assert!(!blank(text), "{text:?} shows something");
    }
}
