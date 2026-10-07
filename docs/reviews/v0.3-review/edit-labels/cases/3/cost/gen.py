"""Writes src/main.una for `shape` at size n (ADR 0075 rule 5: checking stays linear).
- cycle: one function with n editable lists that flow into each other in a cycle, one SCC.
- annotated: the cycle with annotated bindings, which have no label variables (a control).
- ring: n mutually recursive generic functions, each feeding its label variable from the next.
- maps: n generic functions, caller first, each storing the next one's solved map in its own.
- chain (rchain: declared callee first): n generic functions, each with an editable list fed by the next one's result.
"""
import sys
shape, n = sys.argv[1], int(sys.argv[2])
# With a third argument `leak`, the entry point declares a public result, which must be
# rejected (U4001) whatever n: the labels reach through every level.
leak = len(sys.argv) > 3
out = ["module main", ""]
if shape in ("cycle", "annotated"):
    out += ["pub f(x: internal Int) -> internal Int", "="]
    note = ": List[internal Int]" if shape == "annotated" else ""
    out += [f"  let edit b{i}{note} = []" for i in range(n)]
    out += ["  for k in 0..<2"]
    out += [f"    b{i} = b{(i + 1) % n}" for i in range(n)]
    out += ["    b0 = b0.append(item: x)", "  b0.len"]
elif shape == "ring":
    # n mutually recursive generic functions, each feeding its label variable from the next.
    for i in range(n):
        nxt = (i + 1) % n
        out += [f"r{i}[T](x: T, d: Int) -> List[T]", "="]
        out += ["  let edit acc = []", "  if d > 0", f"    for y in r{nxt}(x, d: d - 1)", "      acc = acc.append(item: y)"]
        out += ["  acc = acc.append(item: x)" if i == n - 1 else "  acc = acc", "  acc", ""]
    out += [f"pub g(x: internal Int) -> List[{'' if leak else 'internal '}Int] = r0(x, d: 3)"]
elif shape == "maps":
    # n generic functions declared caller first; each stores the next one's solved map in its own.
    for i in range(n):
        out += [f"m{i}[T: Ord](xs: List[T]) -> Map[Int, Map[T, Int]]", "=", "  let edit m = empty_map()"]
        if i + 1 < n:
            out += [f"  let inner = m{i + 1}(xs)", "  let first = inner.get(key: 0)", "    if none use empty_map()", "  edit m.set(key: 0, value: first)"]
        else:
            out += ["  let edit c = empty_map()", "  for x in xs", "    let k = c.get(key: x)", "      if none use 0", "    edit c.set(key: x, value: k + 1)", "  edit m.set(key: 0, value: c)"]
        out += ["  m", ""]
    out += ["pub g(xs: List[internal Text]) -> " + ("Int" if leak else "internal Int"), "=", "  let r = m0(xs)", "  let inner = r.get(key: 0)", "    if none use empty_map()", "  inner.len"]
else:
    for i in (range(n) if shape == "chain" else reversed(range(n))):
        out += [f"f{i}[T](x: T) -> List[T]", "="]
        out += ["  let edit acc = []"]
        if i + 1 < n:
            out += [f"  for y in f{i + 1}(x)", "    acc = acc.append(item: y)"]
        else:
            out += ["  acc = acc.append(item: x)"]
        out += ["  acc", ""]
    out += [f"pub g(x: internal Int) -> List[{'' if leak else 'internal '}Int] = f0(x)"]
open("src/main.una", "w").write("\n".join(out) + "\n")
