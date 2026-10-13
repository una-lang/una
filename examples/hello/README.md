# hello

The smallest package: it prints one line.

## What it shows

- `una.toml` names the package, picks [edition](../../docs/guide/configure/package/edition.md) `0.1`, and says how `una run` starts it: the [entry](../../docs/guide/configure/run/entry.md) module `hello`, with the [grant](../../docs/guide/configure/run/grants.md) `console`.
- `src/hello.una` has one function, `main`. Its header says, with [`uses`](../../docs/guide/language/capabilities/uses.md), the only thing it may do: `@console: Console[print]`, a [host capability](../../docs/guide/language/capabilities/host.md) that `una run` provides because of the grant.
- `verification.lock` pins the solver, as every package does ([`verification.lock`](../../docs/guide/configure/verification-lock.md)).

## What `una check` proves

There is nothing to prove here: the program has no contract. The check still confirms its authority by typing: `main` may print on the console and do nothing else, and a call to any other operation would be an error.

## Run it

From this directory:

```sh
una check .   # copy examples/hello
una test .
una run .
una build .
./target/una/hello
```

`una test` finds no examples to run. `una run .` and the native executable print:

```text
Hello, Una
```

## Edition

Edition `0.1`.
