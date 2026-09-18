//! `nunki init` — make an existing repository orchestrable (SPEC 4.2).
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
    /// It exists and differs from what `nunki` would write, so the suggestion
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

/// The stacks `nunki init` can write a fragment for. Three were promised (SPEC
/// 4.2); one is written, and asking for another says so rather than leaving
/// an empty directory that reads as configured.
pub const KNOWN_STACKS: [&str; 1] = ["rust"];

/// Make the repository at `root` orchestrable, with its home at `home`.
///
/// Into the repository go only the rules an agent reads — `AGENTS.md`, the
/// import that makes Claude Code read them, and `.gitattributes` when none
/// exists. The configuration, the HQ and the stack fragments go into the
/// home: development tooling is not a project's to carry in its history.
pub fn init(root: &Path, home: &Path, stacks: &[String]) -> Result<Vec<Action>, InitError> {
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

    // Re-run on a project that already has a configuration: top up the
    // stacks it declares, rather than looking at none.
    //
    // `--stack` has no default, so a bare `nunki init` used to skip the
    // fragment loop entirely and say nothing about it — measured on
    // 2026-09-17, a project missing the `caches.txt` a release had added got
    // four "kept" lines and no hint that its fragments were never examined.
    // The remedy existed (`nunki init --stack rust`) and was undiscoverable.
    let declared;
    let stacks = if stacks.is_empty() {
        declared = declares(home);
        declared.as_slice()
    } else {
        stacks
    };

    let mut actions = Vec::new();

    // The HQ first: everything nunki owns lives there, outside the tree.
    let hq_root = home.join(crate::project::HQ_DIR);
    for dir in ["state", "locks", "missions"] {
        let path = hq_root.join(dir);
        if !path.is_dir() {
            std::fs::create_dir_all(&path).map_err(|e| InitError::Io(path.clone(), e))?;
            actions.push(Action::Created(path));
        }
    }

    create_if_absent(
        home,
        crate::project::CONFIG_FILE,
        &nunki_yaml(root, stacks),
        &mut actions,
    )?;
    create_if_absent(root, "AGENTS.md", AGENTS_MD, &mut actions)?;
    claude_md(root, &mut actions)?;
    gitattributes(root, &mut actions)?;

    for stack in stacks {
        // A stack `nunki` ships no fragment for gets no directory. Only an
        // explicit `--stack` is checked against the known list; the declared
        // ones come from a configuration a human wrote, and one naming a
        // stack this release does not carry yet would otherwise leave an
        // empty folder behind.
        let files = fragment(stack);
        if files.is_empty() {
            continue;
        }
        let dir = home.join(crate::project::STACKS_DIR).join(stack);
        std::fs::create_dir_all(&dir).map_err(|e| InitError::Io(dir.clone(), e))?;
        for (name, body, executable) in files {
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

/// The stacks a project already declares, or none when it declares nothing
/// yet.
///
/// Read here rather than through [`crate::project::Project::open`] because
/// `init` is the one verb that runs before a project exists: an unreadable or
/// absent configuration is the ordinary case on a first run, not an error.
fn declares(home: &Path) -> Vec<String> {
    let text = match std::fs::read_to_string(home.join(crate::project::CONFIG_FILE)) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    match serde_yaml_ng::from_str::<crate::project::Config>(&text) {
        Ok(config) => config.stacks,
        Err(_) => Vec::new(),
    }
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
            "it exists, and nunki never overwrites a file a human edits".to_string(),
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
    // not the question. `nunki check` already knew this; `init` did not, and
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
        // Not rewritten, and not silently accepted either: `nunki check` is red
        // until a human adds the import, because otherwise the rules of the
        // place are never read by this harness.
        "it does not import AGENTS.md — add `@AGENTS.md` to it; `nunki check` is \
         red until you do"
            .to_string()
    };
    actions.push(Action::LeftAlone(path, why));
    Ok(())
}

/// LF, whatever the machine that writes it (SPEC 4.2 bis). Absent, `nunki`
/// writes it; present, `nunki` does not decide for the project.
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
    let suggestion = root.join(".gitattributes.nunki");
    if !suggestion.exists() {
        std::fs::write(&suggestion, BODY).map_err(|e| InitError::Io(suggestion.clone(), e))?;
    }
    actions.push(Action::DepositedBeside {
        kept: path,
        suggestion,
    });
    Ok(())
}

fn nunki_yaml(root: &Path, stacks: &[String]) -> String {
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
        "# What this project declares to nunki (SPEC 4.1). A mission header beats
# this file for what it redeclares. It lives in the project's home, outside the
# repository, and is never mounted into a container.

# The repository this file belongs to. The home is named after the
# repository's directory, so another repository of the same name is refused
# here rather than handed this configuration.
root: {root}

harness: claude-code

# No agent may reach the forge: a slot's origin is unreachable, and an
# allowlist naming it would undo that. Put your forge's domain here.
forge: []

{list}

# How nunki starts the application for the integrator and the security agent
# (SPEC 4.2). Absent, the stack's own `run.sh` is used; `none` for a library,
# whose security agent works on the code and the build artefact. A mission
# header may refine it.
# run: none

# Which model the agents run on, when the harness takes one. Absent, the
# harness keeps its own default — `nunki logs` names the one a run used. A
# mission header may refine it. nunki checks no name: what a name means is the
# harness's business, and the harness refuses what it does not know.
# model: claude-sonnet-5

# What the harness does with a permission it would otherwise ask about.
# Nobody is there to ask in an autonomous container, so the only question is
# which way the silence falls. `auto` lets the harness's own safety checks
# decide and nudges the agent to keep working rather than stop for a
# clarification; `dontAsk` allows only what is pre-approved and denies
# everything else. Whatever the mode, nunki refuses the prompt itself, so a run
# never waits for a human who is not there.
permission_mode: auto

# The project's own Compose file, whose services and networks nunki merges into
# every system profile. They are lifted once per slot and never stopped
# between two profiles, so what the integrator laid down survives.
# services_file: compose.yaml

protected_branches:
  - main
  - dev

# Who refuses a push to a protected branch besides nunki's own gate: `forge`
# (the default) — `nunki check` asks the forge, and an unprotected branch is red.
# `by_hand` when the forge cannot, as on a private repository on GitHub's
# free plan, and you hold the rule yourself: `nunki check` does not ask, and
# says so.
# forge_protection: by_hand

protected_paths:
  # Refused outright. The battery, the allowlist and the Dockerfile are not
  # here: they live in this home, out of the tree, and reach the container
  # read-only. Add the project's own paths — CI workflows, the rules an agent
  # reads.
  refuse: []
  # Refused only where the file already exists on the base.
  refuse_if_exists: []
",
        root = root.display()
    )
}

