"""Interleaved benchmark for ADR 0107's evidence: for each kernel, its variants are checked
to print the same output first, then run in a discarded warm-up round and N interleaved
rounds, timed with the monotonic clock. Prints medians, minima and maxima.
Usage: python3 -I bench.py <rounds> <kernel>:<label>=<exe>,<label>=<exe>... ..."""
import statistics, subprocess, sys, time

def run(exe):
    start = time.perf_counter()
    out = subprocess.run([exe], capture_output=True, text=True)
    elapsed = time.perf_counter() - start
    if out.returncode != 0:
        sys.exit(f"{exe} exited {out.returncode}: {out.stderr}")
    return elapsed, out.stdout.strip()

rounds = int(sys.argv[1])
for spec in sys.argv[2:]:
    kernel, variants = spec.split(":", 1)
    pairs = [v.split("=", 1) for v in variants.split(",")]
    outs = {label: run(exe)[1] for label, exe in pairs}
    if len(set(outs.values())) != 1:
        sys.exit(f"{kernel}: outputs differ: {outs}")
    times = {label: [] for label, _ in pairs}
    for _ in range(rounds):
        for label, exe in pairs:
            times[label].append(run(exe)[0])
    base = statistics.median(times[pairs[0][0]])
    print(f"{kernel} (output {next(iter(outs.values()))!r}, {rounds} rounds):")
    for label, _ in pairs:
        ts = times[label]
        m = statistics.median(ts)
        print(f"  {label:10} median {m:.4f} s  min {min(ts):.4f}  max {max(ts):.4f}  {base / m:.1f}x vs {pairs[0][0]}")
