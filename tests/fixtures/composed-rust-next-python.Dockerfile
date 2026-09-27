# Composed by nunki from the fragments of rust, then next, python (SPEC 4.2,
# "plusieurs stacks"). Do not edit: `nunki slot rebuild` writes it again.
# The coder's image for a Rust project, built by `nunki slot rebuild`.
ARG BASE=debian:bookworm-slim
# Node's stage, for adding Next.js onto another stack's image (SPEC 4.2,
# "plusieurs stacks"). `COPY --from` does not expand a variable, hence a stage.
ARG NODE=22
# Python's stages, for adding it onto another stack's image (SPEC 4.2,
# "plusieurs stacks"): the interpreter comes from the official image rather
# than being reinstalled, so its OpenSSL stays the one Debian patches and the
# image scan sees. `COPY --from` does not expand a variable, hence a stage.
ARG PYTHON=3.12
ARG UV_VERSION=0.12.17
FROM node:${NODE}-bookworm-slim AS nunki-next-node
FROM python:${PYTHON}-slim-bookworm AS nunki-python-runtime
FROM ghcr.io/astral-sh/uv:${UV_VERSION} AS nunki-python-uv
FROM ${BASE}

# Passed by nunki at build time: the human's own ids, so that what the agent
# writes belongs to the human on the host.
ARG UID=1000
ARG GID=1000
# What the repository pins, passed by nunki at build time from `versions.txt`
# beside this file (SPEC 4.2). The defaults stand when the repository pins
# nothing: `stable`, and no component or target beyond the battery's own.
ARG RUST_VERSION=stable
ARG RUST_COMPONENTS=
ARG RUST_TARGETS=

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

# The toolchain the repository's `rust-toolchain.toml` names, with its
# components and targets, installed here because inside a run rustup cannot
# fetch one: `static.rust-lang.org` is on no allowlist. `clippy` and `rustfmt`
# whatever the repository says, since the battery calls both. Measured on
# 2026-09-26 with `--network none`: an image built this way for `1.98.0` and
# `wasm32-unknown-unknown,x86_64-unknown-linux-musl` answers `rustc 1.98.0`
# under a `rust-toolchain.toml` pinning it, and lists all three targets.
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
  | sh -s -- -y --no-modify-path --profile minimal \
      --default-toolchain ${RUST_VERSION} \
      --component clippy,rustfmt${RUST_COMPONENTS:+,${RUST_COMPONENTS}} \
      ${RUST_TARGETS:+--target ${RUST_TARGETS}}

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