const AGENTS_MD: &str = "# Working here

The rules of this place, read by whichever harness is driving. `CLAUDE.md`
imports this file; there is one set of rules, not one per tool.

Written by `nunki init` as a starting point — replace it with the rules that
actually hold in this project.

## What an agent may not do

- Never push, never merge, never reach the forge. The human pushes.
- Never touch a protected path (see `nunki.yaml`), and never a protected branch.
- Never ask a blocking question: in an autonomous run there is nobody to
  answer. Refusing is safe; asking is not.

## What a run must leave behind

- The lot committed and proved, or the failure stated plainly.
- A commitable tree.
- A `ÉTAT DE REPRISE` block at the top of `JOURNAL.md`, rewritten at every
  checkpoint and before stopping. It names the commit it describes: `nunki`
  refuses a block that does not carry the current `HEAD`.
- `PR.md`, the pull request the mission delivers, written as the work goes.
  It is a deliverable a gate looks for, and an empty one is a red gate.
- For the coder, that block ends with `Lot: <lot> — done`, or
  `Lot: <lot> — failed: <why>`: the line `nunki` reads to know the lot is done.
  `nunki` reads it **inside** the block — between its heading and the next one —
  so a line left further down the file is one `nunki` will not see.

## Infrastructure an agent cannot reach

