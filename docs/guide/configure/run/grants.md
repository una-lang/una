---
construct: "[run] grants"
editions: ["0.1", "0.2"]
spec: ["spec/13 §7", "spec/13 §5", "spec/05 §7", "spec/14 §3", "spec/24 §2", "spec/07 §1"]
codes: [U3012, U7002]
---

# `[run] grants`

## What it does

`grants` lists the host capabilities, the authority to use the console, files, the clock and the like, that `una run` gives the entry's `main`. Each is written as a grant spec, a short text such as `"console"` or `"files:./data:read"` that names one capability and how far it reaches. There is no ambient authority, no authority a program has just by running: a host capability that is not granted is not there, and `una check` reports a `main` that needs one.

Learn more: [The host's capabilities and grants](/learn/capabilities/the-hosts-capabilities-and-grants/).

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.2"

[run]
entry = "shop"
grants = ["console"]
```

`src/shop.una`

```una
module shop

pub main() -> Unit
  uses @console: Console[print]
=
  @console.print("the shop is open")
```

## Forms

```text
[run]
grants = ["console", "clock", "env", "keys:<keyfile>", "files:<path>:read", "files:<path>:write", "files:<path>:read,write", ...]
```

The grant specs:

| Grant spec | Grants |
| --- | --- |
| `console` | `Console[print]` |
| `clock` | `Clock[now]` |
| `env` | `Env[get]` |
| `keys:<keyfile>` | `Keys[hmac]`, with the keys of the file |
| `files:<path>:read`, `files:<path>:write`, `files:<path>:read,write` | the `Files` operations of the mode, under the path |
| `http:<origin>`, `http:<origin>:<methods>` (edition `0.2`) | `Http` operations for the origin |

`una run` also takes `--grant <spec>` flags; the grants of `una.toml` and of the flags apply together.

## Examples

A host operation of `main` that no grant allows is reported before the program runs. Here the grant reads files and `main` writes one:

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.2"

[run]
entry = "shop"
grants = ["files:./data:read"]
```

`src/shop.una`

```una
-- fails with U3012
module shop

pub main() -> Unit
  uses @files: Files[read, write]
=
  @files.write("orders.txt", content: "order 42")
    if error use unit
```

## Edge cases

**Only grant specs**:

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.2"

[run]
entry = "shop"
grants = ["network"]
```

`src/shop.una`

```una
-- fails with U7002
module shop

pub price(cents: Int) -> Int = cents
```

Further rules:

- **Files stay under the grant.** A `Files` path is resolved under the grant's path, and path resolution never escapes it. Races on a path and hard links are outside the model.
- **Relative paths** in `una.toml` grants resolve against the package root, and those of flags against the current directory.
- **Host capabilities.** See [host capabilities and grants](../../language/capabilities/host.md).

## Diagnostics

- [U3012](../../../../crates/una-explain/explain/U3012.md): a host operation of `main` that no grant allows.
- [U7002](../../../../crates/una-explain/explain/U7002.md): a grant that is not a grant spec.

## Edition

The grant specs of v0.1 are in editions `0.1` and `0.2`. Edition `0.2` adds the `http:` grants.

## Normative source

- `spec/13 §7`: `[run]`; `spec/13 §5`: grant specs and `una run`.
- `spec/05 §7`: host capabilities; `spec/14 §3`: their operations; `spec/24 §2`: `Http` grants.
- `spec/07 §1`: what is outside the model.
