# Changelog

Every release of `nunki`, newest first. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). Before 1.0, a
minor version may change what `nunki.yaml`, a mission's header or a verb
accepts.

A version's section is written when it is released, from the pull requests
merged since the last one. `AGENTS.md`, under "Releases", says how.

## [Unreleased]

### Fixed

- **`nunki slot rebuild --help` now says which stack `--stack` takes.** It
  said the option defaulted to "the only one declared". It defaults to the
  first stack `nunki.yaml` declares, the primary, and takes no other: the
  agent's image is built from it, with the other declared stacks added onto
  it. The help also names the prober among the images it builds.

## [0.2.0] - 2026-11-04

Gate 7 changes regime. A `standard` or `critical` mission owes one
mutation campaign, then one recheck at its final `HEAD`, instead of a
campaign at each return to the coder, and the Python and Next.js campaigns
answer for the lines a branch changed. It is a minor version because
`nunki push` refuses more at `standard`, and a Next.js survivor's id
changes.

After upgrading, run `nunki init --refresh` in each project, which takes
the new campaign scripts where nobody has touched them, then `nunki slot
rebuild`.

### Changed

- **A `standard` or `critical` mission now owes one mutation campaign, then
  one recheck at its final `HEAD`, instead of a campaign at each return to
  the coder.** Measured on a `critical` mission of a Python project: eleven
  full campaigns, nine of which mostly found the same survivors again.
  - The campaign runs the first time the mission reaches its final gates.
    After it, the final gates judge the campaign on file and the coder's
    outcomes, whatever was committed since, and run nothing.
  - The recheck runs once every other stage is green. It measures again
    the files changed since the campaign, and the campaign's survivors in
    the other files: by name for the Rust and Next.js stacks, by their
    whole file for Python. When nothing changed since the campaign, it
    does not run.
  - A `standard` mission now goes through that last stage, as only
    `critical` did, and `nunki push` at `standard` refuses unless the
    measure on file ran on the pushed commit.
  - On a measure of the content being judged, a coder's `killed` on a
    survivor that measure found is no longer an outcome: the test it names
    ran against the mutant, and the mutant lived. Gate 7 says so beside the
    survivor, and `nunki push` refuses it too.
  - The Rust `mutation.sh` looks for a named mutant in every file of the
    crate, and says so with a new line, `# nunki-by-name-anywhere`. In a
    project set up before, `nunki init --refresh` replaces the script if
    nobody has touched it; until then its Rust survivors are rechecked by
    file.
  - The Next.js `mutation.sh` measures a named survivor again by the span
    of the code it replaces, which Stryker takes as it takes a line range,
    and says so with the same two lines. Of what a span runs, only the
    mutant named is reported. A survivor's id now carries that span.
  - With those lines, a hardening mission and `nunki mutants run --owed`
    take a Next.js project's debt by name too. A survivor the debt holds
    from before has an id without a span: a campaign handed it measures
    nothing, and says why. `nunki mutants run --whole` replaces the debt.
  - In a Next.js project set up before, `nunki init --refresh` replaces
    the script if nobody has touched it. The first campaign after it is
    full, and an outcome the coder gave a survivor under its old id has to
    be given again.
  - Missions in flight: a `standard` mission already `Verified` on a
    campaign of other content is refused by `nunki push` until `nunki
    mission mutants` runs the recheck. One at its final gates with a
    campaign of other content on file is judged on it, rather than running
    a new one.
  - `mission list` names that last stage `recheck`, where it said
    `campaign`.
- **A Python mutation campaign now answers for the lines the branch
  changed, not for whole files.** mutmut tests only the functions and
  methods that hold a changed line, and the campaign counts and reports
  only the mutants whose change reaches one, file by file.
  - A `standard` Python mission is judged on its share of killed mutants,
    no longer as `critical`. The limitation listed in 0.1.0 is lifted for
    Python.
  - Its recheck can be partial: it measures again only the files changed
    since the campaign, and the files of the campaign's survivors.
  - A survivor on a line the branch did not change, in a function it did,
    is no longer reported.
  - Each mutant's line is read through `get_diff_for_mutant`, a function
    inside mutmut that 3.8.0 has. With a mutmut that lacks it, the
    campaign measures nothing and says why.
  - In a project set up before, `nunki init --refresh` replaces the
    campaign script if nobody has touched it.
- **A Next.js mutation campaign now answers for the lines the branch
  changed, not for whole files.** Stryker is handed one line range for each
  run of lines the branch added, and the campaign counts and reports what
  it mutated there, file by file.
  - A `standard` Next.js mission is judged on its share of killed mutants,
    no longer as `critical`. The limitation listed in 0.1.0 is lifted for
    Next.js too.
  - Its recheck can be partial, as for Rust and Python.
  - Stryker mutates only what sits wholly inside a range: a block or a
    function body that reaches past the changed lines is not measured.
  - A survivor's description no longer starts with its status, which
    changed from one campaign to the next. An outcome already given to a
    Next.js survivor is therefore not carried to the first campaign after
    the update, and an equivalence the project's registry holds for one is
    not proposed again: it has to be ruled once more.
  - A project whose Stryker configuration turns on incremental mode no
    longer has the campaign reuse an earlier run's results.
  - In a project set up before, `nunki init --refresh` replaces the
    campaign script if nobody has touched it.
- **The firewall and prober images are tagged `0.2.0`**, as every
  version tags them with its own number. After upgrading, run
  `nunki slot rebuild` in each project before its next launch. Until
  then a launch refuses, saying the images are missing.

### Fixed

- **Two Next.js survivors no longer share an id.** The id held where the
  replaced code started, not where it ended. On `if (value >= high && high
  > low)`, `true` in place of the whole condition and `true` in place of
  its left side had one id, so one outcome answered both.
- **A Next.js mutation campaign now mutates every source the branch
  touched.** It handed Stryker one `--mutate` per file, and Stryker keeps
  only the last. On a branch touching several sources, all but one went
  unmeasured, the campaign still said it had finished, and gate 7 could
  pass on them.
  - A Next.js dynamic route, such as `app/[slug]/page.tsx`, was never
    mutated either. Stryker read its brackets as a pattern, and could
    mutate another file in its place, such as `app/s/page.tsx`.
  - A path holding a space made the campaign fail to run: Stryker read
    half of it as its configuration file. It is now mutated like any
    other.
  - A path holding a comma, which Stryker cannot be handed, now stops the
    campaign with a message rather than going unmeasured.
  - In a project set up before, `nunki init --refresh` replaces the
    campaign script if nobody has touched it.
- **A Next.js branch whose only touched source it removed no longer leaves
  gate 7 asking for a campaign for ever.** Handed that path alone, Stryker
  ran no test and exited 1, "No tests were executed", which the campaign
  read as a run that could not complete. It now hands Stryker no source the
  branch removed.
- **`nunki mission wait` no longer returns at once while a campaign runs
  with no monitor** (#183). It said exit 14, "nothing moves the mission",
  and that `nunki verify` would start a campaign, when one was in flight.
  It now waits while the campaign runs, then returns 14 saying the
  campaign is over and that `nunki verify` reads it back. A campaign that
  dies before its last line keeps it waiting until the campaign's deadline.

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

[Unreleased]: https://github.com/ArnaudSene/nunki/compare/v0.2.0...dev
[0.2.0]: https://github.com/ArnaudSene/nunki/releases/tag/v0.2.0
[0.1.1]: https://github.com/ArnaudSene/nunki/releases/tag/v0.1.1
[0.1.0]: https://github.com/ArnaudSene/nunki/releases/tag/v0.1.0