The coder's container has no database, no queue and no third-party API, and
never will. Code that talks to one is written against a seam — a trait, a
function, an interface — and proved with a stand-in behind it: the query
built, the rows mapped, what an empty answer means. Nothing is left unproved
because the service is absent, and no test is `#[ignore]`d to make a battery
green. The real thing is the integrator's, against the service itself, in the
system tests.

## Commit messages

What changed and why it had to, and nothing about the tooling: no
`Co-Authored-By`, no session link, no trailer naming a harness, a model or a
tool. The commit's author already says which role wrote it.
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
const DOCKERFILE_RUST: &str = r#"# The coder's image for a Rust project, built by `nunki slot rebuild`.
ARG BASE=debian:bookworm-slim
FROM ${BASE}

# Passed by nunki at build time: the human's own ids, so that what the agent
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
      ca-certificates curl git build-essential pkg-config tmux jq \
 && rm -rf /var/lib/apt/lists/*

# `trufflehog` is what finds the secrets gate 8 reports (SPEC 4.4). It is not
# a stack fragment but a tool of the scan itself — the same for Rust, Python or
# Next.js, since a secret has no ecosystem — and it enters here, at build time
# on the host, never downloaded from inside an agent's container.
#
# Pinned and checked against the checksums its release publishes: a binary
# nobody verifies is a dependency nobody reviewed (section 7). Raising the
# version means raising the two sums with it, and the build fails loudly if
# they disagree.
#
# `gitleaks` was here first and the image scan refused it, rightly: its 8.30.1
# binary carried 33 critical or high advisories **with fixes published**, from
# `golang.org/x/crypto v0.35.0` — thirteen months stale at its own release.
# This one is measured current, and being a Go binary the scan keeps looking
# inside it at every build. A tool the gate cannot inspect would not be safer,
# it would only stop being asked about.
ARG TRUFFLEHOG=3.97.5
RUN set -eu; \
    case "$(dpkg --print-architecture)" in \
      amd64) arch=amd64; sum=e3d97199c565c37ca6152750197f667e08ae6a1edf5911fbdec168622b28620c ;; \
      arm64) arch=arm64; sum=e5c8b2418b0a7c78cf4c47ac783c52c63f989e4e536cfe328b5271e819b6d52d ;; \
      *) echo "trufflehog ships no build for $(dpkg --print-architecture)" >&2; exit 1 ;; \
    esac; \
    curl -fsSL -o /tmp/trufflehog.tgz \
      "https://github.com/trufflesecurity/trufflehog/releases/download/v${TRUFFLEHOG}/trufflehog_${TRUFFLEHOG}_linux_${arch}.tar.gz"; \
    echo "${sum}  /tmp/trufflehog.tgz" | sha256sum -c -; \
    tar -xzf /tmp/trufflehog.tgz -C /usr/local/bin trufflehog; \
    rm /tmp/trufflehog.tgz; \
    trufflehog --version

# The group may already exist under that id (on macOS, gid 20 is `dialout`
# here); either way the agent ends up in it.
RUN groupadd -g ${GID} agent || true \
 && useradd -m -u ${UID} -g ${GID} -s /bin/bash agent

# Mount points, created by root because they sit at the filesystem root, then
# handed to the agent: a named volume mounted over a root-owned directory is
# born root-owned, and the toolchain cannot write in it (SPEC 4.2 bis). The
# agent cannot create them itself — it is not root, which is the point.
RUN mkdir -p /work/tree /work/mission /run/nunki \
 && chown -R ${UID}:${GID} /work /run/nunki

USER agent
ENV RUSTUP_HOME=/home/agent/.rustup \
    CARGO_HOME=/home/agent/.cargo \
    PATH=/home/agent/.cargo/bin:${PATH}

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
  | sh -s -- -y --no-modify-path --profile minimal \
      --default-toolchain ${RUST_VERSION} --component clippy,rustfmt

RUN mkdir -p /home/agent/.cargo/registry /home/agent/.harness

# The two commands the battery and the campaign call, installed in the
# project's own image because they are what this stack declares and not
# something nunki brings. `prepush.sh` beside this file runs
# `cargo deny check`; an image without it fails gate 6 for a reason no
# agent can repair — the image is built from this file, on the host, and no
# agent can reach it. Measured on
# 2026-09-14: a coder spent its three attempts diagnosing that exact
# hole, correctly, and the mission was handed over having built both its
# lots.
#
# The mutation campaign gate 7 plays (SPEC 4.4). Installed here, in the
# project's own image, because the campaign is what this stack declares and
# not something nunki brings: `mutation.sh` beside this file is what calls it.
# Measured on 2026-09-10, cargo-mutants 27.1.0 on this image: 20 s and 115 MB
# at build time, none at campaign time — which is the right way round, since
# `nunki slot rebuild` is rare and a campaign is not.
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
 && cargo install cargo-deny --locked \
 && mkdir -p /home/agent/.cargo/installed/cargo-mutants \
              /home/agent/.cargo/installed/cargo-deny \
 && cp /home/agent/.cargo/registry/src/*/cargo-mutants-*/Cargo.lock \
       /home/agent/.cargo/installed/cargo-mutants/Cargo.lock \
 && cp /home/agent/.cargo/registry/src/*/cargo-deny-*/Cargo.lock \
       /home/agent/.cargo/installed/cargo-deny/Cargo.lock \
 && rm -rf /home/agent/.cargo/registry/*
"#;

/// The mutation campaign a Rust project runs (SPEC 4.4, gate 7): declared by
/// the stack, launched by `nunki`, and — living in the project's home, mounted
/// read-only — not an agent's to weaken while it is being gated by it.
const MUTATION_RUST: &str = r#"#!/bin/sh
# The mutation campaign for a Rust project (SPEC 4.4, gate 7).
#
# `nunki` calls this as `mutation.sh <campaign-id> <path>...`, from the clean
# copy of HEAD inside the slot's container. The id comes first so the campaign
# is identifiable from its own command line; this script does not need it.
#
# It prints **one JSON object per line** on stdout, one per surviving mutant:
#   {"id":"…","file":"…","line":12,"description":"…"}
# `nunki` ignores anything that is not one, so progress may go to stdout freely —
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
# cargo-mutants 27.1.0 — and that is a result, not a failure: `nunki` reads the
# survivors rather than the status.
#
# `--exclude-re "replace main -> "` drops one survivor nobody could ever
# answer. cargo-mutants replaces a whole function body with
# `Default::default()` whenever the return type allows it, and
# `fn main() -> ExitCode` always allows it — but no unit test calls `main`,
# so that mutant cannot be killed by any test the coder is able to write.
# Left in, it costs a lot for nothing: measured on 2026-09-13, a coder spent
# a run extracting `main`'s body into a testable function, and the mutant
# simply reappeared on the thin wrapper that was left.
#
# Measured the same day, on a crate with a binary and a library: 19 mutants
# without it, 18 with it. It removes `src/main.rs`'s whole body and keeps
# `src/lib.rs`'s `replace run -> ExitCode` — the same shape, in the function
# `main` delegates to, and that one a test can and must kill. The exclusion
# is on the mutation, never on the file: a `main.rs` carrying real code still
# owes every mutant in it.
# shellcheck disable=SC2086
cargo mutants --in-place --no-shuffle --exclude-re "replace main -> " --output "$out" $files >&2 || true

# `--output DIR` writes into `DIR/mutants.out/`, not into `DIR` (measured on
# 27.1.0). Reading the wrong path was the whole campaign silently failing.
missed="$out/mutants.out/missed.txt"
if [ ! -f "$missed" ]; then
  echo "nunki: the campaign left no $missed" >&2
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
# mutant is that line, so that is the name nunki uses.
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
/// the mission; replaceable by `nunki.yaml`, refinable by a mission header, and
/// amendable by the integrator as part of its wiring.
/// The mechanical security of a Rust project (SPEC 4.4, gate 8): the
/// dependency audit and the secret scan. Like the campaign, it is
/// declared by the stack, launched by `nunki`, and mounted read-only — what
/// judges the agent is not the agent's to weaken.
///
/// Measured against the real tool on 2026-09-17 before it shipped, which is
/// the ritual `mutation.sh` went through: cargo-deny writes its JSON to
/// **stderr**, and `recurse(.parents[0]?)` never terminates because a node
/// without `parents` yields `null` rather than nothing.
const SECURITY_RUST: &str = r##"#!/bin/sh
# The mechanical security of a Rust project (SPEC 4.4, gate 8).
#
# `nunki` calls this as `security.sh <base-commit>`, from the clean copy of
# HEAD inside the slot's container. A commit and not a branch name: the copy is
# a detached clone and carries no branch, so a name would resolve to nothing. It prints **one JSON object per line** on
# stdout, one per finding:
#
#   {"id":"…","kind":"…","where":"…","via":"…","fix":"…",
#    "accepted":"…","was_at_base":false}
#
# `nunki` reads those seven fields and nothing else — it knows no tool, no
# lockfile and no advisory database. Anything that is not one of these lines
# is ignored, so progress may go to stdout freely, though this script keeps
# its chatter on stderr.
#
# SPEC 4.4 gives gate 8 three families: the dependency audit (`cargo-deny`),
# the secret scan (`trufflehog`), and the static analysis — which is `clippy`,
# already run by the battery at gate 6, so nothing here repeats it.
set -eu

base="${1:?usage: security.sh <base-commit> [advisory-db] [mission-dir]}"
# Second argument and not an environment variable: the agent owns its own
# environment inside the container, and a variable would let it point this at
# an empty directory — no findings, and a green gate. `nunki` is what invokes
# the gate, so `nunki` is what says where the database is.
db="${2:-/work/advisories}"
mission="${3:-/work/mission}"

[ -d "$db" ] || { echo "nunki: no advisory database at $db" >&2; exit 69; }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# A configuration with **no `ignore`**: this is the unfiltered view, the one
# that still reports what the project has accepted. `--config` is what makes
# it possible without a sandbox — measured 2026-09-17, cargo-deny reads the
# file it is given and no other.
printf '[advisories]\ndb-path = "%s"\n' "$db" > "$work/none.toml"

# `--offline` keeps it from fetching the database, which lives on a forge no
# agent may reach (SPEC 4.1 bis). It also keeps cargo from downloading crates,
# so the slot's registry cache must be warm — the battery has already built by
# the time this runs.
#
# **The JSON goes to stderr**, not stdout (measured 2026-09-17: with
# `2>/dev/null` the output is empty). Findings make it exit non-zero, and that
# is a result rather than a failure.
audit() {
  cargo deny --offline --config "$2" --manifest-path "$1/Cargo.toml" \
    --format json check advisories 2>&1 >/dev/null || true
}

ids() { grep '"type":"diagnostic"' | sed -n 's/.*"id":"\([^"]*\)".*/\1/p' | sort -u; }

