# nunki

`nunki` is a **mission orchestrator for AI coding agents** working on an existing
repository. A human frames a mission; `nunki` runs it in an isolated slot with a
coder, an integrator and a security agent, gates the result, and hands the
push back to the human. It is not a scaffolder: it creates no project and
chooses no stack.

It is stack-agnostic (a stack only changes declared fragments: battery,
caches, domains) and harness-agnostic (Claude Code today; the harness is an
interchangeable executor behind an adapter).

[`SPEC.md`](SPEC.md) is the authority on what the system does and why (in
French). [`AGENTS.md`](AGENTS.md) holds the rules for anyone working in this
repository.

## Install

What you need:

- **Rust 1.85 or later**, to build `nunki` — `rustup` gives you one.
- **git**.
- **A container engine** with Compose: Docker, or OrbStack. Every agent runs
  in a container behind a firewall sidecar; nothing runs on your machine.
- **A Claude subscription**, and the Claude Code CLI **once on your machine**,
  to mint a long-lived token. The agents' own CLI lives in their image, not
  here.

Build and install the binary:

```sh
git clone https://github.com/ArnaudSene/nunki
cd nunki
cargo install --path .       # puts `nunki` in ~/.cargo/bin
nunki --version
```

Declare the subscription `nunki` spends. The token is written under `~/.nunki`,
never in a repository, and reaches a container as an environment variable at
launch:

```sh
mkdir -p ~/.nunki/accounts
claude setup-token > ~/.nunki/accounts/main.token   # a browser gesture, once
chmod 600 ~/.nunki/accounts/main.token
cat > ~/.nunki/accounts.yaml <<'YAML'
default: main
accounts:
  main:
    harness: claude-code
    token_file: accounts/main.token
YAML
nunki account list
```

## Set a repository up

From inside the repository you want orchestrated:

```sh
nunki init --stack rust         # rust, python and next are the stacks shipped
```

It creates what is absent and never overwrites a file you edit — it says what
it left alone. In the repository, only what the agents read: `AGENTS.md` (the
rules), `CLAUDE.md` importing it, and a `.gitattributes` entry. Everything
else is tooling, and tooling stays out of your history — it goes into the
project's home, named by the session `init` opens for the repository:

```text
~/.nunki/sessions.json    the ledger: one identifier, one repository
~/.nunki/<id>/
    nunki.yaml            the project's configuration, never mounted
    hq/                   state, locks, missions, profiles — never mounted
    stacks/<stack>/       Dockerfile, allowlist, battery, mutation campaign, launch script
```

A home is named by an identifier and not by your repository's directory, so
two repositories called `api` are two projects rather than a collision.
`nunki sessions` lists what this machine holds, and `nunki adopt <id>` points
a session at the repository you are in after moving or renaming it.

The scripts under `stacks/` reach the agent's container read-only, one file at
a time; the Dockerfile and the allowlist are read on this machine only.

Read `~/.nunki/<project>/nunki.yaml` before going further. Its `protected_branches`,
`protected_paths` and `forge_protection` are what the perimeter gate enforces.
Two more decide how the agents run: `model` (which model they use, absent
means the harness's own default) and `permission_mode` (`auto` by default —
what the harness does with a permission it would otherwise ask a human
about, since nobody is there to ask).
Then commit the three files `nunki init` wrote in the repository, build the
images and clone a slot:

```sh
nunki slot rebuild --stack rust # the agent's image and the firewall sidecar
nunki slot add one              # a clone at ../<project>-slots/one
nunki check                     # what is held, and what could not be checked
```

## A first mission

```sh
nunki mission new m1 \
  --branch mission/first \
  --lot "L1:parse the header" \
  --lot "L2:reject a malformed one" \
  --model claude-opus-5-5 \
  --about "What the mission is for, in your words."
```

That writes `<home>/hq/missions/m1/MISSION.md`: a YAML header `nunki`
reads, and prose the agent reads. Edit the prose, then start it:

```sh
nunki mission start m1 --slot one
```

The coder's first run is launched, detached, and a monitor is started for the
mission. From then on `nunki` drives: it reads each run back, plays the gates,
launches the next lot, waits out a harness that fails, and stops a run that
would pass your subscription's threshold.

