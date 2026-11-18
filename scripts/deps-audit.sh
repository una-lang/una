#!/bin/bash
# The dependency audit (#770): the deps gate of scripts/release-gates.sh (docs/release-gates.md).
# Run from the repository root. It fails closed: a missing or mismatched tool, an advisory
# database it cannot fetch, or any finding is a failure, never a skip.
#
# 1. cargo-deny at the version of supply-chain/tools.lock.
# 2. The RustSec advisory database, fetched now; its commit and date are printed for the record.
#    A fetch can succeed and leave an old database (a cached clone, a stalled mirror), so the
#    database's last commit must be at most MAX_AGE_DAYS old by the current UTC time.
# 3. cargo deny, against that database with no further fetch: advisories (deny.toml has no
#    ignore without a ruling), licenses, bans and sources (crates.io only), with Cargo.lock as is;
#    then the same for the fuzz targets' crate (fuzz/, #766), with fuzz/Cargo.lock as is.
# 4. The supply-chain review record: crates/una-cli/tests/dependencies.rs, which checks that
#    supply-chain/crates.tsv lists exactly the crates.io packages of Cargo.lock, and
#    supply-chain/fuzz-crates.tsv those of fuzz/Cargo.lock that Cargo.lock does not have.
#
# Usage: scripts/deps-audit.sh
set -u

MAX_AGE_DAYS=7

fail() {
    echo "deps: FAIL: $1" >&2
    exit 1
}

want=$(sed -n 's/^cargo-deny = "\(.*\)"$/\1/p' supply-chain/tools.lock)
[ -n "$want" ] || fail "supply-chain/tools.lock names no cargo-deny version"
have=$(cargo deny --version 2> /dev/null) \
    || fail "cargo-deny is missing: cargo install --locked cargo-deny@$want"
[ "$have" = "cargo-deny $want" ] \
    || fail "$have, but supply-chain/tools.lock pins $want: cargo install --locked cargo-deny@$want"
tool=$(command -v cargo-deny)
echo "tool: $have, sha256 $(shasum -a 256 "$tool" | cut -d' ' -f1)"

dbs=${CARGO_HOME:-$HOME/.cargo}/advisory-dbs
cargo deny fetch db || fail "the advisory database could not be fetched"
found=()
for db in "$dbs"/advisory-db-*/; do
    [ -d "$db/.git" ] && found+=("$db")
done
[ ${#found[@]} -eq 1 ] || fail "expected one advisory database in $dbs, found ${#found[@]}"
commit=$(git -C "${found[0]}" log -1 --format='%H %cI') || fail "the advisory database has no commit"
echo "advisory-db: $commit"
committed=$(git -C "${found[0]}" log -1 --format=%ct) || fail "the advisory database has no commit time"
age=$(($(date -u +%s) - committed))
echo "advisory-db age: $((age / 86400)) days $((age % 86400 / 3600)) hours (at most $MAX_AGE_DAYS days)"
[ "$age" -le $((MAX_AGE_DAYS * 86400)) ] \
    || fail "the advisory database's last commit is older than $MAX_AGE_DAYS days: a stale fetch"

echo "== cargo deny"
cargo deny --locked --offline check advisories bans licenses sources || fail "cargo deny"
echo "== cargo deny, the fuzz targets"
cargo deny --manifest-path fuzz/Cargo.toml --config deny.toml --locked --offline check \
    advisories bans licenses sources || fail "cargo deny, the fuzz targets"

echo "== the supply-chain review record"
cargo test --locked -p una-cli --test dependencies || fail "the supply-chain review record"

echo "deps: PASS"
