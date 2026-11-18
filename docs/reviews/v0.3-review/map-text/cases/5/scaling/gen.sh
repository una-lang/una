#!/bin/sh
# Writes src/main.una for `shape` at size n: `search` finds a…ab in a…a (two-way worst case),
# `slices` takes slice(i, i + 1) of every position of a non-ASCII text, `map` sets and reads n keys.
shape=$1; n=$2
case $shape in
search) body="  let edit hay = \"\"
  for i in 0..<$n
    hay = \"{hay}a\"
  let edit needle = \"\"
  for i in 0..<($n / 2)
    needle = \"{needle}a\"
  needle = \"{needle}b\"
  let found = hay.find(text: needle)
  let count = if needle.len > 0 then hay.split(on: needle).len else 0
  @console.print(\"{found} {count}\")";;
slices) body="  let edit t = \"\"
  for i in 0..<$n
    t = \"{t}é\"
  let edit count = 0
  for i in 0..<t.len
    if i + 1 <= t.len
      let one = t.slice(start: i, end: i + 1)
      count = count + one.len
  @console.print(\"{count}\")";;
map) body="  let edit m: Map[Int, Int] = empty_map()
  for i in 0..<$n
    edit m.set(key: (i * 7919) mod $n, value: i)
  let edit total = 0
  for i in 0..<$n
    let v = m.get(key: i)
      if none use 0
    total = total + v
  @console.print(\"{m.len} {total}\")";;
esac
printf 'module main\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n%s\n' "$body" > src/main.una
