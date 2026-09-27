#!/bin/sh
# The mechanical security of a Next.js project (SPEC 4.4, gate 8).
#
# `nunki` calls this as `security.sh <base-commit> [advisory-db] [mission-dir]
# [secrets-file]`, from the clean copy of HEAD inside the slot's container. A
# commit and not a branch name: the copy is a detached clone and carries no
# branch, so a name would resolve to nothing. It prints **one JSON object per
# line** on stdout, one per finding:
#
#   {"id":"…","kind":"…","where":"…","via":"…","fix":"…",
#    "accepted":"…","was_at_base":false}
#
# `nunki` reads those seven fields and nothing else — it knows no tool, no
# lockfile and no advisory database. Anything that is not one of these lines
# is ignored, so progress may go to stdout freely, though this script keeps
# its chatter on stderr.
#
# SPEC 4.4 gives gate 8 three families: the dependency audit (`osv-scanner`),
# the secret scan (`trufflehog`), and the static analysis — which is `eslint`,
# already run by the battery at gate 6, so nothing here repeats it.
set -eu

base="${1:?usage: security.sh <base-commit> [advisory-db] [mission-dir] [secrets]}"
# Second argument and not an environment variable: the agent owns its own
# environment inside the container, and a variable would let it point this at
# an empty directory — no findings, and a green gate. `nunki` is what invokes
# the gate, so `nunki` is what says where the database is.
db="${2:-/nunki/advisories}"
mission="${3:-/work/mission}"
secrets="${4:-/nunki/secrets.txt}"

[ -d "$db" ] || { echo "nunki: no advisory database at $db" >&2; exit 69; }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# osv-scanner reads its offline database from `$XDG_CACHE_HOME/osv-scalibr`,
# and there is no flag for it: measured on 2.6.0, no `--local-db-path`
# exists and the cache directory is the only knob. `advisories.txt` names the
# host's `~/.cache/osv-scalibr` and nunki mounts **that one directory** at
# $db — not the whole of `~/.cache`, which holds everything else this machine
# caches. A cache directory made here, with that one name linked into it, is
# how the tool finds its database without being handed the rest.
mkdir -p "$work/cache"
ln -s "$db" "$work/cache/osv-scalibr"
XDG_CACHE_HOME="$work/cache"
export XDG_CACHE_HOME

# A configuration with **no ignore**, for the runs that must see everything.
#
# osv-scanner loads `osv-scanner.toml` from the scanned tree on its own —
# "Loaded filter from: /w/osv-scanner.toml", measured on 2.6.0 with no
# `--config` at all. So the run meant to be unfiltered was filtered by the
# very file whose effect it exists to measure: every acceptance would come
# back empty, and an agent could silence any finding by writing that file.
# `--config` is what makes a view without it possible, exactly as it is for
# cargo-deny on the Rust side.
printf '# nothing is ignored here: this is the unfiltered view\n' > "$work/none.toml"

lock=""
for candidate in pnpm-lock.yaml package-lock.json yarn.lock; do
  if [ -f "$candidate" ]; then
    lock="$candidate"
    break
  fi
done

# A dependency audit with nothing to read is not an audit that found nothing.
# Reported as a finding rather than as an exit status, because a non-zero
# status here is not read as red: `nunki` maps 66, 67, 69 and 70 and takes
# everything else as "the scan ran". A finding the branch brought and nobody
# accepted is what makes the gate red, which is what this is.
if [ -z "$lock" ]; then
  printf '%s\n' '{"id":"nunki:no-lockfile","kind":"other","where":"pnpm-lock.yaml","via":"","fix":"","accepted":"","was_at_base":false}'
  echo "nunki: no pnpm-lock.yaml, package-lock.json or yarn.lock, so nothing could be audited" >&2
