#!/bin/sh
# Builds ADR 0107's evidence: release una and runtime, the three Una kernels, and the C
# twins with the flags of ADR 0106 (RB-2). Usage: prepare.sh <worktree>
set -eu
wt=${1:?}
here=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
export CARGO_BUILD_JOBS=6
(cd "$wt" && nice -n 10 cargo build -q --release -p una-cli -p una-rt)
for k in k1 k2 k3; do "$wt/target/release/una" --json build "$here/una-$k" > "$here/build-$k.json"; cat "$here/build-$k.json"; echo; done
cd "$here/c"
for f in k1_boxed k1_layout k1_indexed k2_boxed k2_layout k3_boxed k3_layout; do
  cc -O2 -fwrapv -fno-strict-aliasing -fno-delete-null-pointer-checks -o "$f" "$f.c"
done
ls -la
