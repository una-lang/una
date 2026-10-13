---
construct: "policy: declassification"
editions: ["0.1", "0.2"]
spec: ["spec/07 §5", "spec/07 §6", "spec/02 §4", "spec/13 §6", "spec/14 §3"]
codes: [U4002, U4004, U4005]
---

# `policy`: declassification

## What it does

Labels only rise, except through a `policy`: a declared, named release of data from one label to a lower one, through a function that the policy names (`spec/07 §5`). A policy lives in a trusted module, and every call of it is listed in the build report with its purpose and owner (`spec/07 §5` rule 5, `spec/07 §6`). So every place where data goes down is visible and reviewed.

`una.toml`

```toml
[package]
name = "journal"
version = "0.1.0"
edition = "0.2"

[labels]
categories = ["Private"]

[trusted]
modules = ["release"]
```

`src/release.una`

```una
trusted module release

pub policy length_band
  from confidential[Private] Text
  to public Nat
  via hundreds
  purpose "show how much was written without showing what"
  owner journal_owner

hundreds(text: confidential[Private] Text) -> public Nat = text.len / 100
```

`src/show.una`

```una
module show

import release (length_band)

pub size(text: confidential[Private] Text) -> Nat = length_band(text)
```

## Forms

```ebnf
policy_decl     = [ "pub" ] "policy" lower_ident NEWLINE INDENT
                    "from" type NEWLINE
                    "to" type NEWLINE
                    "via" lower_ident NEWLINE
                    "purpose" text_lit NEWLINE
                    "owner" lower_ident NEWLINE
                  DEDENT ;
```

This is `spec/02 §4`.

- **`from`** and **`to`**: the types before and after the release. They differ only in their labels (`spec/07 §5` rule 4).
- **`via`**: the private function of the trusted module that computes the release (`spec/07 §5` rule 3).
- **`purpose`** and **`owner`**: why the release exists and who answers for it. The build report lists them at every call (`spec/07 §5` rule 5).

A policy `p` is called as a function, `p(v)`, with the `uses` and `fails` of its `via` function (`spec/07 §5` rule 1).

## Examples

Above, `size` hands its caller a `public` number computed from `confidential` text: only the length band leaves, through `length_band`, and the text stays labeled.

A policy is allowed only in a trusted module (`spec/07 §6`):

`una.toml`

```toml
[package]
name = "journal"
version = "0.1.0"
edition = "0.2"

[labels]
categories = ["Private"]
```

`src/release.una`

```una
-- fails with U4004
module release

pub policy length_band
  from confidential[Private] Text
  to public Nat
  via hundreds
  purpose "show how much was written without showing what"
  owner journal_owner

hundreds(text: confidential[Private] Text) -> public Nat = text.len / 100
```

## Edge cases

**The `via` function is private.** The policy is its only entry point, so no other code can call it with labeled data (`spec/07 §5` rule 3):

`una.toml`

```toml
[package]
name = "journal"
version = "0.1.0"
edition = "0.2"

[labels]
categories = ["Private"]

[trusted]
modules = ["release"]
```

`src/release.una`

```una
-- fails with U4005
trusted module release

pub policy length_band
  from confidential[Private] Text
  to public Nat
  via hundreds
  purpose "show how much was written without showing what"
  owner journal_owner

pub hundreds(text: confidential[Private] Text) -> public Nat = text.len / 100
```

**No `requires` on `via`.** A policy call checks its argument against `from` only, so the `via` body may assume nothing more (`spec/07 §5` rule 4):

`una.toml`

```toml
[package]
name = "journal"
version = "0.1.0"
edition = "0.2"

[labels]
categories = ["Private"]

[trusted]
modules = ["release"]
```

`src/release.una`

```una
-- fails with U4005
trusted module release

pub policy length_band
  from confidential[Private] Text
  to public Nat
  via hundreds
  purpose "show how much was written without showing what"
  owner journal_owner

hundreds(text: confidential[Private] Text) -> public Nat
  requires text.len > 0
=
  text.len / 100
```

**No release under a labeled program counter.** At a call `p(v)`, the program counter must be below the `to` label. Choosing whether to release by a `secret` condition would reveal it (`spec/07 §5` rule 2):

`una.toml`

```toml
[package]
name = "journal"
version = "0.1.0"
edition = "0.2"

[labels]
categories = ["Private"]

[trusted]
modules = ["release"]
```

`src/release.una`

```una
trusted module release

pub policy length_band
  from confidential[Private] Text
  to public Nat
  via hundreds
  purpose "show how much was written without showing what"
  owner journal_owner

hundreds(text: confidential[Private] Text) -> public Nat = text.len / 100
```

`src/show.una`

```una
-- fails with U4002
module show

import release (length_band)

pub size(text: confidential[Private] Text, flag: secret Bool) -> secret Nat = if flag then length_band(text) else 0
```

Further rules:

- **Inside `via`, labels are not checked.** The `via` function is part of the trusted base; its refinements, preconditions and postconditions are still proven (`spec/07 §5` rule 3).
- **Labeled failures.** The failure of a policy call is labeled by its `to` label, so `if error pass` and `if error fail` on it are U4002 when that label is above `public` (`spec/07 §5` rule 6).
- **Keys.** The result of `Keys.hmac` is labeled `internal` joined with every label of its data; lowering it further takes a policy, as `spec/07 §5`'s example does for a phone number (`spec/14 §3`, ADR 0039).

## Diagnostics

- [U4002](../../../../crates/una-explain/explain/U4002.md): a policy call under a program counter above its `to` label.
- [U4004](../../../../crates/una-explain/explain/U4004.md): a policy outside a trusted module.
- [U4005](../../../../crates/una-explain/explain/U4005.md): a `via` function that is public, has a `requires` clause, or whose types do not match the policy's.

## Edition

Policies are the same in editions `0.1` and `0.2`.

## Normative source

- `spec/07 §5`: declassification policies, rules 1 to 6; `spec/07 §6`: trusted modules.
- `spec/02 §4`: the grammar.
- `spec/13 §6`: the build report.
- `spec/14 §3`: `Keys.hmac` and its labels.
