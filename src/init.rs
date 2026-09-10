//! `hq init` — make an existing repository orchestrable (SPEC 4.2).
//!
//! Three rules, all from SPEC 3.3, and they are what makes this verb
//! replayable: it **creates what is absent**, it **never overwrites a file a
//! human is meant to edit** — it deposits its version beside it — and it
//! **keeps no manifest** and uninstalls nothing. Running it twice changes
//! nothing the second time.

use std::path::{Path, PathBuf};

/// What `init` did, or would do. Every path it touched, and every one it
/// deliberately did not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Created(PathBuf),
    /// It exists and belongs to a human; here is why it was left alone.
    LeftAlone(PathBuf, String),
    /// It exists and differs from what `hq` would write, so the suggestion
    /// went next to it under this name.
    DepositedBeside {
        kept: PathBuf,
        suggestion: PathBuf,
    },
}

impl Action {
    pub fn render(&self) -> String {
        match self {
            Action::Created(p) => format!("created  {}", p.display()),
            Action::LeftAlone(p, why) => format!("kept     {} — {why}", p.display()),
            Action::DepositedBeside { kept, suggestion } => format!(
                "kept     {} — its replacement is beside it, at {}",
                kept.display(),
                suggestion.display()
            ),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum InitError {
    #[error("{0} is not a directory")]
    NotADirectory(PathBuf),
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("no stack fragment for {0:?}; known: {1}")]
    UnknownStack(String, String),
}

/// The stacks `hq init` can write a fragment for. Three were promised (SPEC
/// 4.2); one is written, and asking for another says so rather than leaving
/// an empty directory that reads as configured.
pub const KNOWN_STACKS: [&str; 1] = ["rust"];

pub fn init(root: &Path, hq_root: &Path, stacks: &[String]) -> Result<Vec<Action>, InitError> {
    if !root.is_dir() {
        return Err(InitError::NotADirectory(root.to_path_buf()));
    }
    for stack in stacks {
        if !KNOWN_STACKS.contains(&stack.as_str()) {
            return Err(InitError::UnknownStack(
                stack.clone(),
                KNOWN_STACKS.join(", "),
            ));
        }
    }

    let mut actions = Vec::new();

    // The HQ first: everything hq owns lives there, outside the tree.
    for dir in ["state", "locks", "missions"] {
        let path = hq_root.join(dir);
        if !path.is_dir() {
            std::fs::create_dir_all(&path).map_err(|e| InitError::Io(path.clone(), e))?;
            actions.push(Action::Created(path));
        }
    }

    create_if_absent(root, "hq.yaml", &hq_yaml(stacks), &mut actions)?;
    create_if_absent(root, "AGENTS.md", AGENTS_MD, &mut actions)?;
    claude_md(root, &mut actions)?;
    gitattributes(root, &mut actions)?;

    for stack in stacks {
        let dir = root.join(".hq").join("stacks").join(stack);
        std::fs::create_dir_all(&dir).map_err(|e| InitError::Io(dir.clone(), e))?;
        for (name, body, executable) in fragment(stack) {
            let path = dir.join(name);
            if path.exists() {
                actions.push(Action::LeftAlone(
                    path,
                    "a stack fragment belongs to the project".to_string(),
                ));
                continue;
            }
            std::fs::write(&path, body).map_err(|e| InitError::Io(path.clone(), e))?;
            if executable {
                make_executable(&path)?;
            }
            actions.push(Action::Created(path));
        }
    }

    Ok(actions)
}

fn create_if_absent(
    root: &Path,
    name: &str,
    body: &str,
    actions: &mut Vec<Action>,
) -> Result<(), InitError> {
    let path = root.join(name);
    if path.exists() {
        actions.push(Action::LeftAlone(
            path,
            "it exists, and hq never overwrites a file a human edits".to_string(),
        ));
        return Ok(());
    }
    std::fs::write(&path, body).map_err(|e| InitError::Io(path.clone(), e))?;
    actions.push(Action::Created(path));
    Ok(())
}

/// `CLAUDE.md` is the one file whose *content* matters to another tool:
/// Claude Code reads it and not `AGENTS.md`, so it must import it (SPEC 4.1).
fn claude_md(root: &Path, actions: &mut Vec<Action>) -> Result<(), InitError> {
    let path = root.join("CLAUDE.md");
    if !path.exists() {
        std::fs::write(&path, "@AGENTS.md\n").map_err(|e| InitError::Io(path.clone(), e))?;
        actions.push(Action::Created(path));
        return Ok(());
    }
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let why = if text.contains("AGENTS.md") {
        "it already imports AGENTS.md".to_string()
    } else {
        // Not rewritten, and not silently accepted either: `hq check` is red
        // until a human adds the import, because otherwise the rules of the
        // place are never read by this harness.
        "it does not import AGENTS.md — add `@AGENTS.md` to it; `hq check` is \
         red until you do"
            .to_string()
    };
    actions.push(Action::LeftAlone(path, why));
    Ok(())
}

/// LF, whatever the machine that writes it (SPEC 4.2 bis). Absent, `hq`
/// writes it; present, `hq` does not decide for the project.
fn gitattributes(root: &Path, actions: &mut Vec<Action>) -> Result<(), InitError> {
    let path = root.join(".gitattributes");
    const BODY: &str =
        "# LF everywhere, whatever the machine that writes it.\n* text=auto eol=lf\n";
    if !path.exists() {
        std::fs::write(&path, BODY).map_err(|e| InitError::Io(path.clone(), e))?;
        actions.push(Action::Created(path));
        return Ok(());
    }
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    if text.contains("eol=lf") {
        actions.push(Action::LeftAlone(path, "it already pins LF".to_string()));
        return Ok(());
    }
    let suggestion = root.join(".gitattributes.hq");
    if !suggestion.exists() {
        std::fs::write(&suggestion, BODY).map_err(|e| InitError::Io(suggestion.clone(), e))?;
    }
    actions.push(Action::DepositedBeside {
        kept: path,
        suggestion,
    });
    Ok(())
}

fn hq_yaml(stacks: &[String]) -> String {
    let list = if stacks.is_empty() {
        "stacks: []".to_string()
    } else {
        format!(
            "stacks:\n{}",
            stacks
                .iter()
                .map(|s| format!("  - {s}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };
    format!(
        "# What this project declares to hq (SPEC 4.1). A mission header beats
# this file for what it redeclares.

harness: claude-code

# No agent may reach the forge: a slot's origin is unreachable, and an
# allowlist naming it would undo that. Put your forge's domain here.
forge: []

{list}

protected_branches:
  - main
  - dev

protected_paths:
  # Refused outright.
  refuse: []
  # Refused only where the file already exists on the base.
  refuse_if_exists: []
"
    )
}

const AGENTS_MD: &str = "# Working here

The rules of this place, read by whichever harness is driving. `CLAUDE.md`
imports this file; there is one set of rules, not one per tool.

Written by `hq init` as a starting point — replace it with the rules that
actually hold in this project.

## What an agent may not do

- Never push, never merge, never reach the forge. The human pushes.
- Never touch a protected path (see `hq.yaml`), and never a protected branch.
- Never ask a blocking question: in an autonomous run there is nobody to
  answer. Refusing is safe; asking is not.

## What a run must leave behind

- The lot committed and proved, or the failure stated plainly.
- A commitable tree.
- A `ÉTAT DE REPRISE` block at the top of `JOURNAL.md`, rewritten at every
  checkpoint and before stopping.
";

/// The files of a stack fragment: `(name, body, executable)`.
fn fragment(stack: &str) -> Vec<(&'static str, String, bool)> {
    match stack {
        "rust" => vec![
            (
                "allow.txt",
                "# What a Rust build must reach, and nothing else (SPEC 4.1 bis, rule 6).\n\
                 # One name per line; `#` comments. No forge: hq check is red if one appears.\n\
                 static.crates.io\n\
                 index.crates.io\n\
                 crates.io\n"
                    .to_string(),
                false,
            ),
            (
                "prepush.sh",
                "#!/bin/sh\n\
                 # The battery for a Rust project: what must be silent before anything\n\
                 # leaves a slot (SPEC 4.4).\n\
                 set -eu\n\n\
                 cargo fmt --all -- --check\n\
                 cargo clippy --all-targets --all-features -- -D warnings\n\
                 cargo test --all-features\n\
                 cargo deny check\n"
                    .to_string(),
                true,
            ),
            (
                "writable.txt",
                "# Directories an execution must be able to write when the tree is\n\
                 # mounted read-only (SPEC 4.2). One relative path per line.\n\
                 target\n"
                    .to_string(),
                false,
            ),
        ],
        _ => Vec::new(),
    }
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), InitError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| InitError::Io(path.to_path_buf(), e))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), InitError> {
    Ok(())
}
