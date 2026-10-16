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

**The security model, in one paragraph.** An agent runs in a container with
no capability, under your uid, behind a firewall sidecar it cannot reach; it
sees its slot's tree and its mission folder, nothing else. What it writes in
the slot — code, but also the slot's `.git`: configuration, hooks, refs — is
never executed or believed on your machine. `nunki` runs no git inside a slot:
every git it needs about one runs in a **mirror** beside the slot, outside
every mount, that only the host writes. The slot's commits reach the mirror
through a fetch that hashes every object, its refs are read from their files,
and no configuration of the slot's is ever read — so a `core.fsmonitor`, a
hook, a filter or a pager planted there has nothing to run it. Nor does a
git of the host's ever start inside the slot's tree: **slots do not support
submodules**. A commit holding a gitlink is refused, with nothing run, by
every operation that would take the tree as work tree (gate 1, `slot reset`,
the launch's checkout), and those few run with submodule descent switched
off as well. A slot whose object store borrows another repository's
(`objects/info/alternates`), or holds anything but plain files and
directories, is refused before it is read. Whether a
slot's tree is clean is `nunki`'s own comparison with `HEAD`, byte for byte,
never what the slot's index says nor what its `.gitattributes` would make of
a file — so a project that relies on conversion at checkout (`text`,
`eol`, `ident`) reads as not clean. The gates measure a branch from the base
`nunki` recorded itself from your repository, never from a ref the slot
holds. `mission fetch` and `nunki push` take the mission's branch alone, and
only when it is the very commit the gates and verdicts judged: never a tag
the agent made, nor a commit added after. `nunki check` names any key a
slot's `.git/config` carries that git would execute, and any gitlink its
`HEAD` holds. What `nunki`
writes back into a slot (a branch, its checkout) it writes as files. Nothing
reaches the forge but `nunki push`, on your explicit word.

## Install

What you need:

- **Rust 1.89 or later**, to build `nunki` — `rustup` gives you one.
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
`security_findings` decides what the Security section `nunki push` writes
says while the security agent's findings are accepted and not fixed: their
count only (`withhold`, the default), or the report and every finding
(`publish`).
Then commit the three files `nunki init` wrote in the repository, build the
images and clone a slot:

```sh
nunki slot rebuild --stack rust # the agent's image and the firewall sidecar
nunki slot add one              # a clone at ../<project>-slots/one
nunki slot list                 # each slot, and the mission that occupies it
nunki check                     # what is held, and what could not be checked
```

### The project's services

A mission whose integration declares services needs them lifted beside the
integrator. `services_file` in `nunki.yaml` names a file of the repository
that declares them, in a closed subset of Compose. The file is written by
whoever writes the tree, the coder included, so nunki does not hand it to
the engine as it is. It reads it into a model of its own and refuses the
**whole file** at the first thing outside it:

- at the top level, `services` and `volumes`, and a `name`, which is ignored;
- per service, `image`, `environment`, `command`, `entrypoint`, `healthcheck`
  (`test`, `interval`, `timeout`, `retries`, `start_period`,
  `start_interval`), `depends_on` (services of the same file, with a
  `condition` or none), `working_dir` (an absolute path) and `volumes`;
- `environment` is a mapping from a name to a non-empty quoted string. A
  bare entry would be filled from your environment, so it is refused;
- a mount names a volume declared in the file's own `volumes` block, by name
  only (`dbdata:`, no value), at an absolute path, optionally `:ro`;
- `ports` are read and dropped: a service is reached by its name.

Everything else is refused, with the service and the key named:
`privileged`, `network_mode`, a bind mount, `env_file`, `build`, `user`, a
top-level `networks`, an `x-` field, and any key a later Compose adds. So is
any YAML construct that resolves to something other than what is written:
anchors, aliases, tags, merge keys (`<<`), duplicate keys, directives and a
second document. Whether the file holds an anchor, an alias or a tag is
what the YAML parser itself reports, wherever the construct stands. So is
a raw NEL, LS or PS character (U+0085, U+2028, U+2029), which a YAML 1.1
reader takes for a line break, and a carriage return not followed by a line
feed: lines end with LF or CRLF. Write the character as an escape (`"\N"`,
`"\L"`, `"\P"`, `"\r"`) if you mean it.

A tag is refused in every YAML file nunki reads, not only here — `nunki.yaml`,
a mission header, `accounts.yaml`, `me.yaml` and the profiles nunki writes:
two YAML libraries read the same tag to different values (`!!binary aGk=`
is `aGk=` to one and `hi` to the other), and nunki writes none.

A plain scalar that reads as a number is a number: `image: 012`, a
variable set to `1_000` or a volume named `00` is refused where a name or a
string is expected. Quote it (`"012"`). nunki's reader follows YAML 1.2:
only `true` and `false` are booleans, so `no` and `on` stay strings.

What is lifted is not the file but nunki's rendering of it. Keys come in a
fixed order, every string is quoted and written in ASCII, and every `$` is
doubled so Compose substitutes nothing. A file nunki refuses starts no
service, and the profile that needed it does not start either.

**You approve that rendering, by its digest.** nunki reads the file from
the commit the slot's `HEAD` names, never from the tree, so an edit that is
not committed changes nothing. It lifts the services only if you approved
the digest of that rendering:

```sh
nunki services --show [<mission>]   # the rendering, its digest, approved or not
nunki services --approve <digest>   # approve it, if the file has not changed since
```

`--show` reads the repository's `HEAD`, or the slot of the mission you name.
`--approve` approves a digest only while it is still the rendering of the
file in the repository or one of the slots. If the file changed after you
looked, the approval is refused. Approvals live in the project's HQ
(`hq/services.json`, with who approved what and when, and the rendering
itself). No container mounts that file. A definition you approved once
serves every mission until it changes.

When a change comes in — a coder's edit included — the system profile is
not started. The mission is handed back to you with the file and the new
digest; `nunki check --mission <id>` and `--slot <name>` show the same as a
red line. Read the rendering, approve it, then
`nunki mission retry <id> --because <what you approved>`. Nothing was spent
in the meantime.

A project that already had a services file before approvals existed needs
nothing else. `nunki services --show` prints its digest, and one command,
`nunki services --approve <that digest>`, is its first approval.

Before a profile starts, nunki takes down the services of the slot whose
definition is not the approved one, such as an older version or a service
the file no longer names. It removes their containers and never their
volumes. `nunki exec` and the gates, when they find a slot down, lift the
profile the last launch wrote through the same check, and start nothing
if one of its services is not an approved definition.

**nunki fences what it lifts.** A system profile starts only the services
the mission declares, and the ones they depend on. Every other service in
the file stays down. Each lifted service gets a set of restrictions from
nunki itself, and the file supplies none of them:

- it joins one network, nunki's, which is declared `internal` and so has no
  route out, and which has no gateway address on the host, so the host's
  own ports are out of reach too: no IPv4 address on its bridge (a Docker
  bridge option; not measured on Podman), and IPv6 turned off on it. The
  firewall joins that network too, so the agent reaches a service by its
  name, through the firewall's rules.
- no port is published. The launch says in one line which `ports` it
  dropped.
- every capability is dropped, and a fixed few are given back (`CHOWN`,
  `SETGID`, `SETUID`), which is what the official images measured need to
  start from root and drop to their own user.
- it gets `no-new-privileges`, a 2 GB memory limit and a 1024-process
  limit.
- it has no CPU bound and no disk bound: a runaway service can slow the
  machine and fill its disk, not leave the fence.

The coder's profile starts no service. The ones already up keep running
across the switch: nunki never removes them for being absent from a
profile, and only takes down definitions nobody approved.

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
reads, and prose the agent reads. Every string nunki writes there, as in the
Compose files it generates, is quoted (`branch: 'mission/first'`), so that
no reader takes a date or `1_000` for something else. Edit the prose, then
start it:

```sh
nunki mission start m1 --slot one
```

The slot is put on the mission's branch, made from the project's base as the
repository has it now, and the slot's own copy of that base (`dev`, say) is
brought forward to it — only forward: one holding a commit the project lacks
is left where it is, and `mission start` says so. The coder's first run is
launched, detached, and a monitor is started for the mission. From then on
`nunki` drives: it reads each run back, plays the gates, launches the next
lot, waits out a harness that fails, and stops a run that would pass your
subscription's threshold.

While it works:

```sh
nunki mission status m1     # the stage, the run, what it has spent
nunki logs m1 --last        # the run, readably
nunki mission watch m1      # follow the run in progress
nunki verify m1             # read back and launch what is owed, by hand
```

If you need to intervene: `nunki mission stop m1 --because "why"` holds the
mission (`--now` ends the turn in progress too) — the reason is required, and
it is shown wherever the hold is: `mission status`, `watch`, `wait`, `verify`,
the monitor's last line — `nunki mission resume m1` lifts the hold,
`nunki mission say m1 "..."` leaves an instruction for the next run, and
`nunki mission kill m1` is the emergency brake.

`nunki mission list` gives each mission's stage in one word (`framed` for one
never started, then `coding`, `gates`, `integration`, `security`, `findings`,
`campaign`, `handed-over`, `verified`, `pushed`), and `--active` keeps only the
missions started and not archived. `nunki slot list` says which mission
occupies each slot, or that it is free. A mission framed and never started is
closed like any other: `nunki mission end <id> --because "..."` writes why to
its `FOLLOWUP_HQ.md`, and `nunki mission archive <id>` moves its folder.

