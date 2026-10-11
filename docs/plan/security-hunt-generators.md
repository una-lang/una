# The generators and mutators of the security hunt

- Issue: #513, part (a) of `docs/plan/security-hunt.md`
- Date: 2026-10-10
- Status: design (L2), no code yet

## 1. What they produce

Every program the generators write is a small package:
- a `una.toml`, with `[labels]`, `[trusted]` and `[run]` as the shape needs;
- `src/` modules;
- for a source-level variation of the oracle, the positions of its varied literals.

Each program carries an operator id from §4, the `spec/07` rule it targets, a seed, and an **expected verdict class**. The class decides which oracle judges it:

| Class | Meaning | Judged by | A finding when |
|---|---|---|---|
| **must-reject** | the operator injects a flow that `spec/07` forbids | `una check` alone | `una check` accepts it |
| **must-keep** | the operator rewrites an accepted or rejected program without changing any label, flow or effect | `una check` alone | the verdict changes: rejected to accepted is a candidate leak; accepted to rejected is an over-strict checker |
| **open** | the operator writes a program the spec allows or forbids by rules the generator does not decide | the re-check (b), the reverse (c), the two-run oracle (e) | as those parts say |

The first two classes are two more cheap oracles. They need only `una check`, about one run per program, so they are the first campaigns. A must-reject program that is accepted also goes to the two-run oracle, which turns it into a concrete witness, or shows that the flow is not observable.

## 2. How programs are written

- **Generators, not text surgery.** Most operators are templates with holes: a hole is a type, a label, an expression of a given type and label, a statement, a sink, a branch, a wrapper. A small Python model of the surface syntax (`spec/02`) fills them and prints canonical text. So a program parses by construction, and `una fmt --check` is a test of the printer.
- **Mutators over the corpus.** For part (b) and the must-keep class, mutators start from the conformance programs and the examples that `una check` accepts:
  1. They first run `una fmt` on them, so that the canonical form (`spec/15`) makes line-level edits reliable.
  2. A mutator edits declarations, statements and expressions by their canonical layout.
  3. A mutant that does not parse is dropped and counted against its operator, never judged.
- **Inside the importer's vocabulary**, for part (b). A mutator that targets the re-check uses only what the importer reads: `if`, `case` on `Option` and `Result`, `attempt`, `handle`, loops, `Files`, `Http`, `Env.get`, one other operation, generics, `map`, `filter`, `fold`, `first_where`, B1 policies, and the primitives the model has. Otherwise its programs come back "not re-checked", and they belong to the oracle instead.
- **Both editions.** Each operator states its editions. A program is written once per edition it applies to, since the rules differ (edition 0.1's defense in depth, ADR 0067's summaries).
- **Code.** Python 3, the standard library only, in `scripts/security-hunt/`. Programs and evidence go to a directory outside the repository, under the oracle's temporary root. Nothing generated is committed except minimized repros of findings, which follow the S-xx process.

## 3. Values

Literals that the two-run oracle varies are written behind an annotated binding, `let s: secret T = <literal>`, at the start of the function that uses them. That keeps the variation a single edit. The values are those of `docs/plan/security-hunt-oracle.md` §2.

## 4. The operators

Each family targets one row of `docs/reviews/security-hunt/coverage.md`. The families come in the order of its §2: trusted modules and policies first, since the model assumes every policy is lawful.

### T. Trusted modules and policies (`spec/07 §5`, §6, §7 rule 5)

