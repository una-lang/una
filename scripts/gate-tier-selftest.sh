#!/bin/bash
# Self-test of scripts/gate-tier.sh (docs/compiler/testing.md section 8): each case lists changed
# paths, the tier they must give and, optionally, a text that an `extra:` line must hold. A later
# edit that makes one of these paths fast, or drops a suite it must add, fails here. The tests gate
# runs this script before its tests, in both tiers.
#
# Case format: `<tier>|<path>[ <path>...]|<extra text or empty>`, or, for entries with modes as
# `git diff --raw` lists them, `raw:<tier>|<old mode> <new mode> <path>|<extra text or empty>`.
# Usage: scripts/gate-tier-selftest.sh
set -u
here=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=${UNA_GATE_ROOT:-$(CDPATH='' cd -- "$here/.." && pwd)}
cd "$root" || exit 2
tier_script="$here/gate-tier.sh"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/gate-tier-selftest.XXXXXX") || exit 2
trap 'rm -rf "${tmp:?}"' EXIT
export UNA_GATE_READERS_CACHE="$tmp/readers"
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
    case "$want" in
    raw:*)
        want=${want#raw:}
        set -- $paths
        printf ':%s %s 0000000 0000000 M\t%s\n' "$1" "$2" "$3" > "$tmp/raw"
        out=$(env -u UNA_GATE_TIER -u UNA_HARNESS_GATE_HELD "$tier_script" --raw "$tmp/raw") ;;
    *)
        printf '%s\n' $paths > "$tmp/paths"
        out=$(env -u UNA_GATE_TIER -u UNA_HARNESS_GATE_HELD "$tier_script" --paths "$tmp/paths") ;;
    esac
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
full|crates/una-fmt/src/lib.rs|
fast|crates/una-explain/src/lib.rs|package(una-explain)
full|crates/una-lsp/src/lib.rs|
full|crates/una-diag/src/human.rs|
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
full|tests/serve/answer_timeout/consumer.jsonl|
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
# Adversarial cases (review rounds 1 and 2 and own): diagnostics, rewriting crates, snapshots,
# root files, case variants, dot segments, corpora, configuration look-alikes.
full|crates/una-diag/src/diagnostic.rs|
full|crates/una-diag/src/lib.rs|
full|crates/una-diag/src/json.rs|
full|crates/una-diag/tests/json_shape.rs|
full|crates/una-diag/tests/snapshots/human_rendering__x.snap|
full|crates/una-fmt/src/layout.rs|
full|crates/una-fmt/tests/format.rs|
full|crates/una-lsp/src/server.rs|
full|crates/una-lsp/tests/session.rs|
full|crates/una-report/src/lib.rs|
fast|crates/una-explain/explain/U0101.md|una-cli::explain_examples
fast|crates/una-cli/tests/snapshots/verification__queries_hello.snap|una-cli::verification
fast|crates/una-cli/tests/snapshots/verification__queries_wordfreq.snap|una-cli::verification
fast|crates/una-cli/tests/snapshots/runtime__run_hello_json.snap|una-cli::runtime
fast|crates/una-cli/tests/snapshots/runtime__test_events_json.snap|una-cli::runtime
fast|crates/una-cli/tests/snapshots/guide__x.snap|una-cli::guide
full|crates/una-cli/tests/snapshots/native__x.snap|
full|crates/una-cli/tests/snapshots/no_prefix.snap|
full|crates/una-cli/tests/snapshots/nosuchsuite__x.snap|
fast|README.md|una-cli::guide
fast|images/logo.svg|una-cli::guide
full|crates/una-lsp/../../docs/x.md|
full|tests/conformance/SECURITY/x.una|
full|tests/conformance/Security/flow.una|
full|docs/reviews/v0.1-review/ADVERSARIAL/new/src/main.una|
full|docs/reviews/r/d/adversarial/c/src/main.una|
full|Crates/una-rt/src/lib.rs|
full|CRATES/una-cli/src/main.rs|
full|crates/UNA-RT/src/lib.rs|
full|Scripts/release.sh|
full|./crates/una-rt/src/lib.rs|
full|./docs/a.md|
full|docs/../crates/una-rt/src/lib.rs|
full|docs/./../scripts/release-gates.sh|
full|changelog.d/../Cargo.lock|
full|site/../scripts/x.sh|
full|images/../install.sh|
full|docs/compiler/../../.cargo/config.toml|
full|docs//a.md|
full|crates\una-rt\src\lib.rs|
full|"crates/una-rt/src/\303\251.rs"|
full|crates/una-rt/tests/ffi.rs|
full|crates/una-http/tests/data/x.bin|
full|crates/una-interp/tests/confine.rs|
full|tests/native/unsupported.txt|
full|tests/conformance/runtime/within_runs/ita.lock|
full|tests/conformance/runtime/for_ever_on_stop/ita.lock|
full|tests/conformance/security/limits_lambda_handler/src/main.una|
fast|tests/conformance/names/new_case.una|una-cli::runtime
fast|tests/conformance/fmt/new_case.una|una-cli::verification
full|tests/never_named_corpus/x.txt|
fast|tests/fixtures/examples/events/x.una|una-cli::guide
full|.gitattributes|
full|.gitmodules|
full|.envrc|
full|rust-toolchain|
full|Makefile|
full|justfile|
full|fuzz/Cargo.toml|
full|supply-chain/musl.lock|
full|.agents/harness.toml|
full|ops/run-gates.sh|
full|crates/una-check/std/testing.una|
full|crates/una-cli/build.rs|
full|crates/una-explain/Cargo.toml|
full|crates/una-explain/build.rs|
full|examples/README.md|
full|examples/wordfreq/src/main.una|
fast|docs/guide/README.md|una-cli::guide
fast|docs/spec/13-diagnostics-and-abi.md|
fast|docs/std/listing.json|
fast|docs/std/builtins.json|
fast|formal/README.md|
fast|.claude/plan/NEXT.md|
fast|.mcp.json|
fast|LICENSE|
fast|CHANGELOG.md|
fast|SECURITY.md|
fast|CONTRIBUTING.md|
full|docs/reviews/v0.1-review/adversarial|
fast|docs/reviews/v0.1-review/README.md|
fast|changelog.d/README.md|
full|crates/una-cli/tests/common/case_lists.rs|
full|crates/una-cli/tests/conformance/discovery.rs|
fast|crates/una-cli/tests/conformance/outcome.rs|una-cli::conformance
fast|crates/una-cli/tests/lsp.rs|una-cli::lsp
fast|crates/una-cli/tests/verification.rs|una-cli::verification
fast|crates/una-cli/tests/guide.rs|una-cli::guide
full|crates/una-cli/src/std_listing.rs crates/una-cli/src/lib.rs|
full|docs/adr/0001-x.md crates/una-diag/src/diagnostic.rs|
# Round 3: case variants of fast patterns, corpus manifests, modes.
full|Examples/hello/README.md|
full|CRATES/una-check/src/notes.md|
full|Scripts/x.md|
full|Crates/una-rt/README.md|
full|Docs/reviews/v0.1-review/Adversarial/x/src/main.una|
full|TESTS/conformance/security/x.una|
full|tests/conformance/runtime/una.toml|
full|tests/conformance/runtime/ita.una|
full|tests/conformance/una.toml|
full|tests/conformance/ita.una|
fast|tests/conformance/types/c/una.toml|tests__conformance__types__c
fast|tests/conformance/types/c/ita.una|una-cli::verification
fast|LICENSE|
raw:full|000000 120000 docs/link.rs|
raw:full|100644 120000 docs/std|
raw:full|000000 120000 docs/reviews/zz|
raw:full|000000 160000 docs/sub|
raw:full|160000 000000 docs/sub|
raw:full|100644 100755 docs/a.md|
raw:full|100755 100644 crates/una-cli/tests/guide.rs|
raw:full|120000 100644 tests/conformance/types/c.una|
raw:fast|100644 100644 docs/a.md|
raw:fast|000000 100644 docs/new.md|
raw:fast|100644 000000 docs/old.md|
raw:full|100644 100644 crates/una-rt/src/lib.rs|
# The fast binaries of a skipped suite run whenever their package changes, in either tier (#2361)
full|crates/una-sys/src/solver/sandbox.rs|binary_id(una-sys::solver_size)
full|crates/una-sys/src/lib.rs|binary_id(una-sys::base_size)
full|crates/una-sys/tests/poll.rs|binary_id(una-sys::solver_size)
full|crates/una-http/src/serve/thread.rs|binary_id(una-http::serve_engine)
full|crates/una-http/src/serve/thread.rs|binary_id(una-http::serve_faults)
full|crates/una-http/src/serve/thread.rs|binary_id(una-http::serve_scripts)
full|crates/una-http/src/serve/thread.rs|binary_id(una-http::serve_quiet)
full|crates/una-http/src/tls/provider.rs|binary_id(una-http::provider_parts)
# Mixed changes: one full path makes the change full
full|docs/spec/07-information-flow.md crates/una-check/src/lib.rs|
full|docs/adr/0138-x.md crates/una-rt/src/lib.rs|
full|changelog.d/1.md scripts/release.sh|
CASES