What a run leaves running in the agent's container is stopped once it ends:
every process a run starts carries `NUNKI_RUN=<its session>` in its
environment, and once the run has ended, `nunki verify` stops every process
still carrying it — `TERM`, then `KILL` a few seconds later — and says how
many. A convenience rather than a guard: a process that clears its own
environment is not found, and what contains an agent is its container.

When a bound runs out — a lot's attempts, a role's, or the returns to the
coder — `nunki` stops and hands the mission to you. `nunki mission retry m1
--because "what changed"` takes it back, on the work it stopped on. A lot's or
a role's attempts are handed back whole. The returns to the coder are not:
they are counted over the mission's whole life, so a retry on spent volets
grants **one** — written, dated, in `FOLLOWUP_HQ.md` as "Volet 4 of 3, granted
by the HQ: <why>" — and the next red verdict hands the mission back again.
`mission status` prints `volets    4 / 3 (1 granted by the HQ)`, so how far past
its cap a mission went is never a guess. A retry from any other handover
leaves the volet count alone. The reason is required and it is not
paperwork: the tree and the cause have not moved on their own, so the next
run reads it before anything else. A mission you called off yourself with
`mission end` is a decision, not a bound, and is refused.

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

A report the rigor does not count as blocking says so, and names `accept`
first:

```text
m1 · findings · security round 1 of 1 found: … — it does not block: at standard, security round 1, a report whose worst finding is MEDIUM does not block — only a HIGH does · awaits the HQ: `nunki mission accept m1 --because <why>` lifts it, or `nunki mission iterate m1 --override --because <why>` sends it back as a departure from the rigor
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
| Gate 7's campaigns | none | the first full, a later one partial when the chain allows it | the same chain, to `Verified` and to `nunki push`; gate 7 is played again at the final `HEAD` before `Verified` |
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
since its fork point. At `standard` and at `critical`, a later one — after
a volet, say — is **partial**: the same campaign, restricted to the touched
files whose content changed since the previous campaign's `HEAD`. Its base
is still the fork point, so each of those files is measured exactly as a
full campaign would measure it. Which files changed is git's answer on
blobs (`git diff --name-only --no-renames`); no diff hunk is ever read. A
file renamed is a file removed and a file added.

A campaign is partial only when all of these hold, and full otherwise, with
the reason in `MUTANTS.json`:

- the mission is at `standard` or `critical` (a `prototype` runs none);
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
  campaigns only. With several stacks, only those the campaign calls count —
  the ones owning a touched path — and each stack's command runs where its
  script does, in the stack's own directory. A stack the branch did not
  touch neither counts nor can make the campaign full. When a version
  cannot be read, the reason in `MUTANTS.json` names the stack, the command
  and what it answered;
- it was not asked with `nunki mission mutants --again`.

**Per-file counts.** Before its last line, `mutation.sh` prints one line per
file with that file's counts, `{"measured":"src/lib.rs","tried":12,"found":13}`,
and its last line says `"by_file":true`. nunki trusts them only when each
file is named once, every one is a path nunki handed that campaign, and
they add up to the totals. The Rust template gives them, keyed by the file
cargo-mutants' own account names for each mutant. The Python and Next.js
ones measure whole files, give no count at all, and so never chain: every
campaign of theirs is full.

**Survivors reported.** The last line may also say how many survivors the
tool reported, `"missed":3`, taken from the tool's own account and never
from the lines the script printed. When it does, nunki compares it with the
survivor lines it read, and a campaign where the two differ — fewer lines,
a survivor lost; more, a line something else wrote — measured nothing: it
is not recorded, gate 7 is red with both numbers, and `nunki push` has
nothing to answer with. The Rust template gives it, from cargo-mutants'
`outcomes.json`, and writes every line with `jq`, so a tab or a newline in
a file's name is carried, escaped, and never splits a survivor in two. The
Python and Next.js templates write theirs with a JSON encoder and give no
count: their tools keep no tally of their own over the touched files, and
a count taken from the lines they print would always agree with them. A
survivor's name is printed with its control characters escaped (`\u{9}`).

**How a chain is judged: once.** `MUTANTS.json` keeps, for each touched
file, the counts of the latest campaign that measured it. A file unchanged
since keeps its counts and its survivors exactly as they were — same file,
same lines — and what the HQ said on them comes back the way it does between
any two campaigns: a ruling as a proposal marked carried, awaiting
`--ratify`, and a refusal as a refusal. A file changed, removed or renamed
takes its old counts and survivors with it; what the new campaign found in
it replaces them. Gate 7, and `nunki push` after it, judge that
reconstruction once, as they judge any campaign of the mission's rigor: at
`standard`, at its threshold — the sum of every file's tried, and the
survivors left without an outcome; at `critical`, with no threshold — every
survivor listed, kept from an unchanged file or found by the latest
campaign, needs an outcome. These are the numbers one
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
either, at any rigor, and nothing in the mission does. `--again` forces a
full campaign, and today it is the only remedy: a campaign outside the
mission, over the whole code, is decided in SPEC 4.4 and not yet built.

**The final campaign, at `critical`.** The chain holds to the end: a
`critical` mission is verified, and pushed, on a campaign at its final
`HEAD` that passes, partial or full. When every other stage it owes is
green — the gates, the integrator and the security verdict as its shape
requires: `CLEAR`, a lift, or rounds spent with nothing left to lift — it
stands at a last stage, the final campaign, where gate 7 is played again
alone:

- a campaign on file at `HEAD` passes: the mission is `Verified`. When
  nothing moved `HEAD` since the gates, that is the gates' own campaign,
  and nothing runs again;
- none is on file at `HEAD`'s content — a commit came after the gates:
  `nunki` launches one, partial when the chain allows it, watched like any
  campaign, and looks again once it is read back;
- it has survivors without an outcome: they go back to the coder as a volet,
  as any campaign's do, and gate 7 is played again at this stage before
  `Verified`.

`nunki push` at `critical` refuses unless the campaign on file was run on
the content of the pushed commit and passes; its refusal names
`nunki mission mutants <mission>`, which runs one there.

### A budget, and a sample when the campaign does not fit it

The mission's `mutation_minutes` (45 by default) is each campaign's
**budget**. `nunki` hands the stack's `mutation.sh` three values:
- the budget in seconds, `NUNKI_MUTATION_BUDGET`;
- the seconds a mutant took on the project's last campaign that tried at
  least five, `NUNKI_MUTATION_PACE`. It is kept in `campaign-pace.json`
  under the project's HQ; the script assumes 60 without it;
- for a partial campaign, the fraction of the campaign it continues,
  `NUNKI_MUTATION_SAMPLE`.

The Rust script lists the mutants its scope reaches (`cargo mutants --list
--json`). When they do not fit the budget, it measures a fraction of them,
1/*k*, with *k* as small as fits. The budget holds `budget / pace` mutants,
less one for the baseline, which rebuilds the crate as a mutant does.

- **The sample is chosen before the tool starts, by the key that carries
  rulings.** That key is the file and the description, never the line. A
  mutant is in when its key's hash is 0 modulo *k*. A mutant a return to the
  coder did not change stays in the sample whatever moved above it, and one
  added elsewhere does not reshuffle the others. Choosing by position
  (`--shard`) would, and every return would bring survivors never seen.
- **The sample reaches the tool as one `-F`,** and is checked first:
  `--list` with that filter must name exactly the mutants chosen. A listing
  that fails, a filter that selects otherwise, or an empty sample when the
  scope holds mutants is a campaign that measured nothing, never a run of
  the whole.
- **What it ran at is recorded.** The script says it on a line before its
  terminal one, `{"sampled":k,"listed":N,"paced":s,"pace":m}`. `MUTANTS.json`
  keeps the fraction in the chain, and `mission status` says "a sample of
  1/3 of 48". Under the filter the tool's own tallies are the sample's, so
  gate 7 judges the sample whole: the threshold at `standard`, an outcome
  for every sampled survivor at `critical`.
- **A partial campaign runs at its chain's fraction,** even when its few
  files would fit whole, so that the reconstruction is one sample. At
  another fraction, it is not recorded alone. A full campaign sizes its
  own.
- **Rust only.** The Python and Next.js scripts ignore the budget and run
  whole. On a project with several stacks, the Rust stack gets the whole
  budget and the others run whole after it, under the same deadline.
- **What is lost.** A real survivor outside the sample passes the mission.
  The off-mission campaign SPEC 4.4 records as decided is what would find
  it.

Measured on cargo-mutants 27.1.0, in the Rust image, on this repository:
- **The sample and its check.** The 48 mutants of a diff, with a budget
  that fits 16 of them, gave a sample of 1/3: 18 mutants. The filter
  checked out, only those 18 ran, and each had the outcome it had in the
  whole campaign: 17 caught, 1 unviable.
- **One more commit.** It added 9 mutants in another file, run at the
  chain's fraction. The sample kept the 18, took 4 of the 9 new ones, and
  dropped none.

### A campaign stopped short: the deadline, and `--cancel`

A campaign's deadline is its budget and half of it again: 68 minutes for
the default 45. It only guards against a campaign that hangs, and gate 7
and the launcher read it through one function. After the upgrade, a
campaign on file that was stopped at its old deadline, the budget itself,
is owed once more, since the deadline is now longer. One still running past
its deadline is stopped, and it is **not** read as a campaign that
measured nothing: the survivors the tool had already reported are recorded
in `MUTANTS.json`, marked as from an incomplete campaign (`"stopped"`), with
the HQ's rulings carried as on any replay. An incomplete campaign **never**
makes gate 7 green, nor lets `nunki push` through, nor lets the next
campaign be partial on it. It does not go back to the coder on its own
either: gate 7 stops the flow for the HQ, which chooses —

- raise `mutation_minutes` in `MISSION.md` and `nunki mission reframe
  <mission> --yes`: past the deadline it was stopped at, a campaign is owed
  again, and the monitor starts it;
- or send the survivors found so far to the coder with `nunki mission
  iterate <mission> --because <why>`: a volet, as a red gate 7 opens one,
  with the reason in `FOLLOWUP_HQ.md`.

**Following a campaign.** `nunki mission mutants <mission>` with no flag
starts a campaign whenever none is in flight, so a loop that polls it to wait
for the end starts a new one each time the last one ends.
`nunki mission mutants <mission> --status` only reads: it says the campaign
in flight in the mission's slot — since when, how far into its deadline,
full or partial, how many lines its log holds — or that none is, then the
campaigns on file. It asks the container nothing and reads nothing back, so
what it says is the record's word until a reading asks; `nunki mission
status` says the same line first.

`nunki mission mutants <mission> --cancel --because <why>` calls off the
campaign in flight, for a campaign the HQ knows is wasted: it is stopped as
the deadline stops one, recorded as incomplete in the same way, and the
reason is written to `FOLLOWUP_HQ.md`. The mission stays at its gates, where
`iterate --because` sends the survivors on, or `nunki mission mutants
<mission> --again` runs a new campaign. A campaign that had already ended is
read back instead, and its result kept; a paused one is refused until
`nunki mission resume`.

**How a campaign is stopped.** `nunki` records only one pid for a campaign:
the script's. Stopping that pid alone would leave `cargo-mutants`, its
builds and its test binaries running, reparented, and still rewriting the
clean copy of `HEAD`. So the deadline, `--cancel` and a profile switch all
go through the same stop, and it stops the **whole** campaign. Every
process the campaign started carries its `NUNKI_RUN` tag (the stem of its
log), at whatever depth. Every process in the agent container that carries
the tag gets `TERM`, then `KILL` if it is still there a few seconds later.
The stop needs only a POSIX shell, `tr` and `grep`, and no `procps`. It
runs `/bin/sh` by its path, and finds `tr`, `grep` and `sleep` in the
system's directories only, never on the `PATH` the container was given,
whose first directory the agent's own uid can write.

Then the container is **asked again** what still carries the tag, twice, a
second apart, so that a process that forks and exits between two entries
of `/proc` is not missed. Zombies are not counted, since they run nothing.
Only two answers of "nothing" count as a stop. A container that has exited,
or that the engine answered it no longer knows, also counts, because
nothing can run in it. An engine that could not be reached is no answer.

A campaign whose **script has ended** is stopped the same way before it is
read back: what the script started may still be running, whether the
script finished, died, or was killed by something the campaign ran. And a
campaign whose state could not be asked at all is not forgotten: its
record stays filed, and the next reading asks again.

- **Nothing left:** the campaign is recorded as incomplete, as above (one
  whose script had ended is read back as it always was instead, and its
  copy is left to the refresh every door runs first), and
  its record is forgotten. Then the clean copy is reset to its commit, by
  the same refresh the battery and the gates use, so that no mutated file
  stays in it. The verb says that the copy was reset, or that it could
  **not** be reset and why. A `--cancel` whose reset failed exits non-zero.
- **Something left, or the check could not run** (the engine did not
  answer, a script did not say it had run): the campaign's record **stays
  filed**. So every door into the copy stays shut: the battery, gate 8,
  `nunki exec`, `nunki mission gates` and the next campaign. Nothing is
  recorded as stopped, and nothing is written to `MUTANTS.json` or
  `FOLLOWUP_HQ.md`.
  - `--cancel` fails, naming the pids still running, or saying why it
    could not tell. Running the same command again tries the stop again.
  - Past the deadline, the campaign is reported as still running and past
    its deadline, with the same words. `nunki mission mutants <mission>`
    exits non-zero, and the next reading tries the stop again. A campaign
    whose script ended, or whose state could not be asked, is reported the
    same way.
- **A profile switch** stops the campaign the same way, but a leftover
  does not fail the switch: the switch removes the agent's container right
  after, and every process in it goes with it. The record is forgotten only
  once that container is removed, never before. If the removal fails, the
  record stays.

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

### Campaigns outside missions: `nunki mutants`

A mission measures what its branch changed, and a sample of it when it does
not fit the budget. What neither sees is caught by a campaign you launch on
the project's base, when you choose: there is no scheduler.

```sh
nunki mutants run --slot <slot>            # the files changed since the last completed one
nunki mutants run --slot <slot> --whole    # the whole code
nunki mutants status                       # read it back, and say the debt
```

- **What it runs on.** The base as your repository has it: `--base`, `dev`
  by default as for `mission new`, at the commit the host records, never a
  ref of the slot. It runs in the clean copy of a slot no mission holds, put
  on that commit; the slot's own checkout does not move. A slot a mission
  holds is refused, and so is one no mission ever ran in, which has no
  container yet.
- **The two forms.**
  - Without `--whole`, it runs on the files changed since the last
    completed campaign outside missions, each mutated whole: the fork point
    is the empty tree, so every line of a file handed counts as changed
    (measured on 27.1.0, `--in-diff` against it lists the same mutants as
    `--file`), and a file removed, or with nothing to mutate, measures
    nothing, where `--file` with no fork point leaves no `outcomes.json` and
    the campaign could not run. With none
    completed, it is refused and names `--whole`. With nothing changed
    since, nothing starts.
  - With `--whole`, it runs on every tracked file with no fork point. Each
    stack reads the files it owns; cargo-mutants ignores the paths that are
    not Rust (measured on 27.1.0).
- **No sample, a long deadline.** It is where the whole is measured, so it
  is given no budget. `--minutes` (720 by default) only stops one that hangs.
- **Asking never starts one.** `run` refuses while a campaign is filed in
  the slot. `status` reads it back, asking the container, unlike `nunki
  mission mutants --status`, which reads only the record. It never starts
  one, so a loop polling it waits.
- **Never a mission's.** Its in-flight record is the slot's, marked outside
  missions. A mission's reading of that slot, the launch of its campaign or
  of its run, is refused and names `nunki mutants status`.
- **The debt.** A campaign that said it finished and measured, by the same
  rules as a mission's, lands in the project's debt (`mutants-debt.json`
  under the HQ), with the registry of equivalences applied.
  - A campaign on the whole code replaces the debt.
  - One on the files changed since replaces, for each of them (removed
    and renamed ones included), what the debt owed there with what it found
    there, and keeps the survivors of unchanged files as they were: the
    chain's rule, and its assumption that a mutant measured in a file nobody
    changed still exists and still survives. No hunk is read. Measured on
    changed lines only, an owed survivor nobody measured again stayed owed
    at a line an edit had moved, and a new survivor could take its entry
    under the same key: the description names the function, not the
    position, and `a > b && c > d` gives two `replace > with == in keep`.
  - So a file's survivors all come from one campaign, two under one key
    stay two, and one found again carries what its campaign said of it. No
    two entries share an id: one found under an id already owed replaces
    it. A campaign filed before this rule, on changed lines only, drops
    nothing by file.
  - A campaign stopped short, or that measured nothing, leaves the debt as
    it was.
- **Not built yet:** listing and ruling the debt in one go (`nunki mutants
  debt`), and the hardening mission that writes the tests. SPEC 4.4 records
  both as decided.
- **The disk.** The sweep and the 2 GiB floor apply here as in a mission.
  What the sweep frees on a large crate is not measured, and on a nearly
  full disk shared with other services, a whole-crate campaign will meet
  the floor before it ends.

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

  While the tool runs, every 30 seconds, a `.dwo` in `target/debug/deps`
  is removed when its name is not on that list and no build has touched its
  file for ten minutes; above one job, the same in the copies. The image
  keeps debug information beside the objects
  (`CARGO_PROFILE_DEV_SPLIT_DEBUGINFO=unpacked`), so every mutant's rebuild
  writes them under new names: measured on a crate whose mutant takes 47 s,
  about 11 GB an hour until the campaign ended. A rebuild links every
  unchanged unit's `.dwo` from rustc's incremental session under a new
  name, which moves the file's status time: a file a build still uses keeps
  moving, and its extra names cost nothing. What fills the disk is a file
  no build uses any more, a unit a mutant changed and the one it replaced,
  held only by names in `deps`: its time stops, and ten minutes on, all
  those names go. No test reads a `.dwo`, but a build in progress does:
  swept as soon as they were written, one mutant out of 48 failed to build
  and was counted unviable instead of caught. Hence the ten minutes, and
  rustc's incremental directories are never swept.
  Measured on this repository, the same 48 mutants in place: the results
  are the same mutant by mutant with and without the sweep (40 caught,
  8 unviable), and the sweep removed 5,946 names, whose files weighed
  about 120 MB, in 19 minutes; the space it freed is not measured. This
  repository's leak is small; on the crate that showed it, what the sweep
  removes there is not measured.

  Each sweep also reads the space left under `target/`: below 2 GiB, the
  tool is stopped and the campaign fails, saying how much was left; below
  2 GiB at the start, it does not start. A campaign that did not complete
  names the status cargo-mutants gave, and explains only that one. A
  limit: a campaign stopped at the floor writes nothing to `MUTANTS.json`,
  so gate 7 reads it as any campaign that measured nothing — red, and sent
  back to the coder, who cannot fix a disk.
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

The rigor binds the HQ's verbs too, not only the agents. `mission iterate`
on a `FINDINGS` is refused when the rigor says the report does not block:
at `standard`, a report with no `HIGH`; at `critical`, from the second
round on, a report with no `HIGH`. A report whose findings carry no
severity blocks, as it always did. The refusal names the two ways on:
`mission accept --because` (the finding is written as accepted), or
`mission iterate --override --because <why>`, written, dated, in
`FOLLOWUP_HQ.md` as a departure from the rigor. A verified mission sent
back on review (`mission iterate --because`) spends a volet like any other,
and at `standard` a second review needs `--override` as well. Sending the
survivors of a campaign that stopped short (`mission iterate --because`, at
the gates) is no review: the rigor does not bound it, and there is nothing
to override. `mission
status` lists every override with its reason, and `nunki push` adds them to
the pull request under "Departures from the rigor", so a reader sees how
often the rigor was set aside.

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

A finding accepted is not a finding fixed, and the pull request is
published. The security agent therefore does not write in `PR.md` — its
profile mounts the file read-only — and `nunki push` writes the pull
request's Security section itself, from the security verdict it stands on.
The body sent is `PR.md` as the coder and the integrator wrote it, then the
departures from the rigor, then that section, last:

- on a `CLEAR`, the commit attacked, then the report, quoted, as the agent
  wrote it;
- on a `FINDINGS` lifted — by `mission accept` or by `nunki` — a note: how
  many findings per severity, accepted by whom, and that their details are
  withheld until they are fixed. No word of the report, of a finding's
  title or of its reason reaches the forge, in the title or the body;
- with no security agent, no section.

Where the rounds were spent, the section also names the commits the
security agent did not attack. The report and the findings come from what
`nunki` kept when the verdict was concluded, never from `VERDICT.json`,
which every role rewrites; when it kept nothing, the section says it could
not read them. `PR.md` is left as it is, `nunki push` says how many findings
it withheld, and `VERDICT.json` still holds them in full; `mission status`,
once `mission accept` has lifted them, shows the stage and no longer the
report. With no `PR.md`, or an empty one, the branch is pushed and no pull
request is opened. A repository only its owners read can say
`security_findings: publish` in `nunki.yaml`: the section then names who
accepted the findings, lists each with its severity, its title and its
reason, and quotes the report. Absent, it is `withhold`.

What the coder or the integrator writes in `PR.md` is published as written,
a finding they describe included: `nunki` does not read their prose, and
the HQ reads `PR.md` before a push. One case is caught, coarsely and on
purpose: a mission whose security agent ran under an older `nunki` wrote its
section in `PR.md`. So a push that must withhold **refuses before anything
is pushed** when a line of `PR.md` has a `#` before its first letter and
those letters begin `security`, in any case, such as `## Security review`;
it names each such line and the two ways past: edit `PR.md` (the HQ may),
or `security_findings: publish`. It never cuts. A heading in another form,
such as `<h2>Security</h2>`, is not caught, and falls under the rule above.

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
  `FOLLOWUP_HQ.md` (what the HQ tells the agents), and the agents' own
  `JOURNAL.md`, `VERDICT.json` and `PR.md` — the last written by the coder
  and the integrator, and read-only for the security agent — and
  `MUTANTS.triage.json`, where the coder answers a campaign's survivors:
  written by the coder alone, and read-only for the integrator and the
  security agent, since survivors only ever go back to the coder. Its **header** is frozen when the
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
  iterate on or lift), the final campaign (`critical` only), awaiting a
  human, verified. A stage the mission does
  not declare is absent by declaration, not skipped.
