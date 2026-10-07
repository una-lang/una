"""Borrow-inference edge cases, aimed at reference counting (debug runtime) and semantics."""

H = """module m

pub kind Limit = | Full

pub kind Box
  xs: List[Int]
  t: Text

pub show(xs: List[Int]) -> Text = xs.fold(initial: "", step: fn(line, x) -> if line == "" then "{x}" else "{line} {x}")

"""
MAIN = "pub main() -> Unit\n  uses @console: Console[print]\n=\n"
CASES = []


def case(name, body, expected):
    CASES.append((name, H + body, expected))


case("owned-stored-in-record-and-returned-twice", """boxed(acc: List[Int], i: Int) -> Box
=
  let g = acc.append(item: i)
  Box(xs: g, t: show(g))

pair(acc: List[Int]) -> List[Box]
=
  let a = boxed(acc, i: 1)
  let b = boxed(acc, i: 2)
  [a, b]

""" + MAIN + """  let base = [0]
  let bs = pair(base)
  let ts = bs.map(each: fn(b) -> b.t)
  let dash = "-"
  @console.print("{ts.len} {show(base)} {ts.at(index: 0).or_else(default: dash)}")
""", "2 0 0 1\n")

case("owned-in-when-scrutinee", """pick(acc: List[Int], o: Option[Int]) -> List[Int]
=
  when o
    is Some(v) -> acc.append(item: v)
    is None -> acc

""" + MAIN + """  let edit a = [1]
  a = pick(a, o: Some(2))
  a = pick(a, o: None)
  let b = a
  a = pick(a, o: Some(3))
  @console.print("{show(a)} / {show(b)}")
""", "1 2 3 / 1 2\n")

case("owned-failure-after-store", """stash(acc: List[Int], i: Int) -> Box
  fails Limit
=
  let g = acc.append(item: i)
  let box = Box(xs: g, t: "x")
  if i > 5
    fail Full
  box

""" + MAIN + """  let a = [1, 2]
  let r = attempt stash(a, i: 9)
  let s = stash(a, i: 3)
    if error use Box(xs: [], t: "none")
  @console.print("{r is Ok} {show(a)} {show(s.xs)}")
""", "false 1 2 1 2 3\n")

case("handler-passes-to-owned", """pub capability Sink
  op put(x: Int) -> List[Int]

grown(acc: List[Int], i: Int) -> List[Int] = acc.append(item: i)

handler from_seed(seed: List[Int]) for Sink
  op put(x) = grown(seed, i: x)

use_sink() -> Int
  uses @sink: Sink[put]
=
  let a = @sink.put(1)
  let b = @sink.put(2)
  a.len * 10 + b.len

""" + MAIN + """  let seed = [5, 6]
  let n = use_sink() with @sink = from_seed(seed)
  @console.print("{n} {show(seed)}")
""", "33 5 6\n")

case("owned-param-captured-then-moved", """later(acc: List[Int], i: Int) -> Int
=
  let f = fn(k: Int) -> acc.len + k
  let moved = grown(acc, i: i)
  f(0) * 100 + moved.len

grown(acc: List[Int], i: Int) -> List[Int] = acc.append(item: i)

""" + MAIN + """  @console.print("{later([1, 2, 3], i: 4)}")
""", "304\n")

case("recursion-via-function-value", """step(acc: List[Int], n: Int, f: fn(List[Int], Int) -> List[Int]) -> List[Int]
=
  if n == 0
    acc
  else
    step(f(acc, n), n: n - 1, f: f)

grown(acc: List[Int], i: Int) -> List[Int] = acc.append(item: i)

""" + MAIN + """  let base = [0]
  let out = step(base, n: 3, f: grown)
  @console.print("{show(out)} / {show(base)}")
""", "0 3 2 1 / 0\n")

case("generic-owned-in-tasks", """grow[T](xs: List[T], x: T) -> List[T] = xs.append(item: x)

""" + MAIN + """  let seed = [Box(xs: [1], t: "a")]
  let outs = together for i in 0..<3
    let edit acc = seed
    acc = grow(acc, x: Box(xs: [i], t: "b"))
    acc = grow(acc, x: seed.at(index: 0).or_else(default: Box(xs: [], t: "")))
    acc.len
  @console.print("{show(outs)} {seed.len}")
""", "3 3 3 1\n")

case("text-owned-mutual-recursion", """ping(t: Text, n: Int) -> Text = if n == 0 then t else pong("{t}i", n: n - 1)

pong(t: Text, n: Int) -> Text = if n == 0 then t else ping("{t}o", n: n - 1)

""" + MAIN + """  let s = "<"
  let a = ping(s, n: 5)
  let b = ping(a, n: 2)
  @console.print("{a} {b} {s}")
""", "<ioioi <ioioiio <\n")

case("edit-and-owned-same-binding-twice", """mix(edit a: List[Int], b: List[Int]) -> Unit
=
  a = a.append(item: b.len)
  a = a.append(item: b.len)

""" + MAIN + """  let edit x = [1]
  let y = x
  mix(edit x, b: y)
  @console.print("{show(x)} / {show(y)}")
""", "1 1 1 / 1\n")

case("deep-recursion-owned-text", """build(t: Text, n: Int) -> Text = if n == 0 then t else build("{t}x", n: n - 1)

""" + MAIN + """  let s = build("", n: 5000)
  @console.print("{s.len}")
""", "5000\n")
