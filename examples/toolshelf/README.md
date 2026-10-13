# toolshelf

A community tool shelf: members check tools out for a number of days and bring them back, under two house rules.

## What it shows

`src/toolshelf/tools.una`:

- Two [requirements](../../docs/guide/language/requirements/requirement.md): `SHELF_1`, at most three tools per member, and `SHELF_2`, a checkout of one to twenty-eight days. `check_out` and `bring_back` name them with `derived from`.
- The rules as types: `KeepDays = Int in 1..28` and a member's `holding: Int in 0..3` ([`where` and refinements](../../docs/guide/language/types/where.md)).
- Two [predicates](../../docs/guide/language/types/predicate.md), `can_check_out` and `overdue`, declared `reflect`, so the solver sees their bodies.
- `check_out` [`requires`](../../docs/guide/language/functions/requires.md) `can_check_out(member)` and [`ensures`](../../docs/guide/language/functions/ensures.md) the due day and the member's new count. Its body turns the shelf's failure and a missing tool into its own error with [`if error fail` and `if none fail`](../../docs/guide/language/errors/if-error.md).
- `late_days` states with two `ensures` that a tool is late exactly when it is `overdue`.
- A [capability](../../docs/guide/language/capabilities/capability.md), `Shelf`, with two operations that can fail, and two [handlers](../../docs/guide/language/capabilities/handler.md): a shelf holding a list of tools, and one that is offline.
- Five [`example`](../../docs/guide/language/examples/example.md) blocks, including failures (`expect fails NotOnShelf`).

`src/toolshelf/desk.una`:

- `check_out_anywhere` tries the main shelf and, when that fails, the backup shelf. It catches the first failure as a value with [`attempt`](../../docs/guide/language/errors/attempt.md), and binds each shelf to `check_out`'s `@shelf` with [`with`](../../docs/guide/language/capabilities/with.md).
- Its examples check which shelf was used (`expect not performed @backup_shelf.pick`).

`src/toolshelf/main.una` checks a ladder out today, from the host capability `Clock`, and prints how late it would be after 20 days.

## What `una check` proves

- Every `ensures` of `check_out`, `bring_back`, `late_days` and `check_out_anywhere`.
- A member never holds more than three tools: each call to `check_out` meets `can_check_out`, and the new count stays in `0..3`.
- Every checkout lasts from 1 to 28 days at every call.

`una report .` lists the two requirements with the functions derived from them.

## Run it

From this directory:

```sh
una check .   # copy examples/toolshelf
una test .
una report .
una run .
una build .
./target/una/toolshelf-example
```

`una run .` and the native executable print:

```text
Ana took the ladder for 14 days
returned 20 days later, it is 6 days late
```

## Edition

Edition `0.1` ([edition](../../docs/guide/configure/package/edition.md)).
