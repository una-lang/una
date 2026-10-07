#!/bin/bash
# Dumps the #336 corpus queries on linux/amd64 in the local-CI image, for a byte diff with macOS.
# Run only after the coordinator's release gates are done. It uses its own target volume
# (una3-r336-target), so it never touches the gates' volumes. The container is discarded at exit.
set -euo pipefail
S=$(cd "$(dirname "$0")/.." && pwd)  # the directory that holds una-336/ and r336/
T="$S/una-336"
toolchain=$(sed -n 's/^channel *= *"\([^"]*\)"$/\1/p' "$T/rust-toolchain.toml")
image="una-local-ci:$toolchain"
docker image inspect "$image" >/dev/null 2>&1 || docker build --platform linux/amd64 --build-arg "TOOLCHAIN=$toolchain" --tag "$image" "$T/scripts/local-ci"
# The program list, rewritten to container paths.
sed -e "s|^$T/|/work/|" -e "s|^$S/r336/cases/|/cases/|" "$S/r336/programs.txt" > "$S/r336/programs-linux.txt"
mkdir -p "$S/r336/q-linux"
docker run --rm --platform linux/amd64 \
  --volume "$T:/work" --volume una3-r336-target:/work/target \
  --volume "$S/r336/cases:/cases" --volume "$S/r336/q-linux:/out" \
  --volume "$S/r336/programs-linux.txt:/programs.txt:ro" \
  --volume una-local-ci-registry:/opt/cargo/registry --volume una-local-ci-z3:/opt/z3 \
  --workdir /work "$image" \
  sh -c '[ -x /opt/z3/bin/z3 ] || scripts/install-z3.sh /opt/z3; PATH=/opt/z3/bin:$PATH cargo build --release --locked -p una-cli --example dumpq && target/release/examples/dumpq verification.lock /out $(cat /programs.txt)'
