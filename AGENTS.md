# Working on nunki

The rules of this place. They apply to every agent and every human working in
this repository, whichever harness is driving — Claude Code, Codex, OpenCode
or another. `CLAUDE.md` is a symlink to this file; there is one set of rules,
not one per tool.

## 1. What this project is

`nunki` is a **mission orchestrator for AI coding agents**. A human frames
a mission, `nunki` runs it in an isolated slot with a coder, an integrator and a
security agent, gates the result, and hands the push back to the human.

It is **not a scaffolder**. It does not create projects, choose stacks, or
generate application code. If a change starts to look like project generation,
it is out of scope — say so rather than build it.

`SPEC.md` at the repository root is the authority on what the system does and
why. It is the one file written in French, because its author asked for it.
Read the section your change touches before changing anything, and cite the
section number in doc comments (`SPEC 4.1 bis`, `SPEC 4.5`). If the code and
the spec disagree, one of the two is wrong and the discrepancy is the finding
— do not silently pick a side.

## 2. Language

Everything that lands in this repository or on the forge is **English**:
source, comments, doc comments, documentation, commit messages, pull request
titles and descriptions, issue text, review comments.

The exceptions, both deliberate: `SPEC.md`, and the HQ files that live outside
this repository, under `hq/` in this project's own home — the one
`nunki sessions` names (journal, dashboard, discussions)
— those belong to the project's owner and stay in French. Conversation with
him is in French too; the record is not.

## 3. Git

- **Never commit to `dev` or `main`, and never push to them.** Work on a
  branch (`feat/…`, `fix/…`, `docs/…`) and open a pull request **targeting
  `dev`**. One piece of work, one branch, one pull request.
- **Never commit without being asked.** The owner decides when work is
  committed and when it is pushed. Ask; do not infer approval from silence or
  from a green test run.
- **Never force-push a shared branch**, never rewrite published history,
  never `git add -A` without looking at what it swept in.
- Commit messages say **what changed and why it had to**, not what files were
  touched. A subject line under ~72 characters, then a body that a reader six
  months from now can act on. When a decision was measured rather than
  reasoned, put the measurement in the message.
- **Never put a harness session link in a pull request.** Whatever
  attribution a harness suggests, a pull request description is read by
  people who cannot open that link and should not have to; it carries the
  change and its reasons, nothing else. Commit messages end with the session
  link on its own line — that is a trailer in the repository's own history,
  not something published on the forge.

## 4. Proof

The doctrine of this project is that **a restriction is worth what its proof
is worth**. Applied to our own work:

- **Prove by executing.** Do not claim a command works, a flag exists, or an
  engine behaves a certain way because it reads that way in documentation.
  Run it, and put the result in the commit message. Several of `SPEC.md`'s
  decisions were corrected this way.
- **A test that cannot fail proves nothing.** After writing a test for a
  decision, break the decision line and check the test goes red. Do this by
  hand for every load-bearing line — it is the ritual that has caught real
  defects here.
- **A probe is worth nothing if it would fail without the guard.** Two
  firewall probes once "passed" because the address was unreachable anyway.
  Before trusting a negative result, verify the same attempt succeeds in an
  unguarded container.
- **A check that cannot say "I do not know" will lie.** A liveness answer
  squeezed into a `bool` reports an engine that did not answer, a container
  taken down and a machine that slept as one thing: a dead agent. Whenever a
  question can fail for a reason that is not about its subject, that reason
  gets its own answer — measured here, and it had already put a false defect
  in a pull request.
- **Report faithfully.** If something is untested, say which part. If a test
  was skipped, say so. Never describe intended behaviour as verified. A
  finding that turns out to be wrong is corrected where it was published, not
  quietly dropped.

## 5. Tests

- Integration tests live in `tests/<area>.rs`, one file per area, and are
  named as sentences: `the_agent_joins_the_firewall_and_waits_for_it`.
- **Golden files** under `tests/fixtures/` freeze generated output.
  Regenerate with `HQ_BLESS=1 cargo test --test compose` and **read the diff**
  — a golden refreshed without reading it is a test deleted.
- **Live tests are `#[ignore]`d** and never run in CI: one needs a human's
  subscription, the others lift real containers. Run them by hand after
  touching what they cover, and say in the commit message that you did:

  ```sh
  cargo test --test claude_code -- --ignored     # a real headless run
  cargo test --test firewall   -- --ignored      # a real fenced pair
  ```

