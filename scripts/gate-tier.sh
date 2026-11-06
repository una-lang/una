#!/bin/bash
# The tier of the `tests` gate for the current worktree (docs/compiler/testing.md, "Gate tiers"):
# `full` (the whole suite) or `fast` (a subset of a few minutes, scripts/gate-tests.sh).
#
# Output, one item per line:
#   line 1        `full` or `fast`
#   `reason: ...` why, one line per path that decides (or a summary for fast)
#   `extra: ...`  (fast only) a nextest filterset of tests the change needs beyond the fixed subset:
#                 the test binaries it edits, the native differential of the programs it edits, the
#                 suites that read the files it edits
#   `skipped: ...` (fast only) the nextest filterset of the suites the fast tier leaves out
#
# The tier is `full` when UNA_GATE_TIER=full, when the changed paths cannot be computed, or when
# any changed path is runtime, soundness, security or supply-chain code, the build, test or
# toolchain configuration, a gate or release script, a security conformance case, an adversarial
# review case, a shared test helper or the native differential's own test file. Every path that
# no rule names is `full` too: a path is `fast` only when a rule below says so.
#
# Changed paths: `git diff --name-only --no-renames <base>...HEAD` (base: UNA_GATE_BASE, default
# origin/main), plus what is staged or modified against HEAD (a merge-train worktree holds its
# merge in the index) and untracked files. Bash 3.2 (macOS) compatible.
#
# Usage: scripts/gate-tier.sh [--paths FILE]   (--paths classifies the paths listed in FILE, one per
# line, instead of the worktree's changes: for reviewing the rule on past pull requests)
set -u

paths_file=""
if [ "${1:-}" = --paths ]; then
    paths_file=${2:?--paths needs a file}
    case "$paths_file" in /*) ;; *) paths_file="$PWD/$paths_file" ;; esac
fi

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root" || exit 2

tier=fast
reasons=""
extras=""
add_reason() { reasons="${reasons}reason: $1
"; }
add_extra() {
    case "
$extras" in
    *"
$1
"*) ;;
    *) extras="${extras}$1
" ;;
    esac
}
go_full() { tier=full; add_reason "$1"; }

if [ "${UNA_GATE_TIER:-}" = full ]; then
    go_full "UNA_GATE_TIER=full is set (a workflow or the nightly run asks for the whole suite)"
fi

# Under the engineering harness (which sets UNA_HARNESS_GATE_HELD=<run>:<gate>), the workflows that change
# security, soundness, performance or a release run the full tier, as the gate script of the harness also asks.
held=${UNA_HARNESS_GATE_HELD:-}
if [ -n "$held" ]; then
    run=${held%%:*} dir=$root
    while [ "$dir" != / ]; do
        dir=$(dirname "$dir")
        manifest="$dir/.una-harness/runs/$run/manifest.json"
        [ -f "$manifest" ] || continue
        wf=$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["work_item"]["workflow"])' "$manifest" 2>/dev/null)
        case "$wf" in
        security-fix | soundness-change | performance | release*) go_full "the harness run $run has the workflow $wf" ;;
        esac
        break
    done
fi

base=${UNA_GATE_BASE:-origin/main}
paths=""
if [ -n "$paths_file" ]; then
    paths=$(sed '/^$/d' "$paths_file" | sort -u)
    base="the paths of $2"
elif ! git rev-parse --verify -q "$base^{commit}" >/dev/null; then
    go_full "the base $base does not resolve to a commit, so the changed paths are unknown"
elif ! committed=$(git diff --name-only --no-renames "$base"...HEAD 2>/dev/null); then
    go_full "git diff against $base failed, so the changed paths are unknown"
else
    pending=$(git diff --name-only --no-renames HEAD 2>/dev/null; git ls-files --others --exclude-standard 2>/dev/null)
    paths=$(printf '%s\n%s\n' "$committed" "$pending" | sed '/^$/d' | sort -u)
fi

# The test name of a program of the native differential (crates/una-test-cases/build.rs, test_name).
mangle() {
    printf '%s\n' "$1" | awk '{
        s = tolower($0); out = ""
        for (i = 1; i <= length(s); i++) {
            c = substr(s, i, 1)
            if (c == "/") out = out "__"; else if (c ~ /[a-z0-9]/) out = out c; else out = out "_"
        }
        if (out ~ /^[0-9]/) out = "_" out
        print out
    }'
}

# The native differential (una-cli::native, `interpreter::` and `linkers::`) has one test per program
# of examples/ and tests/conformance/ (a file or a package directory): the tests of `p` and of each of
# its directories below the corpus root.
native_programs_of() {
    local p=$1 names="" depth_min
    case "$p" in
    examples/*) depth_min=2 ;;
    tests/conformance/*) depth_min=4 ;;
    *) return ;;
    esac
    while :; do
        local n
        n=$(printf '%s' "$p" | awk -F/ '{print NF}')
        [ "$n" -lt "$depth_min" ] && break
        names="${names:+$names|}$(mangle "$p")"
        case "$p" in */*) p=${p%/*} ;; *) break ;; esac
    done
    [ -n "$names" ] && add_extra "binary_id(una-cli::native) & test(/^(interpreter|linkers)::($names)(_[0-9]+)?\$/)"
}

# The suites the fast tier leaves out (scripts/gate-tests.sh takes the list from the `skipped:` line),
# with their test-time sums at opt-level 1 on the gate machine (2026-11-06): the native differential
# (2,555 s), release gates (263 s), the runtime and verification suites (263 s, 216 s), the HTTP,
# serving and redaction suites, the guide and website suites, the slow recovery, profile and
# refinement cases, the interpreter's confinement tests, and every integration test of una-http and
# una-sys (185 s, 70 s). Their unit tests (`kind(lib)`) stay in.
SKIPPED_SUITES="una-cli::native una-cli::release_gates una-cli::runtime una-cli::verification una-cli::serve_runs
una-cli::serve_live una-cli::http una-cli::http_limits una-cli::outcome_redaction una-cli::guide una-cli::site_website
una-cli::profiles una-cli::recovery una-cli::refined_elements una-interp::confine una-http::* una-sys::*"

# The test files of a skipped suite (`crate::name`, `name` may be `*`).
suite_files() {
    local crate=${1%%::*} name=${1#*::}
    if [ "$name" = "*" ]; then
        ls crates/"$crate"/tests/*.rs crates/"$crate"/tests/*/main.rs 2>/dev/null
    else
        ls crates/"$crate"/tests/"$name".rs crates/"$crate"/tests/"$name"/main.rs 2>/dev/null
    fi
}