# Next.js, added by nunki onto the image of this project's primary stack
# (SPEC 4.2, "plusieurs stacks"). Starts as the agent, ends as the agent.
USER root
# `procps`: Stryker spawns `ps` to find its children (see the Next.js image).
RUN apt-get -qq update \
 && apt-get -qq install --no-install-recommends -y procps \
 && rm -rf /var/lib/apt/lists/*
COPY --from=nunki-next-node /usr/local/bin/node /usr/local/bin/node
COPY --from=nunki-next-node /usr/local/lib/node_modules /usr/local/lib/node_modules
COPY --from=nunki-next-node /usr/local/include/node /usr/local/include/node
RUN ln -sf ../lib/node_modules/corepack/dist/corepack.js /usr/local/bin/corepack \
 && ln -sf ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
 && ln -sf ../lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx \
 && node --version
ARG OSV_SCANNER=2.6.0
RUN set -eu; \
    case "$(dpkg --print-architecture)" in \
      amd64) arch=amd64; sum=ca69b3d3cd08f889a49dc0a383122f71cc528b83803671df5fd874d97485b108 ;; \
      arm64) arch=arm64; sum=2c71403eb443d05891c4f268c3ad771cf4f16e5443463fd7851ef8f454d3c7e4 ;; \
      *) echo "osv-scanner ships no build for $(dpkg --print-architecture)" >&2; exit 1 ;; \
    esac; \
    curl -fsSL -o /usr/local/bin/osv-scanner \
      "https://github.com/google/osv-scanner/releases/download/v${OSV_SCANNER}/osv-scanner_linux_${arch}"; \
    echo "${sum}  /usr/local/bin/osv-scanner" | sha256sum -c -; \
    chmod 0755 /usr/local/bin/osv-scanner; \
    osv-scanner --version
# pnpm, baked in rather than fetched on first use.
#
# `corepack enable` only enables it: the binary is downloaded from
# `registry.npmjs.org` the first time it runs, into the **calling user's**
# `~/.cache/node/corepack`. Prepared as root and then handed to `agent`, that
# cache is the wrong user's, and the agent fetches it again — which fails
# outright when the network is closed. Measured 2026-09-21. `COREPACK_HOME`
# puts it somewhere both users read.
ARG PNPM=12.5.1
ENV COREPACK_HOME=/opt/corepack
RUN corepack enable pnpm \
 && corepack prepare pnpm@${PNPM} --activate \
 && chmod -R a+rX /opt/corepack \
 && pnpm --version

# Chromium for the integrator's battery, installed here and nowhere else.
#
# Playwright downloads its browsers from its own CDN, which no role's
# allowlist names and none should: the coder has no use for a browser, and
# the integrator must not be able to fetch one mid-run. So they come with the
# image, in a path both users read.
#
# The version is pinned and **written down**, because a project whose
# `@playwright/test` does not match gets "Looks like Playwright was just
# installed or updated. Please run `playwright install`" — advice it cannot
# follow in here. `system.sh` reads this file and says so instead.
ARG PLAYWRIGHT=1.63.0
ENV PLAYWRIGHT_BROWSERS_PATH=/opt/playwright
RUN npx --yes playwright@${PLAYWRIGHT} install --with-deps chromium \
 && printf '%s\n' "${PLAYWRIGHT}" > /opt/playwright/nunki-version \
 && chmod -R a+rX /opt/playwright

# And then npm goes, with the cache the line above left behind.
#
# This stack installs with pnpm, through corepack, and never calls npm — but
# npm ships inside the Node image with a dependency tree of its own, and that
# tree is what the image scan reports. Measured 2026-09-21, the first time
# this image was scanned: seven critical or high advisories **with fixes
# published**, `tar` 7.5.11 among them, none of them from a line of this
# file. Upgrading npm to its latest took it to four rather than none: its
# bundled tree is always a patch or two behind whatever the advisory database
# knows.
#
# So it is removed rather than chased. It is the last line that needs it —
# `npx` above is how Playwright arrives — and corepack, which is a separate
# package, is what keeps pnpm working. The scan comes back clean.
RUN rm -rf /usr/local/lib/node_modules/npm /usr/local/bin/npm /usr/local/bin/npx /root/.npm \
 && pnpm --version \
 && node --version \
 && ! command -v npm

USER agent

# `NEXT_TELEMETRY_DISABLED` is a perimeter rule, not a preference: Next.js
# phones home to a domain rule 6 does not name, and the refusal would read as
# a flaky network rather than as the rule it is.
ENV NEXT_TELEMETRY_DISABLED=1 \
    PNPM_HOME=/home/agent/.local/share/pnpm \
    PATH=/home/agent/.local/share/pnpm:${PATH}

RUN mkdir -p /home/agent/.local/share/pnpm/store /home/agent/.harness

# Python, added by nunki onto the image of this project's primary stack
# (SPEC 4.2, "plusieurs stacks"). Starts as the agent, ends as the agent.
USER root
RUN apt-get -qq update \
 && apt-get -qq install --no-install-recommends -y \
      libsqlite3-0 libreadline8 libncursesw6 netbase tzdata \
 && rm -rf /var/lib/apt/lists/*
COPY --from=nunki-python-runtime /usr/local/ /usr/local/
RUN ldconfig && python3 --version
COPY --from=nunki-python-uv /uv /uvx /usr/local/bin/
ARG OSV_SCANNER=2.6.0
RUN set -eu; \
    case "$(dpkg --print-architecture)" in \
      amd64) arch=amd64; sum=ca69b3d3cd08f889a49dc0a383122f71cc528b83803671df5fd874d97485b108 ;; \
      arm64) arch=arm64; sum=2c71403eb443d05891c4f268c3ad771cf4f16e5443463fd7851ef8f454d3c7e4 ;; \
      *) echo "osv-scanner ships no build for $(dpkg --print-architecture)" >&2; exit 1 ;; \
    esac; \
    curl -fsSL -o /usr/local/bin/osv-scanner \
      "https://github.com/google/osv-scanner/releases/download/v${OSV_SCANNER}/osv-scanner_linux_${arch}"; \
    echo "${sum}  /usr/local/bin/osv-scanner" | sha256sum -c -; \
    chmod 0755 /usr/local/bin/osv-scanner; \
    osv-scanner --version
USER agent

# `UV_PYTHON_DOWNLOADS=never` is a perimeter rule, not a preference: without
# it `uv` fetches an interpreter from a forge the coder's allowlist does not
# name (SPEC 4.1 bis, rule 6), and the failure would read as a flaky network
# rather than as the rule it is. The base image's Python is the one this
# stack runs.
#
# `UV_LINK_MODE=copy` because the cache is a named volume and the tree is a
# bind mount: hardlinking across the two filesystems cannot work, and uv
# warns and falls back on every sync without it.
ENV UV_PYTHON_DOWNLOADS=never \
    UV_LINK_MODE=copy \
    UV_CACHE_DIR=/home/agent/.cache/uv \
    PATH=/home/agent/.local/bin:${PATH}

RUN mkdir -p /home/agent/.cache/uv /home/agent/.harness
