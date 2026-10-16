# wordfreq

Counts how often each word of a line occurs, and prints the counts as one JSON object.

This is the finished program of the [getting started tutorial](../../docs/guide/getting-started.md), which builds it step by step. Read it after `hello`: it is still one module, but it has a contract that the solver proves, a map built in a loop, and JSON output. It is the smallest example of edition `0.2`, the edition that adds editable bindings and `for` loops.

What it demonstrates:

- a contract, [`requires`](../../docs/guide/language/functions/requires.md) and [`ensures`](../../docs/guide/language/functions/ensures.md), and how a missing precondition shows up as a counterexample;
- a `Map[Text, Int]` built with [`let edit`](../../docs/guide/language/control-flow/let-edit.md) and a [`for`](../../docs/guide/language/control-flow/for.md) loop;
- absence as a value: `Map.get` returns an `Option`, handled with [`if none use`](../../docs/guide/language/errors/if-error.md);
- [`example`](../../docs/guide/language/examples/example.md) blocks that `una test` runs.

## Run it

From this directory:

```sh
una check .   # copy examples/wordfreq
una test .
una run .
una build .
./target/una/wordfreq
```

`una run .` and the native executable print:

```text
{"and":2,"bat":1,"cat":1,"hat":1,"the":3}
```

The keys come out in ascending order because a `Map` keeps its keys sorted. `una test .` runs the two examples at the end of the module:

```text
example wordfreq "a word's first letter" ... ok
example wordfreq "each word is counted" ... ok

2 examples: 2 passed, 0 failed
```

## How it works

The program is one module, `src/wordfreq.una`, and reads as a pipeline: `main` splits a line into `words`, builds the `counts`, and prints the `report`.

### Splitting a line: `words`

```text
pub words(line: Text) -> List[Text]
=
  let space = " "
  line.split(on: space).filter(keep: fn(part) -> part.len > 0)
```

`split` has a precondition: the separator is not empty (`requires on.len > 0`). The solver proves it here, because `space` is a literal of length 1. `filter` takes a lambda, `fn(part) -> part.len > 0`, and drops the empty parts that two spaces in a row would leave.

### A contract: `initial`

```text
pub initial(word: Text) -> Text
  requires word.len >= 1
  ensures result.len == 1
=
  word.slice(start: 0, end: 1)
```

`main` does not call `initial`. It is in the package to show a contract that the solver proves:

- `requires word.len >= 1` is a promise that every caller must keep. The body may assume it.
- `ensures result.len == 1` is a promise that the function keeps. The solver proves it from the body and from what `slice` promises.
- `slice` has its own precondition: both positions lie inside the text. It holds because of the `requires`.

`una obligations .` lists every obligation of the package and its status. Each line says `(cached)` because the `una check .` above already proved it; on a fresh copy the listing has no `(cached)`:

```text
proven   ob-ee99f1a645a5 precondition  src/wordfreq.una:7:3 space.len > 0 (cached)
proven   ob-1ef23ba876c5 postcondition src/wordfreq.una:12:11 result.len == 1 (cached)
proven   ob-bdc97550ae15 precondition  src/wordfreq.una:14:3 0 <= 0 and 0 <= 1 and 1 <= word.len (cached)
proven   ob-68828c279e76 precondition  src/wordfreq.una:41:6 "una".len >= 1 (cached)
```

The last line is the example `do initial("una")`: an example is a call like any other, so it must meet the `requires` too.

### Counting: `counts`

```text
pub counts(words: List[Text]) -> Map[Text, Int]
=
  let edit counts: Map[Text, Int] = empty_map()
  for word in words
    let seen = counts.get(key: word)
      if none use 0
    edit counts.set(key: word, value: seen + 1)
  counts
```

Una has values, not references. `let edit` declares a binding that the function may change, and `edit counts.set(...)` gives `counts` a new map. A backend may update a value in place when nothing else holds it, so the loop need not copy the map on each word. The specification allows this and makes it unobservable: the program means the same either way (spec/17 §7, rule 3).

`counts.get(key: word)` returns an `Option[Int]`: a word seen for the first time has no entry. `if none use 0` handles that case on the line below the call, so `seen` is a plain `Int`. Una asks for one effect per line: each call that can be absent or fail is handled right under it.

### Printing JSON: `report` and `main`

`report` copies each count into a `Map[Text, Json]` with `Number(...)`, wraps the map in `Object(...)` and calls `to_json()`. `main` holds the only authority of the program, `uses @console: Console[print]`, and `una.toml` grants it.

## What `una check` proves

- `initial` returns one character whenever its argument has at least one.
- Every call meets the precondition of what it calls: the separator of `split` in `words`, the bounds of `slice` in `initial`, and `initial`'s own `requires` in its example.
- Everything else is checked by typing: the types of every expression, the `Option` that `get` returns, and the authority of `main`.

## Common mistakes

### A function that does not say what it needs

Delete the `requires word.len >= 1` line from `initial`. `una check .` cannot prove `slice`'s precondition any more, and shows a value that breaks it:

```text
[U5001] Error: precondition `0 <= 0 and 0 <= 1 and 1 <= word.len` of this call is refuted
    ╭─[ src/wordfreq.una:13:3 ]
    │
 13 │   word.slice(start: 0, end: 1)
    │   ──────────────┬─────────────
    │                 ╰─────────────── precondition `0 <= 0 and 0 <= 1 and 1 <= word.len` of this call is refuted
    │
    │ Help 1: repair 1: strengthen the type of the parameter `word` to a refined kind that implies `0 <= 0 and 0 <= 1 and 1 <= word.len`
    │
    │ Help 2: repair 2: add a guard (`if` or a `when` arm) that handles the values where `0 <= 0 and 0 <= 1 and 1 <= word.len` does not hold
    │
    │ Help 3: repair 3 (escape: checked; a runtime test replaces the static proof: the program fails at run time where the proof would have rejected it; it counts against [escape_budget] and is listed in the build report): check the value with `checked(value, 0 <= 0 and 0 <= 1 and 1 <= word.len) else fail <Variant>`
    │
    │ Note 1: obligation ob-bdc97550ae15 (precondition) is refuted: 0 <= 0 and 0 <= 1 and 1 <= word.len
    │
    │ Note 2: counterexample: word = Text#0, word.len = 0
    │
    │ Note 3: run `una explain U5001` for an explanation
────╯
```

Read the counterexample first: `word.len = 0`, an empty word. `Text#0` stands for some text that the solver does not spell out. The repairs are ranked, and they change only the function's own code. The fix that the example uses, a `requires`, is not among them. Repair 1, a refined parameter type, moves the duty to the callers in the same way, where the solver can check it. A `checked` (repair 3) would test it at run time instead, and costs part of the escape budget.

### A caller that breaks the contract

Put the `requires` back, and change the first example to `do initial("")`. The solver now refutes the call, and points at the clause it breaks:

```text
[U5001] Error: precondition `"".len >= 1` of this call is refuted
    ╭─[ src/wordfreq.una:41:6 ]
    │
 11 │   requires word.len >= 1
    │            ──────┬──────
    │                  ╰──────── requires clause
    │
 41 │   do initial("")
    │      ─────┬─────
    │           ╰─────── precondition `"".len >= 1` of this call is refuted
    │
    │ Note 1: obligation ob-b9d5c8950b3e (precondition) is refuted: "".len >= 1
    │
    │ Note 2: run `una explain U5001` for an explanation
────╯
```

### Using an `Option` as if it were a value

Delete the `if none use 0` line in `counts`. `seen` is then an `Option[Int]`, and the addition does not type:

```text
[U2001] Error: an operand of `+` must be `Int`, found `Option[Int]`
    ╭─[ src/wordfreq.una:22:39 ]
    │
 22 │     edit counts.set(key: word, value: seen + 1)
    │                                       ──┬─
    │                                         ╰─── an operand of `+` must be `Int`, found `Option[Int]`
    │
    │ Note: run `una explain U2001` for an explanation
────╯
```

> [!TIP]
> `una --json check .` prints the same diagnostics as JSON documents, with the obligation, the counterexample and the ranked repairs as fields. An editor or an agent can act on them without parsing text.

## Extend it

1. **Ignore case.** Make "The" and "the" the same word: call `to_lower` on the line before you split it, `words(to_lower(text))`, and change the text in `main` to mix cases. The output stays the same. `to_lower` exists from edition `0.2`, and it promises `result.len == t.len`.
2. **Keep the repeated words.** Add `pub repeated(counts: Map[Text, Int]) -> List[Text]`, which keeps the keys whose count is above 1. *Hint:* `counts.entries()` is a list of entries with `key` and `value`, so `filter` and `map` do the job in one line. Add an example: `do repeated(counts(words("a b a")))` and `expect result == ["a"]`.
3. **Count a file.** Read the line from a file instead of a literal. Declare `uses @files: Files[read]` on `main`, grant `"files:input:read"` in `una.toml`, and read `@files.read("line.txt")`. Paths are relative to the granted directory, so create `input/line.txt` with a line of words. The read can fail, so handle it on the line below, for example with `if error use ""`. Then run `una check .`: the text that `Files.read` returns is labeled `internal`, and `words` takes an unlabeled, that is `public`, `Text`, so the call is a U4001, a flow from `internal` to `public`. File contents do not reach the console by accident. The `journal` example shows how a package releases such data on purpose, through a policy.

## Learn more

- Construct pages: [`requires`](../../docs/guide/language/functions/requires.md), [`ensures`](../../docs/guide/language/functions/ensures.md), [`let edit`](../../docs/guide/language/control-flow/let-edit.md), [`for`](../../docs/guide/language/control-flow/for.md), [`if none use`](../../docs/guide/language/errors/if-error.md), [`example`](../../docs/guide/language/examples/example.md).
- Tutorial: [Getting started](../../docs/guide/getting-started.md#3-a-contract-and-a-proof-that-fails).
- Learn, the tutorial on the Una website: chapter 5, "Collections and the prelude"; chapter 7, "Contracts and the verifier".
- Specification: [clauses (spec/04 §2)](../../docs/spec/04-functions-and-contracts.md), [one effect per line (spec/04 §4)](../../docs/spec/04-functions-and-contracts.md), [counterexamples (spec/12 §7)](../../docs/spec/12-verification.md), [editable bindings (spec/17 §2)](../../docs/spec/17-memory-model.md), [backend freedom (spec/17 §7)](../../docs/spec/17-memory-model.md), [maps](../../docs/spec/21-maps.md), [JSON](../../docs/spec/23-json.md).

## Edition

Edition `0.2`, for `let edit` and `for`, which edition `0.1` does not have ([edition](../../docs/guide/configure/package/edition.md)).
