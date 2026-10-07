#!/bin/sh
# Writes src/main.una for `shape` at size n (delta pass on bc05a8d, #320, N-42).
shape=$1; n=$2
case $shape in
lambda) body="  let m = run_with(fn(acc, i) -> acc.with(key: i, value: i * 2), n: $n)
  @console.print(\"{m.len}\")";;
named) body="  let m = run_with(add, n: $n)
  @console.print(\"{m.len}\")";;
entries) body="  let m = run_with(add, n: $n)
  let inverted = m.entries().fold(initial: empty_map(), step: fn(acc, e) -> acc.with(key: e.value, value: e.key))
  @console.print(\"{inverted.len}\")";;
counts) body="  let xs = numbers($n)
  let counts = xs.fold(initial: empty_map(), step: fn(m, x) -> bump(m, k: x mod 1000))
  @console.print(\"{counts.len}\")";;
text) body="  let xs = numbers($n)
  let t = xs.fold(initial: \"\", step: fn(acc, x) -> \"{acc}é\")
  @console.print(\"{t.len}\")";;
esac
cat > src/main.una <<UNA
module main

add(m: Map[Int, Int], i: Int) -> Map[Int, Int] = m.with(key: i, value: i * 2)

bump(m: Map[Int, Int], k: Int) -> Map[Int, Int]
=
  let n = m.get(key: k)
    if none use 0
  m.with(key: k, value: n + 1)

run_with(f: fn(Map[Int, Int], Int) -> Map[Int, Int], n: Int) -> Map[Int, Int]
=
  let edit acc: Map[Int, Int] = empty_map()
  for i in 0..<n
    acc = f(acc, i)
  acc

numbers(n: Int) -> List[Int]
=
  let edit xs: List[Int] = []
  for i in 0..<n
    xs = xs.append(item: i)
  xs

pub main() -> Unit
  uses @console: Console[print]
=
$body
UNA