While it works:

```sh
nunki mission status m1     # the stage, the run, what it has spent
nunki logs m1 --last        # the run, readably
nunki mission watch m1      # follow the run in progress
nunki verify m1             # read back and launch what is owed, by hand
```

If you need to intervene: `nunki mission stop m1` holds the mission (`--now` ends
the turn in progress too), `nunki mission resume m1` lifts the hold,
`nunki mission say m1 "..."` leaves an instruction for the next run, and
`nunki mission kill m1` is the emergency brake.

When a bound runs out — a lot's attempts, a role's, or the returns to the
coder — `nunki` stops and hands the mission to you. `nunki mission retry m1
--because "what changed"` takes it back, on the work it stopped on, with the
bounds handed back whole. The reason is required and it is not paperwork: the
tree and the cause have not moved on their own, so the next run reads it
before anything else. A mission you called off yourself with `mission end` is
a decision, not a bound, and is refused.

### Following a mission

A mission runs for hours. Rather than polling `mission status`, block until it
needs someone:

```sh
nunki mission wait m1                  # returns when the mission stops
nunki mission wait m1 --timeout 8h     # or gives up after eight hours
nunki mission wait m1 --json           # the same, as one JSON object
```

It returns at once if the mission is already stopped, and otherwise reads the
state again every 30 seconds (`--every`). It prints one line — what stopped,
and who it waits on —

```text
m1 · findings · security round 1 of 1 found: … · awaits the HQ: `nunki mission iterate m1` sends it back to the coder, or `nunki mission accept m1 --because <why>` lifts it
```

and exits with a code that says which stop it is:

| Stop | Code |
|---|---|
| Verified, or already pushed or archived | 0 |
| Findings from the security agent | 10 |
| Handed over to the human, a ruling included | 11 |
| Held, or the account's window spent | 12 |
| The monitor stopped on an error, or a gate could not be played | 13 |
| No monitor runs while the mission is still being driven | 14 |
| `--timeout` reached | 15 |

`wait` drives nothing: it takes no lock and launches nothing, so two of them on
the same mission are harmless. (Opening the HQ's state creates its
directories if they are missing; that is all it may write.) The monitor ends
its log on the same line.

`--json` prints the same fields as the line, already made printable: what an
agent wrote reaches it with the same `\u{1b}` escapes, then JSON's own.

When the mission reads `VERIFIED`, the push is yours and yours alone:

```sh
nunki mission fetch m1      # bring its commits into the repository to read them
nunki push m1 --yes         # push the branch, and open the pull request
nunki mission archive m1    # close it: the folder and state move under archive/
```

`nunki push` opens the pull request when a GitHub token that may do so sits at
`~/.nunki/forge-token`, beside the accounts — it is yours, not a project's,
and every project on the same forge reads it. Without one it pushes the branch
and hands you the address to open the pull request at yourself.

GitHub is the one forge `nunki` has an adapter for. A remote on another one is
**said to be on another one**: the branch is still pushed, and no address is
guessed at from a shape that is GitHub's.

## Choosing a rigor

A mission declares how much verification asks of it, with `--rigor` on
`mission new`, or for the whole project with `rigor:` in `nunki.yaml`:

| | `prototype` | `standard` | `critical` (the default) |
|---|---|---|---|
| Gate 7, the mutation campaign | not played | passes when the share of tried mutants killed reaches `mutation_threshold` (80 unless `nunki.yaml` says otherwise); a survivor with an outcome, a coder's proposal included, counts as killed | every survivor needs an outcome — killed, a bug, the HQ's equivalence, or the coder's proposal of one |
| Security agent rounds | none | at most 1 | at most 3 |
| Integration | none | as declared | as declared |

`critical` is what a mission that says nothing gets. Use `standard` for
ordinary work, where a good share of killed mutants and one security round
are enough. Keep `critical` for code exposed to hostile input. A `prototype`
runs the coder and the mechanical gates only, so `mission new`, the start of
the mission and `mission reframe` all refuse one with a service or a security
agent.

