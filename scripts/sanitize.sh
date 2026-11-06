#!/bin/sh
# Checks the native runtime's memory safety with the checking toolchain of
# scripts/check-toolchain.toml (#766), which never builds a release artifact:
#
# 1. Miri runs the unit tests of `una-rt`, the crate whose `ffi.rs` holds the runtime's `unsafe`
#    (ADR 0058; `una-sys` and `una-z3` hold the rest, docs/security/tcb.md §3.3), and reports
#    undefined behavior in the Rust code they reach. The
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
# The test profile of the per-PR gate is opt-level 1 for speed (#1697); the release evidence keeps
# the unoptimized build it always had. Crates with their own opt-level in Cargo.toml (una-rt and
# una-http at 0, una-link, object and sha2 at 3) keep it: the variable sets the profile-wide value only.
export CARGO_PROFILE_DEV_OPT_LEVEL=0

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
# `miri` in una-rt, any module and any form, but the ignore attributes checked above. Today
# eighteen tests that run smaller under Miri, where each allocation is slow (#778): two deep
# values at depth 1,000 rather than 1,000,000, a copy between heaps at depth 1,000 rather than
# 200,000, 200 rounds of `find` rather than 10,000, the entry points with a pool of one word of
# each kind and 20 drawn tuples, and the copies of ADR 0113 (#777): 1,000 exports rather than
# 1,000,000, imports of 100 and 1,000 objects rather than up to 100,000, a list 3 segments deep
# rather than 100,000, a staged block of 3 tasks rather than 70, in batches of 2, lists of 10
# and 100 texts counted rather than up to 100,000, blocks nested 3 deep rather than 200, and 4
# and 8 traced blocks rather than up to 256, and flat lists of 1,000 integers compared rather
# than 100,000 (#815), and texts of 4,301 and 17,200 digits rather than 100,000 and 1,000,000
# refused by the integer digit cap, whose timing runs only outside Miri, two mentions (#914),
# two tests of the timer thread (#1151): its exact firing at offsets 199,999 ns apart rather
# than 19,997, and a history of 500 cancelled timers rather than 10,000, one mention for both,
# and maps of 16 and 64 keys built rather than up to 65,536 (#886), one mention,
# and two fresh chains of 64 and 256 levels compared rather than up to 100,000 (#926), one
# mention. Their full sizes run in the ordinary tests. And one constant: the
# debug quarantine of freed slots is 64 rather than 65,536, so that the test of slot reuse runs
# under Miri and Miri checks the reuse.
# Since #1065 the three reductions in the tests of tasks.rs (the staged block, the start depths
# and the traced blocks) take no effect: those tests fork tasks onto coroutines, whose stacks and
# switch Miri cannot run, so they are ignored under Miri with the scheduler's own tests (#1145).
# The timing wheel and its five reduced tests (#1151, #1239) moved to una-http (#1006), which
# Miri does not run: una-http forbids `unsafe` through the workspace lints, so Miri would check
# only the standard library and the dependencies beneath it there (#1432).
reduced=$(grep -rn 'miri' crates/una-rt | grep -vc 'cfg_attr(miri, ignore')
[ "$reduced" = 19 ] || fail "una-rt mentions miri $reduced times outside its ignore attributes, and this script states 19"

asan=$(rustc +"$channel" --print sysroot)/lib/rustlib/$host/lib/librustc-nightly_rt.asan.dylib
[ -f "$asan" ] || fail "the checking toolchain has no AddressSanitizer runtime at $asan"
step "una-rt with AddressSanitizer" \
    env RUSTFLAGS="-Zsanitizer=address --cfg una_asan" cargo +"$channel" build -p una-rt \
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
asan_tests_binaries=$(env RUSTFLAGS="-Zsanitizer=address --cfg una_asan" "$linker_var=/usr/bin/cc" CC=/usr/bin/cc \
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
# `allocator_may_return_null=1` (#1700): the runtime takes its large allocations through the
# fallible `try_reserve` of the standard library and refuses the call with U6003 when the
# allocator returns null, as the native allocator does for a request it cannot meet. Outside a
# run no memory budget stops such a request first, and a unit test asks for 2^59 bytes there
# (`empty_bit_set(size: 2^62)`); AddressSanitizer's default is to abort on a request above its
# own maximum rather than return null, which would end the binary on a path the native runtime
# takes without fault. Memory errors are still reported and still abort.
step "The unit tests of una-rt with AddressSanitizer" \
    env ASAN_OPTIONS="abort_on_error=1:detect_leaks=0:allocator_may_return_null=1" "$asan_tests_binary"
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

# `una build` links through the system linker here (`UNA_LINKER=cc`, a development build's
# choice, ADR 0124 rule 19, `spec/16 §6` rule 7): only it can add the sanitizer's runtime, a
# library outside libSystem that the internal linker refuses.
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
    (cd "$canary" && env UNA_LINKER=cc UNA_RT_LIB="$runtime" PATH="$shim:$PATH" "$root/target/debug/una" build .)
}
step "The canary, linked through the shim" build_canary
ASAN_OPTIONS="verbosity=1" "$canary/target/una/canary" 2>&1 \
    | grep -q "AddressSanitizer: libc interceptors initialized" \
    || fail "AddressSanitizer is not active in an executable linked through the shim"
# And the runtime it links is the instrumented one: its code calls AddressSanitizer's checks.
nm -u "$canary/target/una/canary" | grep -q '___asan_report' \
    || fail "the runtime linked through the shim is not instrumented with AddressSanitizer"

step "The native equivalence test against it" \
    env "$linker_var=/usr/bin/cc" CC=/usr/bin/cc UNA_LINKER=cc UNA_RT_LIB="$runtime" PATH="$shim:$PATH" \
    ASAN_OPTIONS="abort_on_error=1:detect_leaks=0" \
    cargo test -p una-cli --test native -- interpreter:: the_generated_programs_are_the_runnable_programs_on_disk \
    the_native_lists_name_runnable_programs_only --test-threads=1 --nocapture

# 3. libFuzzer: the targets of fuzz/, a crate outside the workspace with its own Cargo.lock,
# built with the flags of scripts/fuzz-targets.sh and no other tool: libFuzzer's coverage of
# every edge and comparison, AddressSanitizer, debug assertions and overflow checks, and
# `--cfg una_fuzz`, which alone gives `una-rt` the two hooks the targets call. Each target runs a
# fixed number of inputs from its committed seeds (fuzz/seeds/<target>) with a fixed seed, so a
# run's size never depends on the machine's speed; the time limit is a ceiling only, and a run
# that reaches it before its count fails. The inputs the fuzzer adds stay under target/fuzz,
# never committed. LeakSanitizer does not run on macOS, so no leak is detected here. The
# targets, their counts and floors, and the steps below are those of scripts/fuzz-targets.sh,
# which scripts/fuzz-smoke.sh runs too, at smaller counts.
. "$root/scripts/fuzz-targets.sh"
fuzz_out=$root/target/fuzz
step "The fuzz targets, with libFuzzer and AddressSanitizer" fuzz_build
fuzz_bin=$fuzz_out/build/$host/release
FUZZ_CEILING=600

fuzz_check_listed
fuzz_check_instrumented
fuzz_canary
for entry in $FUZZ_TARGETS; do
    fuzz_target "${entry%%:*}" "${entry#*:}" 1
done

echo "sanitize: PASS ($channel, $host)"
