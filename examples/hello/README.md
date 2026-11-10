# hello

The smallest Una package: one function that prints one line.

Start here. The program is trivial, but every package has the same three files that this one has, and the line it prints already goes through a capability. In Una, a function acts on the outside world only through capabilities, a public function names them in its header, and `una run` hands a program only the host capabilities that its `una.toml` grants. This example shows that rule at its smallest size.

What it demonstrates:

- the three files of a package: `una.toml`, `verification.lock` and a module under `src/`;
- an entry point and a grant ([`entry`](../../docs/guide/configure/run/entry.md), [`grants`](../../docs/guide/configure/run/grants.md));
- a function header that declares its authority with [`uses`](../../docs/guide/language/capabilities/uses.md), and a [host capability](../../docs/guide/language/capabilities/host.md), `Console`.

## Run it

From this directory:

```sh
una check .   # copy examples/hello
una test .
una run .
una build .
./target/una/hello
```

`una run .` and the native executable both print:

```text una="run ."
Hello, Una
```

`una check .` prints nothing and exits with status 0: the package has no errors. `una test .` finds no `example` blocks, so it reports `0 examples: 0 passed, 0 failed`. `una build .` compiles the package to a native executable and prints `built target/una/hello`.

## How it works

### The manifest: `una.toml`

```text file="una.toml"
[package]
name = "hello"
version = "0.1.0"
edition = "0.1"

[run]
entry = "hello"
grants = ["console"]
```

`[package]` names the package and picks the [edition](../../docs/guide/configure/package/edition.md) of the language it is written in. `[run]` tells `una run` and `una build` where the program starts: `main` in the module `hello`. `grants` lists the host capabilities that the program receives when it runs. Here it gets the console and nothing else: no files, no clock, no network, no environment.

### The module: `src/hello.una`

```text file="src/hello.una"
module hello

pub main() -> Unit
  uses @console: Console[print]
=
  @console.print("Hello, Una")
```

Read the header from top to bottom:

1. `pub main() -> Unit`: a public function with no parameters that returns nothing useful.
2. `uses @console: Console[print]`: the function needs a capability of type `Console`, bound to the name `@console`, and of that capability it uses only the operation `print`. The `@` marks a capability. A capability is not a value: you cannot store it, return it or pass it as an ordinary argument.
3. `=` ends the header. The body follows it.

The body calls `@console.print`. Because the operation is named in the header, a reviewer knows what `main` may do without reading the body. A caller of `main` would need to hold the same authority.

### The lock: `verification.lock`

`verification.lock` pins the solver that proves the package's contracts: its name, version, logic, step budget and random seed. The same lock gives the same proofs on every machine. This package has no contract to prove, but every package carries the file ([`verification.lock`](../../docs/guide/configure/verification-lock.md)).

## What `una check` proves

There is nothing for the solver to prove: the program has no contract. The checker still confirms its authority by typing. `main` may print on the console and do nothing else, and a call to any other operation is an error. `una report .` states that authority in one line:

```text una="report ."
...
  hello.main (pub): @console: Console[print]
...
```

## Common mistakes

### Using a capability the header does not declare

Remove the `uses` line and write `main` as a one-line function:

```text file="src/hello.una" after="mistakes/1"
pub main() -> Unit = @console.print("Hello, Una")
```

`una check .` rejects it. Here and in the other READMEs, a diagnostic is shown as `una check .` prints it in a terminal without colors, except that trailing spaces are removed, and a line `...` in an excerpt of a file or of an output stands for lines left out:

```text una="check ." after="mistakes/1"
[U3004] Error: `@console` is used here, but this code declares no `uses`, so it is pure
   ╭─[ src/hello.una:3:22 ]
   │
 3 │ pub main() -> Unit = @console.print("Hello, Una")
   │                      ────┬───
   │                          ╰───── `@console` is used here, but this code declares no `uses`, so it is pure
   │
   │ Help: repair 1 (widens authority: the function then holds this authority, and every caller must hold it too: check that the operation belongs to this function's job): declare `uses @console: ...` in the header, with the capability and the operations used
   │
   │ Note: run `una explain U3004` for an explanation
───╯
```

A function without `uses` is pure. The repair says what it costs: the authority spreads to every caller, so add it only when the operation belongs to the function's job.

### Declaring a capability that the package does not grant

Give `main` a clock as well, but leave `grants` as it is:

```text file="src/hello.una" after="mistakes/2"
pub main() -> Unit
  uses @console: Console[print]
  uses @clock: Clock[now]
=
  let now = @clock.now()
  @console.print("Hello, Una, at {now}")
```

`una check .` reports the missing grant:

```text una="check ." after="mistakes/2"
[U3012] Error: `main` uses `now` of `Clock`, which `[run] grants` does not grant
   ╭─[ src/hello.una:5:3 ]
   │
 5 │   uses @clock: Clock[now]
   │   ───────────┬───────────
   │              ╰───────────── `main` uses `now` of `Clock`, which `[run] grants` does not grant
   │
   │ Help: repair 1 (widens grant: the program then gets this host authority when it runs): grant `now` of `Clock` in `[run] grants` of `una.toml`, or with `--grant`
   │
   │ Note: run `una explain U3012` for an explanation
───╯
```

There is no ambient authority. A host capability that is not granted is not there, and `una run .` refuses to start the program with the same code. Add `"clock"` to `grants`, or pass `--grant clock` to `una run`.

## Extend it

1. **Greet by name.** Move the text into a pure function, `pub greeting(name: Text) -> Text = "Hello, {name}"`, call it from `main`, and add an `example` block below it: `do greeting("Ada")` and `expect result == "Hello, Ada"`. `una test .` now reports `1 example: 1 passed, 0 failed`. A pure function needs no `uses`, and its examples need no fakes.
2. **Print the time.** Declare `uses @clock: Clock[now]` on `main`, print the value of `@clock.now()` (milliseconds since the Unix epoch), and grant `"clock"` in `una.toml`. Then remove the grant again and read the U3012 above.
3. **Read the report.** Run `una report .` and find the `authority` part and the host capabilities in the trusted base. Each new `uses` that you add shows up there.

## Learn more

- Construct pages: [`uses`](../../docs/guide/language/capabilities/uses.md), [host capabilities](../../docs/guide/language/capabilities/host.md), [capabilities](../../docs/guide/language/capabilities/capability.md), [`example`](../../docs/guide/language/examples/example.md).
- Configuration: [`entry`](../../docs/guide/configure/run/entry.md), [`grants`](../../docs/guide/configure/run/grants.md), [`edition`](../../docs/guide/configure/package/edition.md), [`verification.lock`](../../docs/guide/configure/verification-lock.md).
- Tutorial: [Getting started](../../docs/guide/getting-started.md).
- Learn, the tutorial on the Una website: [Hello, Una](/learn/hello/); [Capabilities](/learn/capabilities/).
- Specification: [capabilities and effects (spec/05 §3)](../../docs/spec/05-capabilities-and-effects.md), [host capabilities (spec/05 §7)](../../docs/spec/05-capabilities-and-effects.md), [the entry point (spec/04 §8)](../../docs/spec/04-functions-and-contracts.md), [the command line (spec/13 §5)](../../docs/spec/13-diagnostics-and-abi.md).

## Edition

Edition `0.1`. The program uses nothing that edition `0.2` added, so it reads the same in every edition ([edition](../../docs/guide/configure/package/edition.md)).