Python and Next.js projects judge `standard` as `critical` until their
campaigns count changed lines: mutmut and Stryker are run over whole touched
files, so they give no count, and every survivor needs an outcome there.

### Gate 7's campaigns, full and partial

A mission's campaigns form a chain, and its unit is the **file**. The first
campaign is **full**: every touched file, mutated where the branch changed it
since its fork point. At `standard`, a later one — after a volet, say — is
**partial**: the same campaign, restricted to the touched files whose content
changed since the previous campaign's `HEAD`. Its base is still the fork
point, so each of those files is measured exactly as a full campaign would
measure it. Which files changed is git's answer on blobs (`git diff
--name-only --no-renames`); no diff hunk is ever read. A file renamed is a
file removed and a file added.

A campaign is partial only when all of these hold, and full otherwise, with
the reason in `MUTANTS.json`:

- the mission is at `standard`;
- a previous campaign of the mission is on file and gate 7 passes on it;
- its `HEAD` is an ancestor of the current one;
- it ran from the same fork point: a base merged in, or a hunk the base
  cherry-picked, moves what the branch changed in every file;
- it counted each file, and those counts add up to its total (below);
- the files changed since it could be listed, and every file it counted
  that did not change is one the branch touches at `HEAD`, present there;
- the stack's `mutation.sh` and its tool's version are the ones it ran with —
  read through the script's `# nunki-tool-version: <command>` line, which
  every shipped `mutation.sh` carries; a script without it runs full
  campaigns only;
- it was not asked with `nunki mission mutants --again`.

**Per-file counts.** Before its last line, `mutation.sh` prints one line per
file with that file's counts, `{"measured":"src/lib.rs","tried":12,"found":13}`,
and its last line says `"by_file":true`. nunki trusts them only when each
file is named once, every one is a path nunki handed that campaign, and
they add up to the totals. The Rust template gives them, keyed by the file
cargo-mutants' own listing names for each mutant. The Python and Next.js
ones measure whole files, give no count at all, and so never chain: every
campaign of theirs is full.

**How a chain is judged: once.** `MUTANTS.json` keeps, for each touched
file, the counts of the latest campaign that measured it. A file unchanged
since keeps its counts and its survivors exactly as they were — same file,
same lines — and what the HQ said on them comes back the way it does between
any two campaigns: a ruling as a proposal marked carried, awaiting
`--ratify`, and a refusal as a refusal. A file changed, removed or renamed
takes its old counts and survivors with it; what the new campaign found in
it replaces them. Gate 7, and `nunki push` after it, judge that
reconstruction once, at the mission's threshold: the sum of every file's
tried, and the survivors left without an outcome. These are the numbers one
full campaign at `HEAD` would give, under the assumption below. A volet that
re-mutates well-killed code cannot pad the share: its new counts replace the
old ones rather than adding to them.

A partial campaign whose own counts cannot be trusted — none given, ones that
do not add up, a count or a survivor in a file that did not change — is
recorded with every survivor listed and no count. Gate 7 then judges it as
`critical` does, and the next campaign is full. `mission status`, the
monitor's log and the follow-up name each campaign (full or partial, from
which commit, what it tried), then the one campaign gate 7 judged.

**The known limit.** A partial campaign assumes that a mutant measured
earlier in a file nobody has changed since still exists, and is still
killed. A volet that weakens a test can make the second false; one that
stops compiling an unchanged file — its `mod` line removed — makes the
first false, and that file keeps its counts. The chain will not see
either. `critical` never makes the assumption — its campaigns are always
full — and `--again` forces a full campaign at any rigor.

### Running mutants at once: `mutation_jobs`

`mutation_jobs` in `nunki.yaml` — a whole number of at least 1, refused
otherwise, `1` when absent — is how many mutants a campaign runs at once.
`nunki` hands it to the stack's `mutation.sh` in `NUNKI_MUTATION_JOBS`, as it
hands the base in `NUNKI_BASE`; the terminal line, the counts and what a
campaign's status means are the same at any value.

