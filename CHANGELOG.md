# Changelog

Every release of `nunki`, newest first. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). Before 1.0, a
minor version may change what `nunki.yaml`, a mission's header or a verb
accepts.

A version's section is written when it is released, from the pull requests
merged since the last one. `AGENTS.md`, under "Releases", says how.

## [Unreleased]

## [0.1.1] - 2026-10-29

A security release: the Next.js stack's default pnpm bundled an undici
with three high advisories.

### Security

- **The Next.js stack's pnpm is now 12.16.1 by default, up from 12.5.1.**
  The undici 6.28.1 that pnpm 12.5.1 bundles carries three high
  advisories: CVE-2026-109383, CVE-2026-109869 and CVE-2026-87018.
  pnpm 12.16.1 bundles undici 6.29.1, which fixes all three.
  - A project that pins pnpm with `packageManager` in its `package.json`
    keeps its pin.
  - In a project set up before, `nunki init --refresh` replaces the
    Dockerfile if nobody has touched it, and `nunki slot rebuild` then
    rebuilds the image.

### Changed

- **The firewall and prober images are tagged `0.1.1`**, as every
  version tags them with its own number. After upgrading, run
  `nunki slot rebuild` in each project before its next launch. Until
  then a launch refuses, saying the images are missing.

## [0.1.0] - 2026-10-28

The first release. `nunki` orchestrates missions run by AI coding agents on
an existing repository. A human frames a mission; `nunki` runs it in an
isolated slot with a coder, an integrator and a security agent, gates the
result, and hands the push back to the human.

### Added

- **`nunki init`** makes an existing repository orchestrable. In the
  repository it writes only `AGENTS.md`, `CLAUDE.md` and a `.gitattributes`
  entry. The stack's Dockerfile, allowlist, battery, mutation campaign and
  launch script go to the project's home under `~/.nunki`, with
  `nunki.yaml`. Three stacks ship: Rust, Python and Next.js, alone or
  together in one project. `nunki sessions` and `nunki adopt` keep a home
  pointed at its repository.
- **Slots**, through `nunki slot add`, `list`, `rebuild`, `reset` and `rm`. A
  slot is a local clone without hard links, with its own containers and
  volumes.
- **Missions.** `nunki mission new` frames one and `start` launches its
  first run. `status`, `watch` and `wait` follow it, and so does `nunki
  logs`. `pause`, `resume`, `stop`, `kill` and `say` act on a run.
  `reframe`, `iterate`, `retry`, `accept`, `fetch`, `end` and `archive` act
  on the mission.
- **Three roles**: a coder, then an integrator and a security agent, each
  in runs of its own and within its own perimeter.
- **Confinement.** An agent runs in a container with no capability, under
  the host's uid, behind a firewall sidecar it cannot reach. It sees its
  slot's tree and its mission folder, nothing else. Every git `nunki` runs
  about a slot runs in a mirror beside it that only the host writes.
- **Eight gates**: a clean tree, a branch ahead of its base, the journal's
  resume block, the perimeter commit by commit, the deliverable, the
  battery, the mutation campaign, and mechanical security (dependency
  audit, secret scan, static analysis). `nunki mission gates` plays the
  first four on what a slot holds.
- **Rigor.** `prototype`, `standard` and `critical`, the default, set how
  much verification asks of a mission: gate 7's campaigns, the security
  agent's rounds, the integration.
- **Mutation campaigns**, full or partial, with a budget, a sample and a
  deadline, and several mutants at once with `mutation_jobs`. `nunki
  mutants` runs campaigns outside missions and keeps the project's debt of
  survivors.
- **The monitor.** `nunki verify` reads a mission back and launches what is
  owed. A monitor per mission calls it for you: it watches a run every
  minute, stops it at the account's threshold, and calls `verify` again
  when a wait ends.
- **Accounts.** `~/.nunki/accounts.yaml` declares the subscriptions
  missions spend, and `nunki account list` names them. `nunki` measures
  each account's five-hour and weekly windows, waits for a reset, and lets
  a mission switch accounts between two runs.
- **Project services**, declared in a services file the human approves
  with `nunki services`, lifted for the integrator and the security agent.
- **`nunki push`** pushes a verified branch, on an explicit `--yes`, and
  opens its pull request, or gives the address to open it at without a
  token that may. It is the only verb that writes to the forge.
- **`nunki check`** says whether the repository, its slots and their
  containers hold what the specification describes. **`nunki exec`**
  replays a command in a slot's container.

### Known limitations

- **Docker is the only container engine**: Docker Desktop or OrbStack on
  macOS, Docker Desktop or Docker Engine under WSL. Podman is a second
  target, not implemented. Compose v2.38 is the oldest supported.
- **Claude Code is the only harness.**
- **macOS and Linux under WSL** are the platforms. Native Windows is not a
  target.
- **Slots do not support submodules.**
- **Python and Next.js missions judge `standard` as `critical`.** Their
  campaigns mutate whole touched files, so they give no count of changed
  lines.
- **No prebuilt binary.** `nunki` is built from source, with Rust 1.89 or
  later and a C compiler.

[Unreleased]: https://github.com/ArnaudSene/nunki/compare/v0.1.1...dev
[0.1.1]: https://github.com/ArnaudSene/nunki/releases/tag/v0.1.1
[0.1.0]: https://github.com/ArnaudSene/nunki/releases/tag/v0.1.0
