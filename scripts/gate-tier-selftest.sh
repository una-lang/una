#!/bin/bash
# Self-test of scripts/gate-tier.sh (docs/compiler/testing.md section 8): each case lists changed
# paths, the tier they must give and, optionally, a text that an `extra:` line must hold. A later
# edit that makes one of these paths fast, or drops a suite it must add, fails here. The tests gate
# runs this script before its tests, in both tiers.
#
# Case format: `<tier>|<path>[ <path>...]|<extra text or empty>`.
# Usage: scripts/gate-tier-selftest.sh
set -u
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root" || exit 2
tmp=$(mktemp -d "${TMPDIR:-/tmp}/gate-tier-selftest.XXXXXX") || exit 2
trap 'rm -rf "${tmp:?}"' EXIT
fails=0 n=0

check() { # <name> <expected tier> <extra text> <output>
    local got
    got=$(printf '%s\n' "$4" | sed -n 1p)
    n=$((n + 1))
    if [ "$got" != "$2" ]; then
        echo "FAIL $1: expected $2, got $got"; printf '%s\n' "$4" | sed 's/^/    /'; fails=$((fails + 1)); return
    fi
    if [ -n "$3" ] && ! printf '%s\n' "$4" | grep '^extra: ' | grep -qF -- "$3"; then
        echo "FAIL $1: no extra holds '$3'"; printf '%s\n' "$4" | sed 's/^/    /'; fails=$((fails + 1))
    fi
}

while IFS='|' read -r want paths extra; do
    case "$want" in '' | '#'*) continue ;; esac
    printf '%s\n' $paths > "$tmp/paths"
    out=$(env -u UNA_GATE_TIER -u UNA_HARNESS_GATE_HELD scripts/gate-tier.sh --paths "$tmp/paths")
    check "$paths" "$want" "$extra" "$out"
