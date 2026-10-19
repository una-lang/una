# 11. Dynamic semantics

Status: Normative, v0.1.

## 1. The Una Abstract Machine

The Una Abstract Machine (UAM) is the reference meaning of Una programs. The v0.1 interpreter implements it directly over Core IR. Any future backend MUST refine it: for every program and host behavior, the backend's results and traces MUST be ones the UAM can produce. (v0.2, ADR 0055.) The native backend is such a refinement, and CI tests it against the interpreter on every runnable program of the repository's suites (`spec/16`). (v0.2, ADR 0065.) The tasks of a `together` block run concurrently, in any interleaving (E-Together, `spec/19 §6`); the interpreter runs them sequentially, in branch order, which is one of them.

Labels, refinements and contracts are static. The UAM carries no labels at runtime, except for the static argument labels that Core IR attaches to each operation call for trace redaction (§5).

## 2. Runtime objects

```text
values        v  ::= n | true | false | "text" | unit | C(v, ...) | [v, ...] | ⟨λ, ρ, H⟩ | hashed(bytes)
results       r  ::= ok(v) | fail(C(v, ...))
environment   ρ  ::= variable ↦ value
cap env       H  ::= binding ↦ instance
instance      ι  ::= host(C, grant, id) | handled(h, v̄, H_h, id)
trace         t  ::= [event, ...]
world         W  host state (stdout, clock, environment, files, keys)
```

Every instance has a stable `id`. Rebinding (`with c = d`) shares the instance, so events are attributed to the instance regardless of the name used at the call site.

A closure ⟨λ, ρ, H⟩ keeps both the environment and the capability environment in scope where it is made (E-Lam, §4), and its body runs under them wherever it is called (E-App): the bindings around the call do not reach it. A function named as a value is elaborated into a lambda that calls it, made where it is named, so its `uses` resolve there (`spec/05 §6` rule 7).

## 3. Judgment

$$
\rho;\, H \vdash \langle e, W \rangle \Downarrow \langle r, W', t \rangle
$$

Big-step, left-to-right, call-by-value. Traces concatenate in evaluation order ($t_1 \cdot t_2$).

## 4. Rules