- A test needing a container engine must **skip cleanly** without one, not
  fail: CI runs on macOS where there is none.
- Tests that drive a container engine honour `HQ_COMPOSE`, so the same
  battery can be replayed under another version. The development machine and
  CI are three major versions apart on purpose — that gap is the only
  cross-version coverage this project has:

  ```sh
  HQ_COMPOSE=/path/to/docker-compose-2.38.2 cargo test --test firewall -- --ignored
  ```
- Never weaken a test to make it pass. If a test is wrong, fix the test in
  its own commit and say why it was wrong.

## 6. Rust

- Edition 2024, minimum 1.85, licence Apache-2.0. One binary `nunki`, one
  library.
- Before every commit, all five, and they must be silent — each read from
  its own exit code, never through a pipe:

  ```sh
  cargo fmt --all
  cargo clippy --all-targets --all-features -- -D warnings
  cargo test --all-features
  cargo deny check              # licences, bans, sources
  cargo audit --deny warnings   # known vulnerabilities (RustSec)
  ```

  `cargo audit` is not redundant with `cargo deny`: CI plays both, and on
  2026-09-10 it was `cargo audit` alone that caught RUSTSEC-2026-0009 in a
  dependency `cargo deny` had let through locally.

- Module layout follows the spec, not the other way round: `harness/`
  (SPEC 4.3), `mission/` (4.1, 4.5), `state/` (4.2), `compose/` (4.2),
  `perimeter/` (4.1 bis).
- **Boundaries are traits** with a fake implementation for tests: `Harness`
  for the coding agent, `Spawner` for launching processes, and the engine
  adapter to come. Nothing harness-specific or engine-specific leaks past its
  boundary.
- No `unwrap()` or `panic!()` on anything a user's environment can cause —
  return an error that names the file, the path or the value. `expect()` is
  acceptable only for an invariant the code itself guarantees, with the
  reason in the message.
- Errors are `thiserror` enums whose messages a human can act on without
  reading the source.

## 7. Dependencies

- **crates.io only.** No git dependencies: a git dependency is one nobody
  reviews. `deny.toml` enforces this, along with the permissive licence list
  and the ban on wildcard versions.
- Adding a dependency is a decision, not a reflex. Prefer the standard
  library; prefer a few lines of our own over a crate we would not read.
- In CI, **only GitHub's own actions** (`checkout`, `cache`). The toolchain
  ships with the runners. An action nobody reads is a dependency nobody
  reviews. Those are referenced by **major tag**, so a patch-level fix
  arrives on its own; the digest a tag could hide protects against a threat
  already inside GitHub's trust boundary, which is the runner's anyway. If a
  third-party action ever earns its place, it is **pinned to a commit
  digest** with the version in a comment — different owner, different rule.

## 8. What `nunki` may never do

These are the product's own restrictions (SPEC 3), and they constrain what we
are allowed to implement:

- `nunki` **never writes into a repository it orchestrates** beyond what
  `nunki init` explicitly deposits, never deletes a file it did not create,
  never keeps a manifest of ownership.
- A target project's stack fragments — Dockerfile, allowlist, battery,
  campaign, launch script — belong to **that project**, and live in its home
  (`~/.nunki/<id>/stacks/`, the session's), never in its repository. They are
  what `nunki init` writes for it; they are not where `nunki` keeps its own
  assets.
- No agent ever pushes, merges, or reaches the forge. `nunki push` does, on an
  explicit human argument, and it is the only verb that touches the forge in
  write.
- An agent container holds **no capability**, runs under the host's uid, and
  its perimeter is enforced by a sidecar it cannot reach — never by a hook,
  which is comfort and not enforcement.
- Guards must hold **without any harness cooperation**: container, git and
  review gates are the enforcement. Anything relying on a specific harness is
  a convenience, and must be described as one.

## 9. Scope and judgement

- Do what was asked, in full. Do not narrow the scope quietly, and do not
  widen it because something nearby looked improvable.
- If part of the work is blocked, finish everything else and say plainly what
  is left and why.
- When a decision genuinely belongs to the owner — an architecture fork, a
  naming convention, anything that changes `SPEC.md` — put the options and a
  recommendation in front of him and wait. Everything else, decide and move.
- Uncertainty is stated once, plainly, and then the work continues under a
  named assumption.
