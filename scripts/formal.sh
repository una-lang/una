#!/bin/sh
# The formal track's checks (docs/plan/formal-track.md §6, #46): builds the Lean development in
# formal/ with the pinned toolchain, then checks that no theorem listed in formal/Theorems.lean
# depends on `sorry` or on an axiom beyond Lean's own three (`propext`, `Classical.choice`,
# `Quot.sound`). Installs the toolchain when it is missing: in UNA_FORMAL_LEAN when it is set (the
# containers of scripts/local-ci.sh give a volume), else in the user's cache, one directory per
# version shared by every checkout, since a Lean release is about 2 GB.
#
# Usage: scripts/formal.sh
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
toolchain=$(cat "$root/formal/lean-toolchain")
version=${toolchain#leanprover/lean4:}
lean_home=${UNA_FORMAL_LEAN:-${XDG_CACHE_HOME:-$HOME/.cache}/una/lean/$version}
if [ ! -x "$lean_home/bin/lake" ]; then
    echo "== installing the Lean release pinned in formal/lean-toolchain in $lean_home"
    "$root/scripts/install-lean.sh" "$lean_home"
fi
PATH="$lean_home/bin:$PATH"
export PATH
cd "$root/formal"

echo "== restrictions by constructor (#535)"
"$root/scripts/formal-restrictions.sh"

echo "== lake build"
lake build

echo "== axioms of the stated theorems"
axioms=$(lake env lean Theorems.lean)
echo "$axioms"
listed=$(grep -c '^#print axioms' Theorems.lean)
reported=$(printf '%s\n' "$axioms" | grep -c "depends on axioms\|does not depend on any axioms" || true)
if [ "$reported" -ne "$listed" ]; then
    echo "error: formal/Theorems.lean lists $listed theorems, and Lean reported the axioms of $reported" >&2
    exit 1
fi
if printf '%s\n' "$axioms" | grep -q 'sorryAx'; then
    echo "error: a stated theorem depends on sorry" >&2
    exit 1
fi
unexpected=$(printf '%s\n' "$axioms" | grep 'depends on axioms' | sed 's/.*depends on axioms: \[\(.*\)\]/\1/' | tr ',' '\n' | sed 's/^ *//' | grep -v -x -e propext -e Classical.choice -e Quot.sound -e '' || true)
if [ -n "$unexpected" ]; then
    echo "error: a stated theorem depends on axioms beyond Lean's own: $unexpected" >&2
    exit 1
fi
echo "formal: PASS ($listed theorems, no sorry)"