- **Rust:** at `1`, `cargo mutants --in-place`, one mutant at a time in the
  clean copy and its warm cache, as before. Above, `--jobs N --copy-vcs
  true`: each job builds in a copy of the tree of its own, from a cold
  cache, with `.git` beside it as in place, under
  `$TMPDIR/nunki-campaign-copies`, removed when the campaign ends.
- **Python:** mutmut's `--max-children N`. mutmut already runs its mutants
  in forked children; the setting caps how many. Without the variable
  nothing is passed and mutmut keeps its own default. Not run against a real
  mutmut here: it is not in this image.
- **Next.js:** Stryker's `--concurrency N`, the number of test runner
  processes. Not run against a real Stryker yet; an option it refused would
  fail the campaign, never pass it.

The default is `1` because more was not worth it where it was measured — on
this repository, 53 mutants, 12 cores, each run from an emptied cache:

| run | wall | per mutant | peak disk | outcome |
|---|---|---|---|---|
| in place | 533 s | 9.5 s | 2.5 GB | 45 caught, 2 missed, 1 timeout |
| 2 jobs | 463 s | 15.8 s | 4.8 GB | the same |
| 4 jobs | 492 s | 32.4 s | 9.1 GB | the same |
| 12 jobs | 607 s | 121 s | 26.8 GB | 41 caught, 0 missed, 7 timeouts |

Building one mutant already keeps every core busy, so jobs queue for them.
And a loaded machine overruns the test timeout cargo-mutants sets from an
unloaded baseline: at 12 jobs both survivors came back as timeouts, which
count as killed. Raise it for a project whose build leaves cores idle, and
measure there first.

### What a campaign leaves behind

A campaign leaves the build cache as it found it. Every mutant builds the
crate from its mutated source, and nothing used to remove what it built: a
slot after about thirty campaigns held 6.8 million files in its copy's
`target/debug/deps`, cargo scanned them at every build, and a mutant went
from 7 s to 70 s.

- **Rust:** `mutation.sh` records the cache's file list before cargo-mutants
  runs, and when the campaign ends — complete, failed, or stopped by `nunki`
  past its deadline — removes every file not on that list, and every listed
  file written since. The first is most of it: each rebuild hard-links its
  unchanged units' `.dwo` files under new names, which keep their old
  times, so a time marker alone would miss them. The second is the crate's
  own artefacts, which a mutant rewrites under the baseline's names. The
  dependencies stay, and the next build recompiles the crate once. A
  campaign killed outright leaves its list, and the next one cleans up
  first. Above one job, the copies go to `$TMPDIR/nunki-campaign-copies`,
  outside the tree, and are removed the same way. Measured against the real
  cargo-mutants on this repository, in place, at two jobs and stopped
  mid-run: no file the campaign added was left.
- **Python:** what mutmut leaves per mutant is all in `mutants/`, cleared
  before the run and when the script exits.
- **Next.js:** Stryker's sandbox, `.stryker-tmp/`, is removed by
  `--cleanTempDir always`, and cleared before the run for when Stryker was
  killed.

`nunki check --slot <name>` measures the copy's `target/` in the slot's
container, when it is up — it lifts nothing for this — and goes amber
(`!!`) past 250,000 files or 50 GiB: a sign the stack's `mutation.sh` is an
older one, or a project's own, that does not clean. Amber is not red, and
`nunki check` still exits as held. The thresholds are measured: a cache
built once for this repository's tests holds about 11,000 files, and one
uncleaned campaign of 23 mutants added 67,450. A cache past them is not
emptied by a newer `mutation.sh`; empty the copy's `target/` once, from
inside the slot.

The rigor is frozen in the header like the bounds, and `mission reframe`
shows a change of it. Once the security rounds are spent, the agent is not
called again. A spent cap never verifies a red verdict: if the last round
concluded `FINDINGS`, green gates bring the mission back to those findings,
where `mission accept` lifts them or `mission iterate` spends a volet, and
`FOLLOWUP_HQ.md` says the fix was not attacked again. A lifted report stays
lifted: a later review whose volet comes through the gates green is verified
without bringing it back. `nunki mission status` prints the rigor and the rounds played.

