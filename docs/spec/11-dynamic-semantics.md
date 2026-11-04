# 11. Dynamic semantics

Status: Normative, v0.1.

## 1. The Una Abstract Machine

The Una Abstract Machine (UAM) is the reference meaning of Una programs. The v0.1 interpreter implements it directly over Core IR. Any future backend MUST refine it: for every program and host behavior, the backend's results and traces MUST be ones the UAM can produce. (v0.2, ADR 0055.) The native backend is such a refinement, and CI tests it against the interpreter on every runnable program of the repository's suites (`spec/16`).

Labels, refinements and contracts are static. The UAM carries no labels at runtime, except for the static argument labels that Core IR attaches to each operation call for trace redaction (§5).

## 2. Runtime objects

```text
values        v  ::= n | true | false | "text" | unit | C(v, ...) | [v, ...] | ⟨λ, ρ⟩ | hashed(bytes)
results       r  ::= ok(v) | fail(C(v, ...))
environment   ρ  ::= variable ↦ value
cap env       H  ::= binding ↦ instance
instance      ι  ::= host(C, grant, id) | handled(h, v̄, H_h, id)
trace         t  ::= [event, ...]
world         W  host state (stdout, clock, environment, files, keys)
```

Every instance has a stable `id`. Rebinding (`with c = d`) shares the instance, so events are attributed to the instance regardless of the name used at the call site.

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

Values in traces and outputs are JSON strings holding their Una form: `"42"`, `"\"Ana\""`, `"true"`, `"unit"`, `"Open"`, `"Some(3)"`, `"Registration(name: \"Ana\", event: ...)"`, `"[1, 2]"`; a `Hashed` value is `"hashed:<hex>"`, and a function `"<function>"`. Inside any rendered value, a field whose declared type is labeled above `public` is `<redacted:level>`, with the label's level only; an operation's `ok` value is redacted by the label of the binder of its result; `unit` carries no information and is never redacted. An event's outcome is `{"kind":"ok","value":...}`, `{"kind":"fail","variant":...,"value":...}`, `{"kind":"redacted","level":...}` or `{"kind":"aborted"}` (ADR 0032).

(S-45, #1552, ADR 0032 amendment, proposed (`docs/plan/0032-outcome-redaction-amendment-draft.md`).) In the trace of `una run`, an operation's outcome is rendered at its **outcome label**: the label of the decision whether the operation fails and of its payload (`spec/07 §7` rule 4: the deciding arguments capped at `accepts`, and the file-state label for `Files`, ADR 0085), joined with the labels of its failure type and of its result's binder. Core IR carries it on each operation (`spec/09 §4`). When it is above `public`, the outcome is `{"kind":"redacted","level":"<level>"}`, with its level only: whether the operation returned or failed, the variant, the payload and the value are not written, and no text is built from the value. Whether an outcome is redacted depends on the operation site alone, never on a value. An operation in a body that no label rule checks, a policy's `via` function, has the outcome label `⊤`, and so has one the checker left no label for. `aborted` is not a decision of the operation and keeps its kind. So a later operation's outcome no longer shows what an `internal` value decided, such as the `internal` path of a `Files.write`, which its own event shows as `<redacted:internal>`: in a package whose file-state label is `internal`, every `Files` outcome is redacted. The program could not observe these outcomes either (U4003 on a branch that prints); only the trace did. This rule covers what an event shows, not whether it occurs: each operation invocation still appends one event, so an operation under a branch on an `internal` value shows by occurrence, and which events appear, how many, and the `seq` of later events can depend on that value on this line. The trace of an example (§6) keeps its outcomes in clear, as rendered above, since an example reaches no host capability and its data come from its source.

## 6. Example harness

For each example, the harness builds `H0` from its `given` lines (one fresh instance per line), evaluates the desugared `do` expression with an empty environment and an empty world, then evaluates the expectations against the result and trace (`spec/08 §2`). Examples cannot reach host capabilities, so they are deterministic.

## 7. Limits

The interpreter enforces a maximum call depth (default 10,000 frames, `--max-depth`). Exceeding it aborts with runtime diagnostic U6003. This is outside the noninterference model (`spec/07 §1`). The depth counts the calls of functions, predicates, lambdas, policies and handler operations in progress. The evaluator runs with a stack sized from the maximum depth, and aborts with U6003 when that stack runs short before the depth does, so it never overflows (ADR 0032).
