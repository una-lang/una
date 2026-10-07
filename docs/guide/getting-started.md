# Getting started with Una

This tutorial takes you from installing Una to a verified program built as a native executable. You write `wordfreq`, which counts the words of a text and prints the counts as JSON. On the way you meet a contract the solver proves, a proof that fails with a counterexample, examples that run as tests, capabilities and their grants, and the native backend.

Every code block of this tutorial is checked by the test suite (`crates/una-cli/tests/guide.rs`), and the finished program is `examples/wordfreq`.

## 1. Install

The installer puts a release in `~/.una`: the `una` binary, the native runtime that `una build` links, and Z3 at the version the proofs need.

```sh
curl -fsSL https://raw.githubusercontent.com/una-lang/una/main/install.sh | sh
export PATH="$HOME/.una/bin:$PATH"
```

It installs only an archive whose digest the release's validation record lists. `una build` also needs a C compiler as `cc`, to link. The README describes the other ways to install, and how to uninstall.

Check that it works by asking `una` to explain a diagnostic code:

```sh
una explain U5001
```

## 2. A package

A package is a directory with `una.toml`, `verification.lock` and the sources under `src/`. Make a directory `wordfreq` with the three files below:
- `una.toml` names the package, picks edition `0.2` and says how `una run` starts it.
- `verification.lock` pins the solver, so that a proof means the same everywhere.
- The module `wordfreq` lives in `src/wordfreq.una`. Start with the words of a line, and a `main` that prints how many there are.

`una.toml`
```toml
[package]
name = "wordfreq"
version = "0.1.0"
edition = "0.2"

[run]
entry = "wordfreq"
grants = ["console"]
```

`verification.lock`
```toml
encoding_version = "una-smt-1"
solver           = "z3"
solver_version   = "5.1.0"
logic            = "QF_UFLIA"
step_budget      = 5000000
random_seed      = 0
```

`src/wordfreq.una`
```una
module wordfreq

--- The words of a line: the parts between spaces, without the empty ones.
pub words(line: Text) -> List[Text]
=
  let space = " "
  line.split(on: space).filter(keep: fn(part) -> part.len > 0)

pub main() -> Unit
  uses @console: Console[print]
=
  let text = "the cat and the hat and the bat"
  @console.print("{words(text).len} words")
```

- A function is an equation: its header, then `=`, then its body.
- `split` and `filter` are prelude functions, called with method syntax.
- `main` says which capabilities it uses: `@console`, with the operation `print`. A line without `@` touches nothing outside the program.

In the `wordfreq` directory, check it and run it:

```sh
una check .   # copy examples/wordfreq
una run .
```

```text
8 words
```

`una check` runs every check, the proofs included, and prints nothing when all pass. Its source must also be in canonical form; `una fmt .` rewrites it so.

## 3. A contract, and a proof that fails

Add a function that takes the first letter of a word. A first try:

`una.toml`
```toml
[package]
name = "wordfreq"
version = "0.1.0"
edition = "0.2"
```

`src/wordfreq.una`
```una
-- fails with U5001
module wordfreq

--- The first letter of a word.
pub initial(word: Text) -> Text
  ensures result.len == 1
=
  word.slice(start: 0, end: 1)
```

`una check` proves contracts with the solver, and here it refutes one:

```text
[U5001] Error: precondition `0 <= 0 and 0 <= 1 and 1 <= word.len` of this call is refuted
    ╭─[ src/wordfreq.una:7:3 ]
    │
  7 │   word.slice(start: 0, end: 1)
    │   ──────────────┬─────────────
    │                 ╰─────────────── precondition `0 <= 0 and 0 <= 1 and 1 <= word.len` of this call is refuted
    │
    │ Note 2: counterexample: word = Text#0, word.len = 0
```

`slice` requires its positions to lie in the text, and the counterexample shows a word where they do not: an empty one. The diagnostic also proposes repairs; with `una --json check`, it is a JSON document an agent can act on.

The fix is to say what `initial` needs, with `requires`. Every caller then has to prove it, and the body may assume it:

`una.toml`
```toml
[package]
name = "wordfreq"
version = "0.1.0"
edition = "0.2"
```

