#!/bin/bash
# The tier of the `tests` gate for the current worktree (docs/compiler/testing.md, "Gate tiers"):
# `full` (the whole suite) or `fast` (a subset of the tests, for documentation, corpus and test-only
# changes; scripts/gate-tests.sh).
#
# Output, one item per line:
#   line 1        `full` or `fast`
#   `reason: ...` why, one line per path that decides (or a summary for fast)
#   `extra: ...`  a nextest filterset of tests the change needs beyond the fixed subset: the test
#                 binaries it edits, the native differential of the programs it edits, the suites that
#                 read the files it edits, and the fast binaries of a skipped suite in a package it
#                 changes (`fast_of_long`; in the full tier, only these)
#   `skipped: ...` (fast only) the nextest filterset of the suites the fast tier leaves out
#
# The tier is `full` when UNA_GATE_TIER=full, when the run of the engineering harness that holds the
# gate (UNA_HARNESS_GATE_HELD) has a security-fix, soundness-change, performance or release workflow
# or a workflow that cannot be read, when the changed paths cannot be computed or are empty, or when
# any changed path is not one that a rule below names as fast. The rules name what may be fast; every
# other path (runtime, soundness, security and supply-chain code, configuration, scripts outside a
# short list, shared test helpers, ...) is full. Nothing can lower the tier: there is no setting that
# makes a change fast, and the base of the comparison is always origin/main.
#
# Changed paths: `git diff --raw --no-renames origin/main...HEAD`, plus what is staged or modified
# against HEAD (a merge-train worktree holds its merge in the index) and untracked files. A symlink,
# a submodule (gitlink) or a mode change on either side of an entry is full. Every path is also
# classified in lowercase (a case-insensitive file system opens `SECURITY` as `security`), keeping
# the higher tier. Bash 3.2 (macOS) compatible. scripts/gate-tier-selftest.sh holds the rule to a
# list of cases.
#
# The worktree is UNA_GATE_ROOT, else the directory above this script: the trusted launcher of the
# tests gate runs the copy of this script that origin/main holds, from a temporary directory.
#
# Usage: scripts/gate-tier.sh [--paths FILE | --raw FILE]
#   --paths classifies the paths listed in FILE, one per line; --raw classifies lines in the format of
#   `git diff --raw` (modes included). Both replace the worktree's changes: for review and the self-test.
set -u

paths_file="" raw_file=""
case "${1:-}" in
--paths | --raw)
    f=${2:?$1 needs a file}
    case "$f" in /*) ;; *) f="$PWD/$f" ;; esac
    [ -f "$f" ] || { echo "gate-tier: no such file: $f" >&2; exit 2; }
    if [ "$1" = --paths ]; then paths_file=$f; else raw_file=$f; fi ;;
esac

root=${UNA_GATE_ROOT:-$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)}
cd "$root" || exit 2

tier=fast
reasons=""
extras=""
long_extras=""
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
    run=${held%%:*} dir=$root wf=""
    while [ "$dir" != / ]; do
        dir=$(dirname "$dir")
        manifest="$dir/.una-harness/runs/$run/manifest.json"
        [ -f "$manifest" ] || continue
        wf=$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["work_item"]["workflow"])' "$manifest" 2>/dev/null)
        break
    done
    case "$wf" in
    "") go_full "the workflow of the harness run $run cannot be read (fail closed)" ;;
    security-fix | soundness-change | performance | release*) go_full "the harness run $run has the workflow $wf" ;;
    esac
fi

base=origin/main
paths=""
raw=""
if [ -n "$paths_file" ]; then
    paths=$(sed '/^$/d' "$paths_file" | sort -u)
    base="the paths of $2"
elif [ -n "$raw_file" ]; then
    raw=$(sed '/^$/d' "$raw_file")
    base="the entries of $2"
elif ! git rev-parse --verify -q "$base^{commit}" >/dev/null; then
    go_full "the base $base does not resolve to a commit, so the changed paths are unknown"
elif ! committed=$(git diff --raw --no-renames "$base"...HEAD 2>/dev/null); then
    go_full "git diff against $base failed, so the changed paths are unknown"
else
    pending=$(git diff --raw --no-renames HEAD 2>/dev/null)
    # Untracked files as `git diff --raw` would list them once added (a symlink keeps its mode).
    untracked=$(git ls-files --others --exclude-standard 2>/dev/null | while IFS= read -r u; do
        if [ -L "$u" ]; then m=120000; elif [ -x "$u" ]; then m=100755; else m=100644; fi
        printf ':000000 %s 0000000 0000000 A\t%s\n' "$m" "$u"
    done)
    raw=$(printf '%s\n%s\n%s\n' "$committed" "$pending" "$untracked" | sed '/^$/d')
fi
if [ -n "$raw" ]; then
    # `:<old mode> <new mode> <old sha> <new sha> <status>TAB<path>`: symlinks (120000), gitlinks
    # (160000) and mode changes are full.
    tab=$(printf '\t')
    while IFS="$tab" read -r meta rpath; do
        [ -n "$meta" ] || continue
        om=$(printf '%s' "$meta" | awk '{print substr($1, 2)}')
        nm=$(printf '%s' "$meta" | awk '{print $2}')
        case "$om $nm" in
        *120000* | *160000*) go_full "$rpath: a symlink or a submodule (mode $om to $nm)" ;;
        esac
        if [ "$om" != "$nm" ] && [ "$om" != 000000 ] && [ "$nm" != 000000 ]; then
            go_full "$rpath: a mode change ($om to $nm)"
        fi
    done <<RAW
$raw
RAW
    paths=$(printf '%s\n' "$raw" | awk -F'\t' '{print $2}' | sed '/^$/d' | sort -u)
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

# The binaries of the skipped suites that are fast and deterministic: static checks of the package's
# own source (size caps) and engine tests over scripted connections, a clock the test drives or bytes
# in memory, with no wall-clock bound, no live socket and no child process (#2361). A change of the
# package runs them in either tier (an `extra:` line), so a check cannot miss an invariant of the
# package it changes (#2359: `una-sys::solver_size` broke unseen); the slow or timing-dependent
# binaries of those suites stay for the nightly. Test time on the gate machine under load
# (2026-11-27): una-sys 0.5 s, una-http 18 s (provider_parts 12 s of it). `una-interp::confine`
# (90 s) and the una-cli suites (9 s and up, each) have none; a builder names the una-cli ones with
# `ops/pr-check.sh --suites`.
fast_of_long() {
    case "$1" in
    una-sys) echo "una-sys::base_size una-sys::solver_size" ;;
    una-http) echo "una-http::serve_engine una-http::serve_faults una-http::serve_scripts una-http::serve_quiet una-http::provider_parts" ;;
    esac
}

# A changed path in a crate with fast binaries in a skipped suite adds them.
fast_long_of() {
    local crate b
    case "$1" in crates/*/*) ;; *) return 0 ;; esac
    crate=${1#crates/}; crate=${crate%%/*}
    for b in $(fast_of_long "$crate"); do
        add_extra "binary_id($b)"
        case "
$long_extras" in
        *"
binary_id($b)
"*) ;;
        *) long_extras="${long_extras}binary_id($b)
