"""Extra cases for the #312 recheck (N-42): function values, closures, fold, `together for`."""

H = """module m

pub show(xs: List[Int]) -> Text = xs.fold(initial: "", step: fn(line, x) -> if line == "" then "{x}" else "{line} {x}")

pushed(acc: List[Int], i: Int) -> List[Int] = acc.append(item: i)

"""
MAIN = "pub main() -> Unit\n  uses @console: Console[print]\n=\n"
CASES = []


def case(name, body, expected):
    CASES.append((name, H + body, expected))


case("fv-called-twice-same-list", """twice(f: fn(List[Int], Int) -> List[Int], xs: List[Int]) -> Text
=
  let a = f(xs, 1)
  let b = f(xs, 2)
  "{show(a)} / {show(b)} / {show(xs)}"

""" + MAIN + """  let base = [0]
  @console.print(twice(pushed, xs: base))
  let g = fn(ys: List[Int], i: Int) -> ys.append(item: i)
  @console.print(twice(g, xs: base))
  @console.print(show(base))
""", "0 1 / 0 2 / 0\n0 1 / 0 2 / 0\n0\n")

case("fv-same-list-two-args", """both(f: fn(List[Int], List[Int]) -> List[Int], xs: List[Int]) -> List[Int] = f(xs, xs)

""" + MAIN + """  let base = [1, 2]
  let joined = both(fn(a: List[Int], b: List[Int]) -> a.append(item: b.len), xs: base)
  @console.print("{show(joined)} / {show(base)}")
""", "1 2 2 / 1 2\n")

case("closure-captures-list-it-is-passed", MAIN + """  let xs = [1, 2]
  let f = fn(ys: List[Int]) -> ys.append(item: xs.len).len * 10 + xs.len
  let a = f(xs)
  let b = f(xs)
  let edit grown = xs
  grown = grown.append(item: 3)
  let c = f(grown)
  @console.print("{a} {b} {c} {show(xs)} {show(grown)}")
""", "32 32 42 1 2 1 2 3\n")

case("closure-captures-and-returns", MAIN + """  let xs = [5]
  let keep = fn(ys: List[Int]) -> ys.append(item: xs.at(index: 0).or_else(default: 0))
  let a = keep(xs)
  let b = keep(a)
  @console.print("{show(a)} / {show(b)} / {show(xs)}")
""", "5 5 / 5 5 5 / 5\n")

case("fold-initial-is-folded-list", MAIN + """  let xs = [1, 2, 3]
  let out = xs.fold(initial: xs, step: fn(acc, x) -> acc.append(item: x * 10))
  let again = xs.fold(initial: xs, step: fn(acc, x) -> acc.append(item: acc.len))
  @console.print("{show(out)} / {show(again)} / {show(xs)}")
""", "1 2 3 10 20 30 / 1 2 3 3 4 5 / 1 2 3\n")

case("fold-step-captures-state-alias", MAIN + """  let seed = [9]
  let out = [1, 2].fold(initial: seed, step: fn(acc, x) -> acc.append(item: seed.len + x))
  @console.print("{show(out)} / {show(seed)}")
""", "9 2 3 / 9\n")

case("map-filter-items-retained", MAIN + """  let rows = [[1], [2, 2], [3, 3, 3]]
  let grown = rows.map(each: fn(r) -> r.append(item: 0))
  let kept = rows.filter(keep: fn(r) -> r.append(item: 0).len > 2)
  let first = rows.first_where(keep: fn(r) -> r.append(item: 9).len == 3)
  let lens = rows.map(each: fn(r) -> r.len)
  let glens = grown.map(each: fn(r) -> r.len)
  @console.print("{show(lens)} / {show(glens)} / {kept.len} / {first.or_else(default: []).len}")
""", "1 2 3 / 2 3 4 / 2 / 2\n")

case("together-for-task-appends", MAIN + """  let base = [1]
  let outs = together for i in 0..<3
    let edit acc = base
    acc = pushed(acc, i: i)
    acc = acc.append(item: i * 10)
    acc
  let lens = outs.map(each: fn(o) -> o.len)
  let lasts = outs.map(each: fn(o) -> o.at(index: 2).or_else(default: -1))
  let edit t = "s"
  let ts = together for i in 0..<2
    let edit u = t
    u = "{u}{i}"
    u = "{u}!"
    u.len
  @console.print("{show(lens)} / {show(lasts)} / {show(base)} / {show(ts)} {t}")
""", "3 3 3 / 0 10 20 / 1 / 3 3 s\n")

case("together-for-items-to-lambda", MAIN + """  let rows = [[1], [2]]
  let outs = together for r in rows
    let edit x = r
    x = x.append(item: 7)
    x.len
  @console.print("{show(outs)} / {rows.at(index: 0).or_else(default: []).len}")
""", "2 2 / 1\n")

case("fv-stored-in-record", """pub kind Op
  apply: fn(List[Int], Int) -> List[Int]

""" + MAIN + """  let oper = Op(apply: pushed)
  let base = [4]
  let a = oper.apply(base, 5)
  let b = oper.apply(a, 6)
  @console.print("{show(a)} / {show(b)} / {show(base)}")
""", "4 5 / 4 5 6 / 4\n")
