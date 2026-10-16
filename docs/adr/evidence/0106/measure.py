"""ADR 0106 evidence: build the kernels three ways, then time them interleaved, per kernel.

    python3 -I -B measure.py BENCH_DIR UNA_BIN ROUNDS

Builds (timed once each, outside the run timing): `una build` of each kernel's Una package
(Cranelift), `clang -O2 -fwrapv -fno-strict-aliasing` of the C kernels, `rustc -O` of the Rust
kernels. Per kernel, every variant's output must be identical before any timing. Then one
warm-up round and ROUNDS rounds, each running every variant of the kernel once in a rotated
order; reports the median, min and max per variant and the ratio of medians to C.
"""
import json, os, statistics, subprocess, sys, time

bench, una, rounds = sys.argv[1], sys.argv[2], int(sys.argv[3])
out = os.path.join(bench, "out")
os.makedirs(out, exist_ok=True)


def timed(cmd, **kw):
    start = time.perf_counter()
    subprocess.run(cmd, check=True, **kw)
    return time.perf_counter() - start


builds = {}
for k in ("fib", "mix", "listsum"):
    builds[f"una-{k}"] = timed([una, "build", os.path.join(bench, f"una-{k}"), "--out", os.path.join(out, f"una-{k}")],
                              stdout=subprocess.DEVNULL)
builds["c"] = timed(["clang", "-O2", "-fwrapv", "-fno-strict-aliasing", "-o", os.path.join(out, "kernels-c"),
                     os.path.join(bench, "c", "kernels.c")])
builds["rust"] = timed(["rustc", "-O", "-o", os.path.join(out, "kernels-rs"), os.path.join(bench, "rs", "kernels.rs")])

kernels = {
    "K1 fib(32)": {"una (cranelift)": [os.path.join(out, "una-fib")],
                   "c (clang -O2 -fwrapv)": [os.path.join(out, "kernels-c"), "fib"],
                   "rust (rustc -O)": [os.path.join(out, "kernels-rs"), "fib"]},
    "K2 mix(5e7)": {"una (cranelift)": [os.path.join(out, "una-mix")],
                    "c (clang -O2 -fwrapv)": [os.path.join(out, "kernels-c"), "mix"],
                    "rust (rustc -O)": [os.path.join(out, "kernels-rs"), "mix"]},
    "K3 listsum(5e6)": {"una (cranelift)": [os.path.join(out, "una-listsum")]},
}

result = {"rounds": rounds, "build_seconds": builds, "kernels": {}}
for kernel, variants in kernels.items():
    outputs = {n: subprocess.run(cmd, capture_output=True, text=True, check=True).stdout.strip() for n, cmd in variants.items()}
    assert len(set(outputs.values())) == 1, (kernel, outputs)
    times = {n: [] for n in variants}
    names = list(variants)
    for r in range(rounds + 1):
        order = names[r % len(names):] + names[:r % len(names)]
        for n in order:
            t = timed(variants[n], stdout=subprocess.DEVNULL)
            if r > 0:
                times[n].append(t)
    stats = {n: {"median": statistics.median(v), "min": min(v), "max": max(v)} for n, v in times.items()}
    base = stats.get("c (clang -O2 -fwrapv)", {}).get("median")
    if base:
        for n in stats:
            stats[n]["vs_c"] = stats[n]["median"] / base
    result["kernels"][kernel] = {"output": next(iter(outputs.values())), "seconds": stats}
print(json.dumps(result, indent=2))
