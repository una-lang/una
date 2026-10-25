#!/bin/sh
# Checks the native runtime's memory safety with the checking toolchain of
# scripts/check-toolchain.toml (#766), which never builds a release artifact:
#
# 1. Miri runs the unit tests of `una-rt`, the crate whose `ffi.rs` holds the only `unsafe` of
#    the workspace (ADR 0058), and reports undefined behavior in the Rust code they reach. The
#    tests Miri does not run are ignored in the code with their reason, and the set it ignores
#    must be exactly scripts/sanitize-miri-ignored.txt, so it cannot change unseen.
# 2. AddressSanitizer: `una-rt` is built with `-Zsanitizer=address`, and the native equivalence
#    test (spec/16 §5) runs every runnable program of the repository against it. The link is the
#    gate's own: a `cc` placed first on this script's PATH adds the toolchain's AddressSanitizer
#    runtime to what `una build` links, so `una` itself has no hook for it. Any report aborts the
#    program and fails the test.
# 3. libFuzzer: the fuzz targets of fuzz/ (the literals, the entry points on any words, the JSON
#    parser, the readers of a provenance, an archive and ita.lock, and the parsers and verifiers
#    of the documents a package registry serves) run a fixed number of inputs each, under
#    AddressSanitizer, after a canary target whose planted crash must be found, kept and
#    minimized. A crash prints where its input and the minimized input are kept, under
#    target/fuzz.
#
# It ends with PASS, or FAIL and the step that failed. A missing or different checking toolchain
# is a FAIL, never a skip. macOS only for now: the Linux image has no checking toolchain yet.
#
# Usage: scripts/sanitize.sh
set -u

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

fail() {
    echo "sanitize: FAIL ($1)" >&2
    exit 1
}

step() {
    name=$1
    shift
    echo "== $name"
    "$@" || fail "$name"
}

channel=$(sed -n 's/^channel *= *"\(.*\)"$/\1/p' scripts/check-toolchain.toml)
commit=$(sed -n 's/^commit *= *"\(.*\)"$/\1/p' scripts/check-toolchain.toml)
[ -n "$channel" ] && [ -n "$commit" ] || fail "scripts/check-toolchain.toml names no channel and commit"
have=$(rustc +"$channel" -vV 2>/dev/null | sed -n 's/^commit-hash: //p')
[ "$have" = "$commit" ] \
    || fail "the checking toolchain $channel ($commit) is missing or differs; run scripts/install-check-toolchain.sh"
host=$(rustc +"$channel" -vV | sed -n 's/^host: //p')
[ "$(uname -s)" = Darwin ] || fail "macOS only for now"

miri_log=$root/target/sanitize/miri.log
mkdir -p "$root/target/sanitize"
echo "== Miri on the unit tests of una-rt"
# Permissive provenance: the runtime's handles are integers, and Miri then accepts the casts
# from integers to pointers that strict provenance would refuse, a known weakening of its checks.
env MIRIFLAGS="-Zmiri-permissive-provenance" cargo +"$channel" miri test -p una-rt --lib \
    > "$miri_log" 2>&1 || { cat "$miri_log"; fail "Miri on the unit tests of una-rt"; }
grep -E '^test result' "$miri_log"
# The tests Miri ignored are exactly those of scripts/sanitize-miri-ignored.txt: a new skip, or
# one that disappears, fails here rather than passing unseen.
sed -n 's/^test \([^ ]*\) \.\.\. ignored.*$/\1/p' "$miri_log" | sort > "$root/target/sanitize/ignored.txt"
grep -v '^#' scripts/sanitize-miri-ignored.txt | sort > "$root/target/sanitize/expected.txt"
diff "$root/target/sanitize/expected.txt" "$root/target/sanitize/ignored.txt" >&2 \
    || fail "the tests Miri ignores differ from scripts/sanitize-miri-ignored.txt (above: < expected, > ignored)"
# Every test of una-rt is one Miri knows, by name: a test compiled out under Miri fails here.
# And every one of them passed or was ignored, by the counts of Miri's summary (a test's own
# output can break its result line, so the lines are not read one by one).
cargo test -p una-rt --lib -- --list 2>/dev/null | sed -n 's/: test$//p' | sort > "$root/target/sanitize/all.txt"
env MIRIFLAGS="-Zmiri-permissive-provenance" cargo +"$channel" miri test -p una-rt --lib -- --list 2>/dev/null \
    | sed -n 's/: test$//p' | sort > "$root/target/sanitize/miri-all.txt"
[ -s "$root/target/sanitize/all.txt" ] || fail "the tests of una-rt could not be listed"
diff "$root/target/sanitize/all.txt" "$root/target/sanitize/miri-all.txt" >&2 \
    || fail "the tests of una-rt differ from those Miri compiles (above: < all, > under Miri)"
