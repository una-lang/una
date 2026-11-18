# 0038. Soundness rules for function values, generics, handlers, policies, names and file grants

- Status: Accepted
- Date: 2026-09-30
- Deciders: Leonardo
- Resolves: review findings R-001 to R-011 (Critical); escalation D-093
- Amended by: 0045 (D1: the consistency check runs in every build, not only in debug builds); 0046 (D3.1: labels through user generic kinds follow the variance of each parameter); 0047 (D5: a `via` function requires nothing beyond `from`); 0048 (D9: hard links are out of model); 0051 (D3.4: the structural prelude functions label each position of their result with what it holds)

## Context

The independent review of v0.1 found 11 programs that `una check` accepts although they break the guarantees in the charter. Several of them come from gaps in the specification itself, not only from the implementation:

- the spec has no typing rule for calling a function value;
- it says nothing about instantiating generics with second-class or labeled types;
- it does not fix the program-counter label under which handler bodies are checked;
- its SMT encoding lets user names collide with encoder symbols;
- it does not reserve standard module names;
- it does not define how file grants treat symbolic links.

This ADR closes those gaps. Where a rule below and `findings.md` disagree on details, the rule decides and the finding becomes a test. A finding that reveals a case not covered here becomes a Proposed ADR, not an improvised rule.

**Meta-rule.** Every rule is chosen to be sound first and permissive second. Relaxing any of them later is allowed; tightening would break programs.

## Decision

### D1. Function types keep their effects, failures, refinements and labels everywhere (R-001)

A function type is written $(\mathrm{fn}(\bar\tau) \to \tau_r \;\mathrm{uses}\; K \;\mathrm{fails}\; F)^{\ell}$. Its `uses`, `fails`, refinements and label are part of the type in every position:
- parameters and results;
- record and variant fields;
- type arguments;
- pattern bindings;
- lambda annotations.

**Invariant.** Unification, matching, subtyping (S-Fun) and elaboration never drop any of these. A debug-build consistency check compares checker types with Core IR binder types. If anything was lost, it reports U8001. The same check enforces the "never drop a refinement" condition of ADR 0037.

**Calling a function value** follows this new rule, added to `spec/10` §5:

$$
\frac{\Gamma \vdash a_f : (\mathrm{fn}(\bar\tau) \to \tau_r \;\mathrm{uses}\; K_f \;\mathrm{fails}\; F_f)^{\ell_f} \qquad \Gamma \vdash \bar a <: \bar\tau \qquad K_f \le K \qquad pc \sqcup \ell_f \sqsubseteq \mathrm{bound}(K_f, F_f)}
{\Gamma;K;pc \vdash a_f(\bar a) : \tau_r \sqcup \ell_f \;!\; \mathrm{eff}(K_f);\, F_f}\;(\textsf{T-App})
$$

Consequences:
- **Effects and failures count.** A function value's effects and failures count for the caller exactly like a named call. A pure function cannot print through a function value, and `main` without `fails` cannot fail through one.
- **Labeled function values leak nothing (R-006).** Calling a function value labeled $\ell_f$ raises both the result and the effect check by $\ell_f$, because which function runs depends on labeled data.

### D2. Second-class types are never type arguments (R-002)

- A function type with non-empty `uses` or `fails` is second-class.
- A type parameter cannot be instantiated with a second-class type, or with any type that contains one, such as `List[fn() -> Unit uses @log: Log[write]]`. This is **U3016**, "second-class type used as a type argument".
- Together with `spec/05` §6, this guarantees that an effectful function value can never be stored, returned or carried out of its `with` scope. It always runs on the handler that was in scope where it was created.

### D3. Labels through generics and the prelude (R-004, R-005)

1. **Type arguments carry their labels.** Instantiating `T` with `secret[PII] Phone` binds the whole labeled type. Unification never drops a label.
2. **Symbolic labels inside generic bodies.** In a generic body, a value of type `T` has the symbolic label $\ell_T$. Results derived from it (comparisons, fields, arithmetic, collection operations) carry $\ell_T$ joined with the other operand labels. At a call site, $\ell_T$ is replaced by the label of the instantiated type.
3. **No effects or failures under a symbolic pc.** Branching on a value whose label contains any $\ell_T$ raises the pc symbolically. Inside such a branch, capability operations, calls with effects and failures are rejected (U4002), because their sink constraint cannot be checked generically.
4. **Prelude results.** Every prelude function whose result depends on element or argument values returns a result labeled with the join of the labels of those arguments and of the element type. `spec/14` §1 states this once, for the whole table: `contains`, `first`, `at`, `first_where`, `filter`, `map`, `fold`, `sum`, `is_empty`, `or_else`, `min`, `max`, `abs` and the text functions. Native prelude implementations must declare these labels explicitly.

### D4. Handler bodies are checked against the worst caller (R-003, D-093)