# What the base already carried. A worktree and not a bare lockfile: measured
# 2026-09-17, cargo-deny goes through cargo metadata and refuses a directory
# holding only a manifest and a lockfile ("no targets specified in the
# manifest"). Nothing is compiled here.
: > "$work/base.ids"
if git worktree add -q --detach "$work/base" "$base" 2>/dev/null; then
  audit "$work/base" "$work/none.toml" | ids > "$work/base.ids" || true
  git worktree remove --force "$work/base" 2>/dev/null || true
else
  # 70, and not a warning followed by the audit. Without the base there is no
  # "what this branch brought": every finding would come back new, and the gate
  # would block on what was already there. `nunki` reads this as unplayed — a
  # verdict on the machine rather than on the agent, exactly as 69 is.
  echo "nunki: base $base is not in this repository, so nothing can be compared" >&2
  exit 70
fi

# What the project accepts, as the difference between the two views: an id the
# unfiltered run reports and the project's own configuration does not is one
# `deny.toml` ignores. The reason is read from that file, best effort.
audit . "$work/none.toml" > "$work/all.json"
: > "$work/kept.ids"
if [ -f deny.toml ]; then
  audit . deny.toml | ids > "$work/kept.ids"
fi

reason() {
  sed -n "s/.*id *= *\"$1\".*reason *= *\"\([^\"]*\)\".*/\1/p" deny.toml 2>/dev/null |
    head -1
}