total=$(wc -l < "$root/target/sanitize/all.txt" | tr -d ' ')
expected_ignored=$(wc -l < "$root/target/sanitize/expected.txt" | tr -d ' ')
summary=$(grep -E '^test result: ok\.' "$miri_log" | head -1)
passed=$(echo "$summary" | sed -n 's/.* \([0-9]*\) passed.*/\1/p')
ignored_count=$(echo "$summary" | sed -n 's/.* \([0-9]*\) ignored.*/\1/p')
[ "$ignored_count" = "$expected_ignored" ] && [ $((passed + ignored_count)) = "$total" ] \
    || fail "Miri passed $passed and ignored $ignored_count of $total tests, where $expected_ignored are to be ignored"
# Code that behaves differently under Miri does so only where counted here: every mention of
# `miri` in una-rt, any module and any form, but the ignore attributes checked above. Today fourteen
# tests that run smaller under Miri, where each allocation is slow (#778): two deep values at
# depth 1,000 rather than 1,000,000, a copy between heaps at depth 1,000 rather than 200,000,
# 200 rounds of `find` rather than 10,000, the entry points with a pool of one word of each
# kind and 20 drawn tuples, and the copies of ADR 0113 (#777): 1,000 exports rather than
# 1,000,000, imports of 100 and 1,000 objects rather than up to 100,000, a list 3 segments deep
# rather than 100,000, a staged block of 3 tasks rather than 70, in batches of 2, lists of 10
# and 100 texts counted rather than up to 100,000, blocks nested 3 deep rather than 200, and 4
# and 8 traced blocks rather than up to 256, and flat lists of 1,000 integers compared rather
# than 100,000 (#815), and texts of 4,301 and 17,200 digits rather than 100,000 and 1,000,000
# refused by the integer digit cap, whose timing runs only outside Miri, two mentions (#914).
# Their full sizes run in the ordinary tests. And one constant: the
# debug quarantine of freed slots is 64 rather than 65,536, so that the test of slot reuse runs
# under Miri and Miri checks the reuse.
reduced=$(grep -rn 'miri' crates/una-rt | grep -vc 'cfg_attr(miri, ignore')
[ "$reduced" = 16 ] || fail "una-rt mentions miri $reduced times outside its ignore attributes, and this script states 16"

asan=$(rustc +"$channel" --print sysroot)/lib/rustlib/$host/lib/librustc-nightly_rt.asan.dylib
[ -f "$asan" ] || fail "the checking toolchain has no AddressSanitizer runtime at $asan"
step "una-rt with AddressSanitizer" \
    env RUSTFLAGS="-Zsanitizer=address" cargo +"$channel" build -p una-rt \
    --target "$host" --target-dir target/sanitize

# Cargo's own links (build scripts, procedural macros, the test binaries) name the system `cc`,
# never the shim below: an instrumented procedural macro would abort the compiler. CC as well:
# a build script that compiles C through the `cc` crate never resolves the shim.
linker_var=CARGO_TARGET_$(echo "$host" | tr '[:lower:]-' '[:upper:]_')_LINKER

# The unit tests of una-rt at their full sizes, under AddressSanitizer (#778): Miri runs some of
# them smaller, and the deep ones prove here, as natively, that their walks use no recursion.
# The test binary is the one artifact whose profile is a test's (`"profile":{…"test":true`),
# never the last one cargo happens to print, and there must be exactly one. It must call
# AddressSanitizer's checks, or this step proves nothing.
asan_tests_binaries=$(env RUSTFLAGS="-Zsanitizer=address" "$linker_var=/usr/bin/cc" CC=/usr/bin/cc \
    cargo +"$channel" test -p una-rt --lib --target "$host" --target-dir target/sanitize \
    --no-run --message-format=json 2>/dev/null \
    | grep '"profile":{[^}]*"test":true' | sed -n 's/.*"executable":"\([^"]*\)".*/\1/p')
[ "$(printf '%s\n' "$asan_tests_binaries" | grep -c .)" = 1 ] \
    || fail "the unit tests of una-rt built as $(printf '%s\n' "$asan_tests_binaries" | grep -c .) test binaries, not one"
asan_tests_binary=$asan_tests_binaries
[ -n "$asan_tests_binary" ] && [ -x "$asan_tests_binary" ] \
    || fail "the unit tests of una-rt did not build with AddressSanitizer"
nm -u "$asan_tests_binary" | grep -q '___asan_report' \
    || fail "the unit tests of una-rt are not instrumented with AddressSanitizer"
ASAN_OPTIONS="verbosity=1" "$asan_tests_binary" --list 2>&1 \
    | grep -q "AddressSanitizer: libc interceptors initialized" \
    || fail "AddressSanitizer is not active in the unit tests of una-rt"
