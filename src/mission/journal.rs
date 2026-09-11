//! The line a coder run leaves to say how its lot ended (SPEC 4.1, 4.3).
//!
//! It lives in the `ÉTAT DE REPRISE` block, which the agent rewrites before
//! it stops anyway: one place to read, already demanded by the run contract.
//! The grammar, decided by Arnaud on 2026-09-11:
//!
//! ```text
//! Lot: L2 — done
//! Lot: L2 — failed: the integration test X does not pass
//! ```

/// How a run says its lot ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LotLine {
    Done { lot: String },
    Failed { lot: String, reason: String },
}

/// The last `Lot:` line of the journal's resume block, or `None` when there
/// is no block or no line in it that reads. Only the block: a journal names
/// every lot it ever finished further down, and reading the whole file would
/// take the first lot's `done` for the second's.
pub fn lot_line(journal: &str) -> Option<LotLine> {
    let block = crate::gate::resume_block(journal)?;
    block.lines().rev().find_map(parse)
}

/// Whether the journal says `lot` is done — or, when it does not, why that
/// is a failed attempt, in words the next run and the human can act on.
pub fn judge(journal: &str, lot: &str) -> Result<(), String> {
    match lot_line(journal) {
        Some(LotLine::Done { lot: said }) if said == lot => Ok(()),
        Some(LotLine::Failed { lot: said, reason }) if said == lot => Err(if reason.is_empty() {
            format!("the run said lot {lot} failed, without saying why")
        } else {
            format!("the run said lot {lot} failed: {reason}")
        }),
        Some(LotLine::Done { lot: said } | LotLine::Failed { lot: said, .. }) => Err(format!(
            "the resume block reports lot {said}, and this run was lot {lot}"
        )),
        None => Err(format!(
            "the resume block carries no `Lot: {lot} — done` line: a run that does not say its \
             lot is done has not finished it"
        )),
    }
}

/// One line, read as the grammar says. A list marker before it is
/// tolerated, and so are backquotes around the identifier; the separator is
/// an em dash, an en dash or a hyphen **between spaces** — lot identifiers
/// carry hyphens (`volet-1`), so a bare one separates nothing.
pub fn parse(line: &str) -> Option<LotLine> {
    let line = line.trim();
    let line = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .unwrap_or(line)
        .trim_start();
    let rest = strip_prefix_ignoring_case(line, "lot:")?;
    let (at, width) = [" — ", " – ", " - "]
        .iter()
        .filter_map(|sep| rest.find(sep).map(|at| (at, sep.len())))
        .min_by_key(|(at, _)| *at)?;
    let lot = rest[..at].trim().trim_matches('`').trim().to_string();
    if lot.is_empty() {
        return None;
    }
    let status = rest[at + width..].trim();
    if status.trim_end_matches('.').eq_ignore_ascii_case("done") {
        return Some(LotLine::Done { lot });
    }
    let after = strip_prefix_ignoring_case(status, "failed")?;
    if !(after.is_empty() || after.starts_with(':') || after.starts_with(char::is_whitespace)) {
        return None;
    }
    let reason = after.trim_start();
    let reason = reason
        .strip_prefix(':')
        .unwrap_or(reason)
        .trim()
        .to_string();
    Some(LotLine::Failed { lot, reason })
}

fn strip_prefix_ignoring_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}