$$
\frac{}{\rho;H \vdash \langle a, W\rangle \Downarrow \langle \mathit{ok}(\rho(a)), W, [\,] \rangle}\;(\textsf{E-Atom})
\qquad
\frac{\rho;H \vdash \langle r, W\rangle \Downarrow \langle \mathit{fail}(v), W', t\rangle}
     {\rho;H \vdash \langle \mathtt{let}\; x = r \;\mathtt{in}\; e, W\rangle \Downarrow \langle \mathit{fail}(v), W', t\rangle}\;(\textsf{E-LetFail})
$$

$$
\frac{\rho;H \vdash \langle r, W\rangle \Downarrow \langle \mathit{ok}(v), W_1, t_1\rangle \qquad \rho[x \mapsto v];H \vdash \langle e, W_1\rangle \Downarrow \langle r_2, W_2, t_2\rangle}
     {\rho;H \vdash \langle \mathtt{let}\; x = r \;\mathtt{in}\; e, W\rangle \Downarrow \langle r_2, W_2, t_1 \cdot t_2\rangle}\;(\textsf{E-Let})
$$

$$
\frac{\mathtt{fun}\; f(\bar x) \;\mathtt{uses}\; K_f = e \in \Sigma \qquad [\bar x \mapsto \rho(\bar a)];\; H|_{\mathrm{dom}(K_f)} \vdash \langle e, W\rangle \Downarrow \langle r, W', t\rangle}
     {\rho;H \vdash \langle f(\bar a), W\rangle \Downarrow \langle r, W', t\rangle}\;(\textsf{E-Call})
$$

$$
\frac{}{\rho;H \vdash \langle \lambda(\bar x).\, e, W\rangle \Downarrow \langle \mathit{ok}(\langle \lambda(\bar x).\, e,\, \rho,\, H \rangle), W, [\,] \rangle}\;(\textsf{E-Lam})
$$

$$
\frac{\rho(a) = \langle \lambda(\bar x).\, e,\, \rho_c,\, H_c \rangle \qquad \rho_c[\bar x \mapsto \rho(\bar b)];\; H_c \vdash \langle e, W\rangle \Downarrow \langle r, W', t\rangle}
     {\rho;H \vdash \langle a(\bar b), W\rangle \Downarrow \langle r, W', t\rangle}\;(\textsf{E-App})
$$

$$
\frac{H(c) = \mathit{host}(C, g, id) \qquad (r, W') = \llbracket C.o \rrbracket_g(\rho(\bar a), W)}
     {\rho;H \vdash \langle c.o(\bar a), W\rangle \Downarrow \langle r, W', [\,\mathit{ev}(id, C.o, \rho(\bar a), r)\,] \rangle}\;(\textsf{E-OpHost})
$$

$$
\frac{H(c) = \mathit{handled}(h, \bar v, H_h, id) \qquad \mathtt{op}\; o(\bar y) = e_o \in h \qquad [\bar p \mapsto \bar v,\, \bar y \mapsto \rho(\bar a)];\, H_h \vdash \langle e_o, W\rangle \Downarrow \langle r, W', t\rangle}
     {\rho;H \vdash \langle c.o(\bar a), W\rangle \Downarrow \langle r, W', [\,\mathit{ev}(id, C.o, \rho(\bar a), r)\,] \cdot t \rangle}\;(\textsf{E-OpHandler})
$$

$$
\frac{\rho;\, H[c \mapsto H(d)] \vdash \langle e, W\rangle \Downarrow \langle r, W', t\rangle}
     {\rho;H \vdash \langle \mathtt{with}\; c = d \;\mathtt{in}\; e, W\rangle \Downarrow \langle r, W', t\rangle}\;(\textsf{E-Rebind})
$$

$$
\frac{\iota = \mathit{handled}(h, \rho(\bar a), H|_{\mathrm{dom}(K_h)}, \mathit{fresh}) \qquad \rho;\, H[c \mapsto \iota] \vdash \langle e, W\rangle \Downarrow \langle r, W', t\rangle}
     {\rho;H \vdash \langle \mathtt{with}\; c = h(\bar a) \;\mathtt{in}\; e, W\rangle \Downarrow \langle r, W', t\rangle}\;(\textsf{E-Handler})
$$

$$
\frac{}{\rho;H \vdash \langle \mathtt{fail}\; C(\bar a), W\rangle \Downarrow \langle \mathit{fail}(C(\rho(\bar a))), W, [\,]\rangle}\;(\textsf{E-Fail})
\qquad
\frac{\rho;H \vdash \langle e, W\rangle \Downarrow \langle r, W', t\rangle}
     {\rho;H \vdash \langle \mathtt{attempt}\; e, W\rangle \Downarrow \langle \mathit{ok}(\mathrm{toResult}(r)), W', t\rangle}\;(\textsf{E-Attempt})
$$

$$
\frac{\rho;H \vdash \langle e, W\rangle \Downarrow \langle \mathit{fail}(v), W_1, t_1\rangle \qquad \rho[x \mapsto v];H \vdash \langle e_h, W_1\rangle \Downarrow \langle r, W_2, t_2\rangle}
     {\rho;H \vdash \langle \mathtt{handle}\; e \;\mathtt{with}\; x \Rightarrow e_h, W\rangle \Downarrow \langle r, W_2, t_1 \cdot t_2\rangle}\;(\textsf{E-HandleFail})
$$

$$
\frac{\mathrm{eval}(\varphi[\rho(a)/\mathit{it}]) = \mathit{true}}
     {\rho;H \vdash \langle \mathtt{checked}(a, \varphi, C(\bar b)), W\rangle \Downarrow \langle \mathit{ok}(\rho(a)), W, [\,]\rangle}\;(\textsf{E-CheckOk})
\qquad
\frac{\mathrm{eval}(\varphi[\rho(a)/\mathit{it}]) = \mathit{false}}
     {\rho;H \vdash \langle \mathtt{checked}(a, \varphi, C(\bar b)), W\rangle \Downarrow \langle \mathit{fail}(C(\rho(\bar b))), W, [\,]\rangle}\;(\textsf{E-CheckFail})
$$

The remaining rules are standard: `handle` passes through `ok`; `if` and `case` select a branch; `declassify_p(a)` evaluates the policy's `via` function on `a`; `div` and `mod` are Euclidean (the remainder is non-negative), matching SMT-LIB `div` and `mod`; `and`, `or` and `implies` do not exist in Core (`spec/09 §2`). Evaluating an opaque predicate inside `checked` runs its body; evaluating a reflect predicate runs its RL definition.

A fault that a checked and verified program cannot reach aborts the run with U8001 and exit code 3, at the name of the function in progress, or at 1:1 of the package root (ADR 0032, ADR 0022).

## 5. Traces

Each operation invocation appends one event:

```json
{
  "seq": 3,
  "instance": "given:audit",
  "capability": "Audit",
  "op": "record",
  "args": { "key": "<redacted:internal>", "status": "Open" },
  "outcome": { "kind": "ok", "value": "unit" }
}
```

- `instance` is `given:<name>` for example bindings, `host:<Capability>` for host instances and `with:<handler>#<n>` for handlers installed by `with`.
- Arguments whose static label is above `public` are printed as `<redacted:level>`.
- Handler events appear before the events produced inside the handler body (E-OpHandler).
- Host operation results are recorded in full (subject to redaction), so a future `una replay` can substitute them (`plan/backlog.md`).
- (v0.2, ADR 0065.) An event inside a task of a `together` block carries the task's path, `task`, and the events are listed in branch order; a cancelled task ends with an event of outcome `cancelled` (`spec/19 §7`).
- (Edition 0.4, ADR 0116 rule 11.) The first operation that the deadline of a `for ever within` iteration makes fail gives one event of outcome `{"kind":"timeout"}` per iteration, which names no operation and carries the `task` of the task that runs the loop, where the iteration first observes the failure (`spec/26 §7`).
- (Edition 0.4, ADR 0117 rule 5.) An `e within d` that times out appends one event of outcome `{"kind":"timeout"}`, which names no operation and carries the `task` of its task, after the outcomes of the operations it interrupted (`spec/27 §5`).
- (Edition 0.4, ADR 0117 rules 7 and 10.) The root task and the tasks of `start` write their events as they happen, numbered in the order they are written, with the paths `p.s<c>.<k>`; a started task that was cancelled ends with `cancelled`, and one that ran too deep and ended alone with `{"kind":"aborted"}`, which names no operation (`spec/27 §7`, `§8`).
- (Edition 0.4, ADR 0117 rules 6 and 10.) The events of a `first of` list the winner's first, then those of the other branches that started, in index order, with the paths `p.f<c>.<i>`; an `Http` operation of a cancelled branch that was waiting ends with the outcome `{"kind":"interrupted"}`, and names its operation (`spec/27 §6`).

Values in traces and outputs are JSON strings holding their Una form: `"42"`, `"\"Ana\""`, `"true"`, `"unit"`, `"Open"`, `"Some(3)"`, `"Registration(name: \"Ana\", event: ...)"`, `"[1, 2]"`; a `Hashed` value is `"hashed:<hex>"`, and a function `"<function>"`. Inside any rendered value, a field whose declared type is labeled above `public` is `<redacted:level>`, with the label's level only; an operation's outcome is redacted by the label of the binder of its result; `unit` carries no information and is never redacted. An event's outcome is `{"kind":"ok","value":...}`, `{"kind":"fail","variant":...,"value":...}` or `{"kind":"aborted"}` (ADR 0032).

(v0.2, ADR 0078.) Redaction follows where data come from. It applies to traces, which record what a run exchanged with its host capabilities, data from outside the program. The results of examples, and the values of a counterexample (`spec/12 §7`), come from the source and from the solver, so they are shown in full:
- In a package of edition `0.2`, `una test` renders the result of an example's `do` line, and every value in the reason it fails, without redaction at any position; a `Hashed` stays `hashed:<hex>`. The trace of an example is redacted as above.
- In a package of edition `0.1`, `una test` renders them with the field redaction above, as edition `0.1` always has.
- A counterexample shows the solver's value of every binding, in every edition. A binding whose declared type is labeled above `public`, deeply and joined with its owner's label for a field, carries that label (`spec/13 §1`), since the value is hypothetical data of a labeled type.

(v0.2, ADR 0068.) Interpolation (`spec/10 §12` rule 14) writes a value in this Una form, without redaction, since its text carries the value's labels:
- Integers are written in decimal, `Bool` as `true` or `false`, and `Unit` as `unit`.
- A variant without fields is written as its name. A variant or record with one field is `Name(value)`, and one with several fields is `Name(f: value, g: value)`, in declaration order.
- A list is `[a, b]`, and an empty one is `[]`. `Option` and `Result` are written as variants: `Some(3)`, `None`, `Ok("x")`, `Err(NotFound)`.
- A `Text` inside a structure is quoted with the escapes of `spec/01 §6`. A `Text` interpolated directly is not quoted.
- (2026-10-06, ADR 0068 amendment.) A `Hashed`, at any position and through a type parameter, is written `<hashed>`: interpolation writes no bytes of a hash. Traces and diagnostics keep `hashed:<hex>`.
- The text depends on the value alone. The interpreter and a native backend MUST write the same text (`spec/16 §5`).

## 6. Example harness

For each example, the harness builds `H0` from its `given` lines (one fresh instance per line), evaluates the desugared `do` expression with an empty environment and an empty world, then evaluates the expectations against the result and trace (`spec/08 §2`). Examples cannot reach host capabilities, so they are deterministic.

## 7. Limits

The interpreter enforces a maximum call depth (default 10,000 frames, `--max-depth`). Exceeding it aborts with runtime diagnostic U6003. This is outside the noninterference model (`spec/07 §1`). The depth counts the calls of functions, predicates, lambdas, policies and handler operations in progress. The evaluator runs with a stack sized from the maximum depth, capped at 1 GiB, and aborts with U6003 when that stack runs short before the depth does, so it never overflows (ADR 0032, ADR 0061).

(v0.2, ADR 0060.) A value has no depth limit, and only calls count toward the call depth. A value can be deeper than the call depth, since a `for` loop builds it without calls (`spec/17 §4`). Releasing, comparing, rendering and tracing a value MUST NOT recurse on the native stack over its structure, in the interpreter and in a native backend's runtime (`spec/16`).

(v0.2, ADR 0061.) A run also has a memory budget (default 4096 MiB, `--max-memory <MiB>`, at least 1): the heap it holds beyond what the process held when it started.
- **Checks.** The budget is checked at each call that the depth counts, so at each application in the prelude's higher-order functions, and at each iteration of a `for` loop. (v0.2.) `parse_json` and `to_json` also check it as they read and write, every 4096 values and every 4096 scalar values of a string, at their call (`spec/23 §3` rule 6). (v0.2, ADR 0113, 2026-10-15.) A native executable also checks the budget before each allocation of the copies a `together` block makes of its tasks' inputs and results (`spec/16 §3` rule 3, `spec/19 §8`). Over the budget, it aborts with U6003 at the block, with the message above. (v0.2, ADR 0113, 2026-10-16.) It also checks the budget before it allocates a block's result list, the state of each batch of its tasks, each start of a task's thread and each adoption of a batch's events, and aborts the same way. (v0.2, ADR 0061, 2026-10-17.) A structural comparison (`==`, `!=` and `contains`, on values that are not scalars) compares leaves as it meets them and each distinct pair of values with parts once, and checks before each step the pairs of values with parts it holds, compared and still to compare, counted at 128 bytes each in both engines. Over the budget, it aborts with U6003 at the comparison, with the message above. The count depends on the two values and the parts they share, never on the allocator; since each engine builds and copies values its own way, the point of U6003 for one program can differ between the engines, as for the other checks (see Determinism below).
- **Over the budget.** A call or an iteration that finds the run over its budget aborts with U6003, `this call exceeds the memory limit of N MiB`, with the trace and status of a depth abort.
- **Hard ceiling.** An allocation beyond twice the budget between two checks aborts the process.
- **Determinism.** The depth limit is exact. The point where memory runs out is not, and it is outside the model. The guarantee is that a run over its budget ends with U6003, provided the machine can supply the budget.