grep '"type":"diagnostic"' "$work/all.json" | while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"advisory":{.*"id":"\([^"]*\)".*/\1/p')
  [ -n "$id" ] || continue
  printf '%s' "$line" > "$work/one.json"

  kind=$(jq -r '.fields.advisory.informational // .fields.code' "$work/one.json")
  where=$(jq -r '.fields.graphs[0].Krate | "\(.name) \(.version)"' "$work/one.json")
  # The chain from the finding up to the root crate. Two names means the
  # finding is on a direct dependency and there is nothing to name; more, and
  # `via` is the last one before the root — the direct dependency that brought
  # it in, and the only one this project can actually replace.
  # `.parents[0]?` is not a stopping condition: on a node without `parents`
  # it yields `null` rather than nothing, and the recursion never ends —
  # measured 2026-09-17, jq spun until it was killed. `has` is the guard.
  via=$(jq -r '[.fields.graphs[0]
                | recurse(if has("parents") then .parents[0] else empty end)
                | .Krate.name]
               | if length > 2 then .[-2] else "" end' "$work/one.json")
  fix=$(jq -r '[.fields.notes[] | select(startswith("Solution: Upgrade to "))]
               | if length == 0 then "" else .[0] end' "$work/one.json" |
        sed -e 's/^Solution: Upgrade to //' -e 's/ *(try .*$//')

  # Reported unfiltered and **not** reported through the project's own
  # configuration: that is an id `deny.toml` ignores, and the only reliable
  # way to know it without parsing TOML in a shell.
  accepted=""
  if [ -f deny.toml ] && ! grep -qx "$id" "$work/kept.ids"; then
    accepted="$(reason "$id")"
    [ -n "$accepted" ] || accepted="accepted in deny.toml"
  fi

  was_at_base=false
  grep -qx "$id" "$work/base.ids" 2>/dev/null && was_at_base=true

  jq -cn --arg id "$id" --arg kind "$kind" --arg where "$where" \
     --arg via "$via" --arg fix "$fix" --arg accepted "$accepted" \
     --argjson was_at_base "$was_at_base" \
     '{id:$id, kind:$kind, where:$where, via:$via, fix:$fix,
       accepted:$accepted, was_at_base:$was_at_base}'