# The scenarios that run in a process of their own are run by their parent tests through this
# same binary (`current_exe`), so they run instrumented too, with these options; an
# AddressSanitizer report there ends the scenario with a code its parent test refuses.
step "The unit tests of una-rt with AddressSanitizer" \
    env ASAN_OPTIONS="abort_on_error=1:detect_leaks=0" "$asan_tests_binary"
# The shim lives in this checkout's target directory and is on the PATH of one command only.
shim=$root/target/sanitize/shim
mkdir -p "$shim"
# The AddressSanitizer runtime comes first on the link line: macOS loads it before libSystem,
# or its interceptors are not installed.
cat > "$shim/cc" <<EOF
#!/bin/sh
exec /usr/bin/cc "$asan" -Wl,-rpath,"$(dirname "$asan")" "\$@"
EOF
chmod +x "$shim/cc"
runtime=$root/target/sanitize/$host/debug/libuna_rt.a

step "una and its tests, linked by the system linker" \
    env "$linker_var=/usr/bin/cc" CC=/usr/bin/cc cargo build -p una-cli --tests

# A canary: an executable that `una build` links through the shim runs with AddressSanitizer
# active, or this gate would check nothing.
canary=$root/target/sanitize/canary
mkdir -p "$canary/src"
cp examples/hello/verification.lock "$canary/"
printf '[package]\nname = "canary"\nversion = "0.1.0"\nedition = "0.1"\n\n[run]\nentry = "canary"\ngrants = ["console"]\n' \
    > "$canary/una.toml"
printf 'module canary\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  @console.print("sanitized")\n' \
    > "$canary/src/canary.una"
build_canary() {
    (cd "$canary" && env UNA_RT_LIB="$runtime" PATH="$shim:$PATH" "$root/target/debug/una" build .)
}
step "The canary, linked through the shim" build_canary
ASAN_OPTIONS="verbosity=1" "$canary/target/una/canary" 2>&1 \
    | grep -q "AddressSanitizer: libc interceptors initialized" \
    || fail "AddressSanitizer is not active in an executable linked through the shim"
# And the runtime it links is the instrumented one: its code calls AddressSanitizer's checks.
nm -u "$canary/target/una/canary" | grep -q '___asan_report' \
    || fail "the runtime linked through the shim is not instrumented with AddressSanitizer"

step "The native equivalence test against it" \
    env "$linker_var=/usr/bin/cc" CC=/usr/bin/cc UNA_RT_LIB="$runtime" PATH="$shim:$PATH" \
    ASAN_OPTIONS="abort_on_error=1:detect_leaks=0" \
    cargo test -p una-cli --test native every_runnable_program_runs_natively_as_in_the_interpreter \
    -- --exact --nocapture

# 3. libFuzzer: the targets of fuzz/, a crate outside the workspace with its own Cargo.lock,
# built with the flags below and no other tool: libFuzzer's coverage of every edge and
# comparison, AddressSanitizer, debug assertions and overflow checks, and `--cfg una_fuzz`,
# which alone gives `una-rt` the two hooks the targets call. Each target runs a fixed number
# of inputs from its committed seeds (fuzz/seeds/<target>) with a fixed seed, so a run's size
# never depends on the machine's speed; the time limit is a ceiling only, and a run that
# reaches it before its count fails. The inputs the fuzzer adds stay under target/fuzz, never
# committed. LeakSanitizer does not run on macOS, so no leak is detected here.
fuzz_flags="--cfg una_fuzz -Cpasses=sancov-module -Cllvm-args=-sanitizer-coverage-level=4"
fuzz_flags="$fuzz_flags -Cllvm-args=-sanitizer-coverage-inline-8bit-counters"
fuzz_flags="$fuzz_flags -Cllvm-args=-sanitizer-coverage-pc-table"
fuzz_flags="$fuzz_flags -Cllvm-args=-sanitizer-coverage-trace-compares"
fuzz_flags="$fuzz_flags -Zsanitizer=address -Cdebug-assertions -Coverflow-checks"
fuzz_out=$root/target/fuzz
step "The fuzz targets, with libFuzzer and AddressSanitizer" \
    env RUSTFLAGS="$fuzz_flags" cargo +"$channel" build --locked --release \
    --manifest-path fuzz/Cargo.toml --target "$host" --target-dir "$fuzz_out/build"
fuzz_bin=$fuzz_out/build/$host/release

