# shortlinks

A link shortener: it stores short slugs that lead to longer links, and checks each slug once, where it enters the program.

This example is about failure and validation at a boundary. Text from users is checked once, by a parser that returns a refined type, and from then on the type carries the fact: the store's operations take a `Slug`, so they never see unchecked text. Each layer turns the failures of the layer below into its own error kind, and callers choose between handling a failure on the spot and keeping it as a value.

What it demonstrates:

- a refinement by an [`opaque` predicate](../../docs/guide/language/types/predicate.md), established at run time by [`checked`](../../docs/guide/language/errors/checked.md), under an [escape budget](../../docs/guide/configure/escape-budget/checked-per-module.md);
- error kinds per layer, with [`fails`](../../docs/guide/language/errors/fails.md), [`fail`](../../docs/guide/language/errors/fail.md) and the handling clauses [`if error` and `if none`](../../docs/guide/language/errors/if-error.md);
- failures as values with [`attempt`](../../docs/guide/language/errors/attempt.md) and `Result`;
- a [requirement](../../docs/guide/language/requirements/requirement.md) proposed by a tool, which a person must activate;
- fake stores as [handlers](../../docs/guide/language/capabilities/handler.md), and an [`example`](../../docs/guide/language/examples/example.md) for every failure path.

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

```text una="run ."
docs leads to https://example.org/docs
blog leads to /
```

`main` uses a store that holds one link, `docs`. The slug `blog` is valid but unknown, so `resolve_or_home` falls back to the home page, `/`. `una test .` runs six examples, one for each path through the module:

```text una="test ."
example shortlinks.links "a short link leads to its target" ... ok
example shortlinks.links "a malformed slug is rejected before the store is asked" ... ok
example shortlinks.links "an unknown slug fails" ... ok
example shortlinks.links "an attempt reports a failure as a value" ... ok
example shortlinks.links "without a store the home page is shown" ... ok
example shortlinks.links "a slug is stored" ... ok

6 examples: 6 passed, 0 failed
```

## How it works

`shortlinks.links` holds everything but `main`, which is in `shortlinks.main`.

### A slug is a checked text

```text file="src/shortlinks/links.una"
pub kind Slug = Text where valid_slug(it)

...
pub predicate opaque valid_slug(t: Text) -> Bool = t.len >= 3 and t.len <= 16 and not starts_with(t, prefix: "-")
```

A predicate comes in two forms. The solver sees the body of a `reflect` predicate, so it can prove the fact from other facts. It sees only the name of an `opaque` one, so a value can get the fact only from a test at run time, or from the `ensures` of a capability operation. This package uses the test. `valid_slug` is opaque because a slug comes from a user: no proof can show that a typed string is valid, so the program has to look at it.

### The only way in: `parse_slug`

```text file="src/shortlinks/links.una"
pub parse_slug(raw: Text) -> Slug
  fails SlugError
  derived from LINKS_1
=
  let text = trim(raw)
  checked(text, valid_slug(it)) else fail Malformed
```

`checked(text, valid_slug(it))` evaluates the predicate at run time. When it holds, the result is `text` with the type `Slug`. When it does not, `parse_slug` fails with `Malformed`. Since `checked` replaces a proof with a test, every use costs part of the module's escape budget, which `una.toml` sets to 1:

```text file="una.toml"
[escape_budget]
checked_per_module = 1
```

The default budget is 0, so a reviewer always sees in the manifest that a package tests facts at run time, and `una report .` lists each use: `shortlinks.links: 1 checked, budget 1`.

### One error kind per layer

```text file="src/shortlinks/links.una"
pub resolve(raw: Text) -> Text
  uses @links: Links[get]
  fails LinkError
=
  let slug = parse_slug(raw)
    if error fail BadSlug
  let target = @links.get(slug)
    if error fail StoreDown
    if none fail Unknown(slug: raw)
  target
```

`resolve` declares one error kind, `LinkError = BadSlug | Unknown(slug: Text) | StoreDown`. Each call that can fail is followed by its handling clauses, one effect per line:

