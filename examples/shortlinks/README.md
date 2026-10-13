# shortlinks

A link shortener: it stores short slugs that lead to longer links, and checks each slug once, where it enters the program.

## What it shows

`src/shortlinks/links.una`:

- A [requirement](../../docs/guide/language/requirements/requirement.md), `LINKS_1`, with status `proposed` and a generated source, which `parse_slug` names with `derived from`.
- `Slug = Text where valid_slug(it)`, a [refinement](../../docs/guide/language/types/where.md) by an [`opaque` predicate](../../docs/guide/language/types/predicate.md): the solver does not see its body, so a `Slug` can only come from a runtime check.
- `parse_slug` is that check: [`checked`](../../docs/guide/language/errors/checked.md) tests `valid_slug` at run time and, when it fails, [`fail`](../../docs/guide/language/errors/fail.md)s with `Malformed`. A `checked` uses the module's [escape budget](../../docs/guide/configure/escape-budget/checked-per-module.md), which `una.toml` sets to 1.
- `shorten` and `resolve` turn each lower failure into a `LinkError` with [`if error fail`](../../docs/guide/language/errors/if-error.md), and `resolve` turns a missing link into `Unknown` with `if none fail`.
- `try_resolve` returns the failure as a value with [`attempt`](../../docs/guide/language/errors/attempt.md), and `resolve_or_home` matches on it with [`when`](../../docs/guide/language/control-flow/when.md).
- `lookup` recovers from both a failure and a missing link with `if error use` and `if none use`.
- A [capability](../../docs/guide/language/capabilities/capability.md), `Links`, two [handlers](../../docs/guide/language/capabilities/handler.md), and six [`example`](../../docs/guide/language/examples/example.md) blocks. One checks that a malformed slug never reaches the store (`expect not performed @links.get`).

`src/shortlinks/main.una` resolves a known and an unknown slug, with a store holding one link.

## What `una check` proves

- Every value used as a `Slug` came from `parse_slug`: the store's operations, which take a `Slug`, never see unchecked text.
- The package spends exactly its escape budget: one `checked`, which `una report .` lists.

## Run it

From this directory:

```sh
una check .   # copy examples/shortlinks
una test .
una report .
una run .
una build .
./target/una/shortlinks-example
```

`una run .` and the native executable print:

```text
docs leads to https://example.org/docs
blog leads to /
```

## Edition

Edition `0.1` ([edition](../../docs/guide/configure/package/edition.md)).
