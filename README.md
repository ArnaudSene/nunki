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
nunki init --stack rust         # rust is the stack shipped today
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
  --model claude-sonnet-5 \
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
  volet every later stage is replayed, because a verdict is worth one commit
  and no other.
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
  ```

- **Gates** — deterministic checks, each read from its own result. Gates 1 to
  4 (a clean tree, a branch ahead of its base, a resume block that names
  `HEAD`, the perimeter) are played at the end of every coder run. Gates 5 to
  7 (the deliverable, the battery on a clean copy of `HEAD`, the mutation
  campaign) are played at the final verification.
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
named `nunki-<slot>`; `nunki slot rm` takes them down with the slot, and
`nunki slot reset` clears the volumes while keeping the clone.

Finally, the binary and the accounts, once no project uses them:

```sh
cargo uninstall nunki
rm -rf ~/.nunki/accounts ~/.nunki/accounts.yaml ~/.nunki/usage
```