# Conditions outside the paths.
printf '%s\n' docs/adr/0138-x.md > "$tmp/docs"
check "UNA_GATE_TIER=full" full "" "$(env -u UNA_HARNESS_GATE_HELD UNA_GATE_TIER=full "$tier_script" --paths "$tmp/docs")"
check "UNA_GATE_TIER=fast cannot lower" full "" "$(printf 'crates/una-rt/src/lib.rs\n' > "$tmp/rt"; env -u UNA_HARNESS_GATE_HELD UNA_GATE_TIER=fast "$tier_script" --paths "$tmp/rt")"
check "unreadable harness workflow" full "" "$(env -u UNA_GATE_TIER UNA_HARNESS_GATE_HELD=no-such-run-for-selftest:tests "$tier_script" --paths "$tmp/docs")"
: > "$tmp/empty"
check "empty change" full "" "$(env -u UNA_GATE_TIER -u UNA_HARNESS_GATE_HELD "$tier_script" --paths "$tmp/empty")"
plan=$(env -u UNA_HARNESS_GATE_HELD UNA_GATE_TIER=full UNA_GATE_ROOT="$root" "$here/gate-tests.sh" --plan)
n=$((n + 1))
if [ "$(printf '%s\n' "$plan" | sed -n 1p)" != "gate tier: full" ] || printf '%s\n' "$plan" | grep -q '^filterset:'; then
    echo "FAIL gate-tests.sh --plan under UNA_GATE_TIER=full must print the full header and no filterset"
    printf '%s\n' "$plan" | sed 's/^/    /'; fails=$((fails + 1))
