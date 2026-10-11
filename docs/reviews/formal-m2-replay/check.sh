#!/bin/sh
# The replay's gate (#528): runs run.sh and compares its output with expected.txt, byte for
# byte. Any change in a verdict, a reason or the number of rejected shapes fails, so a change to
# the re-check or to the converter that loses (or changes) what the replay shows cannot pass
# silently. A change that is intended updates expected.txt in the same commit, with its reason
# in README.md.
#
# Usage: docs/reviews/formal-m2-replay/check.sh
set -eu
here=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$here/../../.." && pwd)
# The Lean release pinned in formal/lean-toolchain, found as scripts/formal.sh finds it (which
# local CI runs first, installing it if needed).
toolchain=$(cat "$root/formal/lean-toolchain")
version=${toolchain#leanprover/lean4:}
lean_home=${UNA_FORMAL_LEAN:-${XDG_CACHE_HOME:-$HOME/.cache}/una/lean/$version}
if [ ! -x "$lean_home/bin/lake" ]; then
    "$root/scripts/install-lean.sh" "$lean_home"
fi
PATH="$lean_home/bin:$PATH"
export PATH
got=$(mktemp)
trap 'rm -f "$got"' EXIT
sh "$here/run.sh" > "$got"
if ! diff -u "$here/expected.txt" "$got"; then
    echo "replay: the output differs from expected.txt" >&2
    exit 1
fi
echo "replay: $(grep -c "$(printf '\trejected\t')" "$got") rejected, as expected.txt"