| Id | Program | Class |
|---|---|---|
| T1 | A policy in a module whose header says `trusted` and that `una.toml` does not list; and the reverse | must-reject (U7005, U4004) |
| T2 | A policy in a module that is neither | must-reject (U4004) |
| T3 | The listing in `una.toml` names the module by another spelling: a trailing space, another case, a Unicode lookalike, a file path instead of a module path, a duplicate entry, a module that does not exist | must-reject (U7005), or a usage diagnostic |
| T4 | The `via` function reached other than through its policy: called directly; named as a value; given to `map`, `fold` or `first_where`; stored in a record field; returned by a function; wrapped in a lambda; passed through a generic identity function; called from a handler body; from a `together` task; from another module through an import | must-reject (U4005) |
| T5 | The `via` function declared `pub`, or exported through an alias or a re-export | must-reject (U4005) |
| T6 | The `via` function in another module than its policy | must-reject (U4005) |
| T7 | Types that differ beyond labels: a refinement on one side only, two refined kinds, a record, a generic type, a function type, a `requires` clause | must-reject (U4005) |
| T8 | Policies the spec allows but B1 does not: a non-scalar `to` (a record, a list, `Option`), a `via` function with effects (`Console`, `Files`, `Keys`), a `via` that calls another function, one that calls another policy | open: the two-run oracle. A difference the policy's `to` explains is not a finding; one visible below `to` is |
| T9 | Two policies with one `via` function; two `via` functions with one name in two modules | open |
| T10 | A policy call under a raised pc, in a lambda, in a handler body, under a symbolic pc in a generic function, in an `of` argument, in a `together` task | must-reject when the pc is above `to` (U4002), else open |
| T11 | A policy whose `via` function fails (S-18): `attempt`, `handle`, `if error pass`, `use` and `fail` around the call, with `to` at `public`, `internal` and `secret`, at a public and a raised pc | must-reject or open, by ADR 0090's rule |
| T12 | Two policies composed: the result of one given to the other, `to` lower than the second's `from`, and the reverse | open |

### H. Handlers and the effect floor (§7 rule 4)

| Id | Program | Class |
|---|---|---|
| H1 | An operation body that branches on its parameter, raised by `accepts`, then performs an effect, calls a function with effects, or fails | must-reject when the floor forbids it (U4003, U4002) |
| H2 | An operation body whose result carries its parameter into a result type labeled lower | must-reject (U4001) |
| H3 | `with` installing a handler under a raised pc, and a handler installed public whose operation a secret branch calls | open |
| H4 | A user handler of a host capability (`Keys`, `Files`, `Console`) that breaks the worst caller's rule | must-reject |
| H5 | A handler whose operation fails, with `attempt` and `if error` at the caller under various pcs | open |

### K. `Keys.hmac` (ADR 0039)

| Id | Program | Class |
|---|---|---|
| K1 | A key name that is `secret` or `internal`, or computed under a raised pc | must-reject (U4003) |
| K2 | The result lowered without a policy: through a field, `.len`, an equality, an interpolation (`<hashed>`), `to_text`, a list, `Option` | must-reject where the label falls (U4001, U4003) |
| K3 | `UnknownKey` decided by a key name taken from `Env.get` or `Files.read` (`internal`) | must-reject |
| K4 | `data` with deep labels: a record with a `secret` field, a list of `secret[PII]` | open: the two-run oracle, with V-host varying the key material |

### G. Kinds with type parameters, and invariance (§7 rule 2, ADR 0046)

| Id | Program | Class |
|---|---|---|
| G1 | A kind `Box[T]` whose field is a function of `T`: put a `secret` in and take it out as `public`, through each flow site of rule 2 | must-reject (U4001) |
| G2 | The same with a covariant kind, where the flow is allowed | open: it should be accepted; the two-run oracle checks that it does not leak |
| G3 | Recursive and mutually recursive kinds, with the parameter reaching a function type at the second level | must-reject or open, by rule 2's least solution |
| G4 | A kind parameter instantiated at a function type with effects | open |

### P. Failure payloads (§1, §3 rule 3)

| Id | Program | Class |
|---|---|---|
| P1 | A failure whose payload carries a `secret` field, passed with `if error pass` to `main` | open: U6001 prints only `public` fields, and the oracle compares them |
| P2 | `if error use e` that reads the payload, under a public pc, into a sink | must-reject when the field is above the sink |
| P3 | A payload chosen by a secret branch | must-reject (U4002: a failure under a raised pc) |

### C. `together` (`spec/19`)

As built (`hunt/families/concurrency.py`); in edition 0.1 every program is must-reject U0103, which checks the edition gate.

| Id | Program | Class |
|---|---|---|
| C1 | The number of tasks decided by a `secret` value (a `secret` list, a range with a `secret` bound), with a sink, a failure, or the result's length in a public position | must-reject (U4003, U4002, U4001); twins over a public list of `secret` elements are open |
| C2 | A task's own `secret` element decides a failure or a sink; a branch's binding keeps its label after the block | must-reject |
| C3 | A block under a `secret` program counter whose branch calls a function with effects, or nests a `together for` with a sink | must-reject (U4002, U4003); a public-pc twin is open |
| C4 | Cancellation seen by a sibling: one task's failure is decided by an `internal` value (`Http.get` on an `internal` URL) and passed out of the task, while a sibling task, or code after the block, has a public effect | must-reject (U4002); an `attempt` twin, where the block cannot fail, is open |
| C5 | Probes of C4's premise that a user operation's failure cannot depend on a `secret` argument: a handler that reads its argument through `Http.get` or `Files.read` and fails on the result, and a handler that always or never fails chosen by `with` under a `secret` condition | must-reject |

