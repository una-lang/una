# 11. Dynamic semantics

Status: Normative, v0.1.

## 1. The Una Abstract Machine

The Una Abstract Machine (UAM) is the reference meaning of Una programs. The v0.1 interpreter implements it directly over Core IR. Any future backend MUST refine it: for every program and host behavior, the backend's results and traces MUST be ones the UAM can produce.

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

## 6. Example harness

For each example, the harness builds `H0` from its `given` lines (one fresh instance per line), evaluates the desugared `do` expression with an empty environment and an empty world, then evaluates the expectations against the result and trace (`spec/08 §2`). Examples cannot reach host capabilities, so they are deterministic.

## 7. Limits

The interpreter enforces a maximum call depth (default 10,000 frames, `--max-depth`). Exceeding it aborts with runtime diagnostic U6003. This is outside the noninterference model (`spec/07 §1`).
