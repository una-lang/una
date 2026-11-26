# 10. Static semantics

Status: Normative, v0.1. Rules are defined over Core IR (`spec/09`). Surface diagnostics are reported against the source spans that Core nodes carry.

(v0.5, ADR 0137.) **Binding words.** In edition `0.5`, a binding of the surface language that this chapter writes `let` is written `const`, and one it writes `let edit` is written `var` (`spec/02 §6`, `spec/17 §2`); the rules are unchanged. A Core `let` (`spec/09`) stays. The chapter's Una blocks keep the words of edition `0.4` until issue #1630 moves those of edition `0.5`.

## 1. Judgments

$$
\Gamma;\, K;\, pc \vdash e : \tau \;!\; \varepsilon ;\, F
$$

Under variable context $\Gamma$ (bindings and path facts), capability context $K$ and program-counter label $pc$, expression $e$ has type $\tau$, may perform effects $\varepsilon$ and may fail with error kind $F$ ($\emptyset$ if it cannot fail). A global signature $\Sigma$ of declarations is implicit.

- Effects $\varepsilon$ are sets of pairs $c.o$: operation $o$ on capability binding $c$.
- $\mathrm{eff}(K) = \{ c.o \mid c \mapsto C[O] \in K,\; o \in O \}$.
- $F_1 \sqcup F_2$ is defined when $F_1 = \emptyset$, $F_2 = \emptyset$ or $F_1 = F_2$; otherwise the program is ill-typed (U3008 or U3009).
- $\ell(\tau)$ is the label of $\tau$; $\tau \sqcup \ell$ raises the label of $\tau$ by $\ell$.
- $\Gamma \vDash \varphi$ is an **obligation**: it is discharged by the verifier (`spec/12`), not by the type checker. A rule with an obligation premise type-checks and emits the obligation.

## 2. Subtyping

$$
\frac{\Gamma \vDash \varphi_1 \Rightarrow \varphi_2 \qquad \ell_1 \sqsubseteq \ell_2}
     {\Gamma \vdash \{\nu{:}b \mid \varphi_1\}^{\ell_1} <: \{\nu{:}b \mid \varphi_2\}^{\ell_2}}\;(\textsf{S-Refine})
$$

Capabilities are not values, so they have no subtyping. Instead, **sufficiency** relates the capabilities a function needs to those a context has:

$$
K_{\mathit{need}} \le K_{\mathit{have}} \iff \forall\, c \mapsto C[O] \in K_{\mathit{need}}.\;\; c \mapsto C[O'] \in K_{\mathit{have}} \,\wedge\, O \subseteq O' \qquad (\textsf{Suff})
$$

$$
\frac{\Gamma \vdash \bar\tau_2 <: \bar\tau_1 \quad \Gamma \vdash \tau_r <: \tau_r' \quad K_1 \le K_2 \quad F_1 \subseteq F_2 \quad \ell \sqsubseteq \ell'}
     {\Gamma \vdash (\mathrm{fn}(\bar\tau_1) \to \tau_r \;\mathrm{uses}\; K_1 \;\mathrm{fails}\; F_1)^{\ell} <: (\mathrm{fn}(\bar\tau_2) \to \tau_r' \;\mathrm{uses}\; K_2 \;\mathrm{fails}\; F_2)^{\ell'}}\;(\textsf{S-Fun})
$$

The refined premises of S-Fun, $\bar\tau_2 <: \bar\tau_1$ and $\tau_r <: \tau_r'$ over refined parameters and results, are the `refinement` obligations of function types of `spec/12 §3` (ADR 0037): the type checker compares bases, and the verifier discharges the refinements.

A function type's `uses`, `fails`, refinements and label are part of the type in every position: parameters and results, record and variant fields, type arguments, pattern bindings and lambda annotations (ADR 0038 D1). Unification, matching, S-Fun and elaboration never drop any of them, and S-Fun applies wherever a value flows into a declared type, recursively under `List`, `Option`, `Result` and kind arguments. As an internal invariant, the compiler compares the types of the checker with the binder types of Core IR; a component that was lost is U8001, never a silent pass.

## 3. Atoms and primitives

$$
\frac{x : \{\nu{:}b \mid \varphi\}^{\ell} \in \Gamma}
     {\Gamma;K;pc \vdash x : \{\nu{:}b \mid \nu = x\}^{\ell} \;!\; \emptyset;\emptyset}\;(\textsf{T-Var})
\qquad
\frac{}{\Gamma;K;pc \vdash n : \{\nu{:}\mathrm{Int} \mid \nu = n\}^{\bot} \;!\; \emptyset;\emptyset}\;(\textsf{T-Int})
$$

$$
\frac{\Gamma \vdash a_i : \{\nu{:}\mathrm{Int} \mid \_\}^{\ell_i} \quad (\text{if } \oplus \in \{\mathrm{div}, \mathrm{mod}\}:\; \Gamma \vDash a_2 \neq 0)}
     {\Gamma;K;pc \vdash \oplus(a_1, a_2) : \{\nu{:}\mathrm{Int} \mid \nu = a_1 \oplus a_2\}^{\ell_1 \sqcup \ell_2} \;!\; \emptyset;\emptyset}\;(\textsf{T-Arith})
$$

Comparisons, `not`, `len`, `concat` and `to_text` follow the same pattern: the result label is the join of operand labels, and RL-expressible results are selfified.

## 4. Binding, branching, data

$$
\frac{\Gamma;K;pc \vdash r : \tau_1 \;!\; \varepsilon_1;F_1 \qquad \Gamma, x{:}\tau_1;K;pc \vdash e : \tau_2 \;!\; \varepsilon_2;F_2 \qquad \Gamma \vdash \tau_1 <: \tau_x}
     {\Gamma;K;pc \vdash \mathtt{let}\; x{:}\tau_x = r \;\mathtt{in}\; e : \tau_2 \;!\; \varepsilon_1 \cup \varepsilon_2;\, F_1 \sqcup F_2}\;(\textsf{T-Let})
$$

$$
\frac{\Gamma \vdash a : \mathrm{Bool}^{\ell} \qquad \Gamma, a;\, K;\, pc \sqcup \ell \vdash e_1 : \tau \;!\; \varepsilon;F \qquad \Gamma, \neg a;\, K;\, pc \sqcup \ell \vdash e_2 : \tau \;!\; \varepsilon;F}
     {\Gamma;K;pc \vdash \mathtt{if}\; a \;\mathtt{then}\; e_1 \;\mathtt{else}\; e_2 : \tau \sqcup \ell \;!\; \varepsilon;F}\;(\textsf{T-If})
$$

$$
\frac{\Gamma \vdash a : T[\bar\tau]^{\ell} \qquad \forall i.\;\; \Gamma,\, \bar x_i{:}\bar\tau_i,\, (a\; \mathtt{is}\; C_i);\, K;\, pc \sqcup \ell \vdash e_i : \tau \;!\; \varepsilon;F \qquad \{C_i\} \text{ exhaustive (or else-branch)}}
     {\Gamma;K;pc \vdash \mathtt{case}\; a \;\mathtt{of}\; \{ C_i(\bar x_i) \Rightarrow e_i \} : \tau \sqcup \ell \;!\; \varepsilon;F}\;(\textsf{T-Case})
$$

Exhaustiveness over variants is checked syntactically (U2007). Exhaustiveness of `if` chains produced by range arms is an obligation (`spec/12 §4`, kind `exhaustive`).

## 5. Calls, capabilities, failures

$$
\frac{\begin{array}{c}
f : \forall\bar\alpha.\, (\bar x{:}\bar\tau) \to \tau_r \;\mathrm{uses}\; K_f \;\mathrm{fails}\; F_f \;\mathrm{requires}\; \varphi \;\mathrm{ensures}\; \psi \in \Sigma \\
\Gamma \vdash \bar a <: \bar\tau[\bar\sigma/\bar\alpha] \qquad \Gamma \vDash \varphi[\bar a/\bar x] \qquad K_f \le K \qquad pc \sqsubseteq \mathrm{bound}(f)
\end{array}}
{\Gamma;K;pc \vdash f[\bar\sigma](\bar a) : \{\nu{:}b_r \mid \psi[\bar a/\bar x,\, \nu/\mathit{result}]\}^{\ell(\tau_r)} \;!\; \mathrm{eff}(K_f);\, F_f}\;(\textsf{T-Call})
$$

$$
\frac{\begin{array}{c}
c \mapsto C[O] \in K \qquad o \in O \qquad \mathit{sig}(C.o) = (\bar x{:}\bar\tau) \to \tau_r \;\mathrm{fails}\; F_o \;\mathrm{requires}\; \varphi \;\mathrm{ensures}\; \psi \;\mathrm{accepts}\; \ell_o \\
\Gamma \vdash \bar a <: \bar\tau^{\top} \qquad \Gamma \vDash \varphi[\bar a/\bar x] \qquad pc \sqcup \textstyle\bigsqcup_i \ell(a_i) \sqsubseteq \ell_o
\end{array}}
{\Gamma;K;pc \vdash c.o(\bar a) : \{\nu{:}b_r \mid \psi[\bar a/\bar x,\, \nu/\mathit{result}]\}^{\ell(\tau_r)} \;!\; \{c.o\};\, F_o}\;(\textsf{T-Op})
$$

In T-Op, $\tau^{\top}$ is $\tau$ with every covariant position labeled $\top$: the arguments' data is bounded by $\ell_o$, while contravariant positions and the arguments of invariant kind parameters keep their declared labels (`spec/07 §7` rule 4, ADR 0050).

$\mathrm{bound}(f)$ is the meet of the `accepts` labels of every operation in $\mathrm{eff}(K_f)$, met with $\bot$ if $F_f \neq \emptyset$; it is $\top$ for pure, non-failing $f$ (`spec/07 §3`). $\mathrm{bound}(K, F)$ is the same meet for a capability context and an error kind.

A call of a function value, a parameter, local or field of function type, follows T-App (ADR 0038 D1):

$$
\frac{\Gamma \vdash a_f : (\mathrm{fn}(\bar\tau) \to \tau_r \;\mathrm{uses}\; K_f \;\mathrm{fails}\; F_f)^{\ell_f} \qquad \Gamma \vdash \bar a <: \bar\tau \qquad K_f \le K \qquad pc \sqcup \ell_f \sqsubseteq \mathrm{bound}(K_f, F_f)}
{\Gamma;K;pc \vdash a_f(\bar a) : \tau_r \sqcup \ell_f \;!\; \mathrm{eff}(K_f);\, F_f}\;(\textsf{T-App})
$$

A function value's effects and failures count for the caller exactly like those of a named call, so a pure function cannot perform an effect through a function value, and a function without `fails` cannot fail through one. Which function runs depends on the value, so its label $\ell_f$ raises both the result and the program counter of the call.

**Generic instantiation** (ADR 0038 D2, D3). In T-Call, each $\sigma_i$ that instantiates a type parameter $\alpha_i$ is well formed:

1. A function type with non-empty `uses` or `fails` is **second-class**. No $\sigma_i$ is a second-class type or contains one, such as `List[fn() -> Unit uses @log: Log[write]]` (U3016). Together with §9 rule 2, an effectful or failing function value is never stored, returned or carried out of the `with` scope where it was created.
2. $\sigma_i$ binds the whole labeled type, and unification never drops a label.
3. In the generic body, a value of type $\alpha$ has the symbolic label $\ell_\alpha$ (`spec/07 §7` rule 8). At the call, every $\ell_{\alpha_i}$ in the result's type is replaced by $\ell^{*}(\sigma_i)$, the join of every label of $\sigma_i$.
   - (v0.2, ADR 0067.) In edition `0.2`, for a function declared outside the prelude, the replacement is per position: each position $p$ of the declared result type is raised by $\ell^{*}(\sigma_i)$ for each $\alpha_i$ in the function's result summary $S_f(p)$ (`spec/07 §7` rule 8), and by no other. Edition `0.1` raises every position that holds data by the join of $\ell^{*}(\sigma_i)$ over all the type arguments (D-137).

$$
\frac{C \in \mathrm{variants}(F) \qquad pc = \bot \qquad \Gamma \vdash \bar a <: \mathrm{fields}(C)}
     {\Gamma;K;pc \vdash \mathtt{fail}\; C(\bar a) : \tau \;!\; \emptyset;\, F}\;(\textsf{T-Fail})
\qquad
\frac{\Gamma;K;pc \vdash e : \tau \;!\; \varepsilon;F}
     {\Gamma;K;pc \vdash \mathtt{attempt}\; e : \mathrm{Result}[\tau, F] \;!\; \varepsilon;\emptyset}\;(\textsf{T-Attempt})
$$

$$
\frac{\Gamma;K;pc \vdash e : \tau \;!\; \varepsilon_1;F_1 \qquad \Gamma, x{:}F_1;K;pc \vdash e_h : \tau \;!\; \varepsilon_2;F_2}
     {\Gamma;K;pc \vdash \mathtt{handle}\; e \;\mathtt{with}\; x \Rightarrow e_h : \tau \;!\; \varepsilon_1 \cup \varepsilon_2;\, F_2}\;(\textsf{T-Handle})
$$

## 6. Capability bindings

$$
\frac{d \mapsto C[O] \in K \qquad \Gamma;\, K[c \mapsto C[O]];\, pc \vdash e : \tau \;!\; \varepsilon;F}
     {\Gamma;K;pc \vdash \mathtt{with}\; c = d \;\mathtt{in}\; e : \tau \;!\; \varepsilon[d/c];F}\;(\textsf{T-Rebind})
$$

$$
\frac{\mathtt{handler}\; h(\bar y{:}\bar\tau)\; \mathtt{for}\; C \;\mathtt{uses}\; K_h \in \Sigma \qquad K_h \le K \qquad \Gamma \vdash \bar a <: \bar\tau \qquad \Gamma;\, K[c \mapsto C[\mathrm{ops}(C)]];\, pc \vdash e : \tau \;!\; \varepsilon;F}
     {\Gamma;K;pc \vdash \mathtt{with}\; c = h(\bar a) \;\mathtt{in}\; e : \tau \;!\; (\varepsilon \setminus c.\ast) \cup \mathrm{eff}(K_h);F}\;(\textsf{T-Handler})
$$

## 7. Escape hatches

$$
\frac{\Gamma \vdash a : \{\nu{:}b \mid \varphi_0\}^{\ell} \qquad \varphi \in \mathrm{RL} \cup \mathrm{Opaque} \qquad C \in \mathrm{variants}(F) \qquad pc \sqcup \ell^{*}(a) = \bot}
     {\Gamma;K;pc \vdash \mathtt{checked}(a, \varphi, C) : \{\nu{:}b \mid \varphi_0 \wedge \varphi\}^{\ell} \;!\; \emptyset;F}\;(\textsf{T-Checked})
$$

$$
\frac{\mathtt{policy}\; p : \tau_{\mathit{from}} \leadsto \tau_{\mathit{to}} \;\mathtt{via}\; f \in \Sigma \qquad \Gamma \vdash a <: \tau_{\mathit{from}} \qquad pc \sqsubseteq \ell(\tau_{\mathit{to}}) \qquad K_f \le K}
     {\Gamma;K;pc \vdash \mathtt{declassify}_p(a) : \tau_{\mathit{to}} \;!\; \mathrm{eff}(K_f);F_f}\;(\textsf{T-Declassify})
$$

In T-Checked, $\ell^{*}(a)$ joins every label of the type of $a$: a failure of `checked` reveals whether $\varphi$ holds of $a$, so `checked` on a labeled value, or under a raised program counter, is U4002 (ADR 0031).

## 8. Declarations

$$
\frac{\bar x{:}\bar\tau,\, \varphi;\;\; K_f;\;\; \bot \vdash e : \tau' \;!\; \varepsilon;F \qquad \bar x{:}\bar\tau,\varphi \vdash \tau' <: \{\nu{:}b_r \mid \psi[\nu/\mathit{result}]\}^{\ell(\tau_r)} \qquad \varepsilon \subseteq \mathrm{eff}(K_f) \qquad F \subseteq F_f}
     {\vdash \mathtt{fun}\; f(\bar x{:}\bar\tau) : \tau_r \;\mathrm{uses}\; K_f \;\mathrm{fails}\; F_f \;\mathrm{requires}\; \varphi \;\mathrm{ensures}\; \psi = e \;\; \mathsf{ok}}\;(\textsf{T-Fun})
$$

Handler operations are checked like functions whose parameters, `requires`, `ensures`, `fails` and result type come from the capability's operation signature and whose capability context is the handler's `uses`. They are checked against the worst caller T-Op allows: every parameter is raised to the operation's `accepts` label $\ell_o$, and every effect in the body satisfies the effect floor $\ell_o$ (`spec/07 §7` rule 4, ADR 0038 D4). Private functions without declared `uses` or `fails` take the inferred least solution (`spec/04 §5`) before T-Fun is applied.

(v0.5, ADR 0126 rules 3 to 7, 13 and 14.) **Module constants (T-Const).** A module constant `[pub] const n: T = e` (`spec/02 §1`) is checked as an annotated local `const` of type `T` with no local in scope, with these additions:

1. **Constant expressions.** `e` MUST be one of: a literal (an integer, a float, a decimal, a text without interpolation, `true`, `false`, `unit`); `-` on a constant expression of a numeric type; `+`, `-`, `*`, `/` and `mod` on two of one integer type, `+`, `-`, `*` and `/` on two of type `F64`, and `+` and `-` on two decimals of one scale; `and`, `or`, `not`, `==`, `!=`, `<`, `<=`, `>` and `>=` where they are defined; a constructor of a record or a variant whose field values are constant expressions; a list literal of constant expressions; parentheses; the name of a constant of the module or an imported one. Anything else, as a call, a method call, a lambda, `if`, `when`, a field access, `checked`, `attempt`, `fail`, `result`, `it`, `error`, `implies` or an interpolated text, is U2041 at the expression, naming the construct; a capability reference is U3005. The set is closed.
2. **Evaluation.** The compiler evaluates each value once, at check time, in the order of the constants' dependencies, by the dynamic semantics of `spec/11` for those operators: integers with arbitrary precision and Euclidean division, `F64` as `spec/29` defines each operation, decimals exactly. A division or `mod` by zero, a value at a fixed-width type that does not fit it (`spec/18 §2`), an integer of more than 4,300 decimal digits, and a value of more than 1,000,000 elements, fields and characters together are U2041 at check time, never an obligation. Nothing else fails.
3. **Typing.** The value's type MUST flow into `T` as at an annotated `const`: U2001, U2028 (units, `spec/25 §3`) or U4001 (labels) otherwise. `T` MUST NOT be or hold, at any depth, a field of a kind included, a function type: U2041 at the type. A reference to the constant has type `T`, its refinements at the top as for any annotated binder.
4. **Refinements.** A refinement of `T`, at the top or at a nested position, gives its `refinement` obligation at the value (`spec/12 §3`), whose goal is closed.
5. **Ownership.** A constant is neither an editable binding nor a local binding (`spec/17 §2`, `§3`): an assignment to it and an `edit` argument rooted at it are U2019, and a `take` of it is U2020.

## 9. Syntactic side conditions

1. Capability names $c$ occur only as the receiver of `c.o(...)` and in `with` (U3005). They are never atoms.
2. A lambda whose type has non-empty `uses` or `fails` occurs only directly as a call argument (U3006), and such a type is never a type argument (U3016, §5).
3. Inside the `via` function of a policy, label premises ($\ell \sqsubseteq \ell'$, $pc$ conditions) are not checked (`spec/07 §5`).

## 10. Metatheory targets

These theorems define what `proven` means. They are stated here and mechanized in Lean 4 on a separate track (`plan/roadmap.md §2`).

| # | Theorem | Statement |
| --- | --- | --- |
| T1 | Type safety | A closed well-typed Core expression is a value or a `fail`, or it steps; steps preserve typing |
| T2 | Effect soundness | Every event $c.o$ in a trace of $e$ is in $\varepsilon$ |
| T3 | Failure soundness | If $e$ fails with $C$, then $C \in \mathrm{variants}(F)$ |
| T4 | Capability confinement | Every event acts on a binding reachable from $K$ through `with` bindings in scope; no expression creates authority |
| T5 | Refinement soundness | If $\vdash v : \{\nu{:}b \mid \varphi\}$ and all obligations are proven, then $\varphi[v/\nu]$ holds (opaque facts: established by `checked` or an operation `ensures`, an assumption) |
| T6 | Noninterference | Termination-insensitive noninterference up to declassification, for the channels in `spec/07 §1`. (v0.2, ADR 0065.) With `together`, for the result of each block and for each task's own events (`spec/19 §3`) |
| T7 | Decidability | Type checking is decidable; obligation checking is decidable for RL (`spec/12 §2`) |

## 11. Names

Names resolve before typing, by these rules (ADR 0023).

1. **Namespaces.** Each module has a lower-case namespace (functions, predicates, handlers, policies, and, in edition `0.5`, module constants, ADR 0126 rule 2) and an upper-case one (kinds, their variants, capabilities). Requirement identifiers form one namespace per package, example names one per module, and module paths one per package plus the standard modules of rule 10.
2. **Scopes.** From innermost to outermost: locals (parameters, type parameters, `let`, pattern bindings, lambda parameters, handler and handler-operation parameters); the module's own declarations; its explicit imports; the variants of the sum kinds it imports, since importing a sum kind brings its variants; the prelude (`spec/14 §1, §3`). A name resolves if some scope holds it, and the innermost holder wins. When one scope holds several candidates, such as variants of two imported kinds with the same name, the type checker chooses by type; a name that stays ambiguous is U1001.
3. **Variants in context.** A variant whose kind the context gives is resolved by that kind, whether or not any scope holds it: in a `when` pattern or an `is` test (the kind of the value tested), after `fail` in an expression, a handling clause or `checked` (the error kind of the function), and after `expect fails` (the error kind of the `do` expression).
4. **Unresolved names.** U1001 reports a name that no scope holds: in expressions, callees, constructors outside the contexts of rule 3, type names and type parameters, capability names in `uses` and `for`, handler names in `with` and `given`, the `via` function of a policy, and imports (a module that is neither of the package nor one of the standard modules of rule 10, or a name the module does not declare). U2002 reports a name that resolves to something that is not a type, or an ill-formed type. Field names, method names and argument labels depend on types (§12), capability references `@x` on capability contexts, and label categories on `una.toml` (U4007).
5. **Duplicates.** U1002 reports: two declarations of a module with the same name in one namespace, variants included; an explicit import with the name of a declaration of the module or of another explicit import; a local that rebinds a local of an enclosing scope; two parameters, type parameters, fields, variant fields, operations, handler operations, pattern bindings or lambda parameters of one construct with the same name; two requirements of a package or two examples of a module with the same name; two label categories in `una.toml`. A declaration or import MAY reuse a prelude name, and a local MAY reuse any non-local name: the inner one wins.
6. **Imports.** An imported name MUST be public (U1004, with an edit that adds `pub` to the declaration). An import cycle is U1003, reported once per cycle at the first import of the cycle's first module, in path order. (v0.5, ADR 0126 rule 4.) A module constant MAY name constants declared before or after it; a constant that depends on itself, directly or through others, is U1011, once per cycle, at the first constant of the cycle in path order.
7. **Syntax errors.** In a module with syntax errors, the declarations that parsed are registered and checked for U1002, U1005 and U1008, but bodies are not resolved and imports from that module are not checked, so no diagnostic cascades from a declaration that failed to parse.
8. **Identity** (ADR 0038 D6, D7). Kinds, capabilities, operations, handlers, policies and predicates are identified by their fully qualified name, the module path followed by the name. The host capabilities of `spec/14 §3` are declared in the module `std.host`, so a capability `Files` declared by a user module is a different declaration from `std.host.Files`. Contracts, operation signatures and assumptions are looked up by resolved identity, never by bare name. A module path whose first segment is `std` or `prelude` is reserved for the standard library: a user module with such a path is U1009 at its `module` header.
9. **Conventions (U1005)**, checked where a name is defined: lower-case names (declarations, parameters, locals, fields, operations, module path segments, owners) contain no `__` and do not end with `_`; kinds, variants, capabilities, type parameters and label categories are UpperCamelCase (`[A-Z][a-zA-Z0-9]*`); requirement identifiers match `[A-Z][A-Z0-9]*(_[A-Z0-9]+)*_[0-9]+` (`spec/01 §4`).
10. **Standard modules** (#16). In editions `0.1` to `0.3`, two standard modules can be imported: `std.host`, which exports the host capabilities and their failure kinds (`Console`, `Clock`, `Env`, `Files`, `Keys`, `FileError` and `KeyError`, `spec/14 §3`, and in edition 0.2 also `Http`, `Header`, `Response` and `HttpError`, `spec/24 §1`, #307), and `std.testing` (`spec/14 §4`). An import from `std.host` names the same declarations that the prelude puts in scope, so `import std.host (Files)` changes nothing but makes the source explicit; it follows rules 5 and 6 as any import does. The prelude is always in scope and is not a module that can be imported: an import of any other `std` path, or of `prelude`, is U1001 at the module path. (v0.5, ADR 0119 rule 0, moved from edition `0.4` by ADR 0139.) In edition `0.5`, `std.collections` can be imported too (`spec/28 §2`); in earlier editions an import of it is U1001 at the module path. (v0.4, ADR 0116 rules 0 and 2.) In edition `0.4`, `std.time` and `std.drive` can be imported too (`spec/26 §2`); in earlier editions an import of either is U1001 at the module path. (v0.4, ADR 0117 rule 9.) In edition `0.4`, `std.limit` can be imported too (`spec/27 §10`), and is read only by a package that imports it; in earlier editions an import of it is U1001 at the module path. (v0.4, ADR 0123 rule 0.) In edition `0.4`, `std.host` also exports `HttpServer`, `Method`, `Param`, `Request`, `Exchange`, `Status`, `Reply`, `ServeError` and `ReplyRight` (`spec/31 §1`), which the prelude does not put in scope: only an import brings one into scope, so the import is not redundant, and a kind of the package named `Request` or `Reply` stays legal in every edition. An import of `Method` or `ServeError` brings its variants, as an import of a kind of the package does. In earlier editions an import of these names is U1001, as an unknown name is (rule 4).

## 12. Surface typing

The rules of §3 to §8 act on Core IR; these rules fix what the surface syntax elaborates to, and what it reports (ADR 0024).

1. **Variants in context.** In a `when` pattern or an `is` test, the variant is looked up in the kind of the tested value: U1001 if the kind has no such variant, U2001 if the value is not of a sum kind. After `fail` (in an expression, a handling clause or `checked`), the variant is looked up in the function's declared error kind, and through the scopes of §11 when the function declares none; a variant outside the declared kind is U3009. After `expect fails`, the variant is looked up in the error kind of the `do` expression: a variant outside it is U1001, and when the `do` expression cannot fail, because its error kind is empty, as after `attempt` (D-170), `expect fails V` never holds and is U1001 at `V`, with a help that proposes removing the line (ADR 0044).
2. **Branches and `fail`.** `fail` has a type below every type, so a branch that fails joins any other. The branches of `if` and `when` otherwise have one base type; a target type, when known, is checked in each branch.
3. **Unreachable and repeated arms.** U2008 reports an arm after an `else` arm, a variant or literal arm that repeats an earlier one, and an `else` arm that no value reaches because the arms before it cover every variant. A literal arm, and the literal of an `is` test, is a constant: a text literal with interpolation there is U4008, in every edition (`spec/07 §7` rule 3, S-28).
4. **Field access and method calls.** `e.f` is `.len` on `Text` and `List`, then field `f` of `e`'s record, then a call of a function `f` of one parameter in scope (`spec/04 §3`); if none applies, U1001. `e.f(args)` follows `spec/04 §3`: a field of function type, else a function `f` in scope called with `e` first; if neither exists, U1001.
5. **Function values** (parameters and locals of function type, lambdas) are called with positional arguments; a label is U2005.
6. **Constructors are not values.** A variant or record kind that has fields MUST be applied to arguments; using its name alone is U2009.
7. **Inference.** A type that is still unknown when its declaration has been checked, such as the element type of an empty list that nothing constrains, is U2001 with a hint to annotate it. Lambda arguments are checked after the other arguments of a call, so their parameter types come from the other arguments whatever the order of labels; a value whose type is still unknown where a field, a method or a call needs it is U2001.
8. **Recursive kinds.** A record or sum kind that contains itself anywhere except under `List` or `Option`, including through a variant, a type argument of another kind or a function type, is U2002, once per cycle, at the first field that leads back into it (`spec/03 §2` rule 5).
   - (v0.2, ADR 0060.) In edition `0.2`, this rule is replaced by two:
     - **Inhabited.** The grounded types are the least set such that:
       - `Int`, `Bool`, `Text`, `Unit`, `Hashed`, the fixed-width integers, function types, type parameters, `List[_]` and `Option[_]` are grounded, and `Result[T, E]` is grounded when `T` or `E` is;
       - a record kind is grounded when all its field types are, and a sum kind when all the field types of one of its variants are;
       - an instance `K[A1, ..., An]` is grounded when `K` is, with each parameter replaced by its argument.
       A record or sum kind that is not grounded is U2002 at its name, once per kind: "kind `Stream` has no finite value: every variant contains a kind without one; add a variant without it, as `End`", and for a record "kind `Ring` has no finite value: its field `next` needs a kind without one; make it an `Option` or a `List`".
     - **Regular.** In a set of mutually recursive kinds, an occurrence of a kind of the set, inside the declaration of a kind of the set, has only type parameters of that declaration as its arguments. Otherwise it is U2002 at the field: "`Nest` occurs in its own recursion with the argument `List[T]`; inside a recursive kind, the kinds of its recursion take only type parameters".
9. **Equality on type parameters.** A generic function that compares values of a type parameter, directly or through the generic functions it calls, cannot be instantiated with a type whose values hold functions (U2001 at the instantiation). (v0.2, ADR 0066.) In edition `0.2` the bound is the written or inferred constraint `Eq`, and a type that does not satisfy a constraint is U2025 (`spec/20 §3`).
10. **Labels, in detail.** A bare name equal to a parameter's name is labeled with it, even as the first argument. A constructor with several fields reports an unlabeled argument as U2004; extra arguments of a constructor with one field, and parentheses after a variant without fields, are U2009.
11. **Predicates.** A cycle of calls that contains a predicate, through predicates or functions, reflect or opaque, is U2015 once per cycle; a predicate whose result type is not `Bool` is U2001.
12. **Policies.** The `via` function takes the type of `from` and returns the type of `to`, except for labels and with the same refinements, and has no `requires` clause (U4005, `spec/07 §5` rule 4, ADR 0047). A policy call takes one argument without a label.
13. **Argument counts.** Too many arguments, and a parameter that receives none, are U2001.
14. **Interpolation.** In edition `0.1`, an interpolated expression is an `Int`, a fixed-width integer, a `Bool` or a `Text`; any other type is U2001. (v0.2, ADR 0068.) In edition `0.2`, an interpolated expression MAY have any type that satisfies `Eq` (`spec/20 §1`), except `Hashed`:
    - A type that holds a function, at any position, is U2001 at the interpolation, and the message names the position. `Hashed`, at any position, is U2001 too (`spec/14 §1`).
    - A value of a type parameter `T` needs `T: Eq`, written or inferred for a private function (`spec/20 §3` rule 4); otherwise it is U2026, with the repair that writes `[T: Eq]`.
    - The text is the value's Una form of `spec/11 §5`, without redaction.
    - (v0.5, ADR 0119 rule 3.) An interpolated expression MAY also have a type that does not satisfy `Eq` only because it holds a `HashMap`, a `HashSet` or a `PriorityQueue` at some position, such as a record with a hash-set field (`spec/28 §4`).
    - Its label is the join of every label of the value's type, deep (`spec/07 §7` rule 3), so a `secret` field makes the whole text `secret`.

## 13. Corrections

### 13.1 The label that decides a failure (2026-10-10, ADR 0087, #479)

Found by M3 of the formal track (`docs/plan/formal-track-m3.md` §2). T-Let (§4), T-Attempt and T-Handle (§5) above omit a premise that `spec/07 §7` rule 4 states. The formal track's model has it (`formal/UnaFormal/F1/LTyping.lean`, rules `letR`, `letE`, `attempt` and `handle`), and T6 is proven for it. The checker already enforces it, so no program's verdict changes. Where this section and §4 or §5 differ, this section wins.

1. **Notation.** A failure $F$ carries the label of what decides it, written $F^{\ell_F}$, with $\ell_\emptyset = \bot$. That label is the one `spec/07 §7` rule 4 gives:
   - in T-Op, the arguments that decide the operation's failure, each capped at its `accepts`, joined for `Files` with the package's file-state label (ADR 0085);
   - in T-Call and T-App, $\bot$, since a function's own failure is decided at the public program counter (T-Fail, and $\mathrm{bound}(f)$ is met with $\bot$ when $f$ can fail);
   - for a labeled prelude function, the labels of its arguments.
2. **T-Let.** The body runs only when the right-hand side did not fail, so it is typed under $pc \sqcup \ell_{F_1}$:

$$
\frac{\Gamma;K;pc \vdash r : \tau_1 \;!\; \varepsilon_1;F_1^{\ell_{F_1}} \qquad \Gamma, x{:}\tau_1;K;pc \sqcup \ell_{F_1} \vdash e : \tau_2 \;!\; \varepsilon_2;F_2^{\ell_{F_2}} \qquad \Gamma \vdash \tau_1 <: \tau_x}
     {\Gamma;K;pc \vdash \mathtt{let}\; x{:}\tau_x = r \;\mathtt{in}\; e : \tau_2 \;!\; \varepsilon_1 \cup \varepsilon_2;\, (F_1 \sqcup F_2)^{\ell_{F_1} \sqcup \ell_{F_2}}}\;(\textsf{T-Let, corrected})
$$

3. **T-Attempt.** The `Result` carries the label of what decides it:

$$
\frac{\Gamma;K;pc \vdash e : \tau \;!\; \varepsilon;F^{\ell_F}}
     {\Gamma;K;pc \vdash \mathtt{attempt}\; e : \mathrm{Result}[\tau, F]^{\ell_F} \;!\; \varepsilon;\emptyset}\;(\textsf{T-Attempt, corrected})
$$

4. **T-Handle.** Whether the handler runs, and so which branch gives the value, is decided by $\ell_{F_1}$:

$$
\frac{\Gamma;K;pc \vdash e : \tau \;!\; \varepsilon_1;F_1^{\ell_{F_1}} \qquad \Gamma, x{:}F_1^{\ell_{F_1}};K;pc \sqcup \ell_{F_1} \vdash e_h : \tau \;!\; \varepsilon_2;F_2^{\ell_{F_2}}}
     {\Gamma;K;pc \vdash \mathtt{handle}\; e \;\mathtt{with}\; x \Rightarrow e_h : \tau \sqcup \ell_{F_1} \;!\; \varepsilon_1 \cup \varepsilon_2;\, F_2^{\ell_{F_2}}}\;(\textsf{T-Handle, corrected})
$$

5. **At the surface.** These are the rules that `spec/07 §7` rule 4 states for `attempt`, `if error use`, `if error pass` and `if error fail`. Since `if error pass` and `if error fail` on a failure decided above `public` are U4002, and a handler expression with an effect is U3013 (`spec/04 §4`), no program the checker accepts tells the corrected rules from the ones of §4 and §5.

### 13.2 A `checked` predicate's other reads (2026-10-10, ADR 0089, #488)

Found by S-17. T-Checked (§7) labels the failure of `checked` by its operand alone, but the predicate φ may read other values, and its outcome then depends on them. T-Checked's premise and its failure's label take every label of every value φ reads:

$$
pc \sqcup \ell^{*}(a) \sqcup \bigsqcup_{x \in \mathrm{fv}(\varphi) \setminus \{\mathtt{it}\}} \ell^{*}(x) = \bot
$$

Otherwise the `checked` is U4002 at the `checked`, with the predicate as its related span when a value it reads raises the label, and the operand otherwise. $\ell^{*}(x)$ joins every label of the type of the local `x`, so a field path such as `limits.most` counts every label of `limits`. Names that are not locals, such as predicates and functions, are pure, and their arguments are reads. Where this section and §7 differ, this section wins.

### 13.3 A policy call's failure (2026-10-10, ADR 0090, #496)

Found by S-18. A policy's `via` function is not label-checked (`spec/07 §5` rule 3), and its failure is decided by the `from` value. The failure of a policy call $p(a)$ is labeled $pc \sqcup \ell(\mathit{to})$, the top label of `to`, where its release is visible: T-Declassify (§7) gives the call the failure $F_f^{\,pc \sqcup \ell(\tau_{\mathit{to}})}$ in the notation of §13.1. So `attempt` gives a `Result` with that label, `if error use` runs under it, and `if error pass` and `if error fail` are U4002 when it is above `public`. A policy whose `to` is `public` keeps a public failure. Where this section and §7 differ, this section wins.

### 13.4 A `via` function whose failure reveals nothing (2026-10-13, ADR 0098, #497)

Each `via` function `f` has a failure-decision label $\ell_f$: `public` when its body has the shape below, and $\top$ otherwise. The failure of a policy call $p(a)$ of `p : from ⇝ to via f` is labeled $pc \sqcup \lambda$, where $\lambda = \ell_f$ if $\ell_f \sqsubseteq \ell(\mathit{to})$, and $\lambda = \ell(\mathit{to})$ otherwise. So a `via` function of the shape fails at $pc$, and every other keeps §13.3's label.

`f` has the shape when all of these hold:

- No handler of `Keys` that the package declares is named outside its own declaration and examples, counted as words over the package's declarations and comments, examples excepted; a handler whose capability does not resolve counts as a handler of `Keys`.
- Its body has no `with`.
- Every failure point of its body is a call `@k.hmac(key_name: L, data: e)` in the body's top-level sequence, as a statement or the value of a `let`, where `@k` is a `Keys` binding of its header, `L` is a plain text literal as written, and the failure is handled by `if error pass` or by `if error fail V` with no argument.
- The words `fail` and `pass` in its body are exactly those handling clauses.

The check fails closed: anything it cannot read as the shape gives $\ell_f = \top$. `with` and `given` are the only ways to install a handler at this edition; a construct that installs or rebinds one must extend the shape. Where this section and §13.3 differ, this section wins.
