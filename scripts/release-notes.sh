#!/bin/sh
# Print the release notes of the version Cargo.toml names: its section of
# CHANGELOG.md, heading left out. Exits non-zero, saying why, when that
# section is missing, undated or empty.
#
# Given a tag, it first checks the tag is the one a release of that version
# carries, `v` and the version, and that it points at a commit `origin/main`
# holds. `main` holds what is released, and only a release's pull request
# brings anything there: a release is never of a commit `dev` alone holds,
# nor of one pushed with its tag alone.
#
#   scripts/release-notes.sh            # CI, on every change
#   scripts/release-notes.sh v0.1.0     # the release workflow, on the tag
set -eu

root="$(cd "$(dirname "$0")/.." && pwd)"

version="$(awk '
  /^\[/ { package = ($0 == "[package]") }
  package && /^version *= *"/ { sub(/^version *= *"/, ""); sub(/".*/, ""); print; exit }
' "$root/Cargo.toml")"
if [ -z "$version" ]; then
  echo "Cargo.toml has no version under [package]" >&2
  exit 1
fi

if [ $# -gt 0 ]; then
  tag="$1"
  if [ "$tag" != "v$version" ]; then
    echo "tag $tag does not name the version Cargo.toml holds, $version: its release is tagged v$version" >&2
    exit 1
  fi
  if ! commit="$(git -C "$root" rev-parse --verify --quiet "$tag^{commit}")"; then
    echo "this clone has no tag $tag" >&2
    exit 1
  fi
  if ! git -C "$root" merge-base --is-ancestor "$commit" refs/remotes/origin/main 2>/dev/null; then
    echo "tag $tag points at $commit, which origin/main does not hold (fetched?): a release is tagged on main, once its pull request to main is merged" >&2
    exit 1
  fi
fi

# 10, 11 and 12 rather than 1 and 2, which awk itself exits with when it
# cannot read the file.
status=0
notes="$(awk -v head="## [$version]" '
  !found && index($0, head) == 1 {
    found = 1
    dated = (substr($0, length(head) + 1) ~ /^ - [0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]$/)
    next
  }
  found && (/^## / || /^\[[^]]*\]: /) { exit }
  found {
    line[++n] = $0
    if ($0 ~ /[^ \t]/) { if (!first) first = n; last = n }
  }
  END {
    if (!found) exit 10
    if (!dated) exit 11
    if (!first) exit 12
    for (i = first; i <= last; i++) print line[i]
  }
' "$root/CHANGELOG.md")" || status=$?

case "$status" in
  0) printf '%s\n' "$notes" ;;
  10) echo "CHANGELOG.md has no section for $version: add one headed \`## [$version] - YYYY-MM-DD\`" >&2; exit 1 ;;
  11) echo "CHANGELOG.md's section for $version is not dated: head it \`## [$version] - YYYY-MM-DD\`" >&2; exit 1 ;;
  12) echo "CHANGELOG.md's section for $version is empty" >&2; exit 1 ;;
  *) echo "CHANGELOG.md could not be read" >&2; exit 1 ;;
esac
