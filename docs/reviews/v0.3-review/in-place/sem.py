"""Semantic, memory-safety and parity cases: (name, source, expected stdout)."""

H = """module m

pub kind Limit = | Full

pub show(xs: List[Int]) -> Text = xs.fold(initial: "", step: fn(line, x) -> if line == "" then "{x}" else "{line} {x}")

"""
MAIN = "pub main() -> Unit\n  uses @console: Console[print]\n=\n"
CASES = []


def case(name, body, expected):
    CASES.append((name, H + body, expected))


# 1. function values: a function with an owned parameter, called through a value
case("fv-owned-through-value", """pushed(acc: List[Int], i: Int) -> List[Int] = acc.append(item: i)

apply_n(f: fn(List[Int], Int) -> List[Int], start: List[Int], n: Int) -> List[Int]
=
  let edit acc = start
  for i in 0..<n
    acc = f(acc, i)
  acc

""" + MAIN + """  let base = [9]
  let out = apply_n(pushed, start: base, n: 3)
  let again = apply_n(pushed, start: base, n: 2)
  @console.print("{show(out)} / {show(again)} / {show(base)}")
""", "9 0 1 2 / 9 0 1 / 9\n")

case("fv-lambda-wrapping-owned", """pushed(acc: List[Int], i: Int) -> List[Int] = acc.append(item: i)

""" + MAIN + """  let base = [1, 2]
  let f = fn(xs: List[Int]) -> pushed(xs, i: 7)
  let a = f(base)
  let b = f(base)
  let c = f(a)
  @console.print("{show(a)} / {show(b)} / {show(c)} / {show(base)}")
""", "1 2 7 / 1 2 7 / 1 2 7 7 / 1 2\n")

# 2. generics with descriptors
case("generic-owned-record-and-nested", """pub kind Row
  name: Text
  tags: List[Text]

grow[T](xs: List[T], x: T) -> List[T] = xs.append(item: x)

grow_twice[T](xs: List[T], x: T) -> List[T] = grow(grow(xs, x: x), x: x)

""" + MAIN + """  let r = Row(name: "a", tags: ["t"])
  let rows = grow_twice([r], x: Row(name: "b", tags: []))
  let inner = [1]
  let nested = grow([inner], x: inner)
  let nested2 = grow(nested, x: grow(inner, x: 5))
  let texts = grow_twice(["x"], x: "y")
  @console.print("{rows.len} {rows.at(index: 2).or_else(default: r).name} {r.tags.len} / {nested2.len} {show(inner)} / {texts.len}")
""", "3 b 1 / 3 1 / 3\n")

# 3. mutual recursion; one branch reads the argument after the call
case("mutual-recursion-owned", """ev(acc: List[Int], n: Int) -> List[Int] = if n == 0 then acc else od(acc.append(item: n), n: n - 1)

od(acc: List[Int], n: Int) -> List[Int]
=
  if n == 0
    acc
  else
    let r = ev(acc, n: n - 1)
    r.append(item: acc.len)

""" + MAIN + """  let start = [0]
  let out = ev(start, n: 4)
  @console.print("{show(out)} / {show(start)}")
""", None)

case("recursion-reads-after-call", """build(acc: List[Int], n: Int) -> List[Int]
=
  if n == 0
    acc
  else
    let deeper = build(acc.append(item: n), n: n - 1)
    deeper.append(item: acc.len)

""" + MAIN + """  let start = [100]
  @console.print("{show(build(start, n: 3))} / {show(start)}")
""", "100 3 2 1 3 2 1 / 100\n")

# 4. failures 1 to 3 levels deep, original read on the failure path
case("fail-three-levels-attempt", """lvl3(acc: List[Int], i: Int) -> List[Int]
  fails Limit
=
  let g = acc.append(item: i)
  if g.len > 3
    fail Full
  g

lvl2(acc: List[Int], i: Int) -> List[Int]
  fails Limit
=
  let g = acc.append(item: i)
  lvl3(g, i: i + 1)
    if error pass

lvl1(acc: List[Int], i: Int) -> List[Int]
  fails Limit
=
  lvl2(acc.append(item: i), i: i + 1)
    if error pass

""" + MAIN + """  let a = [1]
  let r = attempt lvl1(a, i: 2)
  let ok = r is Ok
  let b = [1, 2, 3]
  let s = attempt lvl1(b, i: 4)
  @console.print("{ok} {show(a)} / {s is Ok} {show(b)}")
""", "false 1 / false 1 2 3\n")

case("fail-read-in-handler", """bounded(acc: List[Int], i: Int) -> List[Int]
  fails Limit
=
  let g = acc.append(item: i)
  if g.len > 2
    fail Full
  g

via(acc: List[Int], i: Int) -> List[Int]
  fails Limit
=
  bounded(acc, i: i)
    if error pass

""" + MAIN + """  let a = [1, 2]
  let r = via(a, i: 3)
    if error use a
  let c = [1]
  let s = via(c, i: 3)
    if error use c
  @console.print("{show(r)} {show(a)} / {show(s)} {show(c)}")
""", "1 2 1 2 / 1 3 1\n")

case("fail-edit-three-levels", """p3(edit acc: List[Int], i: Int) -> Unit
  fails Limit
=
  acc = acc.append(item: i)
  if acc.len > 2
    fail Full

p2(edit acc: List[Int], i: Int) -> Unit
  fails Limit
=
  acc = acc.append(item: 0)
  p3(edit acc, i: i)
    if error pass

""" + MAIN + """  let edit a: List[Int] = [5]
  p2(edit a, i: 6)
    if error use unit
  let edit b: List[Int] = []
  p2(edit b, i: 6)
    if error use unit
  @console.print("{show(a)} / {show(b)}")
""", "5 / 0 6\n")