- `if error fail BadSlug` turns the parser's `SlugError` into the caller's vocabulary;
- `if error fail StoreDown` does the same for the store's `StoreError`;
- `if none fail Unknown(slug: raw)` turns a missing link, an `Option`, into a failure that carries the slug.

A caller of `resolve` handles only `LinkError`. It never sees the error kinds of the store or of the parser.

### Handling now, or keeping the failure as a value

The module shows three ways to handle a failure of the store:

- `try_resolve` writes `attempt resolve(raw)`, which turns the outcome into a `Result[Text, LinkError]`: `Ok(target)` or `Err(failure)`. It does not fail itself, so it needs no `fails`.
- `resolve_or_home` matches on that `Result` with `when` and returns `/` for any `Err`.
- `lookup` recovers on the spot. `if error use None` treats a store that is down as a missing link, and `if none use "/"` then sends both cases to the home page.

Use a handling clause when the caller can decide at once, and `attempt` when it needs to keep the outcome, for example to report every line of a batch instead of stopping at the first bad one.

### A requirement that a tool proposed

```text file="src/shortlinks/links.una"
requirement LINKS_1
  text "A slug has 3 to 16 characters and never starts with a dash"
  source generated "task 17: short link rules"
  owner web_team
  status proposed
```

`source generated` marks a rule that a tool, such as a coding agent, wrote. Such a requirement must stay `proposed`. Functions derived from it compile as usual, and `una report .` flags them, `LINKS_1 (proposed): shortlinks.links.parse_slug`. Only a person activates it, by changing its source to `human`, `policy` or `regulation`.

## What `una check` proves

- Every value used as a `Slug` came from `parse_slug`: the store's operations, which take a `Slug`, never see unchecked text.
- The package spends exactly its escape budget: one `checked`, which `una report .` lists.
- Every failure is declared or handled: a failure leaves a function only when its `fails` declares that kind.

The proof that a slug is valid rests on one run-time test, in one place. That is the point of the design: to review the validation, you read `valid_slug` and `parse_slug`, and nothing else.

## Common mistakes

### Passing raw text where a checked value is needed

In `resolve`, call the store with the raw text, `@links.get(raw)`:

```text una="check ." after="mistakes/1"
[U5001] Error: refinement `valid_slug(raw)` is refuted
    ╭─[ src/shortlinks/links.una:54:27 ]
    │
 10 │ pub kind Slug = Text where valid_slug(it)
    │                            ───────┬──────
    │                                   ╰──────── where clause
    │
 54 │   let target = @links.get(raw)
    │                           ─┬─
    │                            ╰─── refinement `valid_slug(raw)` is refuted
    │
    │ Help 1: repair 1: strengthen the type of the parameter `raw` to a refined kind that implies `valid_slug(raw)`
    │
    │ Help 2: repair 2: add a guard (`if` or a `when` arm) that handles the values where `valid_slug(raw)` does not hold
    │
    │ Help 3: repair 3 (escape: checked; a runtime test replaces the static proof: the program fails at run time where the proof would have rejected it; it counts against [escape_budget] and is listed in the build report): check the value with `checked(value, valid_slug(raw)) else fail <Variant>`
    │
    │ Note 1: obligation ob-cf53981acc2f (refinement) is refuted: valid_slug(raw)
    │
    │ Note 2: counterexample: raw = Text#1, slug = Text#0
    │
    │ Note 3: run `una explain U5001` for an explanation
────╯
```

The fix is already in scope: pass `slug`, which `parse_slug` returned.

### A second run-time check instead of the parser

In `shorten`, replace the two lines that parse the slug, `let slug = parse_slug(raw)` and its `if error fail BadSlug`, with one inline check: `let slug = checked(raw, valid_slug(it)) else fail BadSlug`. The module now uses `checked` twice:

```text una="check ." after="mistakes/2"
[U7003] Error: module `shortlinks.links` uses `checked` 2 times, and `[escape_budget] checked_per_module` allows 1
    ╭─[ src/shortlinks/links.una:41:14 ]
    │
 41 │   let slug = checked(raw, valid_slug(it)) else fail BadSlug
    │              ───────────────────────┬──────────────────────
    │                                     ╰──────────────────────── module `shortlinks.links` uses `checked` 2 times, and `[escape_budget] checked_per_module` allows 1
    │
    │ Help 1: repair 1: state the fact in a refined type or a `requires` clause, so that the solver proves it, and remove the `checked`
    │
    │ Help 2: repair 2: handle the values where the fact does not hold with a guard (`if` or a `when` arm), and remove the `checked`
    │
    │ Help 3: repair 3 (escape: escape_budget; the module may use more runtime tests in place of proofs; each one is listed in the build report and needs review): raise `checked_per_module` in `[escape_budget]` of `una.toml`
    │
    │ Note: run `una explain U7003` for an explanation
────╯
```

The budget keeps validation in one place. Note also what the inline version loses: the `trim` that `parse_slug` applies.

### Activating a generated requirement

Change `status proposed` to `status active` in `LINKS_1`:

```text una="check ." after="mistakes/3"
[U1007] Error: requirement `LINKS_1` has source `generated`, so its status must be `proposed`
   ╭─[ src/shortlinks/links.una:7:10 ]
   │
 7 │   status active
   │          ───┬──
   │             ╰──── requirement `LINKS_1` has source `generated`, so its status must be `proposed`
   │
   │ Help: repair 1: mark it `proposed`; a person activates it by changing its source; edit src/shortlinks/links.una:7:10..7:16 to "proposed"
   │
   │ Note: run `una explain U1007` for an explanation
───╯
1 of 1 diagnostics have an edit that `una fix` applies (`una fix --dry-run` lists them)
```

## Extend it

1. **Delete a link.** Add an operation to `Links`, `op remove(slug: Slug) -> Unit` with `fails StoreError`. `una check .` reports U3011 for both handlers: a handler must implement every operation of its capability. Implement it in both, then add `pub unshorten(raw: Text) -> Unit`, which parses the slug and removes it, with an example that uses `expect performed @links.remove`.
2. **Test `lookup`.** `lookup` has no example yet. Add two: one with `one_link`, and one with `offline_links` that expects `/`. *Hint:* `lookup` takes a `Slug`, so the `do` line needs one from `parse_slug`. An example's `do` line may call a function that fails.
3. **Activate the rule.** Suppose the slug rules were reviewed and adopted. Change the source of `LINKS_1` to `human` and its status to `active`, and check that `una report .` lists it as `(active)`.

## Learn more

- Construct pages: [predicates](../../docs/guide/language/types/predicate.md), [`where`](../../docs/guide/language/types/where.md), [`checked`](../../docs/guide/language/errors/checked.md), [`fails`](../../docs/guide/language/errors/fails.md), [`fail`](../../docs/guide/language/errors/fail.md), [`if error`](../../docs/guide/language/errors/if-error.md), [`attempt`](../../docs/guide/language/errors/attempt.md), [`requirement`](../../docs/guide/language/requirements/requirement.md), [`handler`](../../docs/guide/language/capabilities/handler.md).
- Configuration: [`checked_per_module`](../../docs/guide/configure/escape-budget/checked-per-module.md).
- Learn, the tutorial on the Una website: "Results and failures" (`/learn/failure/`); "Contracts and the verifier" (`/learn/contracts/`).
- Specification: [predicates (spec/03 §4)](../../docs/spec/03-types.md), [handling clauses (spec/06 §3)](../../docs/spec/06-errors.md), [`attempt` (spec/06 §5)](../../docs/spec/06-errors.md), [`checked` (spec/06 §6)](../../docs/spec/06-errors.md), [requirements (spec/08 §1)](../../docs/spec/08-requirements-and-examples.md).

## Edition

Edition `0.1` ([edition](../../docs/guide/configure/package/edition.md)).
