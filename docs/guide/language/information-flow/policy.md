---
construct: "policy: declassification"
editions: ["0.1", "0.2"]
spec: ["spec/07 §5", "spec/07 §6", "spec/02 §4", "spec/13 §6", "spec/14 §3", "spec/10 §13.4"]
codes: [U4002, U4004, U4005]
---

# `policy`: declassification

## What it does

[Labels](labels.md) only rise, except through a `policy`: a declared, named release of data from one label to a lower one, through a function that the policy names. Lowering a label this way is called declassification. A policy lives in a [trusted module](trusted.md) (a module that `una.toml` lists as allowed to declare policies, and that `una report` lists for review), and every call of it is listed by `una report` with its purpose and owner. So every place where data goes down is visible and reviewed.

Learn more: [Releasing data: policies and trusted modules](/learn/information-flow/releasing-data-policies-and-trusted-modules/).

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.2"

[labels]
categories = ["Private"]

[trusted]
modules = ["shop.release"]
```

`src/shop/release.una`

```una
trusted module shop.release

pub policy length_band
  from confidential[Private] Text
  to public Nat
  via hundreds
  purpose "show how long a customer note is without showing it"
  owner support

hundreds(text: confidential[Private] Text) -> public Nat = text.len / 100
```

`src/shop/notes.una`

```una
module shop.notes

import shop.release (length_band)

pub note_size(note: confidential[Private] Text) -> Nat = length_band(note)
```

## Forms

In the grammar below, quoted words are written as shown, `[ ]` marks an optional part, `{ }` a part that may repeat, `|` separates alternatives, `( )` groups them, and `(* *)` holds a comment.

```ebnf
policy_decl     = [ "pub" ] "policy" lower_ident NEWLINE INDENT
                    "from" type NEWLINE
                    "to" type NEWLINE
                    "via" lower_ident NEWLINE
                    "purpose" text_lit NEWLINE
                    "owner" lower_ident NEWLINE
                  DEDENT ;
```

- **`from`** and **`to`**: the types before and after the release. The `via` function takes a `from` and returns a `to`, with the same types except for labels.
- **`via`**: the private function of the trusted module that computes the release.
- **`purpose`** and **`owner`**: why the release exists and who answers for it. `una report` lists them at every call.

A policy `p` is called as a function, `p(v)`, with the `uses` and `fails` of its `via` function.

## Examples

Above, `note_size` hands its caller a `public` number computed from `confidential` text: only the length band leaves, through `length_band`, and the note stays labeled.

A policy is allowed only in a trusted module:

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.2"

[labels]
categories = ["Private"]
```

`src/shop/release.una`

```una
-- fails with U4004
module shop.release

pub policy length_band
  from confidential[Private] Text
  to public Nat
  via hundreds
  purpose "show how long a customer note is without showing it"
  owner support

hundreds(text: confidential[Private] Text) -> public Nat = text.len / 100
```

## Edge cases

**The `via` function is private.** The policy is its only entry point, so no other code can call it with labeled data:

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.2"

[labels]
categories = ["Private"]

[trusted]
modules = ["shop.release"]
```

`src/shop/release.una`

```una
-- fails with U4005
trusted module shop.release

pub policy length_band
  from confidential[Private] Text
  to public Nat
  via hundreds
  purpose "show how long a customer note is without showing it"
  owner support

pub hundreds(text: confidential[Private] Text) -> public Nat = text.len / 100
```

**No `requires` on `via`.** A policy call checks its argument against `from` only, so the `via` body may assume nothing more:

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.2"

[labels]
categories = ["Private"]

[trusted]
modules = ["shop.release"]
```

`src/shop/release.una`

```una
-- fails with U4005
trusted module shop.release

pub policy length_band
  from confidential[Private] Text
  to public Nat
  via hundreds
  purpose "show how long a customer note is without showing it"
  owner support

hundreds(text: confidential[Private] Text) -> public Nat
  requires text.len > 0
=
  text.len / 100
```

**No release under a labeled program counter.** At a call `p(v)`, the program counter (the label of the conditions that decided whether this line runs; see [Branches and failures](/learn/information-flow/branches-and-failures-implicit-flows/)) must be below the `to` label. Choosing whether to release by a `secret` condition would reveal it:

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.2"

[labels]
categories = ["Private"]

[trusted]
modules = ["shop.release"]
```

`src/shop/release.una`

```una
trusted module shop.release

pub policy length_band
  from confidential[Private] Text
  to public Nat
  via hundreds
  purpose "show how long a customer note is without showing it"
  owner support

hundreds(text: confidential[Private] Text) -> public Nat = text.len / 100
```

`src/shop/notes.una`

```una
-- fails with U4002
module shop.notes

import shop.release (length_band)

pub note_size(note: confidential[Private] Text, flag: secret Bool) -> secret Nat = if flag then length_band(note) else 0
```

Further rules:

- **Inside `via`, labels are not checked.** The `via` function is part of the trusted code that `una report` lists under `tcb`; its refinements, preconditions and postconditions are still proven.
- **Only through its policy.** Any use of a `via` function other than through its policy is U4005.
- **Labeled failures.** The failure of a policy call is labeled by its `to` label joined with the program counter, so `if error pass` and `if error fail` on it are U4002 when that label is above `public`. One exception: a `via` function whose only failures are `Keys.hmac` calls with a literal key name, at the top level of its body and handled by `if error pass` or `if error fail` without a payload, decides nothing from the released value, so its policy's failure is labeled by the program counter alone.
- **Keys.** The result of `Keys.hmac` is labeled `internal` joined with every label of its data; lowering it further takes a policy, such as one from `secret[PII] Phone` to `internal Hashed` whose `via` function computes the HMAC of a phone number.

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
- `spec/10 §13.4`: a `via` function whose failure reveals nothing.
- Decision record: [ADR 0039](../../../adr/0039-labels-of-keys-hmac.md), for the labels of `Keys.hmac`.
