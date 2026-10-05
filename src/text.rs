//! Text an agent or a commit author wrote, made safe to print.
//!
//! A verdict's report, a volet's cause, a survivor's id, a commit subject, a
//! run's stream: `nunki` prints them to the operator's terminal and writes
//! them into `FOLLOWUP_HQ.md`, and none of them is `nunki`'s. A control
//! character among them is an instruction to the terminal, not text — an
//! escape sequence can erase the line it sits on and paint another (a
//! forged "verified … `nunki push`" over a line that says findings), an OSC
//! can write the clipboard or plant a link — and a reader who sees the result
//! has no way to tell. So every such text goes through [`printable`] before
//! it reaches a terminal or a file a human reads, and what was a control
//! character is shown as its escape, `\u{1b}`, rather than dropped: the
//! reader sees that something was there.
//!
//! `nunki mission wait --json` is no exception: its fields are the status
//! line's, made one line by [`one_line`] before they are serialised, so it
//! carries the same `\u{1b}` escapes as the line, and JSON's own escaping
//! applies to what is left.

/// `text` with every control character replaced by its visible escape
/// (`\u{1b}`): the C0 controls and DEL, ESC and so every CSI and OSC, the C1
/// controls (which a terminal may read as CSI or OSC by themselves), the
/// bidirectional controls that make a line read in another order than it is
/// written, and what a reader cannot see: zero-width and invisible
/// characters, the byte-order mark, the tag block, the line and paragraph
/// separators, the variation selectors and the characters that look blank
/// (the soft hyphen, the Hangul fillers, the blank braille pattern). Every
/// visible letter and accent is kept, every emoji too but for a variation
/// selector it carries, and so is the zero-width joiner that holds an emoji
/// sequence together. Line breaks and tabs are kept — this is for text that
/// may span lines — and a `\r\n` is read as the line break it means.
pub fn printable(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' | '\t' => out.push(c),
            c if unsafe_to_print(c) => out.extend(c.escape_unicode()),
            c => out.push(c),
        }
    }
    out
}

/// `text` on one line, and printable: every run of whitespace, line breaks
/// included, made one space, then [`printable`]. A status line that broke
/// in two would be read as two.
pub fn one_line(text: &str) -> String {
    printable(&text.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// [`one_line`], cut after `chars` characters of the text with an ellipsis
/// when it was longer. The cut is made before the escapes are spelled, so it
/// never splits one.
pub fn brief(text: &str, chars: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(chars) {
        Some((at, _)) => format!("{}…", printable(&flat[..at])),
        None => printable(&flat),
    }
}

/// Whether `text` says nothing a reader can see: empty once whitespace and
/// every character [`printable`] escapes as invisible or as a control are
/// removed. The two joiners count as nothing too — [`printable`] keeps the
/// zero-width joiner for the emoji it holds together, but on their own
/// neither shows anything. What a check asking "is there a title, is there a
/// reason" reads, since `str::trim` leaves a zero-width space standing.
pub fn blank(text: &str) -> bool {
    text.chars()
        .all(|c| c.is_whitespace() || unsafe_to_print(c) || matches!(c, '\u{200c}' | '\u{200d}'))
}

/// A character that tells a terminal what to do rather than what to show, or
/// that a reader cannot see at all.
fn unsafe_to_print(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            // Bidirectional controls: the line reads in another order than
            // it is written.
            '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
            // Invisible: a zero-width space, the word joiner and the
            // invisible operators, the byte-order mark, and the tag block,
            // which spells hidden text a model reads and a human does not.
            // The zero-width joiner (U+200D) is not among them: it holds
            // emoji sequences together.
            | '\u{200b}' | '\u{2060}'..='\u{2064}' | '\u{feff}' | '\u{e0000}'..='\u{e007f}'
            // Characters that show nothing of their own: the soft hyphen,
            // the Mongolian vowel separator, the combining grapheme joiner,
            // the variation selectors (which also spell hidden bytes after
            // a visible character, so an emoji's presentation selector is
            // shown escaped too), and the blank-looking letters — the
            // Hangul fillers and the blank braille pattern.
            | '\u{00ad}' | '\u{180e}' | '\u{034f}' | '\u{fe00}'..='\u{fe0f}'
            | '\u{3164}' | '\u{115f}' | '\u{1160}' | '\u{ffa0}' | '\u{2800}'
            // Line and paragraph separators: a break some readers make and a
            // terminal does not. `\n` is the line break this text keeps.
            | '\u{2028}' | '\u{2029}'
        )
}
