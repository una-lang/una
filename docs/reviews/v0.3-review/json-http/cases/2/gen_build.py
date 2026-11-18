"""Writes build/src/build.una: JSON built from Map and Text over a corpus of texts (#308, area 2),
and checks its output against reference.py. Usage: python3 gen_build.py <una binary>"""
import json, os, random, subprocess, sys
import reference

HERE = os.path.dirname(os.path.abspath(__file__))
rng = random.Random(3)
ALPHABET = list("ab,: \"\\/") + ["\n", "\t", "\b", "\f", "\r", "\x00", "\x1f", "\x7f", "é", "😀", " ", "‮", "⁨", "中", "﻿", "{", "}"]
texts = ["", "a", "a,b,,c", "é😀", "  ", "key: value, other: thing"] + [
    "".join(rng.choice(ALPHABET) for _ in range(rng.randrange(1, 12))) for _ in range(60)
]

def lit(t):
    return '"' + "".join("\\u{%x}" % ord(c) for c in t) + '"'

src = """module build

--- How often each character occurs, as a JSON object of counts.
counts(t: Text) -> Json
=
  let edit m: Map[Text, Json] = empty_map()
  for c in t.chars()
    let seen = m.get(key: c)
      if none use Number(value: 0)
    let n = when seen
      is Number(value) -> value
      else -> 0
    edit m.set(key: c, value: Number(value: n + 1))
  Object(fields: m)

--- The parts of a text between commas, as an array, and the text as a key of an object.
shapes(t: Text) -> Json
=
  let parts = t.split(on: ",").map(each: fn(p) -> Str(value: p))
  let m: Map[Text, Json] = empty_map().with(key: t, value: Array(items: parts))
  Object(fields: m.with(key: t.replace(pattern: ",", by: ";"), value: Str(value: t.quote_json())))

line(t: Text) -> Text
=
  let q = if t.quote_json() == Str(value: t).to_json() then "same" else "DIFFERENT"
  let c = counts(t)
  let s = shapes(t)
  let back = attempt parse_json(s.to_json())
  let trip = when back
    is Ok(j) -> if j == s then "trip" else "NO TRIP"
    is Err -> "NO PARSE"
  "{q} {trip} {c.to_json()} {s.to_json()}"

pub main() -> Unit
  uses @console: Console[print]
=
"""
for t in texts:
    src += f"  @console.print(line({lit(t)}))\n"
open(os.path.join(HERE, "build", "src", "build.una"), "w").write(src)

una = os.path.abspath(sys.argv[1])
outs = []
for native in (False, True):
    if native:
        subprocess.run([una, "build", "."], cwd=os.path.join(HERE, "build"), check=True, capture_output=True)
        r = subprocess.run(["./target/una/build"], cwd=os.path.join(HERE, "build"), capture_output=True, text=True)
    else:
        r = subprocess.run([una, "run", "."], cwd=os.path.join(HERE, "build"), capture_output=True, text=True)
    if r.returncode:
        sys.exit(r.stdout + r.stderr)
    outs.append(r.stdout.split("\n")[:-1])
bad = 0
for t, a, b in zip(texts, *outs):
    counts = {}
    for c in t:
        counts[c] = counts.get(c, 0) + 1
    c = reference.write(("obj", {k: ("num", v) for k, v in counts.items()}))
    parts = ("arr", [("str", p) for p in t.split(",")])
    s = {t: parts}
    s[t.replace(",", ";")] = ("str", reference.quote(t))
    s = reference.write(("obj", s))
    expected = f"same trip {c} {s}"
    if a != expected or a != b:
        bad += 1
        print(repr(t)); print("  una   ", a[:300]); print("  ref   ", expected[:300]); print("  native", b[:300])
print(f"{len(texts)} texts, {bad} with differences")
