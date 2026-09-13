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
    // A symlink to AGENTS.md is the other documented shape, and reading
    // through it would ask whether AGENTS.md mentions its own name — which is
    // not the question. `hq check` already knew this; `init` did not, and
    // said so on this very repository.
    if path.is_symlink() {
        actions.push(Action::LeftAlone(
            path,
            "it is a link to AGENTS.md".to_string(),
        ));
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

# How hq starts the application for the integrator and the security agent
# (SPEC 4.2). Absent, the stack's own `run.sh` is used; `none` for a library,
# whose security agent works on the code and the build artefact. A mission
# header may refine it.
# run: none

# Which model the agents run on, when the harness takes one. Absent, the
# harness keeps its own default — `hq logs` names the one a run used. A
# mission header may refine it. hq checks no name: what a name means is the
# harness's business, and the harness refuses what it does not know.
# model: claude-sonnet-5

# What the harness does with a permission it would otherwise ask about.
# Nobody is there to ask in an autonomous container, so the only question is
# which way the silence falls. `auto` lets the harness's own safety checks
# decide and nudges the agent to keep working rather than stop for a
# clarification; `dontAsk` allows only what is pre-approved and denies
# everything else. Whatever the mode, hq refuses the prompt itself, so a run
# never waits for a human who is not there.
permission_mode: auto

# The project's own Compose file, whose services and networks hq merges into
# every system profile. They are lifted once per slot and never stopped
# between two profiles, so what the integrator laid down survives.
# services_file: compose.yaml

protected_branches:
  - main
  - dev

# Who refuses a push to a protected branch besides hq's own gate: `forge`
# (the default) — `hq check` asks the forge, and an unprotected branch is red.
# `by_hand` when the forge cannot, as on a private repository on GitHub's
# free plan, and you hold the rule yourself: `hq check` does not ask, and
# says so.
# forge_protection: by_hand

protected_paths:
  # Refused outright. `.hq/**` is here from the start and should stay: it
  # holds the battery gate 6 replays, the allowlist the firewall is built
  # from and the Dockerfile the agent runs in — the three things that judge
  # and fence an agent are not that agent's to rewrite. The integrator may
  # still amend its launch script there: its own gate 4 is the wiring list
  # its mission declares, not this one. Add your own paths below.
  refuse:
    - .hq/**
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
- For the coder, that block ends with `Lot: <lot> — done`, or
  `Lot: <lot> — failed: <why>`: the line `hq` reads to know the lot is done.
";

/// The agent image for a Rust project.
///
/// Debian and not Alpine: musl costs too much where the work happens —
/// manylinux-style prebuilt artefacts, a different target triple, and a
/// default thread stack of 128 KiB against glibc's 8 MiB (SPEC 4.2, decided
/// 2026-09-09). The firewall sidecar is the one image on Alpine, for reasons
/// that apply to it alone.
///
/// Two things it must do that are easy to forget: run as the **host's** uid,
/// or a file it writes is unreadable on the host and git refuses the tree;
/// and **pre-create the mount points**, or a named volume is born owned by
/// root and the toolchain cannot write in it (SPEC 4.2 bis).
const DOCKERFILE_RUST: &str = r#"# The coder's image for a Rust project, built by `hq slot rebuild`.
ARG BASE=debian:bookworm-slim
FROM ${BASE}

# Passed by hq at build time: the human's own ids, so that what the agent
# writes belongs to the human on the host.
ARG UID=1000
ARG GID=1000
ARG RUST_VERSION=stable

# `upgrade`, and not only `update`. The packages inherited from the base tag
# keep the versions that tag was cut with, so a security fix Debian published
# since would never reach a freshly built image — pulling the tag again does
# not help while the tag itself has not been rebuilt.
#
# Measured on 2026-09-13, on a scan that went red: `libpcre2-8-0` 10.42-1,
# carrying CVE-2026-86145 and CVE-2026-89161 — both HIGH, both with a fix
# published. It arrives with the base image and is installed by no line here,
# so nothing but this one moves it. `apt-get upgrade` takes it to
# 10.42-1+deb12u1, measured in the base image itself.
RUN apt-get -qq update \
 && apt-get -qq -y upgrade \
 && apt-get -qq install --no-install-recommends -y \
      ca-certificates curl git build-essential pkg-config tmux \
 && rm -rf /var/lib/apt/lists/*

# The group may already exist under that id (on macOS, gid 20 is `dialout`
# here); either way the agent ends up in it.
RUN groupadd -g ${GID} agent || true \
 && useradd -m -u ${UID} -g ${GID} -s /bin/bash agent

# Mount points, created by root because they sit at the filesystem root, then
# handed to the agent: a named volume mounted over a root-owned directory is
# born root-owned, and the toolchain cannot write in it (SPEC 4.2 bis). The
# agent cannot create them itself — it is not root, which is the point.
RUN mkdir -p /work/tree /work/mission /run/hq \
 && chown -R ${UID}:${GID} /work /run/hq

USER agent
ENV RUSTUP_HOME=/home/agent/.rustup \
    CARGO_HOME=/home/agent/.cargo \
    PATH=/home/agent/.cargo/bin:${PATH}

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
  | sh -s -- -y --no-modify-path --profile minimal \
      --default-toolchain ${RUST_VERSION} --component clippy,rustfmt

RUN mkdir -p /home/agent/.cargo/registry /home/agent/.harness

# The mutation campaign gate 7 plays (SPEC 4.4). Installed here, in the
# project's own image, because the campaign is what this stack declares and
# not something hq brings: `mutation.sh` beside this file is what calls it.
# Measured on 2026-09-10, cargo-mutants 27.1.0 on this image: 20 s and 115 MB
# at build time, none at campaign time — which is the right way round, since
# `hq slot rebuild` is rare and a campaign is not.
#
# What stays of the build is the binary and the lockfile it was built from,
# kept where an image scanner finds it. The registry is emptied: it held the
# sources of every dependency — 117 MB, each with its own development
# lockfile, which describes nothing installed here and which Trivy read as a
# vulnerable `regex` (measured 2026-09-11). The kept lockfile is named
# `Cargo.lock`, in a folder of its own: a scanner recognises it by that name
# and no other. The registry directory stays, empty, as the mount point of the
# slot's cargo cache.
RUN cargo install cargo-mutants --locked \
 && mkdir -p /home/agent/.cargo/installed/cargo-mutants \
 && cp /home/agent/.cargo/registry/src/*/cargo-mutants-*/Cargo.lock \
       /home/agent/.cargo/installed/cargo-mutants/Cargo.lock \
 && rm -rf /home/agent/.cargo/registry/*
"#;

/// The mutation campaign a Rust project runs (SPEC 4.4, gate 7): declared by
/// the stack, launched by `hq`, and — being under `.hq/` — not an agent's to
/// weaken while it is being gated by it.
const MUTATION_RUST: &str = r#"#!/bin/sh
# The mutation campaign for a Rust project (SPEC 4.4, gate 7).
#
# `hq` calls this as `mutation.sh <campaign-id> <path>...`, from the clean
# copy of HEAD inside the slot's container. The id comes first so the campaign
# is identifiable from its own command line; this script does not need it.
#
# It prints **one JSON object per line** on stdout, one per surviving mutant:
#   {"id":"…","file":"…","line":12,"description":"…"}
# `hq` ignores anything that is not one, so progress may go to stdout freely —
# though this script keeps the tool's own chatter on stderr.
#
# `--in-place` is not a detail: cargo-mutants only reuses a build cache in
# place, and the copy this runs in has its own, warmed once per slot and kept.
# Without it every campaign recompiles from cold, and SPEC section 7 counts
# that hour.
set -eu

campaign="$1"
shift

# Only Rust sources are worth mutating; the touched list holds whatever the
# branch touched.
files=""
for path in "$@"; do
  case "$path" in
    *.rs) files="$files --file $path" ;;
  esac
done
if [ -z "$files" ]; then
  exit 0
fi

out="target/mutants-$campaign"
# The parent has to exist: `--output` creates its own directory and not the
# path above it, and a clean copy of HEAD that has never been built has no
# `target/` at all ("create output parent directory", measured).
mkdir -p "$out"

# A campaign that finds survivors exits non-zero — 2, measured on
# cargo-mutants 27.1.0 — and that is a result, not a failure: `hq` reads the
# survivors rather than the status.
# shellcheck disable=SC2086
cargo mutants --in-place --no-shuffle --output "$out" $files >&2 || true

# `--output DIR` writes into `DIR/mutants.out/`, not into `DIR` (measured on
# 27.1.0). Reading the wrong path was the whole campaign silently failing.
missed="$out/mutants.out/missed.txt"
if [ ! -f "$missed" ]; then
  echo "hq: the campaign left no $missed" >&2
  exit 1
fi

# One mutant per line, as `file:line:col: what it replaced`:
#   src/lib.rs:2:7: replace > with == in keep
#
# **The whole line is the identifier**, and nothing shorter will do. Measured
# on 27.1.0: a single position carries several mutants — `> ==`, `> <` and
# `> >=` are all at `src/lib.rs:2:7` — so `file:line` and even
# `file:line:col` hand the coder survivors it cannot tell apart, in a file
# whose whole purpose is answering them one by one. The tool's own name for a
# mutant is that line, so that is the name hq uses.
while IFS= read -r mutant; do
  [ -n "$mutant" ] || continue
  file=${mutant%%:*}
  rest=${mutant#*:}
  line=${rest%%:*}
  rest=${rest#*:}
  what=${rest#*: }
  escape() { printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g'; }
  printf '{"id":"%s","file":"%s","line":%s,"description":"%s"}\n' \
    "$(escape "$mutant")" "$file" "$line" "$(escape "$what")"
done < "$missed"
"#;

/// How a Rust application is started (SPEC 4.2, rule 2). Shipped by the
/// stack because starting an application is a property of the stack, not of
/// the mission; replaceable by `hq.yaml`, refinable by a mission header, and
/// amendable by the integrator as part of its wiring.
const RUN_RUST: &str = r#"#!/bin/sh
# How an application of this stack is started (SPEC 4.2, "les services et le
# lancement de l'application").
#
# `hq` runs this, detached, from the root of the tree, inside the container
# of the role that will test or attack the application — before that role is
# launched. No agent starts the application; this script is how the project
# says what starting it means.
#
# It is called as `run.sh <id>`. Three things it must do, and the first two
# are how `hq` knows the application is up at all:
#
#  - keep the id on its command line. `hq` recognises this process by it, and
#    that is why the command below is **not** `exec`ed: replacing the process
#    replaces its command line, and the application would be reported as
#    stopped one second after it started (measured, 2026-09-10).
#  - stay in the foreground. A script that forks and exits reports an
#    application that is not running.
#  - listen on the address the mission's services can reach, not only on
#    127.0.0.1, when something outside the container has to reach it.
#
# Replace the command below with whatever starting this project means. A
# project with nothing to start says `run: none` in hq.yaml instead.
set -eu

cargo run --release
"#;

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
            ("mutation.sh", MUTATION_RUST.to_string(), true),
            ("run.sh", RUN_RUST.to_string(), true),
            ("Dockerfile", DOCKERFILE_RUST.to_string(), false),
            (
                crate::project::WRITABLE_FILE,
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
