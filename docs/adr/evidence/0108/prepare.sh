#!/bin/sh
# Builds ADR 0108's evidence: the C kernels, with the flags of ADR 0106 (RB-2).
# k1_boxed, k1_indexed and rt_boxed.h are copies of ADR 0107's kit.
set -eu
here=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
cd "$here/c"
for f in k1_boxed k1_indexed k1_flat k5_indexed k5_flat k6_indexed k6_flat k6c_indexed k6c_flat k7_indexed k7_flat; do
  cc -O2 -fwrapv -fno-strict-aliasing -fno-delete-null-pointer-checks -o "$f" "$f.c"
done
ls -la
