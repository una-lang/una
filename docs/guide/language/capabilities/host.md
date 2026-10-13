---
construct: Host capabilities and grants
editions: ["0.1", "0.2"]
spec: ["spec/05 §7", "spec/14 §3", "spec/13 §5", "spec/13 §7", "spec/07 §4", "spec/07 §7", "spec/24 §2", "spec/24 §5"]
codes: [U3012, U4003, U6005]
---

# Host capabilities and grants

## What it does

Host capabilities are the ones the runtime implements: `Console`, `Clock`, `Env`, `Files` and `Keys`, and from edition `0.2` on also `Http` (`spec/05 §7`, `spec/14 §3`). They enter a program only through `main`'s `uses` clauses, and the runtime provides them only when they are granted, in `una.toml` or with `--grant` (`spec/05 §7` rules 1 and 2). Other functions receive them from their callers by name, like any capability.

`una.toml`

```toml
[package]
name = "hello"
version = "0.1.0"
edition = "0.2"

[run]
entry = "hello"
grants = ["console"]
```

`src/hello.una`

```una
module hello

pub main() -> Unit
  uses @console: Console[print]
=
  @console.print("hello")
```

## Forms

The host capabilities are those of `spec/14 §3`:

| Capability | Operation | Fails | Accepts |
| --- | --- | --- | --- |
| `Console` | `print(line: Text) -> Unit` | | `public` |
| `Clock` | `now() -> Int` | | `public` |
| `Env` | `get(name: Text) -> Option[internal Text]` | | `public` |
| `Files` | `read(path: Text) -> internal Text` | `FileError` | `public` |
| `Files` | `write(path: Text, content: Text) -> Unit` | `FileError` | `internal` |
| `Keys` | `hmac(key_name: Text, data: Text) -> internal Hashed` | `KeyError` | `public` for `key_name`, any label for `data` |

The grant specs are those of `spec/13 §5`:
- `console`, `clock` and `env`;
- `keys:<keyfile>`;
- `files:<path>:read`, `files:<path>:write` and `files:<path>:read,write`;
- from edition `0.2` on, `http:<origin>` and `http:<origin>:<methods>` (`spec/24 §2`).

They are given in `[run] grants` of `una.toml`, or as `--grant` flags of `una run` (`spec/13 §5, §7`).

## Examples

A host operation of `main` that no grant allows is reported by `una check`, before the program runs (`spec/13 §5`):

`una.toml`

```toml
[package]
name = "clock"
version = "0.1.0"
edition = "0.2"

[run]
entry = "clock"
grants = ["console"]
```

`src/clock.una`

```una
-- fails with U3012
module clock

pub main() -> Unit
  uses @console: Console[print]
  uses @clock: Clock[now]
=
  let t = @clock.now()
  @console.print("{t}")
```

What `Env.get` and `Files.read` return is `internal`, so printing it to the console, which accepts only `public`, is rejected. Lowering a label takes a policy (`spec/14 §3`, `spec/07 §4`):

`una.toml`

```toml
[package]
name = "envs"
version = "0.1.0"
edition = "0.2"

[run]
entry = "envs"
grants = ["console", "env"]
```

`src/envs.una`

```una
-- fails with U4003
module envs

pub main() -> Unit
  uses @console: Console[print]
  uses @env: Env[get]
=
  let home = @env.get("HOME")
    if none use ""
  @console.print(home)
```

## Edge cases

Further rules:

- **Files stay inside the grant.** A `Files` path is resolved against the grant's prefix. An absolute path, a `..` that leaves the prefix, or a final component that is a symbolic link fails with `Denied`; path resolution never escapes the prefix (`spec/05 §7` rule 3, `spec/14 §3`).
- **Keys stay outside values.** `Keys` loads its keys from the grant's file, and key material never enters Una values. The result of `hmac` is labeled `internal`, joined with every label of the type of `data`, and lowering it takes a policy (`spec/14 §3`, ADR 0039). `key_name` accepts only `public`, so a labeled key name, or a call under a raised program counter, is U4003.
- **Labeled failures.** Whether a host operation fails is labeled by the arguments that decide it: the path of `Files`, not the content; the key name of `Keys.hmac`, not the data; and every argument of `Http`, so its failures are `internal` (`spec/24 §5` rule 3). A `Files.write` to an `internal` path fails `internal`, and every `Files` failure also carries the package's file-state label, the labels that decide where its writes land and whether they run (`spec/14 §3`, `spec/07 §7` rule 4).
- **Host text.** When `Files.read` or `Env.get` meets bytes that are not valid UTF-8, the run aborts with U6005 (`spec/14 §3`).
- **Out of the model.** Races on a path, hard links, resource limits and other processes are outside the model, and the build report lists them (`spec/14 §3`).
- **User handlers.** Code may implement a host capability with its own handler, such as a fake `Keys` in examples. The host implementations are used only by `una run` (`spec/14 §3`).
- **Declared in `std.host`.** A user capability with the same name is a different declaration (`spec/05 §7` rule 4).

## Diagnostics

- [U3012](../../../../crates/una-explain/explain/U3012.md): a host operation of `main` that no grant allows, or a `uses` of `main` that no grant can provide.
- [U4003](../../../../crates/una-explain/explain/U4003.md): labeled data sent to an operation that does not accept its label.
- [U6005](../../../../crates/una-explain/explain/U6005.md): at run time, host text that is not valid UTF-8.

## Edition

The five host capabilities of v0.1 are in editions `0.1` and `0.2`. Edition `0.2` adds `Http`, with its grants `http:<origin>` and `http:<origin>:<methods>` (`spec/14 §3`, `spec/24 §2`).

## Normative source

- `spec/05 §7`: host capabilities, rules 1 to 4.
- `spec/14 §3`: the operations, `Files` paths, `Keys`, labels and host text.
- `spec/13 §5`: grant specs and `una run`; `spec/13 §7`: `[run] grants`.
- `spec/07 §4`: sinks; `spec/07 §7` rule 4: labeled failures.
- `spec/24 §2`: `Http` grants.
