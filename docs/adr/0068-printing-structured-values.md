# 0068. Printing structured values

- Status: Accepted
- Date: 2026-10-06
- Deciders: Leonardo (accepted on 2026-10-06, D1 to D8 as recommended), on a proposal by Claude (Una-2)
- Resolves: issue #204 (F2 of the native dogfood, una-bench#17)
- Amends: `spec/07 §7` rule 3 (no change of rule; its consequence for structured values), `spec/10 §12` (rule 14), `spec/11 §5` (the Una form is the text of interpolation), `spec/20 §1`, `plan/backlog.md`; the guide (`una-for-agents.md`, `syntax-reference.md`) with the implementation

## Context

Interpolation accepts `Int`, the fixed-width types, `Bool` and `Text`. Any other value is U2001: "interpolation converts `Int`, `Bool` and `Text` to text, and this value is `Point`". The native dogfood of the benchmark references (Una-3, F2) found agents writing a `describe` function for each record, list, `Option` and `Result` they want to print. The guide teaches the same workaround (`una-for-agents.md` §"Interpolation"). Meanwhile `una test`, `una run --json` and the trace already render any value in its Una form (`spec/11 §5`): `Some(3)`, `Registration(name: "Ana", event: Gala)`, `[1, 2]`.

What exists that a design must keep:

- **Labels.** The label of an interpolation is the join of every label of its operands' types, deep (`spec/07 §7` rule 3). `"id {phone}"` is `secret` when `phone` is, and printing it through `Console` is U4003. A record with a `secret` field has a `secret` deep label.
- **Redaction.** The trace and U6001 render a value with each field whose declared type is above `public` replaced by `<redacted:level>` (`spec/11 §5`, `spec/04 §8`). That is a view for diagnostics. It does not change any value.
- **Functions** render as `<function>` in traces. `Hashed` cannot be interpolated or converted to text (`spec/14 §1`).
- **Generic code.** In a generic body, a value of `T` carries the symbolic label `ℓ_T`. Its type, and so whether it holds functions, is not known (`spec/20 §1`).
- **The verifier** reads an interpolation as an unknown text (`ToText` and `Concat` have no term), so it needs no change.
- **Native.** `una-rt` renders values from type descriptions for the trace (`render.rs`). The interpreter does it in `machine.rs` (`render`). Both use a worklist, never the native stack (ADR 0060).

## Decision

Each decision lists its options and recommends one.

### D1. The form

- **(a) Interpolation of any value**, `"{p}"`, with no new name. *Recommended.* This is the form agents wrote and that failed (F2). It needs no new name or call, and `"{p}"` reads the same in a program and in a diagnostic.
- (b) A prelude function, `show(x: T) -> Text`. It is explicit, but it adds a name that agents must learn, and `"{show(p)}"` is noise around the case that matters.
- (c) A method, `p.to_text()`. The fixed-width types already have `to_text` (`spec/18 §2`). Extending it to every type is a third spelling for the same conversion.

### D2. The text

- **(a) The Una form of `spec/11 §5`, without redaction**, the one `una test` and the trace already use. *Recommended.*
  - Integers are in decimal and `Bool` is `true`/`false`. `Unit` is `unit`.
  - A variant without fields is its name. A variant or record with one field is `Name(value)`, and with several is `Name(f: value, g: value)` in declaration order.
  - A list is `[a, b]`, and an empty one is `[]`. `Option` and `Result` are variants: `Some(3)`, `None`, `Ok("x")`, `Err(NotFound)`.
  - A `Text` at the top of an interpolation stays as today, unquoted. Inside a structure it is quoted with the escapes of `spec/01 §6`, as `una_core::quote` writes it, so `["a, b"]` and `["a", "b"]` stay apart.
- (b) A JSON form. It is useful to tools, but foreign to the language, and it differs from what `una test` shows an agent.

There is no width, padding or depth limit. A deep value is rendered with a worklist (ADR 0060).

### D3. Labels and secret fields

The text an interpolation produces must carry an exact label. Two rules are sound.

- **(a) Require public through the existing rule, with no redaction.** *Recommended.*
  - The text holds every field in full, and its label is the deep join of `spec/07 §7` rule 3, unchanged.
  - Printing a record with a `secret` field through `Console` is then U4003, as printing a `secret Int` is today. The repair proposes interpolating its public fields, or a policy.
  - The flow rule needs no change, and so neither does the noninterference argument.
  - An interpolation into a secret sink, such as the `data` of `Keys.hmac`, keeps the whole value. Today's semantics of `"{secret_text}"` stay intact.
  - In a generic body, `"{x}"` with `x: T` is labeled `ℓ_T` (rule 8), instantiated at each call.
