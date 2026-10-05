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
//! `--json` output keeps its own escaping, which already does this.

/// `text` with every control character replaced by its visible escape
/// (`\u{1b}`): the C0 controls and DEL, ESC and so every CSI and OSC, the C1
/// controls (which a terminal may read as CSI or OSC by themselves), and the
/// bidirectional controls that make a line read in another order than it is
/// written. Line breaks and tabs are kept — this is for text that may span
/// lines — and a `\r\n` is read as the line break it means.
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

/// A character that tells a terminal what to do rather than what to show.
fn unsafe_to_print(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
        )
}
