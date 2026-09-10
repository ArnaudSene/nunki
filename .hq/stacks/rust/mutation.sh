#!/bin/sh
# The mutation campaign for a Rust project (SPEC 4.4, gate 7).
#
# `hq` calls this as `mutation.sh <campaign-id> <path>...`, from the clean
# copy of HEAD inside the slot's container. The id comes first so the campaign
# is identifiable from its own command line; this script does not need it.
#
# It prints **one JSON object per line** on stdout, one per surviving mutant:
#   {"id":"…","file":"…","line":12,"description":"…"}
# `hq` ignores anything that is not one, so progress may go to stdout freely —
# though this script keeps the tool's own chatter on stderr.
#
# `--in-place` is not a detail: cargo-mutants only reuses a build cache in
# place, and the copy this runs in has its own, warmed once per slot and kept.
# Without it every campaign recompiles from cold, and SPEC section 7 counts
# that hour.
set -eu

campaign="$1"
shift

# Only Rust sources are worth mutating; the touched list holds whatever the
# branch touched.
files=""
for path in "$@"; do
  case "$path" in
    *.rs) files="$files --file $path" ;;
  esac
done
if [ -z "$files" ]; then
  exit 0
fi

out="target/mutants-$campaign"
# A campaign that finds survivors exits non-zero; that is a result, not a
# failure, and `hq` reads the survivors rather than the status.
# shellcheck disable=SC2086
cargo mutants --in-place --no-shuffle --output "$out" $files >&2 || true

if [ ! -f "$out/missed.txt" ]; then
  echo "hq: the campaign left no $out/missed.txt" >&2
  exit 1
fi

# `missed.txt` holds one mutant per line: `file:line:col: what it replaced`.
while IFS= read -r mutant; do
  [ -n "$mutant" ] || continue
  file=${mutant%%:*}
  rest=${mutant#*:}
  line=${rest%%:*}
  rest=${rest#*:}
  what=${rest#*: }
  escaped=$(printf '%s' "$what" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g')
  printf '{"id":"%s:%s","file":"%s","line":%s,"description":"%s"}\n' \
    "$file" "$line" "$file" "$line" "$escaped"
done < "$out/missed.txt"
