# journal

A private journal: it reads today's entry from a file and prints how long it is, never what it says.

This is the example of information flow. Data carries a label, such as `confidential[Private]`, and the checker follows the label through every computation, branch and failure. Data reaches a place with a lower label only through a declared policy, in a trusted module, and the build report lists every such release with its purpose and owner. The journal prints one fact about the entry, its length band, and the checker rejects every other flow from the entry to the console.

What it demonstrates:

- [labels](../../docs/guide/language/information-flow/labels.md) and a [label category](../../docs/guide/configure/labels/categories.md), `Private`;
- a [`policy`](../../docs/guide/language/information-flow/policy.md) that releases one fact, in a [`trusted module`](../../docs/guide/language/information-flow/trusted.md);
- a [capability](../../docs/guide/language/capabilities/capability.md) whose operation `accepts` labeled data;
- the host capabilities `Files` and `Env`, with a file grant limited to one directory ([`grants`](../../docs/guide/configure/run/grants.md));
- why a failure can reveal data, and how the program handles it with [`if error use`](../../docs/guide/language/errors/if-error.md).

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

The entry, `entries/today.txt`, has a little over 200 characters, and the program shows only the whole hundreds. `una test .` runs the examples of `src/journal/entries.una`:

```text
example journal.entries "an entry is kept" ... ok
example journal.entries "a full diary reports it" ... ok
example journal.entries "a short entry is a calm day" ... ok

3 examples: 3 passed, 0 failed
```

Set `JOURNAL_COPY` to copy the entry as well: with `JOURNAL_COPY=copy.txt`, the run also writes `entries/copy.txt`. The grant confines writes to `entries/`, so a path such as `../outside.txt` is refused, and the program ignores the refusal.

## How it works

The package has three modules: `journal.entries`, the data and the diary; `journal.release`, the trusted module with the policy; and `journal.main`, the program.

### The manifest

```text
[labels]
categories = ["Private"]

[trusted]
modules = ["journal.release"]

[run]
entry = "journal.main"
grants = ["console", "env", "files:entries:read,write"]
```

- `[labels]` declares the categories that the package's labels may name.
- `[trusted]` lists the modules that may hold policies. A module is trusted only when the manifest lists it and its header says `trusted module`, so neither file alone can make it so.
- `files:entries:read,write` grants reading and writing under `entries/` and nowhere else. Paths in the code are relative to that directory: `@files.read("today.txt")` reads `entries/today.txt`.

### Labeled data

```text
pub kind Entry
  day: Int
  body: confidential[Private] Text

pub capability Diary
  op keep(entry: Entry) -> Unit
    fails DiaryError
    accepts confidential[Private]
```

The body of an entry is `confidential`, in the category `Private`. A capability operation states the highest label it accepts. `Console.print` accepts only `public` data, so an entry cannot be printed. `Diary.keep` accepts `confidential[Private]`, so entries can be kept.

Labels spread to everything computed from labeled data:

```text
pub mood(entry: Entry) -> confidential[Private] Mood = if entry.body.len > 280 then Busy else Calm
```

`mood` returns only `Calm` or `Busy`, but which one depends on the body, so the answer is as private as the body. The checker would reject `Mood` without the label: a branch on private data raises the label of what it chooses.

### Releasing one fact: the policy

```text
trusted module journal.release

pub policy length_band
  from confidential[Private] Text
  to public Nat
  via hundreds
  purpose "show how much was written without showing what"
  owner journal_owner

hundreds(text: confidential[Private] Text) -> public Nat = text.len / 100
```

A policy is the only way to lower a label. It names the types before and after, the private function that computes the release (`via`), why the release exists (`purpose`) and who answers for it (`owner`). Code calls the policy like a function, `length_band(text)`. It cannot call `hundreds` directly: the `via` function is private, and the policy is its only entry point. To review every release in the package, you read the trusted modules, which `una report .` lists with their size.

### The program, and a failure that would reveal

```text
  let text = @files.read("today.txt")
    if error use ""
  let hundreds = length_band(text)
  @console.print("today's entry runs to about {hundreds} hundred characters")
```

`Files.read` returns `internal Text`: the content of a file is never `public`. `length_band` accepts it, since `internal` is below `confidential[Private]`, and returns a `public Nat`, which may be printed.

The read can fail, and here a failure must not leave `main`. The program also writes a copy to the path that `JOURNAL_COPY` names, and that path is `internal` data. Where the package's writes land decides which files exist in this run and in later ones, so in this package every `Files` failure carries the label `internal`. Passing it on with `if error pass` would let an observer learn something about internal data from whether the program failed. `if error use ""` keeps the outcome inside the program: a missing entry counts as empty. The comment in `main` gives the same reason.

## What `una check` proves

- `write` returns an entry for the day it was given (`ensures result.day == day`).
- `hundreds` never returns a negative number.
- The entry's text reaches the console only as its length band, through `length_band`. Printing the text itself is an error (U4003), and so is printing `mood`'s answer, which depends on a private body.

The guarantee is noninterference: two runs whose entries differ only in their private text print the same thing, unless a policy released the difference. `una report .` shows each release, and what the guarantee rests on. Among its lines:

```text
information:
  journal.release.length_band at src/journal/main.una:18:18, purpose "show how much was written without showing what", owner journal_owner
```

and, in the trusted base:

```text
  trusted modules: journal.release (11 lines)
  host capabilities: Console, Env, Files
```

## Common mistakes

### Printing the private text

Change the `print` line of `main` to `@console.print("today: {text}")`:

```text
[U4003] Error: `@console.print` accepts `public` data, and this argument is `internal`
    ╭─[ src/journal/main.una:19:18 ]
    │
 19 │   @console.print("today: {text}")
    │                  ───────┬───────
    │                         ╰───────── `@console.print` accepts `public` data, and this argument is `internal`
    │
    │ Help 1: repair 1: send `@console.print` only data labeled `public` or lower, or send this value to an operation that accepts `internal`
    │
    │ Help 2: repair 2 (escape: trusted; the guarantee then rests on trusted code, whose labels are not checked: a policy releases the data, or a module joins the trusted base; it is listed in the build report and needs review): declassify the value through a policy of a trusted module
    │
    │ Note: run `una explain U4003` for an explanation
────╯
```

Interpolation keeps the label of what it interpolates, so the whole line is `internal`. Repair 2 is the design that this example uses: a policy that releases only what may be shown.

### Passing on a failure that depends on internal data

Change `if error use ""` after the read to `if error pass`:

```text
[U4002] Error: `if error pass` depends on `internal` data, so its failure would reveal it; return a labeled value instead of failing
    ╭─[ src/journal/main.una:17:5 ]
    │
 16 │   let text = @files.read("today.txt")
    │              ────────────┬───────────
    │                          ╰───────────── the labeled condition
 17 │     if error pass
    │     ──────┬──────
    │           ╰──────── `if error pass` depends on `internal` data, so its failure would reveal it; return a labeled value instead of failing
    │
    │ Note: run `una explain U4002` for an explanation
────╯
```

Whether a function fails is observable, like a printed line. A failure is allowed only where the decision to fail is `public`.

> [!IMPORTANT]
> Remove the copy to `JOURNAL_COPY` and the same `if error pass` checks: a package whose writes go to public paths keeps public file failures. Information flow depends on what the whole package does with files, not on the one line.

## Extend it

1. **Show the mood.** Import `Entry` and `mood` from `journal.entries` into `journal.main`. In `main`, compute `mood(Entry(day: 1, body: text))`, turn it into `"calm"` or `"busy"` with a `when`, and print that text. `una check .` reports U4003 with the label `confidential[Private]`: the text was chosen by a private value. Then add a second policy to `journal.release`, which needs `import journal.entries (Mood)`: `day_mood`, from `confidential[Private] Mood` to `public Mood`, through a private function that returns its argument, with its own purpose. Apply `day_mood` before the `when`. `una check .` is then clean, the program prints the mood, and `una report .` lists two releases.
2. **A diary on disk.** Add a handler that keeps entries in a file: `handler file_diary() for Diary`, with `uses @files: Files[write]` and `op keep(entry) = @files.write("kept.txt", content: entry.body)`. `una check .` rejects it twice. U3007: `write` can fail, and the operation must handle that. U4003: `Files.write` accepts `internal` data, and a handler of `keep` is checked for its worst caller, one that passes `confidential[Private]` data. So `Files` cannot hold `confidential[Private]` data: a store for private data needs a capability whose operation accepts that label, as `Diary.keep` does.
3. **A finer band.** Make `hundreds` divide by 10, and change the message in `main` to say `tens of characters`. The program prints `today's entry runs to about 24 tens of characters`. The release changes in one trusted function, which is where a reviewer looks for it.

## Learn more

- Construct pages: [labels](../../docs/guide/language/information-flow/labels.md), [`policy`](../../docs/guide/language/information-flow/policy.md), [`trusted module`](../../docs/guide/language/information-flow/trusted.md), [`capability`](../../docs/guide/language/capabilities/capability.md), [`handler`](../../docs/guide/language/capabilities/handler.md), [`fails`](../../docs/guide/language/errors/fails.md), [`if error`](../../docs/guide/language/errors/if-error.md), [`example`](../../docs/guide/language/examples/example.md).
- Configuration: [`categories`](../../docs/guide/configure/labels/categories.md), [trusted `modules`](../../docs/guide/configure/trusted/modules.md), [`grants`](../../docs/guide/configure/run/grants.md).
- Learn, the tutorial on the Una website: chapter 9, "Information flow".
- Specification: [the guarantee (spec/07 §1)](../../docs/spec/07-information-flow.md), [labels (spec/07 §2)](../../docs/spec/07-information-flow.md), [typing rules (spec/07 §3)](../../docs/spec/07-information-flow.md), [sinks (spec/07 §4)](../../docs/spec/07-information-flow.md), [policies (spec/07 §5)](../../docs/spec/07-information-flow.md), [trusted modules (spec/07 §6)](../../docs/spec/07-information-flow.md), [host capabilities (spec/14 §3)](../../docs/spec/14-standard-library.md).

## Edition

Edition `0.1` ([edition](../../docs/guide/configure/package/edition.md)). The record `Entry` would need another name in edition `0.2`, where `Entry` is the built-in type of map entries.
