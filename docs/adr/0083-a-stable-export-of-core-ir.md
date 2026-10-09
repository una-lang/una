# 0083. A stable export of Core IR, `una.core/1`

- Status: Accepted
- Date: 2026-10-08
- Deciders: Leonardo (2026-10-08, written); proposal by Claude (Una-2), for M0 of the formal track (`docs/plan/formal-track.md`, #46)

## Context

The formal track, approved for M0 to M2, checks programs in a Lean model of Una Core. Its first deliverable (M2) is a Lean checker for the label rules over Core IR, run on the Core of every program of the corpus, and its tie to the Rust checker is option B of the plan: a proven Lean checker that re-checks the Core of every program `una check` accepts.

Both need Core IR outside the compiler, in a form a program written in another language can read without guessing.
- `una check --dump-core <module>` prints one module's Core in the text syntax of `spec/09`, and `spec/09 §4` says it "is not a stable interface". It is meant for people and snapshot tests. Its labels print as `secret[PII] + label(T) + label(T*)`, its binders as `x%2`, and nothing promises the form will not change.
- Core IR is the compiler's own data (`una_core::ir`): a Rust structure, with no serialization.

What the formal checker needs from Core:
- every declaration of the package, with its parameters' declared types and labels, its annotations, its results;
- every binder's type with its label, the symbolic labels `T` and `T*` included (ADR 0038 D3, `spec/07 §7` rule 8);
- the operations of capabilities with their `accepts`, the policies, the prelude's signatures (`Program::library`) and the host capabilities (`Program::host`);
- the A-normal form of bodies: lets, branches, calls, operations, failures.

## Decision

1. **A command.** `una core [path]` checks the package at tier 0 and prints its Core IR.
   - With `--json`, it prints one document, `una.core/1`, on stdout.
   - Without `--json`, it prints every module in the text syntax of `spec/09`, as `--dump-core` does for one; that text stays unstable.
   - Core holds only the declarations without diagnostics (`spec/09`). The document says so: `complete` is `true` when the package checked clean, and `false` otherwise, with the diagnostics on stderr and exit code 1. A consumer that re-checks programs uses complete documents only.
2. **The document.**

   ```json
   {
     "schema": "una.core/1",
     "edition": "0.2",
     "complete": true,
     "modules": [ { "path": "app", "decls": [ ... ] } ],
     "library": [ ... ],
     "host": [ ... ]
   }
   ```

   - Every value of a Rust enum of `una_core::ir` is an object whose first key is `kind`, its variant's name in snake case (`"let"`, `"if"`, `"call"`, `"named"`), followed by the variant's fields under their Rust names, in declaration order.
   - A struct is an object with its fields under their Rust names, in declaration order.
   - A label is `{ "level": "secret", "categories": ["PII"], "symbols": ["T", "T*"] }`, with the sets in their sorted order.
   - A source span is `{ "file": "src/app.una", "start": [line, col], "end": [line, col] }`, 1-based, as the diagnostics' locations are.
   - Names are strings, as Core holds them, `x%2` included.
   - An integer literal is a string of decimal digits with an optional `-`, since Una's integers are unbounded.

   The encoding is mechanical: one rule per Rust form. `spec/13 §8` states the rules and lists every `kind` of every node, generated from `una_core::ir` and checked against it by a test, so the listing cannot drift.
3. **Versioning** (ADR 0076).
   - Adding a key keeps the version.
   - The `kind` of a node of Core is an **open** field. Core gains forms as the language grows, and a consumer handles a form it does not know in the one safe way: as outside what it understands. A consumer **MUST** report a declaration with an unknown `kind` as not understood, never as accepted or checked. The formal checker reports it "not re-checked".
   - Every other enumerated value, such as a label's `level`, is closed.
4. **Determinism.** The document is a function of the package's sources and the compiler's version: keys in the order above, sets sorted, modules in the order of `Program::modules`. Two runs give the same bytes, which the identity runs can check.
5. **Where.** The encoding lives in `una-cli`, which already writes every `una.*` document with `serde_json`, as a walk over `una_core::Program`. `una-core` keeps no serialization dependency, and the compiler's own types do not change.

## Complexity

O(size of Core) time and output. The walk visits each node once; the document is as large as the Core it describes.

## Security and the TCB

The command only reads and prints: it runs no program and calls no solver. The document holds the package's own code and types, with labels as type information; no value of a run, no grant, no key. The export joins the trusted base of the formal checker (option B of the plan states it), so it gets the identity runs and a test of its own: every corpus program's document parses, and its forms are the ones `spec/13 §8` lists.

## Consequences

- `spec/13 §5` gains `una core [path]`, and a new §8 states `una.core/1` and lists its node kinds, with `kind` declared open there, as ADR 0076 rule 5 requires.
- `spec/09 §4` points at `una core --json` as the stable form, and keeps `--dump-core` as the debug text.
- The formal checker of M2 reads Core from `una core --json`, and the `formal` gate runs it on the corpus.

## Alternatives considered

- **Parse the text of `--dump-core`.** Rejected: unstable by `spec/09 §4`, per module only, and its label and name syntax is meant for reading, not for a parser in another language.
- **`--json` on `--dump-core`.** Rejected: `--json` already makes `una check` print `una.diagnostics/1`, and stdout holds one document (`spec/13 §5`); a second document under the same command would break that rule.
- **Call the compiler from Lean through a foreign function interface.** Rejected: it ties the Lean build to the Rust toolchain and puts `unsafe` code in the path, for no gain over a file.
- **Every `kind` closed.** Rejected: every new form of Core, which most language features add, would bump the schema, while the safe handling of an unknown form needs nothing beyond "not understood".

## Amendment (2026-10-08): two details of the encoding

Found while implementing the decision, with no change to it; `spec/13 §8` states both.
- A tuple variant has no field names: its one field is written `value`, and its several fields `values`, an array (`Base::Param("T")` is `{"kind": "param", "value": "T"}`).
- A variant's own field named `kind` is written `kind_of`, since `kind` names the variant (`Term::Field`). A struct's field named `kind`, such as `Ctor`'s, keeps its name: a struct is not a variant and has no `kind` of its own.