`nunki push` asks for a security verdict on the branch's `HEAD` while a round
is left. Once the rounds are spent, no verdict can come on a later commit, so
the last one stands for the commits after it: a `CLEAR` as it is, a `FINDINGS`
only once `mission accept` lifted it after it was concluded — never one nobody
lifted. Those commits are named, by push and in `FOLLOWUP_HQ.md`, as not
attacked by the security agent. A `prototype` plays no round, and push asks
it for no security verdict.

The security agent ranks every finding in `VERDICT.json` — `HIGH`, `MEDIUM`,
`LOW` or `INFO`, by what it lets an attacker do — and gives each `LOW` and
`INFO` one sentence on why it can be accepted. When it concludes `FINDINGS`
and every finding is `LOW` or `INFO`, `nunki` lifts the verdict itself, as
`mission accept` would: one acceptance for the verdict as a whole, recorded as
`nunki`'s and not a person's, its reason the findings and the agent's words.
The mission goes on to `Verified`, and `FOLLOWUP_HQ.md`, `nunki mission
status` and `nunki push` ("accepted by nunki (LOW/INFO)") list the findings
it lifted. One `MEDIUM` or `HIGH` stops at `Findings` as before, and so does
anything `nunki` cannot read for certain: no ranking at all, an empty list, an
unknown severity, a `LOW` without its reason. Such a lift obeys every rule
push applies to a human's: it is worth its commit, and a later `FINDINGS` is
not covered by it.

## Vocabulary

The mission is the parent of everything else. It moves through **stages**;
the coder's stage is cut into **lots** and **volets**; each lot is worked in
**attempts**; each attempt is **one run**.

```text
MISSION  (one branch off its base, one slot)
│
├─ stage CODER
│   ├─ lot L1 ─┬─ attempt 1 ── run
│   │          └─ attempt 2 ── run   ← after a failed attempt, in a fresh session
│   ├─ lot L2 … (one commit per lot, the session carried over from L1)
│   └─ volet 1, 2 …  ← back to the coder after a red verdict
│
├─ stage FINAL GATES 5 to 7 + mechanical security gates  (nunki alone, no agent)
├─ stage INTEGRATOR   (if the mission declares services) ── attempts ── runs
├─ stage SECURITY     (if the mission declares `security: agent`) ── attempts ── runs
│
└─ VERIFIED → the human validates → `nunki push`
```

### The mission and where it runs

- **Mission** — one piece of work, framed by a human. A folder at the HQ,
  outside the repository: `MISSION.md` (its header and brief),
  `FOLLOWUP_HQ.md` (what the HQ tells the agents), and the agent's own
  `JOURNAL.md`, `PR.md` and `VERDICT.json`. Its **header** is frozen when the
  mission starts: `nunki` never re-reads it, and `nunki mission reframe` is the only
  way to change it.
- **Slot** — the isolated clone and container where a mission runs, with an
  unreachable origin. One lock per slot: two `nunki` never drive the same slot.
- **Role** — the coder writes the code, lot by lot. The integrator wires it to
  the services it needs and writes system tests. The security agent attacks
  what was built. Each works in its own profile of the slot's container.

### Its progress

- **Stage** — where the mission stands, one at a time: coding, final gates,
  integration, security, findings (the security agent's, for a human to
  iterate on or lift), awaiting a human, verified. A stage the mission does
  not declare is absent by declaration, not skipped.
- **Lot** — a planned unit of the coder's work, listed in the header (`L1`,
  `L2`…). Small, with its own proof, committed in commits that stand alone.
  Only the coder has lots.