- (b) Redact in interpolation, as the trace does.
  - Every position whose static label is above `public` renders as `<redacted:level>`. The text's label is the join of the labels of the positions shown.
  - It is sound: a shown position is public by typing, and what the text reveals beyond it is static (the level, and the shape that public positions already give, such as a list's length, which `.len` labels public).
  - It changes what `"{x}"` means today, though: an interpolation of a `secret Text` into a secret sink would send `<redacted:secret>`. Every existing program that interpolates secret data into a secret sink would change behavior silently.
  - In a generic body the static label of a `T` position is `ℓ_T`, unknown until a call. So the position cannot be redacted there, and it has to carry `ℓ_T` in the text's label anyway.
- (c) Rule (a) for interpolation, plus a prelude function `redact(x: T) -> Text` with rule (b)'s text. It would give a public view of a record with secrets. It adds a name for a rarer need. Deferred: `plan/backlog.md` gains it, to revisit if agents ask.

### D4. Functions inside values

- **(a) Statically excluded through `Eq`.** *Recommended.*
  - Interpolating a value requires its type to satisfy `Eq` (`spec/20 §1`), which is exactly "values hold no functions", or to be one of today's scalars. A type that holds a function is U2001 at the interpolation, with the message naming the function position.
  - In a generic body, `"{x}"` with `x: T` needs `T: Eq`. Without it the interpolation is U2026, with the repair that writes `[T: Eq]`. A private function infers it, as for `==` (`spec/20 §3` rule 4).
  - `Hashed` keeps its exclusion (U2001), though it satisfies `Eq`. Its bytes stay opaque.
- (b) Render a function as `<function>`, as the trace does. It needs no rule, but it prints nothing useful, and a value that holds a closure would lose information silently.

### D5. Determinism

- **(a) The text is a function of the value alone.** *Recommended.*
  - Fields are in declaration order, list items in list order, variant fields in declaration order.
  - The language has no map or set type and no float, so nothing else orders text.
  - A future map type MUST render in the order of its keys under `Ord`, and its ADR states it.
  - No address, hash or allocation order ever appears.

### D6. Interpreter and native

- **(a) Byte-identical, by one specification and the differential test.** *Recommended.*
  - The interpreter renders by the form of the value with the field names of the kind. Native renders from the type description of the interpolated value's static type, which the code generator already emits for the trace (`describe`).
  - In a generic body, the native code renders a `T` value through the descriptor of `spec/20 §7`. D4 makes `T: Eq` the precondition, so a descriptor exists. It gains the type description index for `Structural`.
  - `spec/16 §5` makes the outputs equal, and the conformance and differential tests check it.
- (b) A text produced by the runtime only, without the static type. Field names would then have to live in every object. Rejected: it grows every value.

### D7. Edition

- **(a) Edition `0.2` only.** *Recommended.* In edition `0.1` the U2001 of today stays, so the Gate 1 benchmark (pinned to `0.1`) does not change.
- (b) Both editions. It would change the error set of `0.1` programs, which `spec/13 §7` freezes.

### D8. Diagnostic codes

- Interpolation of a type that holds a function: U2001, with a message that names the function position. No new code (decided).
- Interpolation of a `T` without `Eq` in a public generic function: U2026, with the repair `[T: Eq]` (`spec/20 §3` rule 1).
- Printing a record with a secret field: U4003, unchanged, with a repair that names the field above `public`.

## Consequences

- An agent writes `@console.print("{p}")` for a public record, list, `Option` or `Result`, and sees what `una test` shows.
- Labels and noninterference are unchanged: the deep join of rule 3 already labels the text.
- The guide drops the hand-written `describe` workaround.
- `spec/20 §1` gives `Eq` a second use: interpolation of `T`. Constraint inference for private functions covers it.
- Implementation, after acceptance:
  - the checker: D3, D4 and D8;
  - the interpreter: `render` without redaction, for interpolation;
  - native: a `una_rt_render(ty, word)` without redaction, and the descriptor path for `T`;
  - conformance cases: records, nested lists, `Option`/`Result`, quoting, a secret field (U4003), a function field (U2001), `T` without `Eq` (U2026), and generic `T: Eq` at several types;
  - the differential test, and the guide.

## Alternatives considered

- **Redaction in interpolation (D3 b).** Sound, but it changes the meaning of existing programs, and it cannot be decided in a generic body.
- **A derived `describe` per kind, written by the compiler.** It is the same text with a name per kind, and more surface for no gain.
- **A user-defined rendering (`Show`).** There are no user traits (ADR 0066), and a closed rendering keeps the output of every program predictable.
