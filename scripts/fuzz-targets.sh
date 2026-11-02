# The fuzz targets of fuzz/ and how they run (#766), in one place: scripts/sanitize.sh (the
# `sanitize` release gate, at the full counts) and scripts/fuzz-smoke.sh (a smaller run, #1430)
# both source this file, so a target added here is run by both. It is sourced, never run.
#
# The caller sets, before it calls any function below: the working directory to the checkout's
# root, `channel` (the checking toolchain of scripts/check-toolchain.toml), `host` (its host
# triple), `fuzz_out` (the directory of the build and of each target's run), `fuzz_bin`
# (`$fuzz_out/build/$host/release`), `FUZZ_CEILING` (the time limit of one run, in seconds),
# and a function `fail` that prints its argument and exits nonzero.

# Built with these flags and no other tool: libFuzzer's coverage of every edge and comparison,
# AddressSanitizer, debug assertions and overflow checks, and `--cfg una_fuzz`, which alone gives
# `una-rt` the two hooks the targets call.
fuzz_flags="--cfg una_fuzz -Cpasses=sancov-module -Cllvm-args=-sanitizer-coverage-level=4"
fuzz_flags="$fuzz_flags -Cllvm-args=-sanitizer-coverage-inline-8bit-counters"
fuzz_flags="$fuzz_flags -Cllvm-args=-sanitizer-coverage-pc-table"
fuzz_flags="$fuzz_flags -Cllvm-args=-sanitizer-coverage-trace-compares"
fuzz_flags="$fuzz_flags -Zsanitizer=address --cfg una_asan -Cdebug-assertions -Coverflow-checks"

# The inputs of each target. The counts are measured (docs/release-gates.md, the `sanitize`
# gate): each target takes about a minute on the release machine, about ten times below the
# ceiling of scripts/sanitize.sh.
FUZZ_SEED=766
FUZZ_TARGETS="literal:350000 words:350000 json:450000"
# The floors of provenance, archive and ita_lock (#997) come from a run of 300000 inputs each.
# With a fixed seed a larger count runs those inputs first, so it reaches at least their
# coverage: provenance and archive run about a minute, and ita_lock keeps 300000 (about two
# minutes), since a lower count could end under its floor.
FUZZ_TARGETS="$FUZZ_TARGETS provenance:1280000 archive:1280000 ita_lock:300000"
# The parsers and verifiers of the documents a package registry serves (ADR 0122). Their counts
# and floors are placeholders until a run on the release machine measures them.
FUZZ_TARGETS="$FUZZ_TARGETS registry_note:350000 registry_root:350000 registry_statement:350000 registry_owner:350000 registry_map_tile:350000 registry_mirrors:350000 registry_tile:350000 registry_provenance:350000 registry_archive:350000 registry_proofs:350000 registry_map_walk:350000 registry_owner_replay:350000 registry_location:350000 registry_archive_check:350000 registry_root_chain:350000 registry_yank:350000 registry_advisory:350000 registry_entry:350000 registry_keyfile:350000 registry_bundle:350000 registry_watch:350000 registry_promise:350000"
# The internal linker's ELF reader and relocation engine (ADR 0124 step 4b), seeded with the
# per-type fixtures of crates/una-link/tests/elf-oracle as clang assembles them. Its count and
# floor are placeholders until a run on the release machine measures them.
FUZZ_TARGETS="$FUZZ_TARGETS elf_link:350000"
# The canary's count is a cap only: its planted crash ends its run, after about 30000 inputs.
FUZZ_CANARY_RUNS=1000000
# The coverage each target must reach, so that a target that returns early on every input, or a
# runtime built without libFuzzer's counters, fails rather than running its count for nothing:
# the counters of the instrumented code libFuzzer loads, and the edges (`cov`) and features
# (`ft`) its run ends with. Each floor is nine tenths of a measured run's value
# (docs/release-gates.md).
FUZZ_FLOORS="literal:244243:499:759 words:244798:4473:16955 json:244454:2124:7364"
FUZZ_FLOORS="$FUZZ_FLOORS provenance:143114:340:758 archive:143109:617:1593 ita_lock:144242:2848:9344"
FUZZ_FLOORS="$FUZZ_FLOORS registry_note:1:1:1 registry_root:1:1:1 registry_statement:1:1:1 registry_owner:1:1:1 registry_map_tile:1:1:1 registry_mirrors:1:1:1 registry_tile:1:1:1 registry_provenance:1:1:1 registry_archive:1:1:1 registry_proofs:1:1:1 registry_map_walk:1:1:1 registry_owner_replay:1:1:1 registry_location:1:1:1 registry_archive_check:1:1:1 registry_root_chain:1:1:1 registry_yank:1:1:1 registry_advisory:1:1:1 registry_entry:1:1:1 registry_keyfile:1:1:1 registry_bundle:1:1:1 registry_watch:1:1:1 registry_promise:1:1:1"
FUZZ_FLOORS="$FUZZ_FLOORS elf_link:1:1:1"

