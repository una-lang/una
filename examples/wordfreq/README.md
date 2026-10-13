# wordfreq

Counts how often each word of a line occurs, and prints the counts as one JSON object. It is the finished program of the [getting started tutorial](../../docs/guide/getting-started.md), which builds it step by step.

## What it shows

`src/wordfreq.una`, one module:

- `words` splits a line on spaces and keeps the non-empty parts, with a lambda passed to `filter`.
- `initial` has a contract: [`requires`](../../docs/guide/language/functions/requires.md) `word.len >= 1` and [`ensures`](../../docs/guide/language/functions/ensures.md) `result.len == 1`.
- `counts` builds a `Map[Text, Int]` with [`let edit`](../../docs/guide/language/control-flow/let-edit.md) and a [`for`](../../docs/guide/language/control-flow/for.md) loop, both of edition `0.2`. A missing key counts as 0, with [`if none use`](../../docs/guide/language/errors/if-error.md).
- `report` turns the map into a `Json` object and prints it with `to_json`.
- `main` may only print ([`uses`](../../docs/guide/language/capabilities/uses.md) `@console: Console[print]`).
- Two [`example`](../../docs/guide/language/examples/example.md) blocks, which `una test` runs.

## What `una check` proves

- `initial` returns one character whenever its argument has at least one.
- Every call meets the precondition of what it calls: the bounds of `slice` in `initial`, the non-empty separator of `split` in `words`, and `initial`'s own `requires` in its example.

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

## Edition

Edition `0.2`, for `let edit` and `for`, which edition `0.1` does not have ([edition](../../docs/guide/configure/package/edition.md)).