# A skipped suite that reads a repository file by a literal path ("tests/serve",
# "../../tests/serve", "examples/hello/verification.lock"): a changed path under (or above) one of
# those literals adds the suite to the fast run; the native differential reading it makes the tier full.
readers_of() {
    local p=$1 suite f b lit
    for suite in $SKIPPED_SUITES; do
        for f in $(suite_files "$suite"); do
            b=${f#crates/}; b="${b%%/*}::$(basename "${f%/main.rs}" .rs)"
            for lit in $(grep -oE '"(\.\./)*(docs|examples|scripts|site|tests|fuzz|formal)/[^"{ ]*"' "$f" | tr -d '"' | sed -E 's#^(\.\./)+##; s#/$##' | sort -u); do
                case "$p" in
                "$lit" | "$lit"/*) ;;
                *) case "$lit" in "$p"/*) ;; *) continue ;; esac ;;
                esac
                if [ "$b" = una-cli::native ]; then
                    go_full "$p: read by the native differential (una-cli::native)"
                else
                    add_extra "binary_id($b)"
                fi
                break
            done
        done
    done
}

# A test file or directory of a crate's tests/: the binary it belongs to.
test_binary_of() {
    local p=$1 crate rest name
    crate=${p#crates/}; crate=${crate%%/*}
    rest=${p#crates/"$crate"/tests/}
    case "$rest" in
    */*) name=${rest%%/*}
        if [ -f "crates/$crate/tests/$name/main.rs" ] || [ -f "crates/$crate/tests/$name.rs" ]; then
            add_extra "binary_id($crate::$name)"
        elif [ "$crate" = una-cli ] && [ "$name" = snapshots ]; then
            add_extra "binary_id(una-cli::cli) | binary_id(una-cli::guide) | binary_id(una-cli::install)"
        else
            return 1
        fi ;;
    *.rs) name=${rest%.rs}
        [ -f "$p" ] && add_extra "binary_id($crate::$name)" ;;
    *) return 1 ;;
    esac
    return 0
}

fast_count=0
classify() {
    local p=$1
    case "$p" in
    # Runtime code: the abstract machine, code generation, native runtime, linking, HTTP, system calls, solver binding, registry service.
    crates/una-rt/* | crates/una-interp/* | crates/una-codegen/* | crates/una-http/* | crates/una-sys/* | crates/una-link/* | crates/una-z3/* | crates/una-itaty/*)
        go_full "$p: runtime code" ;;
    # Soundness code: the checker, the verifier, Core IR, and the parser and lexer that feed them.
    crates/una-check/* | crates/una-verify/* | crates/una-core/* | crates/una-syntax/*)
        go_full "$p: soundness code" ;;
    crates/una-diag/src/codes.rs)
        go_full "$p: diagnostic codes" ;;
    crates/una-diag/src/safe_write.rs | crates/una-crypto/* | crates/una-hash/*)
        go_full "$p: security code" ;;
    crates/una-project/*)
        go_full "$p: supply chain (package manager, registry client, locks)" ;;
    crates/una-test-cases/*)
        go_full "$p: the case lists of the sharded suites" ;;
    crates/*/Cargo.toml | crates/*/build.rs)
        go_full "$p: build configuration" ;;
    crates/una-cli/src/args.rs | crates/una-cli/src/contract.rs | crates/una-cli/src/core_json.rs | crates/una-cli/src/fix.rs | \
    crates/una-cli/src/lsp.rs | crates/una-cli/src/migrate.rs | crates/una-cli/src/report.rs | crates/una-cli/src/std_builtins.rs | \
    crates/una-cli/src/std_listing.rs | crates/una-cli/src/trace.rs)
        fast_count=$((fast_count + 1)) ;;
    crates/una-cli/src/*)
        go_full "$p: una-cli code on the check, build, run, package or solver path" ;;
    crates/una-cli/tests/common/*)
        go_full "$p: a test helper shared by every una-cli suite" ;;
    crates/una-cli/tests/native.rs)
        go_full "$p: the native differential suite" ;;
    crates/*/tests/*)
        local crate=${p#crates/}; crate=${crate%%/*}
        case "$crate" in
        una-cli | una-fmt | una-explain | una-report | una-lsp | una-diag)
            if test_binary_of "$p"; then fast_count=$((fast_count + 1)); else go_full "$p: test support of more than one suite"; fi ;;
        *) go_full "$p: a test of $crate" ;;
        esac ;;
    crates/una-fmt/* | crates/una-explain/* | crates/una-report/* | crates/una-lsp/* | crates/una-diag/*)
        local crate=${p#crates/}; crate=${crate%%/*}
        add_extra "package($crate)"
        fast_count=$((fast_count + 1)) ;;
    # Build, test, toolchain, CI and supply-chain configuration.
    Cargo.toml | Cargo.lock | rust-toolchain.toml | clippy.toml | deny.toml | verification.lock | install.sh | \
    .cargo/* | .config/* | .github/* | supply-chain/* | fuzz/*)
        go_full "$p: build, test, toolchain or supply-chain configuration" ;;
    scripts/gate-* | scripts/z3-library.sh | scripts/build-z3.sh | scripts/install-z3.sh | scripts/check-solver.sh | \
    scripts/check-toolchain.toml | scripts/install-check-toolchain.sh | scripts/toolchain-free.sh | \
    scripts/release* | scripts/check-release* | scripts/local-release.sh | scripts/sanitize* | scripts/fuzz* | \
    scripts/security-hunt/* | scripts/hunt-* | scripts/deps-audit.sh | scripts/no-libm* | scripts/check-fma.sh | \
    scripts/load-commands* | scripts/libsystem-exports.sh | scripts/scan-* | scripts/link-oracle-elf.sh | \
    scripts/local-ci* | scripts/container-* | scripts/check-released-sections.sh | scripts/musl* )
        go_full "$p: a gate, release, solver, toolchain or security script" ;;
    tests/conformance/security/*)
        go_full "$p: a security conformance case" ;;
    docs/reviews/*/adversarial/*)
        go_full "$p: an adversarial review case (read by the native differential and the verification suite)" ;;
    tests/conformance/* | examples/*)
        native_programs_of "$p"; readers_of "$p"
        fast_count=$((fast_count + 1)) ;;
    tests/*)
        # Other corpora of tests/ (serve_runs, ...): the suites that read them by path.
        readers_of "$p"
        fast_count=$((fast_count + 1)) ;;
    docs/* | changelog.d/* | site/* | formal/* | images/* | scripts/* | .claude/* | *.md | LICENSE | .gitignore | .gitattributes | .mcp.json)
        readers_of "$p"
        fast_count=$((fast_count + 1)) ;;
    *)
        go_full "$p: no rule names this path (unclassified paths run the whole suite)" ;;
    esac
}

if [ -n "$paths" ]; then
    while IFS= read -r p; do
        classify "$p"
    done <<EOF
$paths
EOF
fi

total=$(printf '%s\n' "$paths" | sed '/^$/d' | wc -l | tr -d ' ')
echo "$tier"
if [ "$tier" = full ]; then
    printf '%s' "$reasons"
else
    if [ "$total" -eq 0 ]; then
        echo "reason: no path changed against $base"
    else
        echo "reason: $total changed path(s) (against $base), none of them runtime, soundness, security, supply-chain or configuration"
    fi
    printf '%s' "$reasons"
    printf '%s' "$extras" | sed '/^$/d; s/^/extra: /'
    printf 'skipped: '
    sep=""
    for suite in $SKIPPED_SUITES; do printf '%sbinary_id(%s)' "$sep" "$suite"; sep=" | "; done
    echo
fi