# Builds the fuzz targets, a crate outside the workspace with its own Cargo.lock.
fuzz_build() {
    env RUSTFLAGS="$fuzz_flags" cargo +"$channel" build --locked --release \
        --manifest-path fuzz/Cargo.toml --target "$host" --target-dir "$fuzz_out/build"
}

# The fuzz targets are exactly the canary and FUZZ_TARGETS: a target added to fuzz/Cargo.toml
# and left out here, or the reverse, fails.
fuzz_check_listed() {
    built_targets=$(sed -n '/^\[\[bin\]\]$/{n;s/^name = "\(.*\)"$/\1/p;}' fuzz/Cargo.toml | sort | tr '\n' ' ')
    listed_targets=$(for entry in canary $FUZZ_TARGETS; do echo "${entry%%:*}"; done | sort | tr '\n' ' ')
    [ "$built_targets" = "$listed_targets" ] \
        || fail "fuzz/Cargo.toml builds the targets $built_targets, but this script runs $listed_targets"
}

# Runs target $1 for $2 inputs; its status is left in $fuzz_rc, and in $fuzz_artifact the
# input it kept for a crash, a timeout or an out-of-memory, if any.
fuzz_run() {
    dir=${fuzz_out:?}/${1:?}
    # A previous run's reproducers are kept, moved aside under the time they were moved; only
    # the canary's, which are its planted crash, are removed.
    if [ "$1" != canary ] && [ -n "$(find "$dir/artifacts" -type f 2>/dev/null | head -1)" ]; then
        kept=$dir/artifacts-$(date -u +%Y%m%dT%H%M%SZ)
        mv "$dir/artifacts" "$kept" && echo "the reproducers of the previous run of $1 are kept in $kept"
    fi
    rm -rf "${dir:?}/corpus" "${dir:?}/artifacts" "${dir:?}/minimized"
    mkdir -p "$dir/corpus" "$dir/artifacts"
    env ASAN_OPTIONS="detect_leaks=0" "$fuzz_bin/$1" -seed="$FUZZ_SEED" -runs="$2" \
        -max_total_time="$FUZZ_CEILING" -timeout=10 -rss_limit_mb=4096 -max_len=4096 \
        -print_final_stats=1 -artifact_prefix="$dir/artifacts/" \
        "$dir/corpus" "fuzz/seeds/$1" > "$dir/log" 2>&1
    fuzz_rc=$?
    fuzz_artifact=$(find "$dir/artifacts" -type f | sort | head -1)
}

# Minimizes the input $fuzz_artifact of target $1 into $fuzz_out/$1/minimized, and prints
# where both are.
fuzz_report() {
    dir=$fuzz_out/$1
    tail -40 "$dir/log" >&2
    if [ -n "$fuzz_artifact" ]; then
        "$fuzz_bin/$1" -minimize_crash=1 -runs=20000 -max_total_time=120 -timeout=10 \
            -exact_artifact_path="$dir/minimized" "$fuzz_artifact" > "$dir/minimize.log" 2>&1
        echo "reproducer: $fuzz_artifact" >&2
        if [ -s "$dir/minimized" ]; then
            echo "minimized: $dir/minimized ($(wc -c < "$dir/minimized" | tr -d ' ') bytes)" >&2
            od -A d -c "$dir/minimized" | head -8 >&2
        else
            echo "minimized: none, see $dir/minimize.log" >&2
        fi
        echo "replay: $fuzz_bin/$1 $fuzz_artifact" >&2
    fi
}

