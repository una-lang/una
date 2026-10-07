"""Compares `parse_json` and `to_json` with reference.py on a handcrafted corpus and on
generated inputs, in both engines (#308, areas 2 and 4).

Usage: python3 compare.py <una binary> [seed] [count]
"""

import os
import random
import subprocess
import sys

import reference

HERE = os.path.dirname(os.path.abspath(__file__))
PROBE = os.path.join(HERE, "probe")
SEP = "\n=====\n"

HANDCRAFTED = [
    "null", "true", "false", " \t\r\n null \n", "nul", "nulll", "tru", "", " ", "﻿null",
    "0", "-0", "01", "-01", "1.", "1.5", "-1.5e10", "1e", "1e+", "1E-0", "0e0", "+1", "-", "--1",
    ".5", "1.0", "1.00", "123456789012345678901234567890", "1" * 4300, "1" * 4301, "-" + "1" * 4301,
    "[" + "1" * 4301 + "]", "1" * 5000 + ".5", "NaN", "Infinity", "-Infinity",
    '""', '"a"', '"\\""', '"\\\\"', '"\\/"', '"\\b\\f\\n\\r\\t"', '"\\u0041"', '"\\u00e9"', '"\\u00E9"',
    '"\\ud83d\\ude00"', '"\\uD83D\\uDE00"', '"\\uD800"', '"\\uDC00"', '"\\uD800\\u0041"', '"\\uD800x"',
    '"\\uD800\\"', '"\\uD800\\uD800"', '"\\uDBFF\\uDFFF"', '"\\u"', '"\\u12"', '"\\u12G4"', '"\\x"',
    '"\\a"', '"a\tb"', '"a\nb"', '"\x1f"', '"\x7f"', '" "', '"‪"', '"⁦"', '"⁩"',
    '"⁪"', '"é😀"', "'a'", '"abc', '"\\', '"\\u00',
    "[]", "[ ]", "[1,]", "[,1]", "[1 2]", "[1,,2]", "[", "]", "[[]", "[[[]]]",
    "{}", "{ }", '{"a":1}', '{"a" : 1 , "b" : 2}', '{"b":1,"a":2}', '{"a":1,"a":2}',
    '{"a":1,"\\u0061":2}', '{"a":1,"a":', '{"a":1,"a":}', '{"a":1,"a"', '{"a"}', '{"a":}', '{a:1}',
    '{"a":1,}', '{,"a":1}', '{"a":1 "b":2}', '{"":1}', '{"":1,"":2}', '{"é":1,"e":2,"z":3,"😀":4}',
    '{"\\u0000":1}', '{"a":{"a":{"a":1}}}', '{"a":[1,{"b":null}]}',
    "[" * 512 + "]" * 512, "[" * 513 + "]" * 513, "[" * 600, '{"a":' * 512 + "1" + "}" * 512,
    '{"a":' * 513 + "1" + "}" * 513, "[" * 512 + "x", "[" * 513 + "x",
    "1 2", "null null", "[] []", "{}x", "1 ", " 1", "true\x00",
]


def gen_value(rng, depth):
    r = rng.random()
    if depth > 6 or r < 0.35:
        choice = rng.randrange(7)
        if choice == 0:
            return None
        if choice == 1:
            return rng.random() < 0.5
        if choice == 2:
            return ("num", rng.randrange(-10**rng.randrange(1, 40), 10**rng.randrange(1, 40)))
        if choice == 3:
            return ("dec", rng.choice(["1.5", "-0.0", "1e5", "2E-3", "0.000", "1.0e+10", "-3.14"]))
        return ("str", gen_text(rng))
    if r < 0.65:
        return ("arr", [gen_value(rng, depth + 1) for _ in range(rng.randrange(5))])
    return ("obj", {gen_text(rng): gen_value(rng, depth + 1) for _ in range(rng.randrange(5))})


ALPHABET = list("abcAZ09 \"\\/\b\f\n\r\t\x00\x01\x1f\x7f") + ["é", "😀", " ", " ", "‫", "⁧", "中", "﻿", "\u0085"]


def gen_text(rng):
    return "".join(rng.choice(ALPHABET) for _ in range(rng.randrange(6)))


