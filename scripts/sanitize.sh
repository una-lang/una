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
# `miri` in una-rt, any module and any form, but the ignore attributes checked above. Today five
# tests that run smaller under Miri, where each allocation is slow (#778): two deep values at
# depth 1,000 rather than 1,000,000, a copy between heaps at depth 1,000 rather than 200,000,
# 200 rounds of `find` rather than 10,000, and the entry points with a pool of one word of each
# kind and 20 drawn tuples. Their full sizes run in the ordinary tests.
reduced=$(grep -rn 'miri' crates/una-rt | grep -vc 'cfg_attr(miri, ignore')
[ "$reduced" = 5 ] || fail "una-rt mentions miri $reduced times outside its ignore attributes, and this script states 5"

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
# The test binary must call AddressSanitizer's checks, or this step proves nothing.
asan_tests_binary=$(env RUSTFLAGS="-Zsanitizer=address" "$linker_var=/usr/bin/cc" CC=/usr/bin/cc \
    cargo +"$channel" test -p una-rt --lib --target "$host" --target-dir target/sanitize \
    --no-run --message-format=json 2>/dev/null \
    | sed -n 's/.*"executable":"\([^"]*\)".*/\1/p' | tail -1)
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

echo "sanitize: PASS ($channel, $host)"