Not generated: the relative order of the effects of different tasks, and how far a cancelled task got, depend on timing, which is outside the model (`spec/07 §1`, `spec/19 §3` rule 3). The two-run oracle compares each task's events alone, never their interleaving.

### S. User sum types, and `is` / `in` (§3 rule 2, §7 rule 3)

| Id | Program | Class |
|---|---|---|
| S1 | `when` on a `secret` sum value, with an effect or a failure in an arm | must-reject (U4003, U4002) |
| S2 | `is` and `in` on `secret` values into a public sink | must-reject |
| S3 | A variant with a `secret` field, matched and printed through each arm | must-reject where the field reaches a sink |

### O. `Ord`, `sort`, `min`, `max` (ADR 0066)

| Id | Program | Class |
|---|---|---|
| O1 | `sort` of a list mixing labels, then the length, the first element, or an equality on the result, into a sink | must-reject at the join |
| O2 | `min` and `max` of a `public` and a `secret` value | must-reject |
| O3 | A generic function constrained by `Ord`, whose result position a comparison chooses | must-reject at a public sink, and open for (b) |

### F. The covered fragment, for part (b)

These operators stay inside the importer's vocabulary, so their programs are re-checked:

- F1. Raise a literal behind `let s: secret T` and let it flow on unchanged. The program is then either rejected by both `una check` and the re-check, or accepted by both.
- F2. Wrap a statement in `if s` on a `secret` condition.
- F3. Wrap a call in `attempt`, `handle`, `if error pass`, `use` or `fail`.
- F4. Route a value through `map`, `filter`, `fold`, `first_where`, a lambda, a generic function, a record field, `Option`, `Result`, or a tuple.
- F5. Put a `Files.write` or `Files.read` under a raised pc, in a lambda, in recursion, or as a value. S-14's shapes.
- F6. Vary an `Http` call's URL, headers and body labels. S-15's shapes.
- F7. Route a secret through the prelude functions the re-check reads since #532, #542 and #545: `at`, at a public and at a secret index, `or_else`, with a secret value and with a secret default, `min`, `max`, `abs`, `slice`, and `is_ok`, of a public result of a secret (structural, so public) and of a result chosen under a secret condition (so secret). A table of its own, so that F1's programs for earlier seeds rebuild unchanged.

The class is open. The judge is the comparison: a declaration that `una check` accepts and the re-check rejects is a candidate.

### R. Refactors that keep every label (must-keep)

Each is applied to accepted and to rejected programs, and must not change the verdict:

- R1. Inline a `let` binding with a declared type, or introduce one, with the same label.
- R2. Extract an expression into a private function with the same declared types and labels, or inline such a function.
- R3. Pass a value through `id[T](x: T) -> T = x`.
- R4. Eta-expand a function value into a lambda, or eta-reduce one.
- R5. Swap the branches of an `if` and negate its condition.
- R6. Rename bindings and functions consistently.
- R7. Move a declaration to another module of the package, and import it.
- R8. Add a binding that nothing uses, with a `public` value.
- R9. Reorder independent declarations.

An exception is known in advance. Rule 8's edition 0.1 defense makes R3 and R4 change verdicts on purpose: an annotated or `let`-bound lambda accepts no decision. Those pairs are excluded by construction, and the exclusion is listed with the operator.

## 5. Order of the campaigns

1. **must-reject and must-keep** of the T family, then H, K, G, P, C, S, O. They need `una check` only, at about one run per program.
2. **Part (b)**, with the F family and the R family's accepted programs.
3. **The two-run oracle** on everything accepted: first the open T8 to T12 programs, then the other families, then the examples under V-host.

Each campaign is heavy work and takes a machine slot between benchmark runs (the coordinator's condition 3). The first slot measures the cost on 500 programs before a full round is planned.

## 6. What is kept

Each campaign writes a table of its results. A row holds:
- the operator, the rule, the edition and the seed;
- the class;
- `una check`'s verdict and diagnostics;
- the re-check's outcome;
- the oracle's outcome.

A row that is a finding also gets its program, its minimized form and the runs' evidence. Counts per operator give the coverage that the campaign reached, and the share of mutants that did not parse.