def render(rng, v):
    """A JSON text for v, with arbitrary whitespace, escapes and key order."""
    ws = lambda: rng.choice(["", "", " ", "\n", "\t ", "\r\n"])
    if v is None or v is True or v is False:
        return reference.write(v)
    kind, x = v
    if kind in ("num", "dec"):
        return x if kind == "dec" else str(x)
    if kind == "str":
        return render_str(rng, x)
    if kind == "arr":
        return "[" + ws() + ",".join(ws() + render(rng, i) + ws() for i in x) + "]"
    keys = list(x)
    rng.shuffle(keys)
    return "{" + ws() + ",".join(ws() + render_str(rng, k) + ws() + ":" + ws() + render(rng, x[k]) + ws() for k in keys) + "}"


def render_str(rng, s):
    out = ['"']
    for ch in s:
        o = ord(ch)
        if ch == '"' or ch == "\\" or o < 0x20 or rng.random() < 0.2:
            if o > 0xFFFF:
                o -= 0x10000
                hi, lo = 0xD800 + (o >> 10), 0xDC00 + (o & 0x3FF)
                out.append("\\u%04X\\u%04x" % (hi, lo))
            elif ch == "/" and rng.random() < 0.5:
                out.append("\\/")
            else:
                out.append(rng.choice(["\\u%04x", "\\u%04X"]) % o)
        else:
            out.append(ch)
    out.append('"')
    return "".join(out)


def mutate(rng, t):
    if not t:
        return t
    i = rng.randrange(len(t))
    op = rng.randrange(4)
    if op == 0:
        return t[:i] + t[i + 1:]
    if op == 1:
        return t[:i] + rng.choice(list('[]{}",:\\u0aeE.-+ \x01')) + t[i:]
    if op == 2:
        return t[:i]
    return t[:i] + rng.choice(list('[]{}",:\\u0aeE.-+')) + t[i + 1:]


def corpus(seed, count):
    rng = random.Random(seed)
    inputs = list(HANDCRAFTED)
    for _ in range(count):
        t = render(rng, gen_value(rng, 0))
        inputs.append(t)
        inputs.append(mutate(rng, t))
    return [t for t in inputs if SEP not in t]


def run(una, inputs, native):
    io = os.path.join(PROBE, "io")
    os.makedirs(io, exist_ok=True)
    with open(os.path.join(io, "in.txt"), "w", encoding="utf-8", newline="") as f:
        f.write(SEP.join(inputs))
    out = os.path.join(io, "out.txt")
    if os.path.exists(out):
        os.remove(out)
    if native:
        subprocess.run([una, "build", "."], cwd=PROBE, check=True, capture_output=True)
        r = subprocess.run(["./target/una/probe"], cwd=PROBE, capture_output=True, text=True)
    else:
        r = subprocess.run([una, "run", "."], cwd=PROBE, capture_output=True, text=True)
    if r.returncode != 0:
        print(r.stdout, r.stderr)
        sys.exit("the probe failed")
    with open(out, encoding="utf-8", newline="") as f:
        return f.read().split("\n")


def main():
    una = os.path.abspath(sys.argv[1])
    seed = int(sys.argv[2]) if len(sys.argv) > 2 else 1
    count = int(sys.argv[3]) if len(sys.argv) > 3 else 300
    inputs = corpus(seed, count)
    interp = run(una, inputs, native=False)
    native = run(una, inputs, native=True)
    assert len(interp) == len(inputs) == len(native), (len(interp), len(inputs), len(native))
    differences = 0
    for t, a, b in zip(inputs, interp, native):
        expected = reference.outcome(t)
        got, fixed = a.rsplit(" | ", 1)
        problems = []
        if got != expected:
            problems.append(f"una: {got[:200]!r}\n      reference: {expected[:200]!r}")
        if expected.startswith("ok") and fixed != "fixed":
            problems.append(f"round trip: {fixed}")
        if a != b:
            problems.append(f"native: {b[:200]!r}")
        if problems:
            differences += 1
            print(f"input {t[:120]!r} (length {len(t)})")
            for p in problems:
                print("   ", p)
    print(f"{len(inputs)} inputs, {differences} with differences")


if __name__ == "__main__":
    main()