At every call site, T-Op guarantees $pc \sqcup \mathrm{label}(\bar a) \sqsubseteq \mathit{accepts}_o$. So a handler operation body is checked for the worst caller that rule allows:

- **Parameters.** Each parameter's declared type is raised to $\mathit{accepts}_o$.
- **Effect floor.** Every capability operation performed inside the body must satisfy $\mathit{accepts}_o \sqcup pc \sqcup \mathrm{label}(\bar a) \sqsubseteq \mathit{accepts}_{\mathit{inner}}$. Every call inside must satisfy $\mathit{accepts}_o \sqcup pc \sqsubseteq \mathrm{bound}(f)$. Otherwise U4002 or U4003.
- **Failures and results.** These follow ordinary data flow. The body starts at $pc = \bot$ for failure checks, and the result must fit the operation's declared return type (U4001). The caller receives both at its own pc, so they reveal nothing beyond the data that flows into them.

Consequence: a handler for an operation that accepts `internal` cannot print that data or signal it to a public sink, while fake stores that fail or return fixed values stay valid.

### D5. Policies still prove refinements (R-007)

Inside a policy's `via` function, only label premises are skipped (`spec/07` §5). Every refinement, precondition and postcondition obligation remains, including the obligation that the result satisfies the refinement of the policy's `to` type. T-Declassify may assume that refinement only because it was proven there.

### D6. Qualified identity for declarations (R-008)

- Kinds, capabilities, operations, handlers, policies and predicates are identified by their fully qualified name: module path plus name. Host capabilities live in `std.host`.
- A user capability named `Files` is a different declaration from `std.host.Files`.
- Contracts, operation signatures and assumptions are looked up by resolved declaration identity, never by bare name.

### D7. Reserved module names (R-010)

Module paths whose first segment is `std` or `prelude` are reserved for the standard library. A user module with such a path is **U1009**, "reserved module name".

### D8. Solver namespace separation (R-009)

Every SMT symbol derived from user content carries the prefix `u!`, and every encoder-internal symbol carries the prefix `i!`. Two examples:
- user field access becomes `u!f!<field>!<Kind>`;
- the variant tag becomes `i!tag!<Kind>`.

Sorts and predicates follow the same scheme. The two namespaces cannot collide by construction, and a test asserts it, using user names that equal every internal symbol. `spec/12` §4 is updated to match.

### D9. File grants and symbolic links (R-011)

`Files` operations resolve paths against the granted prefix as follows:

1. **Normalize without following links.** Reject absolute paths and `..` segments that leave the prefix.
2. **Resolve intermediate components.** Resolve every intermediate directory with its symbolic links followed. The resolved directory must lie inside the canonical grant prefix.
3. **Never follow the final component.** Open it without following a link: if it is a symbolic link, dangling or not, the operation fails with `FileError.Denied`. A write never creates a file through a link.
4. **Races are out of the model.** Races between check and use are outside the v0.1 model, since the interpreter is single-threaded and host capabilities are trusted, and the build report says so.

## Registry changes

| Code | Domain | Meaning |
| --- | --- | --- |
| U1009 | names | Reserved module name |
| U3016 | authority | Second-class type used as a type argument |

## Spec edits (in the same change as the implementation)

| File | Edit |
| --- | --- |
| `spec/10` | T-App (D1); generic instantiation rule (D2, D3); the invariant of D1 |
| `spec/05` §5, §6 | Handler checking (D4); second-class type arguments (D2) |
| `spec/07` §3, §5 | Generics and symbolic labels (D3); handler effect floor (D4); policy obligations (D5) |
| `spec/12` §4 | Namespaced encoding (D8) |
| `spec/13` §4, §7 | U1009, U3016; reserved module paths (D7) |
| `spec/14` §1, §3 | Prelude result labels (D3.4); `Files` path resolution (D9) |

## Consequences

- **All 11 Critical findings get a rule and a regression test.** The 143 adversarial programs become conformance cases with correct expectations.
- **Some programs that v0.1 accepted will be rejected:**
  - generic containers of effectful functions;
  - handlers that leak to public sinks;
  - effects under branches on generic values.

  All of these were unsound or unverifiable.
- **Label polymorphism stays limited to symbolic labels on type parameters.** A fuller treatment, with inferred label constraints in signatures, is a candidate for v0.2 if Gate 1 shows agents need it.

## Alternatives considered

- **Treat type-parameter values as `restricted` inside generic bodies.** Sound and simpler, but every generic call would return restricted data, which is unusable.
- **Check handler bodies with pc = accepts for everything.** Also sound, but it rejects legitimate handlers that fail or return fixed values. D4 keeps the conservative part only for effects.
- **Allow second-class type arguments with an escape analysis.** More expressive, but ADR 0005 chose to avoid escape analysis in v0.1.
