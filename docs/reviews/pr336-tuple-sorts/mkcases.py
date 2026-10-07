"""Writes the adversarial packages for the #336 check under cases/."""
import shutil
from pathlib import Path
D = Path(__file__).resolve().parent / "cases"
LOCK = (D.parent.parent / "una-336/verification.lock").read_text()
def pkg(name, files):
    d = D / name
    if d.exists(): shutil.rmtree(d)
    (d / "src").mkdir(parents=True)
    (d / "una.toml").write_text(f'[package]\nname = "c"\nversion = "0.1.0"\nedition = "0.2"\n')
    (d / "verification.lock").write_text(LOCK)
    for rel, text in files.items():
        p = d / "src" / rel; p.parent.mkdir(parents=True, exist_ok=True); p.write_text(text)

pkg("order-and-arity", {"m.una": """module m

pub swap_order(n: Int) -> Int
  requires n >= 0
  ensures result >= 0
=
  let edit a: Nat = 0
  let edit t = ""
  for i in 0..<n
    a = a + 1
    t = "{t}x"
  let edit u = ""
  let edit b: Nat = 0
  for i in 0..<n
    u = "{u}y"
    b = b + 2
  a + b

pub arity(n: Int) -> Int
  requires n >= 0
  ensures result >= 0
=
  let edit x: Nat = 0
  let edit y: Nat = 0
  for i in 0..<n
    x = x + 1
    y = y + 1
  let edit p: Nat = 0
  let edit q: Nat = 0
  let edit r: Nat = 0
  for i in 0..<n
    p = p + 1
    q = q + 1
    r = r + 1
  x + y + p + q + r

pub wrong(n: Int) -> Int
  requires n >= 0
  ensures result >= 1
=
  let edit x: Nat = 0
  let edit t = ""
  for i in 0..<n
    x = x + 1
    t = "{t}z"
  x
"""})

pkg("nested-loops", {"m.una": """module m

pub nested(n: Int) -> Int
  requires n >= 0
  ensures result >= 0
=
  let edit a: Nat = 0
  let edit b: Nat = 0
  for i in 0..<n
    let edit c: Nat = 0
    for j in 0..<n
      c = c + 1
      a = a + 1
    b = b + c
  a + b
"""})

pkg("refined-labeled", {"m.una": """module m

pub kind Pos = Int where it >= 1

pub refined(n: Int) -> Int
  requires n >= 0
  ensures result >= 1
=
  let edit p: Pos = 1
  let edit k: Nat = 0
  for i in 0..<n
    p = p + 1
    k = k + 1
  p

pub plain(n: Int) -> Int
  requires n >= 0
  ensures result >= 0
=
  let edit p = 1
  let edit k: Nat = 0
  for i in 0..<n
    p = p + 1
    k = k + 1
  k

pub labeled(n: Int, s: secret Int) -> secret Int
  requires n >= 0
=
  let edit acc: secret Int = s
  let edit k: Nat = 0
  for i in 0..<n
    acc = acc + 1
    k = k + 1
  acc
"""})

loopk = """pub kind Tuple2
  a: Nat

pub kind Int8
  v: Nat

pub mix(n: Int) -> Int
  requires n >= 0
  ensures result >= 0
=
  let edit x = Tuple2(a: 0)
  let edit z = Int8(v: 0)
  let edit k: Nat = 0
  for i in 0..<n
    x = Tuple2(a: x.a + 1)
    z = Int8(v: z.v + 1)
    k = k + 1
  x.a + z.v
"""
pkg("kind-names", {"a_b.una": "module a_b\n\n" + loopk, "a/b.una": "module a.b\n\n" + loopk, "m.una": "module m\n\nimport a_b (mix)\n\npub both(n: Int) -> Int\n  requires n >= 0\n=\n  mix(n)\n"})

pkg("generic-twice", {"m.una": """module m

pub kind Box[T]
  item: T

pub repeat[T](x: T, n: Int) -> List[T]
  requires n >= 0
  ensures result.len >= 0
=
  let edit out: List[T] = []
  let edit k: Nat = 0
  for i in 0..<n
    out = out.append(item: x)
    k = k + 1
  out

pub boxes(n: Int) -> Int
  requires n >= 0
  ensures result >= 0
=
  let edit bi = Box(item: 0)
  let edit bt = Box(item: "s")
  let edit xs: List[Int] = []
  let edit o: Option[Text] = None
  for i in 0..<n
    bi = Box(item: bi.item + 1)
    bt = Box(item: "{bt.item}s")
    xs = xs.append(item: i)
    o = Some(bt.item)
  let a = repeat(1, n: n)
  let b = repeat("x", n: n)
  a.len + b.len + xs.len
"""})

wide = ["  let edit v%d: Nat = 0" % i for i in range(60)]
body = "\n".join(wide) + "\n  for i in 0..<n\n" + "\n".join("    v%d = v%d + 1" % (i, i) for i in range(60)) + "\n  " + " + ".join("v%d" % i for i in range(60)) + "\n"
pkg("wide-60", {"m.una": "module m\n\npub wide(n: Int) -> Int\n  requires n >= 0\n  ensures result >= 0\n=\n" + body})

pkg("two-edit-call-in-loop", {"m.una": """module m

pub step(edit a: Nat, edit t: Text) -> Unit
=
  a = a + 1
  t = "{t}q"

pub run(n: Int) -> Int
  requires n >= 0
  ensures result >= 0
=
  let edit a: Nat = 0
  let edit t = ""
  let edit k: Nat = 0
  for i in 0..<n
    step(edit a, edit t)
    k = k + 1
  a + k
"""})
