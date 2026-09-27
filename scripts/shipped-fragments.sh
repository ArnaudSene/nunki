#!/bin/sh
# Rebuild the list of every stack fragment file nunki has ever shipped, as
# `<sha256> <stack>/<file>` lines, from the repository's own history
# (SPEC 4.2, "les fragments suivent nunki").
#
# For each commit that changed `src/init.rs`, and for the working tree, it
# builds that nunki, runs `init --stack <s>` for every stack it knows into a
# throwaway repository and home, and hashes what it wrote. Old releases kept
# their fragments in the repository; every place is read.
#
#   scripts/shipped-fragments.sh > src/fragments/shipped.txt
#
# Slow: one build per commit, sharing one target directory. Run it when a
# template changes; the test that reads the list goes red until it is.
set -eu

repo="$(git rev-parse --show-toplevel)"
work="$(mktemp -d)"
trap 'git -C "$repo" worktree prune; rm -rf "$work"' EXIT
target="${SHIPPED_TARGET:-$work/target}"

hash_of() {
  # stdin: a nunki binary path. stdout: `<sha256> <stack>/<file>` lines.
  bin="$1"
  for stack in rust python next; do
    home="$work/home-$stack"; project="$work/project-$stack"
    rm -rf "$home" "$project"; mkdir -p "$home" "$project"
    git -C "$project" init -q
    ( cd "$project" && HOME="$home" "$bin" init --stack "$stack" ) >/dev/null 2>&1 || continue
    # Wherever that release kept them: the home (`~/.nunki/<id>/stacks/`),
    # or, before 2026-09-15, the repository (`.nunki/stacks/`, `.hq/stacks/`).
    find "$home" "$project" -type f -path "*/stacks/$stack/*" | while read -r file; do
      printf '%s %s/%s\n' "$(shasum -a 256 "$file" | cut -d' ' -f1)" "$stack" "$(basename "$file")"
    done
  done
}

{
  for commit in $(git -C "$repo" log --format=%H dev -- src/init.rs); do
    tree="$work/tree"
    git -C "$repo" worktree add -q --detach "$tree" "$commit" 2>/dev/null || continue
    # The binary was called `hq` before it was called `nunki`.
    rm -f "$target/debug/nunki" "$target/debug/hq"
    if ( cd "$tree" && CARGO_TARGET_DIR="$target" cargo build -q --bins ) >/dev/null 2>&1; then
      for bin in "$target/debug/nunki" "$target/debug/hq"; do
        [ -x "$bin" ] && hash_of "$bin"
      done
    else
      echo "shipped-fragments: $commit does not build, skipped" >&2
    fi
    git -C "$repo" worktree remove --force "$tree"
  done
  ( cd "$repo" && CARGO_TARGET_DIR="$target" cargo build -q --bin nunki ) >/dev/null
  hash_of "$target/debug/nunki"
} | sort -u
