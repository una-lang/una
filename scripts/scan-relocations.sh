#!/bin/sh
# The relocation scan of ADR 0124 rule 24: every relocation of the given objects and archives,
# counted by type, section and kind of referent, through the internal linker's own reader, which
# fails (U8001) on a type outside the supported set. Run it on the runtime of a release
# (target/release-remapped/embed/libuna_rt.a) and on a program's object at every bump of the
# Rust toolchain, Cranelift or a crate with assembly; a new type is added with its test, or the
# bump waits. spec/16 §6 rule 3 lists the supported set.
#
# Usage: scripts/scan-relocations.sh FILE...
set -eu
[ "$#" -ge 1 ] || { echo "usage: scripts/scan-relocations.sh FILE..." >&2; exit 2; }
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
exec cargo run --quiet --release --locked --manifest-path "$root/Cargo.toml" -p una-link --example scan-relocations -- "$@"