- **Lot** — a planned unit of the coder's work, listed in the header (`L1`,
  `L2`…). Small, with its own proof, committed in commits that stand alone.
  Only the coder has lots.
- **Volet** — unplanned coder work, opened by a red verdict (the integrator's
  `BROKEN`, the security agent's `FINDINGS`, or red final gates). Named
  `volet-1`, `volet-2`…, bounded by `max_volets` (3 by default), counted
  over the mission's life: past the cap, each further volet is one a
  `retry` granted, and numbered on (`volet-4`, `volet-5`…). After a
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
  `killed`, `bug` and `equivalent_proposed`. The file is mounted read-write
  in the coder's profile only, so no other role can put an outcome there.
  Anything else written there —
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
reset` clears the volumes while keeping the clone: it takes the slot's
containers down first — an engine will not remove a volume a container still
mounts — through a call that never removes a volume, then removes the slot's
named volumes by name, and says any it could not. It refuses while a run or a
mutation campaign is in flight in the slot. It refuses too when the state of a
mission not archived cannot be read, since it then cannot tell whether that
mission runs there: it names the file and why, and changes nothing. Repair
the file, or, for a mission that is over, move its state file aside: the
mission then reads as never started, and `nunki mission archive` closes it.
A list of missions that cannot be read is not an empty one: when the HQ's
`missions/` directory cannot be read, the reset refuses the same way, naming
the directory. And a run the state store records in the slot blocks the reset
whether or not its mission folder is there: every state file not archived is
read, with or without a `MISSION.md` beside it.

Finally, the binary and the accounts, once no project uses them:

```sh
cargo uninstall nunki
rm -rf ~/.nunki/accounts ~/.nunki/accounts.yaml ~/.nunki/usage
```