done <<'CASES'
# Runtime code
full|crates/una-rt/src/ffi.rs|
full|crates/una-rt/README.md|
full|crates/una-interp/src/machine/mod.rs|
full|crates/una-codegen/src/lib.rs|
full|crates/una-http/src/server.rs|
full|crates/una-http/tests/serve_scripts.rs|
full|crates/una-sys/src/lib.rs|
full|crates/una-link/src/macho.rs|
full|crates/una-z3/build.rs|
full|crates/una-itaty/src/log.rs|
# Soundness code
full|crates/una-check/src/types/mod.rs|
full|crates/una-check/std/prelude.una|
full|crates/una-verify/src/encode.rs|
full|crates/una-core/src/lower.rs|
full|crates/una-syntax/src/parser.rs|
full|crates/una-syntax/src/lexer.rs|
full|crates/una-diag/src/codes.rs|
# Security and supply chain
full|crates/una-crypto/src/ed25519.rs|
full|crates/una-hash/src/lib.rs|
full|crates/una-diag/src/safe_write.rs|
full|crates/una-project/src/lock.rs|
full|crates/una-test-cases/build.rs|
full|crates/una-fmt/Cargo.toml|
full|crates/una-newcrate/src/lib.rs|
# una-cli: every module but the standard library listing
full|crates/una-cli/src/main.rs|
full|crates/una-cli/src/lib.rs|
full|crates/una-cli/src/args.rs|
full|crates/una-cli/src/contract.rs|
full|crates/una-cli/src/core_json.rs|
full|crates/una-cli/src/fix.rs|
full|crates/una-cli/src/lsp.rs|
full|crates/una-cli/src/migrate.rs|
full|crates/una-cli/src/report.rs|
full|crates/una-cli/src/trace.rs|
full|crates/una-cli/src/runtime.rs|
full|crates/una-cli/src/verification.rs|
full|crates/una-cli/src/solver_child.rs|
full|crates/una-cli/src/proof_key.rs|
full|crates/una-cli/src/new_module.rs|
fast|crates/una-cli/src/std_listing.rs|una-cli::std_listing
fast|crates/una-cli/src/std_builtins.rs|una-cli::std_listing
# una-cli tests
full|crates/una-cli/tests/native.rs|
full|crates/una-cli/tests/common/hermetic.rs|
full|crates/una-cli/tests/conformance/discovery.rs|
full|crates/una-cli/tests/README.md|
fast|crates/una-cli/tests/serve_section.rs|una-cli::serve_section
fast|crates/una-cli/tests/runtime.rs|una-cli::runtime
fast|crates/una-cli/tests/release_gates.rs|una-cli::release_gates
fast|crates/una-cli/tests/conformance/main.rs|una-cli::conformance
fast|crates/una-cli/tests/snapshots/cli__help.snap|una-cli::cli
fast|crates/una-fmt/src/lib.rs|package(una-fmt)
fast|crates/una-explain/src/lib.rs|package(una-explain)
fast|crates/una-lsp/src/lib.rs|package(una-lsp)
fast|crates/una-diag/src/human.rs|package(una-diag)
# Configuration
full|Cargo.toml|
full|Cargo.lock|
full|rust-toolchain.toml|
full|.config/nextest.toml|
full|.cargo/config.toml|
full|.github/workflows/ci.yml|
full|supply-chain/z3.lock|
full|deny.toml|
full|clippy.toml|
full|verification.lock|
full|install.sh|
full|fuzz/fuzz_targets/lex.rs|
full|build.rs|
full|some/unknown/path.txt|
# Scripts: full but for two documentation helpers
full|scripts/gate-tier.sh|
full|scripts/gate-tests.sh|
full|scripts/gate-tier-selftest.sh|
full|scripts/release-gates.sh|
full|scripts/site-bundle.sh|
full|scripts/site-website.sh|
full|scripts/site-features.py|
full|scripts/formal.sh|
full|scripts/lean-env.sh|
full|scripts/doc-links.py|
full|scripts/doc-links/anchors.md|
full|scripts/z3-library.sh|
full|scripts/sanitize.sh|
full|scripts/security-hunt/hunt/gate.py|
full|scripts/dense-corpus.py|
full|scripts/new-script.sh|
fast|scripts/changelog-assemble.sh|una-cli::changelog_fragments
fast|scripts/demo/checkpoint-1.sh|una-cli::demo_scripts
# Corpora
full|tests/conformance/security/flow_case.una|
full|docs/reviews/v0.1-review/adversarial/x/src/main.una|
full|examples/hello/src/main.una|
full|examples/pantry/verification.lock|
full|tests/native/rejected.txt|
fast|tests/conformance/types/new_case.una|una-cli::verification
fast|tests/conformance/types/new_case.una|una-cli::runtime
fast|tests/conformance/types/new_case.una|tests__conformance__types__new_case_una
fast|tests/serve/answer_timeout/consumer.jsonl|una-http::serve_scripts
fast|tests/serve_runs/serve_section_health/src/main.una|una-cli::serve_runs
fast|tests/demo/checkpoint-1/hello.una|una-cli::guide
# Documentation
fast|docs/adr/0138-x.md|
fast|docs/spec/07-information-flow.md|una-cli::guide
fast|docs/guide/getting-started.md|una-cli::guide
fast|docs/compiler/testing.md|
fast|changelog.d/1701-gate-tiers.md|
fast|README.md|
fast|site/index.html|
fast|formal/F1/Basic.lean|
fast|.claude/state/STATE.md|
# Mixed changes: one full path makes the change full
full|docs/spec/07-information-flow.md crates/una-check/src/lib.rs|
full|docs/adr/0138-x.md crates/una-rt/src/lib.rs|
full|changelog.d/1.md scripts/release.sh|
CASES

# Conditions outside the paths.
printf '%s\n' docs/adr/0138-x.md > "$tmp/docs"
check "UNA_GATE_TIER=full" full "" "$(env -u UNA_HARNESS_GATE_HELD UNA_GATE_TIER=full scripts/gate-tier.sh --paths "$tmp/docs")"
check "UNA_GATE_TIER=fast cannot lower" full "" "$(printf 'crates/una-rt/src/lib.rs\n' > "$tmp/rt"; env -u UNA_HARNESS_GATE_HELD UNA_GATE_TIER=fast scripts/gate-tier.sh --paths "$tmp/rt")"
check "unreadable harness workflow" full "" "$(env -u UNA_GATE_TIER UNA_HARNESS_GATE_HELD=no-such-run-for-selftest:tests scripts/gate-tier.sh --paths "$tmp/docs")"
: > "$tmp/empty"
check "empty change" full "" "$(env -u UNA_GATE_TIER -u UNA_HARNESS_GATE_HELD scripts/gate-tier.sh --paths "$tmp/empty")"
n=$((n + 1))
if env -u UNA_GATE_TIER -u UNA_HARNESS_GATE_HELD scripts/gate-tier.sh --paths "$tmp/no-such-file" >/dev/null 2>&1; then
    echo "FAIL a missing --paths file must be an error"; fails=$((fails + 1))
fi

if [ "$fails" -ne 0 ]; then
    echo "gate-tier self-test: $fails of $n cases FAILED"; exit 1
fi
echo "gate-tier self-test: $n cases passed"
