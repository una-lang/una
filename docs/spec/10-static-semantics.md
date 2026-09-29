# 10. Static semantics

Status: Normative, v0.1. Rules are defined over Core IR (`spec/09`). Surface diagnostics are reported against the source spans that Core nodes carry.

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
\Gamma \vdash \bar a <: \bar\tau \qquad \Gamma \vDash \varphi[\bar a/\bar x] \qquad pc \sqcup \textstyle\bigsqcup_i \ell(a_i) \sqsubseteq \ell_o
\end{array}}
{\Gamma;K;pc \vdash c.o(\bar a) : \{\nu{:}b_r \mid \psi[\bar a/\bar x,\, \nu/\mathit{result}]\}^{\ell(\tau_r)} \;!\; \{c.o\};\, F_o}\;(\textsf{T-Op})
$$

$\mathrm{bound}(f)$ is the meet of the `accepts` labels of every operation in $\mathrm{eff}(K_f)$, met with $\bot$ if $F_f \neq \emptyset$; it is $\top$ for pure, non-failing $f$ (`spec/07 §3`).

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
\frac{\Gamma \vdash a : \{\nu{:}b \mid \varphi_0\}^{\ell} \qquad \varphi \in \mathrm{RL} \cup \mathrm{Opaque} \qquad C \in \mathrm{variants}(F) \qquad pc = \bot}
     {\Gamma;K;pc \vdash \mathtt{checked}(a, \varphi, C) : \{\nu{:}b \mid \varphi_0 \wedge \varphi\}^{\ell} \;!\; \emptyset;F}\;(\textsf{T-Checked})
$$

$$
\frac{\mathtt{policy}\; p : \tau_{\mathit{from}} \leadsto \tau_{\mathit{to}} \;\mathtt{via}\; f \in \Sigma \qquad \Gamma \vdash a <: \tau_{\mathit{from}} \qquad pc \sqsubseteq \ell(\tau_{\mathit{to}}) \qquad K_f \le K}
     {\Gamma;K;pc \vdash \mathtt{declassify}_p(a) : \tau_{\mathit{to}} \;!\; \mathrm{eff}(K_f);F_f}\;(\textsf{T-Declassify})
$$

## 8. Declarations

$$
\frac{\bar x{:}\bar\tau,\, \varphi;\;\; K_f;\;\; \bot \vdash e : \tau' \;!\; \varepsilon;F \qquad \bar x{:}\bar\tau,\varphi \vdash \tau' <: \{\nu{:}b_r \mid \psi[\nu/\mathit{result}]\}^{\ell(\tau_r)} \qquad \varepsilon \subseteq \mathrm{eff}(K_f) \qquad F \subseteq F_f}
     {\vdash \mathtt{fun}\; f(\bar x{:}\bar\tau) : \tau_r \;\mathrm{uses}\; K_f \;\mathrm{fails}\; F_f \;\mathrm{requires}\; \varphi \;\mathrm{ensures}\; \psi = e \;\; \mathsf{ok}}\;(\textsf{T-Fun})
$$

Handler operations are checked like functions whose parameters, `requires`, `ensures`, `fails` and result type come from the capability's operation signature and whose capability context is the handler's `uses`. Private functions without declared `uses` or `fails` take the inferred least solution (`spec/04 §5`) before T-Fun is applied.

## 9. Syntactic side conditions

1. Capability names $c$ occur only as the receiver of `c.o(...)` and in `with` (U3005). They are never atoms.
2. A lambda whose type has non-empty `uses` or `fails` occurs only directly as a call argument (U3006).
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
| T6 | Noninterference | Termination-insensitive noninterference up to declassification, for the channels in `spec/07 §1` |
| T7 | Decidability | Type checking is decidable; obligation checking is decidable for RL (`spec/12 §2`) |