" ;;
        esac
    done
}

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
readers_table=""
readers_of() {
    local p=$1 suite f b lit
    # UNA_GATE_READERS_CACHE (set by the self-test, which classifies many lists of one tree) names a
    # file that holds the table once built.
    if [ -z "$readers_table" ] && [ -n "${UNA_GATE_READERS_CACHE:-}" ] && [ -s "$UNA_GATE_READERS_CACHE" ]; then
        readers_table=$(cat "$UNA_GATE_READERS_CACHE")
    fi
    if [ -z "$readers_table" ]; then
        # One line per suite and literal: `<binary id> <literal>`, read once per invocation. A literal
        # starts with any top-level entry of the repository (docs, examples, tests, README.md, images, ...).
        local tops
        tops=$(git ls-tree --name-only HEAD 2>/dev/null | grep -E '^[A-Za-z0-9._-]+$' | sed 's/\./\\./g' | paste -sd'|' -)
        [ -n "$tops" ] || tops='docs|examples|scripts|site|tests|fuzz|formal|images|README\.md|CHANGELOG\.md'
        for suite in $SKIPPED_SUITES; do
            for f in $(suite_files "$suite"); do
                b=${f#crates/}; b="${b%%/*}::$(basename "${f%/main.rs}" .rs)"
                # A literal path, a bare directory ("examples") or the prefix of a format string
                # ("tests/conformance/{case}"); matching more than the suite reads only adds tests.
                for lit in $(grep -oE "\"(\\.\\./)*($tops)(/[^\"{ ]*)?" "$f" | tr -d '"' | sed -E 's#^(\.\./)+##; s#/$##' | sort -u); do
                    readers_table="${readers_table}$b $lit
"
                done
            done
        done
        readers_table="${readers_table}-"
        [ -n "${UNA_GATE_READERS_CACHE:-}" ] && printf '%s' "$readers_table" > "$UNA_GATE_READERS_CACHE"
    fi
    while read -r b lit; do
        [ -n "$lit" ] || continue
        case "$p" in
        "$lit" | "$lit"/*) ;;
        *) case "$lit" in "$p"/*) ;; *) continue ;; esac ;;
        esac
        if [ "$b" = una-cli::native ]; then
            go_full "$p: read by the native differential (una-cli::native)"
        else
            add_extra "binary_id($b)"
        fi
    done <<EOF2
$readers_table
EOF2
}

# A corpus directory tests/<d>/: every skipped suite whose source names `<d>` as a path segment or a
# string adds itself (the native differential makes the tier full). A corpus that no test source names
# at all is full: nothing shows which suite reads it.
corpus_readers_of() {
    local p=$1 d suite f b hit=0
    d=${p#tests/}; d=${d%%/*}
    for suite in $SKIPPED_SUITES; do
        for f in $(suite_files "$suite"); do
            grep -qE "[\"/]$d[\"/]" "$f" || continue
            b=${f#crates/}; b="${b%%/*}::$(basename "${f%/main.rs}" .rs)"
            if [ "$b" = una-cli::native ]; then go_full "$p: the native differential reads tests/$d"; else add_extra "binary_id($b)"; fi
        done
    done
    grep -rqlE "[\"/]$d[\"/]" crates/*/tests crates/*/src crates/*/build.rs 2>/dev/null || hit=1
    [ "$hit" = 0 ] || go_full "$p: no test source names tests/$d"
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
        elif [ "$name" = snapshots ]; then
            # insta names a snapshot `<test binary>__<test>.snap`.
            local stem prefix
            stem=$(basename "$rest"); prefix=${stem%%__*}
            if [ "$prefix" != "$stem" ] && [ "$prefix" != native ] && [ -f "crates/$crate/tests/$prefix.rs" ]; then
                add_extra "binary_id($crate::$prefix)"
            else
                return 1
            fi
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
    fast_long_of "$p"
    case "$p" in
    # Runtime code: the abstract machine, code generation, native runtime, linking, HTTP, system calls, solver binding, registry service.
    crates/una-rt/* | crates/una-interp/* | crates/una-codegen/* | crates/una-http/* | crates/una-sys/* | crates/una-link/* | crates/una-z3/* | crates/una-itaty/*)
        go_full "$p: runtime code" ;;
    # Soundness code: the checker, the verifier, Core IR, and the parser and lexer that feed them.
    crates/una-check/* | crates/una-verify/* | crates/una-core/* | crates/una-syntax/*)
        go_full "$p: soundness code" ;;
    crates/una-diag/*)
        go_full "$p: diagnostics (severity, which decides rejection; the obligation status; codes; safe writes)" ;;
    crates/una-crypto/* | crates/una-hash/*)
        go_full "$p: security code" ;;
    crates/una-project/*)
        go_full "$p: supply chain (package manager, registry client, locks)" ;;
    crates/una-test-cases/*)
        go_full "$p: the case lists of the sharded suites" ;;
    crates/*/Cargo.toml | crates/*/build.rs)
        go_full "$p: build configuration" ;;
    # The standard library reference listing (docs/std/listing.json): presentation of what the
    # checker declares, held to it by the std_listing suite, which the fast tier runs. Every other
    # module of una-cli is on the check, build, run, serve, package, solver, grant, limit, contract,
    # manifest, report or repair path (args.rs: grants, limits, default tier; contract.rs: entry points;
    # report.rs: dependency authority; trace.rs: obligation status; fix.rs and migrate.rs: rewrite
    # sources and manifests; lsp.rs: lock and proof cache; core_json.rs: the formal track's input).
    crates/una-cli/src/std_builtins.rs | crates/una-cli/src/std_listing.rs)
        add_extra "binary_id(una-cli::std_listing)"
        fast_count=$((fast_count + 1)) ;;
    crates/una-cli/src/*)
        go_full "$p: una-cli code on the check, build, run, serve, package, solver, grant, limit, contract, manifest, report or repair path" ;;
    crates/una-cli/tests/common/*)
        go_full "$p: a test helper shared by every una-cli suite" ;;
    crates/una-cli/tests/native.rs)
        go_full "$p: the native differential suite" ;;
    crates/una-cli/tests/conformance/discovery.rs)
        go_full "$p: shared with the case lists of crates/una-test-cases" ;;
    crates/*/tests/*)
        local crate=${p#crates/}; crate=${crate%%/*}
        case "$crate" in
        una-cli | una-explain)
            if test_binary_of "$p"; then fast_count=$((fast_count + 1)); else go_full "$p: test support of more than one suite"; fi ;;
        *) go_full "$p: a test of $crate" ;;
        esac ;;
    # The explanation texts of `una explain`: Markdown that no check, build or run reads (the crate is
    # kept out of every executable, #413). Every other crate is full: una-diag decides severity and the
    # obligation status, una-fmt and una-lsp rewrite sources, una-report is a stub with no test of its own.
    crates/una-fmt/* | crates/una-lsp/*)
        go_full "$p: rewrites sources (formatting, quick fixes), so a bug changes the program that is checked and run" ;;
    crates/una-report/*)
        go_full "$p: build report assembly, with no suite of its own to run in the fast tier" ;;
    crates/una-explain/*)
        add_extra "package(una-explain) | binary_id(una-cli::explain_examples)"
        fast_count=$((fast_count + 1)) ;;
    # Build, test, toolchain, CI and supply-chain configuration.
    Cargo.toml | Cargo.lock | rust-toolchain.toml | clippy.toml | deny.toml | verification.lock | install.sh | \
    .cargo/* | .config/* | .github/* | supply-chain/* | fuzz/*)
        go_full "$p: build, test, toolchain or supply-chain configuration" ;;
    # Scripts: full, except two documentation helpers whose own suites the fast tier runs:
    # changelog-assemble.sh (assembles changelog.d into CHANGELOG.md; suite changelog_fragments) and
    # the walkthrough scripts of scripts/demo (suite demo_scripts). No release gate or harness check
    # calls either. Every other script is called by a gate, the release, the solver, the toolchain,
    # the formal track, the website bundle or the security hunt, or produces a test corpus.
    scripts/changelog-assemble.sh)
        add_extra "binary_id(una-cli::changelog_fragments)"
        fast_count=$((fast_count + 1)) ;;
    scripts/demo/*)
        add_extra "binary_id(una-cli::demo_scripts)"
        fast_count=$((fast_count + 1)) ;;
    scripts/*)
        go_full "$p: a script (gates, release, solver, toolchain, formal track, website bundle, hunt or corpus)" ;;
    tests/conformance/security | tests/conformance/security/*)
        go_full "$p: a security conformance case" ;;
    docs/reviews/*/adversarial | docs/reviews/*/adversarial/*)
        go_full "$p: an adversarial review case (read by the native differential and the verification suite)" ;;
    tests/conformance/una.toml | tests/conformance/ita.una | tests/conformance/*/una.toml | tests/conformance/*/ita.una)
        case "${p#tests/conformance/}" in
        */*/*) native_programs_of "$p"; readers_of "$p"
            add_extra "binary_id(una-cli::verification)"; add_extra "binary_id(una-cli::runtime)"
            fast_count=$((fast_count + 1)) ;;
        *) go_full "$p: a manifest at the corpus root or an area root regroups the programs of the native differential" ;;
        esac ;;
    tests/conformance/* | examples/*)
        # The verification and runtime suites read the whole corpus (examples, tests/conformance).
        native_programs_of "$p"; readers_of "$p"
        add_extra "binary_id(una-cli::verification)"
        add_extra "binary_id(una-cli::runtime)"
        fast_count=$((fast_count + 1)) ;;
    tests/*)
        # Other corpora of tests/ (serve_runs, ...): the suites that read them by path or name.
        readers_of "$p"; corpus_readers_of "$p"
        fast_count=$((fast_count + 1)) ;;
    docs/* | changelog.d/* | site/* | formal/* | images/* | .claude/* | *.md | LICENSE | license | .gitignore | .mcp.json)
        readers_of "$p"
        fast_count=$((fast_count + 1)) ;;
    *)
        go_full "$p: no rule names this path (unclassified paths run the whole suite)" ;;
    esac
}

if [ -n "$paths" ]; then
    while IFS= read -r p; do
        case "/$p/" in
        */./* | */../* | *//*) go_full "$p: a path with . or .. segments (not normalized)"; continue ;;
        esac
        classify "$p"
        # A case-insensitive file system opens `Examples/hello/README.md` as `examples/hello/README.md`
        # and `tests/conformance/SECURITY` as `security`: the lowercase path is classified too, and the
        # higher tier wins (it can only add reasons and tests).
        lp=$(printf '%s' "$p" | tr 'A-Z' 'a-z')
        [ "$lp" = "$p" ] || classify "$lp"
    done <<EOF
$paths
EOF
fi

total=$(printf '%s\n' "$paths" | sed '/^$/d' | wc -l | tr -d ' ')
if [ "$total" -eq 0 ]; then
    go_full "no changed path against $base (an empty change runs the whole suite)"
fi
echo "$tier"
if [ "$tier" = full ]; then
    printf '%s' "$reasons"
    # The full tier runs every test; its `extra:` lines name only the fast binaries of the skipped
    # suites that a changed package holds (`fast_of_long`), for a check that runs less than every
    # test (ops/pr-check.sh's lane C adds them to the changed packages' tests).
    printf '%s' "$long_extras" | sed '/^$/d; s/^/extra: /'
else
    echo "reason: $total changed path(s) (against $base), each one named fast by a rule"
    printf '%s' "$reasons"
    printf '%s' "$extras" | sed '/^$/d; s/^/extra: /'
    printf 'skipped: '
    sep=""
    for suite in $SKIPPED_SUITES; do printf '%sbinary_id(%s)' "$sep" "$suite"; sep=" | "; done
    echo
fi