else
  # `--offline` keeps it from fetching anything, including the package
  # metadata it would otherwise ask deps.dev for; `--offline-vulnerabilities`
  # is what points it at the database above. Findings make it exit non-zero,
  # and that is a result rather than a failure.
  #
  # stderr is kept rather than discarded: it is where the tool says it could
  # not load the database, and a scan that silently found nothing is exactly
  # the failure this gate must not have.
  osv-scanner scan source --lockfile "$lock" --offline --offline-vulnerabilities \
    --config "$work/none.toml" --format json > "$work/all.json" \
    2>>"$work/scan.err" || true

  # What the base already carried, audited in a worktree of the base commit.
  : > "$work/base.json"
  if git worktree add -q --detach "$work/base" "$base" 2>/dev/null; then
    if [ -f "$work/base/$lock" ]; then
      osv-scanner scan source --lockfile "$work/base/$lock" --offline \
        --offline-vulnerabilities --config "$work/none.toml" --format json \
        > "$work/base.json" 2>>"$work/scan.err" || true
    fi
    git worktree remove --force "$work/base" 2>/dev/null || true
  else
    # 70, and not a warning followed by the audit. Without the base there is
    # no "what this branch brought": every finding would come back new, and
    # the gate would block on what was already there. `nunki` reads this as
    # unplayed — a verdict on the machine rather than on the agent, exactly
    # as 69 is.
    echo "nunki: base $base is not in this repository, so nothing can be compared" >&2
    exit 70
  fi

  # What the project accepts, as the difference between two views: a group
  # the unfiltered run reports and the project's own configuration does not
  # is one `osv-scanner.toml` ignores. Measured on 2.6.0: an `[[IgnoredVulns]]`
  # written against any alias filters the whole group — the config named
  # GHSA-q2x7-8rv6-6q7h and the group whose first id is PYSEC-2026-1475
  # disappeared — so this is the only reliable way to know it without
  # matching ids by hand.
  : > "$work/kept.json"
  if [ -f osv-scanner.toml ]; then
    osv-scanner scan source --lockfile "$lock" --offline --offline-vulnerabilities \
      --config osv-scanner.toml --format json > "$work/kept.json" \
      2>>"$work/scan.err" || true
  fi

  # Read with node rather than with jq, because the shape is nested and
  # because node is the one interpreter this stack is certain to have — the
  # Python fragment uses python for the same reason. Written to the work
  # directory instead of being mounted: a fragment is what nunki mounts, and
  # adding an eleventh file to that list to hold a helper would make the
  # contract about files rather than about scripts.
  cat > "$work/audit.cjs" <<'JS'
const fs = require("fs");
const { execFileSync } = require("child_process");

const work = process.argv[2];
const lock = process.argv[3];

// One entry per advisory, keyed by the id nunki will report.
//
// `results[].packages[].groups[]` is what collapses the aliases: osv-scanner
// reports the same advisory once as GHSA and once as the ecosystem's own id,
// and a finding reported twice is a finding answered twice.
function groups(path) {
  const found = new Map();
  let report;
  try {
    const text = fs.readFileSync(path, "utf8");
    if (!text.trim()) return found;
    report = JSON.parse(text);
  } catch (e) {
    process.stderr.write("nunki: osv-scanner wrote no readable JSON to " + path + "\n");
    process.exit(1);
  }
  for (const result of report.results ?? []) {
    for (const pkg of result.packages ?? []) {
      const named = pkg.package ?? {};
      const byId = new Map((pkg.vulnerabilities ?? []).map((v) => [v.id, v]));
      for (const group of pkg.groups ?? []) {
        const ids = group.ids ?? [];
        if (!ids.length) continue;
        found.set(ids[0], { named, advisories: ids.map((i) => byId.get(i)).filter(Boolean) });
      }
    }
  }
  return found;
}

// The first published version that fixes it, or "" when none does. Empty is
// the discriminator nunki turns on: an advisory with no fix is a decision to
// relitigate, not a debt to pay down (SPEC 4.4).
//
// **Both** `SEMVER` and `ECOSYSTEM`, and the first is the one that matters
// here. Measured 2026-09-21: npm advisories publish their ranges as `SEMVER`
// — `qs` fixed in 6.16.0 sits in one — and a reader that looked only at
// `ECOSYSTEM`, which is what the PyPI database uses, reported every npm
// finding as having no fix at all. That is not a cosmetic difference: an
// empty `fix` tells a human nothing can be done, and it is what stops nunki
// calling an accepted finding an exception a fix has overtaken.
//
// `GIT` ranges are left out: they name a commit of the upstream repository,
// which is not a version anybody can depend on.
function fixedIn(advisories, name) {
  for (const advisory of advisories) {
    for (const affected of advisory.affected ?? []) {
      if ((affected.package?.name ?? "").toLowerCase() !== name.toLowerCase()) continue;
      for (const span of affected.ranges ?? []) {
        if (span.type !== "SEMVER" && span.type !== "ECOSYSTEM") continue;
        for (const event of span.events ?? []) {
          if (event.fixed) return event.fixed;
        }
      }
    }
  }
  return "";
}