# Each target calls AddressSanitizer's checks, with its runtime active.
fuzz_check_instrumented() {
    for entry in canary $FUZZ_TARGETS; do
        target=${entry%%:*}
        nm -u "$fuzz_bin/$target" | grep -q '___asan_report' \
            || fail "the fuzz target $target is not instrumented with AddressSanitizer"
        mkdir -p "$fuzz_out/$target/check"
        ASAN_OPTIONS="verbosity=1:detect_leaks=0" "$fuzz_bin/$target" -runs=0 \
            -artifact_prefix="$fuzz_out/$target/check/" 2>&1 \
            | grep -q "AddressSanitizer: libc interceptors initialized" \
            || fail "AddressSanitizer is not active in the fuzz target $target"
    done
}

# The self-check: the canary's planted crash, behind the input `una!`, must be found, kept and
# minimized, or a crash of the targets could pass unseen.
fuzz_canary() {
    echo "== The fuzz canary"
    fuzz_run canary "$FUZZ_CANARY_RUNS"
    [ "$fuzz_rc" != 0 ] && [ -n "$fuzz_artifact" ] \
        || { tail -20 "$fuzz_out/canary/log" >&2; fail "the fuzz canary's planted crash was not reported"; }
    fuzz_report canary 2> "$fuzz_out/canary/report"
    [ "$(head -c 4 "$fuzz_out/canary/minimized" 2>/dev/null)" = "una!" ] \
        || { cat "$fuzz_out/canary/report" >&2; fail "the fuzz canary's crash was not minimized to its input"; }
    echo "the canary's crash was reported, kept and minimized"
}

# Runs target $1 for $2 inputs and fails on a crash, a timeout, an out-of-memory, fewer inputs
# than $2, or coverage below its floors. The counters floor is the target's own: the counters of
# the code libFuzzer loads do not depend on the count. The floors of `cov` and `ft` are divided by
# $3 (1 at the gate's counts), never below 1.
fuzz_target() {
    target=$1
    runs=$2
    echo "== The fuzz target $target, $runs inputs"
    fuzz_run "$target" "$runs"
    if [ "$fuzz_rc" != 0 ] || [ -n "$fuzz_artifact" ]; then
        fuzz_report "$target"
        fail "the fuzz target $target found a crash, a timeout or an out-of-memory"
    fi
    executed=$(sed -n 's/^stat::number_of_executed_units: *//p' "$fuzz_out/$target/log" | tail -1)
    [ -n "$executed" ] && [ "$executed" -ge "$runs" ] \
        || { tail -20 "$fuzz_out/$target/log" >&2; fail "the fuzz target $target ran ${executed:-no} inputs of $runs before its ceiling of $FUZZ_CEILING s"; }
    # Its coverage, against the target's floors.
    floors=$(for f in $FUZZ_FLOORS; do [ "${f%%:*}" = "$target" ] && echo "${f#*:}"; done)
    [ -n "$floors" ] || fail "the fuzz target $target has no coverage floors"
    counters=$(sed -n 's/.*(\([0-9]*\) inline 8-bit counters).*/\1/p' "$fuzz_out/$target/log" | head -1)
    done_line=$(grep -E '^#[0-9]+[[:space:]]+DONE ' "$fuzz_out/$target/log" | tail -1)
    cov=$(echo "$done_line" | sed -n 's/.* cov: \([0-9]*\) .*/\1/p')
    ft=$(echo "$done_line" | sed -n 's/.* ft: \([0-9]*\) .*/\1/p')
    floor_counters=$(echo "$floors" | cut -d: -f1)
    floor_cov=$(( $(echo "$floors" | cut -d: -f2) / $3 ))
    floor_ft=$(( $(echo "$floors" | cut -d: -f3) / $3 ))
    [ "$floor_cov" -ge 1 ] || floor_cov=1
    [ "$floor_ft" -ge 1 ] || floor_ft=1
    [ -n "$counters" ] && [ -n "$cov" ] && [ -n "$ft" ] \
        && [ "$counters" -ge "$floor_counters" ] && [ "$cov" -ge "$floor_cov" ] && [ "$ft" -ge "$floor_ft" ] \
        || fail "the fuzz target $target reached ${counters:-no} counters, cov ${cov:-none} and ft ${ft:-none}, below its floors $floor_counters, $floor_cov and $floor_ft"
    grep -E '^Done [0-9]+ runs' "$fuzz_out/$target/log"
    echo "coverage: $counters counters (floor $floor_counters), cov $cov (floor $floor_cov), ft $ft (floor $floor_ft)"
}
