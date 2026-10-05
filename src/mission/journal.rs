//! The line a coder run leaves to say how its lot ended (SPEC 4.1, 4.3).
//!
//! It lives in the `ÉTAT DE REPRISE` block, which the agent rewrites before
//! it stops anyway: one place to read, already demanded by the run contract.
//! The grammar:
//!
//! ```text
//! Lot: L2 — done
//! Lot: L2 — failed: the integration test X does not pass
//! Lot: L2 — awaits ruling: `src/lib.rs:3: replace + with -` changes nothing observable
//! ```
//!
//! The third line ends a run that cannot finish its lot without a ruling the
//! coder is forbidden to give — today, a survivor's equivalence (SPEC 4.4,
//! gate 7). The survivors it awaits a ruling on are named between
//! backquotes, and every backquoted span is read as one: [`awaitable`]
//! checks each against the campaign, so the line is no way out of a hard
//! lot.

/// How a run says its lot ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LotLine {
    Done {
        lot: String,
    },
    Failed {
        lot: String,
        reason: String,
    },
    /// The lot waits on a ruling only the HQ may give; `what` is never empty.
    AwaitsRuling {
        lot: String,
        what: String,
    },
}

/// What the run's word on its lot amounts to, read from the journal alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Judgement {
    Done,
    /// The run awaits a ruling, and names these survivors — not yet checked
    /// against the campaign: that is [`awaitable`]'s.
    AwaitsRuling {
        what: String,
        survivors: Vec<String>,
    },
    /// A failed attempt, and why, in words the next run and the human can
    /// act on.
    Failed(String),
}

/// The last `Lot:` line of the journal's resume block, or `None` when there
/// is no block or no line in it that reads. Only the block: a journal names
/// every lot it ever finished further down, and reading the whole file would
/// take the first lot's `done` for the second's.
pub fn lot_line(journal: &str) -> Option<LotLine> {
    let block = crate::gate::resume_block(journal)?;
    block.lines().rev().find_map(parse)
}

/// Whether the journal says `lot` is done, awaits a ruling, or — when it
/// says neither — why that is a failed attempt.
pub fn judge(journal: &str, lot: &str) -> Judgement {
    match lot_line(journal) {
        Some(LotLine::Done { lot: said }) if said == lot => Judgement::Done,
        Some(LotLine::AwaitsRuling { lot: said, what }) if said == lot => {
            let survivors = named(&what);
            Judgement::AwaitsRuling { what, survivors }
        }
        Some(LotLine::Failed { lot: said, reason }) if said == lot => {
            Judgement::Failed(if reason.is_empty() {
                format!("the run said lot {lot} failed, without saying why")
            } else {
                format!("the run said lot {lot} failed: {reason}")
            })
        }
        Some(
            LotLine::Done { lot: said }
            | LotLine::Failed { lot: said, .. }
            | LotLine::AwaitsRuling { lot: said, .. },
        ) => Judgement::Failed(format!(
            "the resume block reports lot {said}, and this run was lot {lot}"
        )),
        None => Judgement::Failed(format!(
            "the resume block carries no `Lot: {lot} — done` line: a run that does not say its \
             lot is done has not finished it"
        )),
    }
}

/// Whether a ruling may be awaited on `survivors`: at least one is named, and
/// each is a survivor of the current campaign that has no outcome yet —
/// `open`. Anything else is a failed attempt, and the reason says which.
///
/// This is what keeps the third line from being a free exit: a coder may
/// only hand over for a ruling it is forbidden to give, and the only one
/// today is a survivor's equivalence (SPEC 4.4, gate 7).
pub fn awaitable(lot: &str, survivors: &[String], open: &[String]) -> Result<(), String> {
    if survivors.is_empty() {
        return Err(format!(
            "the run said lot {lot} awaits a ruling, and named no survivor between \
             backquotes: only a survivor's equivalence awaits a ruling, and a line that \
             names none is not one"
        ));
    }
    let not_open: Vec<&str> = survivors
        .iter()
        .filter(|id| !open.contains(id))
        .map(String::as_str)
        .collect();
    if !not_open.is_empty() {
        return Err(format!(
            "the run said lot {lot} awaits a ruling on {}, and {} not a survivor of \
             MUTANTS.json still without an outcome: a ruling is awaited only on a \
             survivor nobody has answered",
            quoted(&not_open),
            if not_open.len() == 1 {
                "that is"
            } else {
                "those are"
            }
        ));
    }
    Ok(())
}

/// Every backquoted span of `what`, trimmed, in order, each once.
fn named(what: &str) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for id in what.split('`').skip(1).step_by(2).map(str::trim) {
        if !id.is_empty() && !ids.iter().any(|seen| seen == id) {
            ids.push(id.to_string());
        }
    }
    ids
}

fn quoted(ids: &[&str]) -> String {
    ids.iter()
        .map(|id| format!("`{id}`"))
        .collect::<Vec<_>>()
        .join(", ")
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
    if let Some(after) = strip_prefix_ignoring_case(status, "awaits ruling") {
        // Something has to be awaited: a bare `awaits ruling` is no line.
        if !(after.starts_with(':') || after.starts_with(char::is_whitespace)) {
            return None;
        }
        let what = after.trim_start();
        let what = what.strip_prefix(':').unwrap_or(what).trim();
        if what.is_empty() {
            return None;
        }
        return Some(LotLine::AwaitsRuling {
            lot,
            what: what.to_string(),
        });
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