# 5. closures and handlers inside callees
case("closure-captures-owned-param", """keep(acc: List[Int], i: Int) -> Int
=
  let g = fn(k: Int) -> acc.len + k
  let grown = acc.append(item: i)
  g(0) * 10 + grown.len

""" + MAIN + """  @console.print("{keep([1, 2], i: 3)}")
""", "23\n")

case("handler-seed-not-mutated", """pub capability Sink
  op put(x: Int) -> Int

handler counting(seed: List[Int]) for Sink
  op put(x) = seed.append(item: x).len

three() -> Int
  uses @sink: Sink[put]
=
  let a = @sink.put(1)
  let b = @sink.put(2)
  let c = @sink.put(3)
  a * 100 + b * 10 + c

""" + MAIN + """  let seed = [7, 7]
  let n = three() with @sink = counting(seed)
  @console.print("{n} {show(seed)}")
""", "333 7 7\n")

# 6. aliases: Option, record rebuild, map, list of lists, loop state
case("alias-option-and-rebuild", """pub kind R
  xs: List[Int]
  n: Int

""" + MAIN + """  let edit a = [1]
  let o = Some(a)
  a = a.append(item: 2)
  let edit r = R(xs: [1], n: 0)
  let snapshot = r
  r = R(xs: r.xs.append(item: 9), n: r.n + 1)
  let held = o.or_else(default: [])
  @console.print("{show(held)} {show(a)} / {show(snapshot.xs)} {snapshot.n} {show(r.xs)} {r.n}")
""", "1 1 2 / 1 0 1 9 1\n")

case("alias-map-values", MAIN + """  let edit m: Map[Text, List[Int]] = empty_map()
  edit m.set(key: "a", value: [1])
  let before = m
  let edit got = m.get(key: "a").or_else(default: [])
  got = got.append(item: 2)
  edit m.set(key: "a", value: got)
  edit m.set(key: "b", value: got)
  got = got.append(item: 3)
  let a1 = before.get(key: "a").or_else(default: [])
  let a2 = m.get(key: "a").or_else(default: [])
  let b2 = m.get(key: "b").or_else(default: [])
  @console.print("{show(a1)} / {show(a2)} / {show(b2)} / {show(got)} / {before.len} {m.len}")
""", "1 / 1 2 / 1 2 / 1 2 3 / 1 2\n")

case("alias-list-of-lists", MAIN + """  let edit inner = [1]
  let edit outer: List[List[Int]] = []
  outer = outer.append(item: inner)
  inner = inner.append(item: 2)
  outer = outer.append(item: inner)
  inner = inner.append(item: 3)
  let lens = outer.map(each: fn(xs) -> xs.len)
  @console.print("{show(lens)} {show(inner)}")
""", "1 2 1 2 3\n")

case("loop-state-from-alias", """grow_to(edit acc: List[Int], n: Int) -> Unit
=
  for i in 0..<n
    acc = acc.append(item: i)

""" + MAIN + """  let base = [9]
  let edit work = base
  grow_to(edit work, n: 3)
  let edit work2 = base
  grow_to(edit work2, n: 1)
  @console.print("{show(base)} / {show(work)} / {show(work2)}")
""", "9 / 9 0 1 2 / 9 0\n")

case("loop-visits-list-it-grows", MAIN + """  let edit xs = [1, 2]
  let edit visits = 0
  for x in xs
    xs = xs.append(item: x * 10)
    visits = visits + 1
  @console.print("{visits} {show(xs)}")
""", "2 1 2 10 20\n")

# 7. texts
case("text-alias-and-self", """glue(edit out: Text, part: Text) -> Unit
=
  out = "{out}{part}"

""" + MAIN + """  let edit t = "ab"
  let u = t
  t = "{t}{u}{t}"
  let edit w = "x"
  let w0 = w
  glue(edit w, part: w0)
  glue(edit w, part: w0)
  let edit v = "q"
  let list = [v, v]
  v = "{v}r"
  let q = "?"
  let first = list.at(index: 0).or_else(default: q)
  @console.print("{t} {u} / {w} {w0} / {v} {first}")
""", "ababab ab / xxx x / qr q\n")

case("text-through-three-levels", """t1(edit out: Text, i: Int) -> Unit
=
  out = "{out}{i}"

t2(edit out: Text, i: Int) -> Unit
=
  t1(edit out, i: i)
  out = "{out},"

t3(out: Text, i: Int) -> Text
=
  let edit o = out
  t2(edit o, i: i)
  o

""" + MAIN + """  let start = "<"
  let edit s = start
  for i in 0..<3
    s = t3(s, i: i)
  @console.print("{s} {start}")
""", "<0,1,2, <\n")

# 8. together: captured values passed to owned parameters
case("together-captured-owned", """pushed(acc: List[Int], i: Int) -> List[Int] = acc.append(item: i)

""" + MAIN + """  let base = [1]
  together
    let a = pushed(base, i: 2)
    let b = pushed(base, i: 3)
  let ys = together for i in 0..<3
    let edit y = base
    y = y.append(item: i)
    y.len
  let edit t = "s"
  let ts = together for i in 0..<2
    let edit u = t
    u = "{u}{i}"
    u
  t = "{t}!"
  @console.print("{show(a)} / {show(b)} / {show(ys)} / {show(base)} / {ts.len} {t}")
""", "1 2 / 1 3 / 2 2 2 / 1 / 2 s!\n")