done

# --- secrets ---------------------------------------------------------------
#
# A secret has no `fix` and no `via`: it is revoked, not upgraded, and nothing
# brought it in but the commit that wrote it. `nunki` stops on one and hands it
# to a human, because no action of an agent closes it — removing it in a later
# commit leaves it in the branch's history, and nunki rewrites none.
#
# `--no-ignore-tag` is a rule and not a setting. trufflehog's own exception is
# a `trufflehog:ignore` comment **in the source line**, which the agent edits
# legitimately: without this flag it could silence its own secret with six
# characters. Everything is reported, and what is accepted is decided outside
# the container.
leaks="$work/leaks.jsonl"
trufflehog git "file://$PWD" --json --no-update --no-ignore-tag \
  > "$leaks" 2>/dev/null || true

# What the HQ has already ruled is not a secret. `nunki` renders it read-only
# in the mission folder, the way it renders the allowlist: declared out of the
# agent's reach, readable by it all the same. Absent, nothing is accepted.
accepted_file="$mission/SECRETS.txt"

reason_for() {
  [ -f "$accepted_file" ] || return 0
  sed -n "s|^$1[[:space:]]*||p" "$accepted_file" | head -1
}

while IFS= read -r line; do
  [ -n "$line" ] || continue
  printf '%s' "$line" > "$work/leak.json"
  git_meta=$(jq -r '.SourceMetadata.Data.Git // empty | @json' "$work/leak.json")
  [ -n "$git_meta" ] || continue
  commit=$(printf '%s' "$git_meta" | jq -r '.commit // ""')
  file=$(printf '%s' "$git_meta" | jq -r '.file // ""')
  ln=$(printf '%s' "$git_meta" | jq -r '.line // 0')
  rule=$(jq -r '.DetectorName // "secret"' "$work/leak.json")
  [ -n "$commit" ] && [ -n "$file" ] || continue

  # trufflehog publishes no fingerprint, and a finding must be nameable to be
  # accepted. Built the way gitleaks built its own, so an exception written
  # once keeps naming the same thing.
  id="$commit:$file:$rule:$ln"

  was=false
  git merge-base --is-ancestor "$commit" "$base" 2>/dev/null && was=true

  jq -cn --arg id "$id" --arg where "$file:$ln" \
     --arg accepted "$(reason_for "$id")" --argjson was_at_base "$was" \
     '{id:$id, kind:"secret", where:$where, via:"", fix:"",
       accepted:$accepted, was_at_base:$was_at_base}'