// The direct dependency that pulled `name` in, or "" when it is one. A
// transitive dependency is replaceable by nobody but its parent, so what is
// named is the last link before the project itself (SPEC 4.4). Only pnpm is
// asked, and only offline — measured 2026-09-21, `pnpm why <pkg> --offline`
// prints the chain with no network at all.
function broughtBy(name) {
  if (lock !== "pnpm-lock.yaml") return "";
  let out;
  try {
    out = execFileSync("pnpm", ["why", name, "--offline"], { encoding: "utf8" });
  } catch {
    return "";
  }
  const chain = [];
  for (const line of out.split("\n")) {
    const text = line.replace(/^[^A-Za-z@]*/, "").trim();
    if (!text || text.startsWith("Found ") || text.startsWith("Legend")) continue;
    const link = text.split(" ")[0].replace(/@[^@]*$/, "");
    if (link && chain[chain.length - 1] !== link) chain.push(link);
  }
  return chain.length > 2 ? chain[chain.length - 2] : "";
}

// Every id `osv-scanner.toml` ignores, with the reason beside it.
//
// Read with a reader of its own and not a TOML library: node ships none, and
// the file this looks at has one shape — a list of `[[IgnoredVulns]]` tables
// with an `id` and a `reason`. Anything it cannot read leaves the reason
// empty, and the acceptance still stands, because whether a group is
// accepted is decided by the two views and not by this.
function reasons() {
  const found = new Map();
  let text;
  try {
    text = fs.readFileSync("osv-scanner.toml", "utf8");
  } catch {
    return found;
  }
  for (const block of text.split(/\[\[\s*IgnoredVulns\s*\]\]/).slice(1)) {
    const id = /(^|\n)\s*id\s*=\s*"([^"]*)"/.exec(block);
    const why = /(^|\n)\s*reason\s*=\s*"([^"]*)"/.exec(block);
    if (id) found.set(id[2], why ? why[2] : "");
  }
  return found;
}

const everything = groups(work + "/all.json");
const atBase = new Set(groups(work + "/base.json").keys());
const kept = new Set(groups(work + "/kept.json").keys());
const why = reasons();
const filtered = fs.existsSync("osv-scanner.toml");

for (const id of [...everything.keys()].sort()) {
  const { named, advisories } = everything.get(id);
  const name = named.name ?? "";
  let accepted = "";
  if (filtered && !kept.has(id)) {
    // The group is ignored; the reason may be written against any of its
    // aliases, so every id and alias is tried before falling back.
    for (const advisory of advisories) {
      for (const alias of [advisory.id ?? "", ...(advisory.aliases ?? [])]) {
        if (why.has(alias)) {
          accepted = why.get(alias) || "accepted in osv-scanner.toml";
          break;
        }
      }
      if (accepted) break;
    }
    accepted = accepted || "accepted in osv-scanner.toml";
  }
  process.stdout.write(
    JSON.stringify({
      id,
      kind: "vulnerability",
      where: `${name} ${named.version ?? ""}`.trim(),
      via: broughtBy(name),
      fix: fixedIn(advisories, name),
      accepted,
      was_at_base: atBase.has(id),
    }) + "\n",
  );
}
JS
  node "$work/audit.cjs" "$work" "$lock"
fi

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

# What a human has already ruled is not a secret. It comes from the project's
# HQ, mounted read-only: declared out of the agent's reach, readable by it all
# the same. Absent, nothing is accepted — and absent means absent, because the
# path is not one the agent could create (SPEC 4.4, gate 8).
#
# An exact match on the first field, because a prefix match reads the ruling
# for `…:src/a.rs:Postgres:10` as the ruling for `…:src/a.rs:Postgres:1`, and
# because a file path carries regex metacharacters a pattern would read.
reason_for() {
  [ -f "$secrets" ] || return 0
  awk -v id="$1" '
    /^[[:space:]]*#/ { next }
    $1 == id { $1 = ""; sub(/^[[:space:]]+/, ""); print; exit }
  ' "$secrets"
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