`src/wordfreq.una`
```una
module wordfreq

--- The first letter of a word.
pub initial(word: Text) -> Text
  requires word.len >= 1
  ensures result.len == 1
=
  word.slice(start: 0, end: 1)

example "a word's first letter"
  do initial("una")
  expect result == "u"
```

Now `una check` proves both the precondition of `slice` and the `ensures` of `initial`. The `example` is a test: `una test` runs it, and `una check` proves that `"una"` meets the `requires`.

## 4. Maps and JSON

Count the words with a `Map`, which keeps its keys in order, and write the counts as JSON. This is the whole program:

`una.toml`
```toml
[package]
name = "wordfreq"
version = "0.1.0"
edition = "0.2"

[run]
entry = "wordfreq"
grants = ["console"]
```

`src/wordfreq.una`
```una
module wordfreq

--- The words of a line: the parts between spaces, without the empty ones.
pub words(line: Text) -> List[Text]
=
  let space = " "
  line.split(on: space).filter(keep: fn(part) -> part.len > 0)

--- The first letter of a word.
pub initial(word: Text) -> Text
  requires word.len >= 1
  ensures result.len == 1
=
  word.slice(start: 0, end: 1)

--- How often each word occurs, in ascending order of the words.
pub counts(words: List[Text]) -> Map[Text, Int]
=
  let edit counts: Map[Text, Int] = empty_map()
  for word in words
    let seen = counts.get(key: word)
      if none use 0
    edit counts.set(key: word, value: seen + 1)
  counts

--- The counts as one JSON object.
pub report(counts: Map[Text, Int]) -> Text
=
  let edit fields: Map[Text, Json] = empty_map()
  for entry in counts.entries()
    edit fields.set(key: entry.key, value: Number(entry.value))
  Object(fields).to_json()

pub main() -> Unit
  uses @console: Console[print]
=
  let text = "the cat and the hat and the bat"
  @console.print(report(counts(words(text))))

example "a word's first letter"
  do initial("una")
  expect result == "u"

example "each word is counted"
  do counts(words("a b a"))
  expect result == empty_map().with(key: "a", value: 2).with(key: "b", value: 1)
```

- **Values, not references.** `let edit` declares a binding that the code may change. `edit counts.set(...)` gives `counts` a new map, which is updated in place when nothing else holds it.
- **Absence is a value.** `get` returns an `Option`: `if none use 0` handles a word not seen yet.
- **One effect per line.** Each line that can fail or act is handled on the lines below it, as `if none` here.

```sh
una test .   # copy examples/wordfreq
una run .
```

```text
{"and":2,"bat":1,"cat":1,"hat":1,"the":3}
```

## 5. Capabilities and grants

`main` uses `Console[print]` and nothing else, and `una run` gives it exactly what `[run] grants` lists in `una.toml`. Without `"console"` there, the program does not start: U3012 names the capability that is missing. Grants can also come from the command line, as `una run . --grant console`.

The host offers `Console`, `Clock`, `Env`, `Files` and `Keys`, only to `main`, which passes them to the functions it calls. Text that comes from the outside, as `Files.read` returns, is labeled `internal`, and printing it needs a declassification policy: information flow is checked too (§9 of [the guide for agents](una-for-agents.md)).

## 6. A native executable

`una build` checks the package, then compiles `main` into a native executable, with the grants of `una.toml` built in:

```sh
una build .   # copy examples/wordfreq
./target/una/wordfreq
```

```text
built target/una/wordfreq
{"and":2,"bat":1,"cat":1,"hat":1,"the":3}
```

The executable prints what `una run` prints, and traces the same events: the interpreter is the reference, and the test suite runs every example program in both.

## 7. An editor

The VS Code extension, `images/una-language-support-0.2.0.vsix` in the repository, colors Una source and gives `.una` files their icon and theme. Install it from that file:

```sh
code --install-extension una-language-support-0.2.0.vsix
```

`una lsp` is a language server (LSP 3.17 over stdin and stdout) for any editor that speaks the protocol: it checks, formats and solves as you type, and never runs programs.

## Where next

- [The guide for agents](una-for-agents.md) is the compact reference to the whole language.
- [The syntax reference](syntax-reference.md) lists every construct.
- [The specification](../spec/) is the normative definition.
- `una explain <code>` explains any diagnostic, with examples.