done < "$leaks"
"##;

const RUN_RUST: &str = r#"#!/bin/sh
# How an application of this stack is started (SPEC 4.2, "les services et le
# lancement de l'application").
#
# `nunki` runs this, detached, from the root of the tree, inside the container
# of the role that will test or attack the application — before that role is
# launched. No agent starts the application; this script is how the project
# says what starting it means.
#
# It is called as `run.sh <id>`. Three things it must do, and the first two
# are how `nunki` knows the application is up at all:
#
#  - keep the id on its command line. `nunki` recognises this process by it, and
#    that is why the command below is **not** `exec`ed: replacing the process
#    replaces its command line, and the application would be reported as
#    stopped one second after it started (measured, 2026-09-10).
#  - stay in the foreground. A script that forks and exits reports an
#    application that is not running.
#  - listen on the address the mission's services can reach, not only on
#    127.0.0.1, when something outside the container has to reach it.
#
# Replace the command below with whatever starting this project means. A
# project with nothing to start says `run: none` in nunki.yaml instead.
set -eu

cargo run --release
"#;

/// The integrator's battery for a Rust project (SPEC 4.4, gate 6 for the
/// integrator: its system tests, in the system profile).
///
/// Shipped by the stack because the gate reads it from there, and a stack that
/// ships none holds that gate red for a reason no agent is told — found on
/// 2026-09-15, before the first integration mission was launched.
const SYSTEM_RUST: &str = r##"#!/bin/sh
# The integrator's battery for a Rust project: its system tests, green, in the
# system profile (SPEC 4.4, gate 6 for the integrator).
#
# `nunki` runs this from the clean copy of HEAD inside the integrator's
# container, where the mission's services are reachable. The coder's battery
# (`prepush.sh`) runs where they are not, and so does CI: a system test must
# therefore never run under a plain `cargo test`, or it goes red in both.
#
# The convention that keeps them apart: every system test carries
#
#     #[ignore = "system test: needs the services"]
#
# A plain `cargo test` skips it and says so; `-- --ignored` below runs those
# tests and only those. Ignore nothing else: an ignored test that is not a
# system test would run here and nowhere else.
#
# `--tests` leaves the doc-tests out: an `ignore` code block in a doc comment
# is usually one that does not compile, and `--ignored` would try it.
#
# Pointing the tests at the services is the integrator's wiring, committed with
# them. This script is the stack's: it lives in the project's home and reaches
# the container read-only, so a project whose system tests need more changes
# it there, not in a commit.
set -eu

