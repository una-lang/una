# journal

A private journal: it reads today's entry from a file and prints how long it is, never what it says.

## What it shows

- `una.toml` declares the [label category](../../docs/guide/configure/labels/categories.md) `Private`, names `journal.release` as a [trusted module](../../docs/guide/configure/trusted/modules.md), and [grants](../../docs/guide/configure/run/grants.md) the console, the environment, and reading and writing files under `entries/`.

`src/journal/entries.una`:

- An `Entry` whose `body` is `confidential[Private] Text`, an information-flow [label](../../docs/guide/language/information-flow/labels.md).
- A [capability](../../docs/guide/language/capabilities/capability.md), `Diary`, whose `keep` operation `accepts confidential[Private]`, so private text may be passed to it.
- `mood` reads the body, so its answer is labeled `confidential[Private]` as well.
- `write` [`fails`](../../docs/guide/language/errors/fails.md) `DiaryError` and passes the diary's failure on with [`if error pass`](../../docs/guide/language/errors/if-error.md).
- Two private [handlers](../../docs/guide/language/capabilities/handler.md), one that keeps every entry and one that [`fail`](../../docs/guide/language/errors/fail.md)s, used by three [`example`](../../docs/guide/language/examples/example.md) blocks.

`src/journal/release.una` is a [`trusted module`](../../docs/guide/language/information-flow/trusted.md) with one [`policy`](../../docs/guide/language/information-flow/policy.md), `length_band`: it releases a private text as a `public Nat`, its length in whole hundreds, through `hundreds`, with a stated purpose and owner.

`src/journal/main.una` reads `today.txt` with the host capability `Files`, prints the length band through the policy, and copies the entry to the file that the environment variable `JOURNAL_COPY` names, when it is set. A missing entry counts as empty: the comment in `main` explains why its failure is not passed on.

## What `una check` proves

- `write` returns an entry for the day it was given (`ensures result.day == day`).
- `hundreds` never returns a negative number.
- The entry's text reaches the console only as its length band, through `length_band`. Printing the text itself is an error (U4003), and so is printing `mood`'s answer, which depends on a private body. `una report .` lists each use of the policy with its purpose and owner.

## Run it

From this directory:

```sh
una check .   # copy examples/journal
una test .
una report .
una run .
una build .
./target/una/journal-example
```

`una run .` and the native executable print:

```text
today's entry runs to about 2 hundred characters
```

## Edition

Edition `0.1` ([edition](../../docs/guide/configure/package/edition.md)).