- **Volet** — unplanned coder work, opened by a red verdict (the integrator's
  `BROKEN`, the security agent's `FINDINGS`, or red final gates). Named
  `volet-1`, `volet-2`…, bounded by `max_volets` (3 by default). After a
  volet every later stage is replayed while rounds are left, because a
  verdict is worth one commit and no other; once the security rounds are
  spent, the last verdict stands for the commits after it (see "Choosing a
  rigor").
- **Attempt** — one try at a lot, a volet, or a role's mission. A failure
  costs one, up to `attempts_per_lot` (3 by default); then the mission is
  handed to the human. A harness failure, or a turn `nunki` ended at the
  account's threshold, costs none: the same attempt is replayed.

### A run

- **Run** — one launch of one role's agent in its container, from start to
  its result. It is what `nunki` launches, watches, stops and reads back, and
  what the caps count (`max_runs`, `max_tokens`). The coder has **one run per
  lot**: the run ends when the lot is done and proved, or when it fails.
- **Session** — the harness's conversation. The coder's is resumed from one
  lot to the next, and dropped after a failed attempt, so the next attempt
  starts fresh. The integrator and the security agent start a fresh one each
  run.
- **Resume block** — the `ÉTAT DE REPRISE` section at the top of
  `JOURNAL.md`, rewritten at every checkpoint and before a run stops. It
  names `HEAD`, the lot and the next action. The coder ends it with one line,
  the only one `nunki` reads to know the lot is done:

  ```text
  Lot: L2 — done
  Lot: L2 — failed: the integration test X does not pass
  Lot: L2 — awaits ruling: `src/lib.rs:3: replace + with -` changes nothing observable
  ```

  The third line is for one case: the lot is done but for mutation
  survivors the coder can neither kill nor freeze as a bug, and whose
  equivalence it is forbidden to rule on. Each is named between backquotes,
  and every backquoted span in `<what>` is read as a survivor id, so nothing
  else is quoted. `nunki` checks every one is a survivor of `MUTANTS.json`
  with no outcome (a line naming none is a failed attempt) and hands the
  mission over at once, as `AwaitingRuling`, without spending another
  attempt. The HQ rules (`nunki mission mutants <mission> --equivalent
  <survivor> --because <why>`), then `nunki mission retry` resumes the lot at
  the next attempt — not at the first, and never past the bound: a ruling
  asked on the last attempt is handed over as out of attempts by that retry,
  and only a further `retry` hands the attempts back whole.

  A coder that believes a survivor equivalent need not stop for it: it may
  **propose** the equivalence in `MUTANTS.triage.json`,
  `{"kind": "equivalent_proposed", "why": "<one sentence>"}` — only for a
  mutation that cannot change any observable behaviour, never for one that
  is merely hard to test. Gate 7 counts a proposal as an outcome (at
  `standard`, as a killed mutant), and its note counts proposals apart from
  killed mutants, bugs and the HQ's rulings; a blank `why` is no outcome.
  A survivor with a proposal is not open, so it cannot be awaited too. The
  decision stays the HQ's: `nunki mission mutants <mission> --ratify
  <survivor>` writes exactly the equivalence `--equivalent` writes, with the
  coder's sentence unless `--because` replaces it, and it is that ruling, never
  the proposal, that is carried to the next campaign; `--refuse <survivor>
  --because <why>` removes the proposal, records the refusal and reopens the
  survivor, and the next coder run reads why in `FOLLOWUP_HQ.md`.
  `nunki push` refuses while any proposal of the current campaign is neither
  ratified nor refused, naming each, and it plays gate 7's rule again on the
  campaign as it stands — every survivor answered at `critical`, the share
  reached at `standard` — so a survivor reopened after the gates is never
  pushed. A `--refuse` on a verified mission sends it back to the coder as a
  volet, with the refusal as its cause, and a survivor that already holds
  the HQ's ruling is refused with a pointer to `--lift`.

  **Between campaigns of one mission, `carry` never rules.** Which mutant a
  ruling was about is a heuristic across campaigns — an id is a position,
  and other code, even identical code, can land on it — so a ruling is only
  ever carried as a **proposal** from `nunki`, marked as carried, with its
  sentence and the commit it was given on, for the HQ to ratify or refuse
  (`--ratify --all` takes them all). The only equivalences in a campaign's
  `MUTANTS.json` are the ones the HQ typed on that campaign. `carry` looks
  at the whole previous campaign: the same id, file and description, or
  else — when that pair names exactly one survivor before and one now, and
  the code the mutation replaces occurs once in its file — the same file
  and description under another id. **A refusal is always carried**:
  a new id gets the refusal the HQ gave on any survivor of the same file and
  description, whatever the uniqueness, so a refused proposal written again is
  never an outcome. With no campaign on
  file at `standard` or `critical`, push refuses: no campaign, no push, and it
  names `nunki mission mutants <mission>`, the verb that writes one.
  `--ratify`, `--equivalent`, `--refuse` and `--lift` take the slot's lock
  and act on every survivor carrying the id they name; a mission whose state
  cannot be read is refused, and only one that has not started is ruled on
  without the lock. `mission status` and `mission wait` count the proposals
  that await the HQ by source — the coder, the registry, carried by file and
  mutation — `mission status` and `nunki push` name each with its source and
  sentence, and the follow-up lists them when the final gates pass.
  `--ratify --all` ratifies every pending proposal, whatever its source,
  printing each with its source and sentence first, and rules each with the
  sentence it printed; a proposal refused, ruled or reworded between the
  listing and the ruling fails the verb, and nothing is written. One that
  fails midway stops it, and it says which survivors it had ruled before.
  Ratifying a proposal from the registry keeps the entry's origin — its
  mission, commit, date and sentence — and records the ratification beside
  it.

  **From `MUTANTS.triage.json`, `nunki` reads only what the coder may give**:
  `killed`, `bug` and `equivalent_proposed`. Anything else written there —
  `equivalent`, `equivalent_registered`, `proposed_by_nunki`, or an entry it
  cannot read — is no outcome, never shadows what `MUTANTS.json` holds, and
  is named as refused in gate 7's note and in `nunki push`'s refusal.
  A named test is a name: letters, digits and underscores, three at least
  — a blank name, or one of a letter or two, answers nothing — and it is
  looked for as a whole word. `nunki push` asks, as gate 7 does, that every
  test an outcome names exists, in the tree of the commit it pushes, so
  text added uncommitted in the slot never counts.

  A ruling is given **once per project, for as long as its code stands**.
  `--equivalent` and `--ratify` also enter it in the project's registry of
  equivalences, `hq/equivalences.json` in the project's home — never in the
  repository, never mounted in an agent's container, and written by those
  verbs and `--lift` only: a proposal never enters it, and a refusal is not
  an equivalence. An entry holds the mutant's file and description (never
  its line number), the git blob id of the code the mutation replaces — every
  line of its span, as the tool's own listing gives it (`end_line` in the
  campaign's output; cargo-mutants' `mutants.json`), each trimmed at both
  ends and joined by newlines, so that `printf '%s' '<line>' | git
  hash-object --stdin` reproduces a one-line span — its number of lines, the
  sentence, who ruled, the mission, the commit and the date.

  **Only what is identified without a doubt.** A ruling is entered only when
  the campaign it is given on holds exactly one survivor on its file and
  description, the span is known, and its text occurs exactly once in the
  file at the campaign's commit; otherwise the ruling stands on its mission,
  and the verb says on stderr why the registry was left alone.

  **The registry proposes, it never rules.** When any mission's campaign is
  recorded, a survivor with no outcome and no refusal whose file and
  description match exactly one entry and one survivor, and whose span now
  has the same digest and occurs exactly once in the file, receives a
  **proposal**: `proposed_by_nunki` in `MUTANTS.json`, carrying the HQ's
  sentence and the mission, commit and date it was ruled on. Code that only
  moved still matches; code any line of which changed matches nothing. Gate 7
  counts it as a proposal (an outcome at `critical`, in the share at
  `standard`), the mission goes on, and `nunki push` refuses until the HQ
  ratifies or refuses it, exactly as for a coder's proposal: a wrong match
  costs one refusal, never a wrong ruling. The coder cannot write one. A
  `MUTANTS.json` an older `nunki` wrote with `equivalent_registered` reads as
  a proposal from the registry. A registry proposal is never carried between
  campaigns: each campaign asks the registry again. Ruling again on a
  mutation replaces its entry. The registry is read under its lock; one that
  cannot be read — a zero-length file included — proposes nothing, and
  `FOLLOWUP_HQ.md` says so; a span that cannot be read matches nothing. The
  source is read through objects `nunki` hashes itself, with replace refs and
  grafts off, and a tree that names an entry twice or out of git's order is
  read as nothing. `--lift` on any mission takes a ruling — or `nunki`'s
  proposal of one — out of the registry first and then out of the mission:
  when the registry cannot be locked, read or written, it fails and changes
  nothing, and it still cleans a registry entry its mission no longer holds.
  `nunki mission mutants --registry` lists the entries, each with whether its
  code still stands at the repository's `HEAD` — once, changed, or repeated.

- **Gates** — deterministic checks, each read from its own result. Gates 1 to
  4 (a clean tree, a branch ahead of its base, a resume block that names
  `HEAD`, the perimeter) are played at the end of every coder run. Gates 5 to
  7 (the deliverable, the battery on a clean copy of `HEAD`, the mutation
  campaign) are played at the final verification. A gate nobody can play
  stops the flow rather than counting as green — except gate 7's missing or
  stale campaign, which `nunki` runs itself before looking again.
- **Verdict** — how the integrator (`INTEGRATED` / `BROKEN`) and the security
  agent (`CLEAR` / `FINDINGS`) conclude, in `VERDICT.json`, pinned to the
  `HEAD` it judged. The coder's verdict is implicit: its gates were green.

### Waiting and holding

- **Hold** — `nunki` launches no further run until a human lifts it with
  `nunki mission resume`. Set by `nunki mission stop`, or by `nunki` itself when a cap
  is reached or the harness keeps failing past its ceiling.
- **Wait** — a pause that ends by itself: the harness backing off after a
  failure, the account's five-hour or weekly window past its threshold, or a
  real provider (`shared: true`) held by another mission's integration run.
- **Monitor** — one background process per mission (`nunki mission monitor`,
  started by `mission start`, `verify` and `mission resume`). It watches a run
  every minute, stops it at the account's threshold, and calls `verify` again
  when a wait ends. It stops as soon as the mission needs a human.

## The verbs a human types

`nunki init` sets a repository up. `nunki mission new` frames a mission and
`nunki mission start` launches its first run. From then on `nunki verify` reads back
and launches whatever is owed; the monitor calls it for you. `nunki mission
status`, `logs` and `watch` show where it is; `pause`, `stop`, `resume`,
`kill` and `say` act on a run. `nunki push` carries a verified branch to the
forge, on `--yes`.

Two more are worth knowing. `nunki exec` runs a command in a slot's container,
which is how you replay a proof without having the stack on your machine.
`nunki mission gates` plays gates 1 to 4 on what a slot holds, asking the agent
nothing.

`nunki --help`, and `nunki <verb> --help`, say the rest — every verb carries its
own reasons.

## Uninstall

`nunki` keeps no manifest and installs no hook, so removing it is removing what
you can see. Nothing below is done for you.

Per project, from the repository:

```sh
nunki mission archive <id>          # or `nunki mission end <id> --because "..."`
nunki slot rm one                   # add --force to discard work it still holds
rm AGENTS.md CLAUDE.md
nunki sessions                  # which identifier this repository holds
rm -rf ~/.nunki/<id>            # configuration, HQ, stack fragments
```

Then remove that identifier's line from `~/.nunki/sessions.json`: `nunki`
keeps no manifest, and it will not tidy the ledger for you.

`nunki slot rm` refuses while a slot holds commits the repository lacks:
`nunki mission fetch` brings them over first. `AGENTS.md` may be yours by now —
read it before deleting it — and `nunki init`'s `.gitattributes` entry is the
last trace in the tree.

Then the images and the volumes, which are named after the project and the
slots:

```sh
docker image ls  --filter reference='nunki/*'   # <project>-<stack>, firewall, prober
docker volume ls --filter name=nunki-           # a slot's caches and harness state
```

Remove what those two list. A slot's containers belong to a Compose project
named `nunki-<session>-<slot>`, the session being the first eight characters
of the one `~/.nunki/sessions.json` gives this repository — two projects with
a slot of the same name are two Compose projects, two networks and two sets
of volumes. `nunki slot rm` takes them down with the slot, and `nunki slot
reset` clears the volumes while keeping the clone.

Finally, the binary and the accounts, once no project uses them:

```sh
cargo uninstall nunki
rm -rf ~/.nunki/accounts ~/.nunki/accounts.yaml ~/.nunki/usage
```