# Beside the build rather than under /tmp: `target/` is what the copy of HEAD
# is certain to let an execution write.
mkdir -p target
log=target/nunki-system-tests.log

status=0
cargo test --all-features --tests -- --ignored >"$log" 2>&1 || status=$?
cat "$log" >&2
if [ "$status" -ne 0 ]; then
  exit "$status"
fi

# A battery that ran nothing proved nothing (SPEC 4.4): no system test is red,
# not green.
ran=$(sed -n 's/^test result: ok\. \([0-9][0-9]*\) passed.*/\1/p' "$log" | awk '{ n += $1 } END { print n + 0 }')
if [ "$ran" -eq 0 ]; then
  echo "nunki: no system test ran — mark each one #[ignore = \"system test: needs the services\"]" >&2
  exit 1
fi
"##;

/// The files of a stack fragment: `(name, body, executable)`.
fn fragment(stack: &str) -> Vec<(&'static str, String, bool)> {
    match stack {
        "rust" => vec![
            (
                "allow.txt",
                "# What a Rust build must reach, and nothing else (SPEC 4.1 bis, rule 6).\n\
                 # One name per line; `#` comments. No forge: nunki check is red if one appears.\n\
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
                 # Not `cargo deny check` alone: its `advisories` stage fetches\n\
                 # its database from github.com, and the coder's allowlist names\n\
                 # no forge (SPEC 4.1 bis, rule 6). That stage can never pass in\n\
                 # this container — not for want of a network, but by design — so\n\
                 # asking for it would hold gate 6 red for a reason no agent can\n\
                 # repair. Run the advisories in CI, where the forge is reachable.\n\
                 cargo deny check bans licenses sources\n"
                    .to_string(),
                true,
            ),
            ("mutation.sh", MUTATION_RUST.to_string(), true),
            (crate::gate::SECURITY, SECURITY_RUST.to_string(), true),
            ("run.sh", RUN_RUST.to_string(), true),
            (crate::gate::SYSTEM_BATTERY, SYSTEM_RUST.to_string(), true),
            ("Dockerfile", DOCKERFILE_RUST.to_string(), false),
            (
                crate::project::WRITABLE_FILE,
                "# Directories an execution must be able to write when the tree is\n\
                 # mounted read-only (SPEC 4.2). One relative path per line.\n\
                 target\n"
                    .to_string(),
                false,
            ),
            (
                crate::project::ADVISORIES_FILE,
                "# Where this toolchain's advisory database lives **on the host**.\n\
                 # One path, and nunki mounts it read-only for gate 8.\n\
                 #\n\
                 # nunki does not fill it: the tool owns its own layout, and\n\
                 # cargo-deny refuses a path that is not the one it builds from\n\
                 # the database's URL. Refresh it with the tool itself, on this\n\
                 # machine — `cargo deny check advisories` once is enough — and\n\
                 # nunki will say how old it is.\n\
                 ~/.cargo/advisory-db\n"
                    .to_string(),
                false,
            ),
            (
                crate::project::CACHES_FILE,
                "# Caches this toolchain keeps outside the tree, kept as a named\n\
                 # volume per slot so a rebuilt container does not download the\n\
                 # world again. One absolute container path per line.\n\
                 #\n\
                 # Declared here and not in nunki: a cache belongs to a toolchain,\n\
                 # and nunki is agnostic to the stack.\n\
                 /home/agent/.cargo/registry\n"
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
