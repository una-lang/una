#!/bin/sh
# Installs the checking toolchain of scripts/check-toolchain.toml (#766) through rustup, with
# Miri and the standard library's sources, and verifies its commit. It changes neither the
# default toolchain nor the build toolchain of rust-toolchain.toml.
set -eu
cd "$(dirname "$0")/.."
channel=$(sed -n 's/^channel *= *"\(.*\)"$/\1/p' scripts/check-toolchain.toml)
commit=$(sed -n 's/^commit *= *"\(.*\)"$/\1/p' scripts/check-toolchain.toml)
rustup toolchain install "$channel" --profile minimal --component miri,rust-src --no-self-update
have=$(rustc +"$channel" -vV | sed -n 's/^commit-hash: //p')
if [ "$have" != "$commit" ]; then
    echo "install-check-toolchain: $channel is commit $have, not $commit" >&2
    exit 1
fi
echo "install-check-toolchain: $channel ($commit) ready"