fi
n=$((n + 1))
if env -u UNA_GATE_TIER -u UNA_HARNESS_GATE_HELD "$tier_script" --paths "$tmp/no-such-file" >/dev/null 2>&1; then
    echo "FAIL a missing --paths file must be an error"; fails=$((fails + 1))
fi

# Every fast binary of a skipped suite exists, and none names the slow or timing-dependent ones
# that the nightly keeps; a package without any adds no such line.
for b in $(sed -n '/^fast_of_long() {/,/^}/p' "$tier_script" | grep -oE '[a-z-]+::[a-z_]+'); do
    n=$((n + 1))
    crate=${b%%::*} name=${b#*::}
    if [ ! -f "crates/$crate/tests/$name.rs" ] && [ ! -f "crates/$crate/tests/$name/main.rs" ]; then
        echo "FAIL fast_of_long names $b, which has no test file"; fails=$((fails + 1))
    fi
done
printf '%s
' crates/una-interp/src/lib.rs > "$tmp/interp"
n=$((n + 1))
if env -u UNA_GATE_TIER -u UNA_HARNESS_GATE_HELD "$tier_script" --paths "$tmp/interp" | grep -q '^extra: binary_id(una-interp::confine)'; then
    echo "FAIL una-interp::confine (90 s) must stay for the nightly"; fails=$((fails + 1))
fi

if [ "$fails" -ne 0 ]; then
    echo "gate-tier self-test: $fails of $n cases FAILED"; exit 1
fi
echo "gate-tier self-test: $n cases passed"