# The inputs of each target, and the ceiling of its run in seconds. The counts are measured
# (docs/release-gates.md, the `sanitize` gate): each target takes about a minute on the release
# machine, about ten times below the ceiling.
FUZZ_SEED=766
FUZZ_TARGETS="literal:350000 words:350000 json:450000"
# The counts of provenance, archive and ita_lock (#997) are placeholders, not yet measured on
# the release machine; their floors below are placeholders too, of one counter, edge and
# feature each, until a measured run replaces them (docs/release-gates.md).
FUZZ_TARGETS="$FUZZ_TARGETS provenance:300000 archive:300000 ita_lock:300000"
# The parsers and verifiers of the documents a package registry serves (ADR 0122). Their counts
# and floors are placeholders until a run on the release machine measures them.
FUZZ_TARGETS="$FUZZ_TARGETS registry_note:350000 registry_root:350000 registry_statement:350000 registry_owner:350000 registry_map_tile:350000 registry_mirrors:350000 registry_tile:350000 registry_provenance:350000 registry_archive:350000 registry_proofs:350000 registry_map_walk:350000 registry_owner_replay:350000 registry_location:350000 registry_archive_check:350000 registry_root_chain:350000"
FUZZ_CEILING=600
# The coverage each target must reach, so that a target that returns early on every input, or a
# runtime built without libFuzzer's counters, fails rather than running its count for nothing:
# the counters of the instrumented code libFuzzer loads, and the edges (`cov`) and features
# (`ft`) its run ends with. Each floor is nine tenths of a measured run's value
# (docs/release-gates.md).
FUZZ_FLOORS="literal:244243:499:759 words:244798:4473:16955 json:244454:2124:7364"
FUZZ_FLOORS="$FUZZ_FLOORS provenance:1:1:1 archive:1:1:1 ita_lock:1:1:1"
FUZZ_FLOORS="$FUZZ_FLOORS registry_note:1:1:1 registry_root:1:1:1 registry_statement:1:1:1 registry_owner:1:1:1 registry_map_tile:1:1:1 registry_mirrors:1:1:1 registry_tile:1:1:1 registry_provenance:1:1:1 registry_archive:1:1:1 registry_proofs:1:1:1 registry_map_walk:1:1:1 registry_owner_replay:1:1:1 registry_location:1:1:1 registry_archive_check:1:1:1 registry_root_chain:1:1:1"

# The fuzz targets are exactly the canary and FUZZ_TARGETS: a target added to fuzz/Cargo.toml
# and left out here, or the reverse, fails.
built_targets=$(sed -n '/^\[\[bin\]\]$/{n;s/^name = "\(.*\)"$/\1/p;}' fuzz/Cargo.toml | sort | tr '\n' ' ')
listed_targets=$(for entry in canary $FUZZ_TARGETS; do echo "${entry%%:*}"; done | sort | tr '\n' ' ')
[ "$built_targets" = "$listed_targets" ] \
    || fail "fuzz/Cargo.toml builds the targets $built_targets, but this script runs $listed_targets"

# Runs target $1 for $2 inputs; its status is left in $fuzz_rc, and in $fuzz_artifact the
# input it kept for a crash, a timeout or an out-of-memory, if any.
fuzz_run() {
    dir=$fuzz_out/$1
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

# Minimizes the input $fuzz_artifact of target $1 into target/fuzz/$1/minimized, and prints
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

# The self-check: the canary's planted crash, behind the input `una!`, must be found, kept and
# minimized, or a crash of the targets below could pass unseen.
echo "== The fuzz canary"
fuzz_run canary 1000000
[ "$fuzz_rc" != 0 ] && [ -n "$fuzz_artifact" ] \
    || { tail -20 "$fuzz_out/canary/log" >&2; fail "the fuzz canary's planted crash was not reported"; }
fuzz_report canary 2> "$fuzz_out/canary/report"
[ "$(head -c 4 "$fuzz_out/canary/minimized" 2>/dev/null)" = "una!" ] \
    || { cat "$fuzz_out/canary/report" >&2; fail "the fuzz canary's crash was not minimized to its input"; }
echo "the canary's crash was reported, kept and minimized"

for entry in $FUZZ_TARGETS; do
    target=${entry%%:*}
    runs=${entry#*:}
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
    floor_cov=$(echo "$floors" | cut -d: -f2)
    floor_ft=$(echo "$floors" | cut -d: -f3)
    [ -n "$counters" ] && [ -n "$cov" ] && [ -n "$ft" ] \
        && [ "$counters" -ge "$floor_counters" ] && [ "$cov" -ge "$floor_cov" ] && [ "$ft" -ge "$floor_ft" ] \
        || fail "the fuzz target $target reached ${counters:-no} counters, cov ${cov:-none} and ft ${ft:-none}, below its floors $floor_counters, $floor_cov and $floor_ft"
    grep -E '^Done [0-9]+ runs' "$fuzz_out/$target/log"
    echo "coverage: $counters counters (floor $floor_counters), cov $cov (floor $floor_cov), ft $ft (floor $floor_ft)"
done

echo "sanitize: PASS ($channel, $host)"
