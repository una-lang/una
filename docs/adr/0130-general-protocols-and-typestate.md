# 0130. General protocols and typestate: facts about the resources behind a capability

- Status: Accepted
- Date: 2026-10-31
- Deciders: Una project (accepted on 2026-11-02, with the changes of "Decisions of acceptance")
- Resolves: issue #1290, and in part issue #558 (state about the external system in operation contracts): the design for Suite B's B04. B05's batch form stays with #1332 (example 6).
- Related:
  - ADR 0092 (conditions on allowed effects), whose "Alternatives considered" defers "state on capability declarations" to #558 and says "a condition may later name such facts";
  - ADR 0123 rule 4 (reply rights, held by one task at a time, with one atomic state per exchange), the internal precedent for a protocol state that the language tracks, there at run time;
  - ADR 0005 (second-class capabilities), ADR 0008 (`unknown` is an error), ADR 0071 (what a loop forgets, and the reasons of an `unknown`), ADR 0103 (refined elements);
  - `spec/05 §2` (operation clauses), `§3` rule 4 (conditions), `§6` (second-class rule); `spec/12 §2` (the QF_UFLIA fragment), `§3` (obligations and hypotheses), `§4` (encoding); `spec/17 §5` rule 3 (loops); `spec/19 §4` (`together`).
- Target: the first edition after `0.4` (expected `0.5`), as a preview under the terms of ADR 0121. Editions `0.1` to `0.4` do not change. The benchmark baseline does not move.
- Amends, on acceptance and with the implementation: `spec/02 §3` (capability and header grammar), `spec/05 §2`, `§3`, `§5`, `§9`, `spec/12 §3`, `§4`, `§6`, `spec/13 §3`, `§4`, `§6`, `spec/17 §5` rule 3, `spec/19 §4`.

Every form of Una syntax in this record was a **proposal**: the words `fact`, `starts`, `changes`, `keeps`, `on` (in an operation's clauses) and `no` (in a clause) were chosen for review. On acceptance the project kept them, all contextual, none reserved (Decisions of acceptance 1).

> **In plain words.**
> - **What it adds.** A capability may declare **facts**: named yes-or-no (or small-valued) properties of the resources behind it, such as "this connection is open", "this lock is held" or "this payment is authorized". Its operations say which facts they need (`requires`), which they make true or false (`ensures`), and what a failure does to them.
> - **What the verifier does.** It tracks each fact through the body of every function, step by step, as part of the proof it already does. An operation whose needed fact is not known to hold is a build error, with a counterexample and ranked repairs.
> - **What it does not add.** No new kind of type, no run-time cost, no domain words. Facts exist only in the proofs. Nothing about payments, accounts or agents is built in: a payment is one test case among others.

## Decisions of acceptance (2026-11-02)

The project accepted this record on 2026-11-02 with the changes below, which the rules and slices carry:

1. **The words are kept, and all are contextual.** `fact`, `starts`, `changes`, `keeps`, `on` and `no` keep the forms proposed here (`starts` was missing from the list of proposed words and joins it). None is a reserved keyword: each is a word only in the place where this record gives it a meaning (a capability's body, an operation's clauses, a header's clauses), and an ordinary identifier everywhere else, as `gives`, `first`, `tasks` and `start` are (`spec/01`). Slice 1 (#1325) states the places in `spec/01` and `spec/02`.
2. **`Resolves` is partial.** This record resolves #1290 and, of #558, the design for B04. B05's batch form is not covered (example 6) and stays with the record on facts over list elements (slice 8, #1332).
3. **Slice 7 is a new, pre-registered version of B04.** B04's `task.json` lists `scaffold/src/orders/types.una` as protected, and the task is frozen: slice 7 never edits it. It registers a new version of the task (a variant, with its own id) before any run, and runs it against the implementation with the hidden oracle. A paper check made for acceptance found no functional effect of the extra query of example 5: the reference ends on `is_ok(second)`, the hidden fake's `is_refunded` is stateless (`settled.contains(charge)`), so a second question returns the same `false`, and `predicates.json` holds only `op_count` and `arg_contains` predicates, none on `is_refunded`. That check is on paper and replaces no run: slice 7 runs the variant, its hidden tests h06 and h09 included.
4. **No interference is an assumption, and it is written** (rules 6, 12 and 17). Facts describe the outside system as the program's own operations leave it: they assume that no other actor changes the resources behind a binding between two steps beyond what the operations' clauses state, `fails` and its `on V` lines included. The build report lists that assumption in the Assumed zone with every capability that declares facts.
5. **Target and status confirmed.** The record targets the first edition after `0.4` (expected `0.5`), as a preview under ADR 0121, with no port to `0.4`. Until the protocol-safety theorem lands it is "accepted, not proved" (rule 17).

## Context

### The problem

A capability says which operations a function may perform (`spec/05 §3`), and ADR 0092 says under which conditions over the function's parameters. Neither can say **in which order**, or **in which state of the outside system**, an operation is allowed. Many protocols are exactly that:

- a query runs only on an open connection, and a closed connection is not used again;
- a shared record is written only while its lock is held;
- a stream is read only while it is open, and not after it reported its end;
- a payment is captured only after it was authorized, and not twice.

Suite B shows the cost of the gap. Its Q1 analysis and full run (una-bench `findings/for-v0.2.md` §1, `docs/full-b.md`) list two tasks that Una accepts because nothing can state the rule:

| Task | What the violation does | Why Una accepts it today |
| --- | --- | --- |
| B04 | retries a timed-out refund without asking `is_refunded` first, which may refund a charge twice | `refund` is in the function's `uses`; nothing says when |
| B05 | mails a welcome to an address that left later in the same batch | `send_batch` is in `uses`; the forbidden case depends on what the batch did to the list |

### What already exists

- **Operation contracts** (`spec/05 §2`): `requires` over the arguments, an obligation at every call; `ensures` over the arguments and `result`, an **assumption** about the outside system, listed in the build report's Assumed zone. A fact is the missing piece: a term that `ensures` of one operation can set and `requires` of another can read.
- **The fragment** (`spec/12 §2`): every query is QF_UFLIA, with no quantifier; "axioms that would need quantifiers are instantiated as ground facts for the terms that occur in the query". This record keeps that rule and uses that technique.
- **Loops forget** (`spec/17 §5` rule 3, ADR 0071): the body is checked once, for any iteration, with the bindings it updates fresh; after the loop no fact about them survives. The repair is a helper whose `ensures` states what one step does.
- **A run-time protocol** (ADR 0123 rule 4): a reply right is held by one task, a second reply fails with `NotHeld`, and one atomic transition ends each exchange. ADR 0123 chose a run-time state over "a static rule that every reply right is used exactly once (a linear type), [which] would need a new kind of type for one capability". This record follows the same judgment for the static side: it tracks states in the proofs, keyed by ordinary values, and adds no linear or session type.
- **Second-class capabilities** (ADR 0005): a binding `@b` is a name, never a value, so every operation and every call names statically the binding it acts on. That is what lets the verifier know, without an alias analysis, which outside system each step changes.

### Constraints

- **General purpose.** One mechanism for every protocol. No construct names a domain.
- **Inside the fragment.** No query gains a quantifier or a theory. The encoding uses uninterpreted functions, linear arithmetic, `ite` and fresh symbols only.
- **Modular.** A function is verified from its own body and the headers of what it calls, as today. A caller never looks into a callee's body.
- **Headers are the review boundary** (ADR 0007). What a function needs from, and does to, the state of a binding is in its header.
- **No default that hides an unknown** (ADR 0008). A fact that is not known is not assumed either way.
- **Few new forms** (ADR 0092's syntax constraint). An agent meets as few new words as possible, each in a place where Una already has clauses.

## Decision

### A. Declaring facts

1. **A fact is declared in a capability.** A line `fact <name>(<params>) [-> <type>]` in a capability's body, beside its `op` lines:

   ```una
   pub capability Database
     fact open(conn: Conn)
     op connect(url: Text) -> Conn
       ensures open(result)
       fails DbError
     op query(conn: Conn, sql: Sql) -> Rows
       requires open(conn)
       fails DbError
     op close(conn: Conn) -> Unit
       requires open(conn)
       ensures not open(conn)
   ```

   - **Type.** A fact is `Bool` when it writes no result type. It MAY have the result type `Int`, `Nat` or a payload-free sum kind (a phase, as in example 3). Any other result type is U2043.
   - **Parameters.** Each parameter has a type whose values compare by `==` in the refinement language: `Int` and its refined and fixed-width forms, `Bool`, `Text`, `Hashed`, a payload-free sum, or a kind whose sort the encoding already has. A function type, a capability, a type parameter and a second-class type are U2043. A fact with no parameter is allowed and describes the whole system (`fact ready`).
   - **Names.** Facts and operations of one capability share one namespace: a fact with the name of an operation or of another fact is U1002. A fact is never called and never performed.
   - **Ghost.** A fact has no run-time representation, no storage and no cost in the interpreter or in native code. It exists only in the verifier's queries.

2. **Where a fact may be named.** Only in the refinement language of clauses:
   - inside its capability, by its bare name, in the `requires`, `ensures` and `on` lines of the capability's operations (rule 4);
   - elsewhere, through a binding, as `@b.f(args)`: in a function's or a handler's `requires` and `ensures`, in its `changes` lines (rule 7), and in an ADR 0092 condition (rule 10). `@b` MUST be a binding of the header (U3001 otherwise, as for an operation);
   - a fact in an expression that runs, such as an `if`, a `let`, an argument or a `when`, is U2042, "`@payments.in_doubt` is a fact, which exists only in proofs; ask an operation whose `ensures` tells it". The code exists because a fact has no value at run time, and a program that branches on one would branch on nothing.

3. **A fact belongs to a binding.** Two bindings of one capability are two systems: `@primary.open(c)` and `@replica.open(c)` are independent terms. A rebinding `with @x = @y` (and `with @x = @y[ops]`, ADR 0093) makes the callee's `@x` the caller's `@y`, so they share their facts. A handler installed with `with @x = h(args)` or `given` starts a new binding, whose facts are unknown when it is installed, unless the capability gives them a start value (rule 6).

### B. Establishing, keeping and invalidating facts

4. **What an operation does to facts.**
   - **Normal return.** The fact terms that an operation's `ensures` names are its **change set**. After a normal return, those terms satisfy the `ensures`, instantiated at the arguments and `result`, and every other fact of the binding keeps its value. So `close(conn)` makes `open(conn)` false and leaves `open(d)` unchanged for every `d` other than `conn`.
   - **A query.** A term that an operation's `ensures` names only inside `old(...)` is read, not changed: `old(f(args))` is the fact's value before the operation, which extends `spec/17 §3` rule 6 to facts. So a query that tells a fact without changing the outside system writes `ensures result == old(captured(payment))`, and its change set is empty. A term named outside `old` is in the change set, even in an equation with `result`.
   - **Failure.** An operation that fails with variant `V` of its error kind:
     - with a line `on V keeps`: changes no fact;
     - with a line `on V ensures q`: the terms `q` names satisfy `q`; every other term of the normal change set becomes **unknown**;
     - with no line for `V`: every term of the normal change set becomes unknown. This is the safe default: a timeout may or may not have done the work.
   - `on V` names a variant of the operation's error kind (U1001 otherwise) and appears after `fails`. Two lines for one variant are U1002.

5. **What "unknown" means.** An unknown term is a fresh symbol of the query with no constraint: no fact is assumed, either way. An operation that requires it is U5002 (unknown) or U5001 (refuted) at its site, as for every precondition, with the reason "`@payments.refund` timed out at line 12, so `in_doubt(charge.id)` is unknown". The repair is to ask (rule 12), never to assume.

6. **Start values.** A fact MAY declare the value it has where a binding begins: `fact in_doubt(charge: Text) starts false`. A binding begins where the program first receives it: at `main` and the entry of `[run]` or `[serve]`, from the host's grants; at a `with @x = h(args)` and a `given @x = h(args)`. There, every term of the fact has the start value, for every argument. A function that receives a binding from its caller has no start value: it knows only what its `requires` say. A fact without `starts` is unknown at a binding's beginning. A start value is an assumption about the outside system, listed in the Assumed zone with the capability.
   - **No interference by other actors.** Every fact, with or without a start value, assumes that no other actor (another process, another client of the same service, an operator) changes the resources behind a binding between two steps of the program beyond what the operations' clauses state: their `ensures`, their `fails` and the `on V` lines. A connection that another party closes, or a lock that another process releases, between two steps is outside the model. The build report lists this assumption in the Assumed zone with every capability that declares facts (Decisions of acceptance 4).

### C. Across calls: modular frames

7. **What a function may change.** A function's **frame** for a binding `@b` is the set of fact terms of `@b` that a call of it may change.
   - **By default** it is every fact that an operation listed in the header's `uses @b: C[ops]` can change, for every argument (a whole fact). An operation can change the facts of its normal change set and those its `on V ensures` lines name. The header already lists the operations, so the default frame is visible in the header and costs no new line.
   - **A `changes` line narrows it**: `changes @payments.in_doubt(charge.id)` says the call changes `in_doubt` of `@payments` only at `charge.id`. Several lines are a union. `changes @payments.in_doubt` names the whole fact. A fact of `@b` that no `changes` line names, while another fact of `@b` has one, is in the frame only if an operation of the header can change it, as by default: a `changes` line narrows only the fact it names.
   - **The body is checked against the frame.** At each operation and each call inside the body whose change set or frame may reach a term of `@b.f` outside the listed ones, the verifier proves that it does not: an obligation of the new kind `fact_frame`, whose goal is the disjunction of the argument equalities (`args = charge.id`), ground. Refuted, U5001; unknown, U5002: "`@payments.refund` may change `in_doubt(c)`, which the `changes` of `refund_charge` does not list".
   - **The caller assumes the frame.** After a call, the caller knows: the callee's `ensures`, instantiated; every fact term outside the callee's frame, unchanged; every term inside it, unknown except what the `ensures` say. After a call that fails, as for an operation: the terms of the frame are unknown, since `x is Err` carries nothing (`spec/12 §3`).
   - **No inference.** A private function with inferred `uses` (`spec/05 §9` rule 2) has the default frame and no fact in its `requires` or `ensures`, as ADR 0092 rule 4 decides for conditions. What a function needs and does is written in its header.

8. **Whole-fact clauses.** A `requires` or `ensures` of a function MAY state that a `Bool` fact is false at every argument: `requires no @payments.in_doubt`, where `no f` reads "no term of `f` holds".
   - **As a hypothesis** (a `requires` in the callee, an `ensures` in the caller, a start value of `false`), it is instantiated, as `spec/12 §2` already does for axioms, at every argument tuple of that fact that occurs in the query.
   - **As a goal** (a `requires` at a call, an `ensures` at a return), its negation is "some term holds", which takes one fresh constant per parameter: `@payments.in_doubt(k)` with `k` fresh. The query stays quantifier free.
   - **Why it is complete for these forms.** Every other assertion of a query constrains a fact only at the argument tuples that occur in it, so a model that satisfies the instances can be extended to every other argument with the value `false` without changing any assertion. A model the solver returns is therefore a real counterexample of the stated contracts. The formal track proves this lemma (rule 17).
   - `starts true` and `no` over a fact of another type are U2043: only "nowhere true" is stated as a whole, which is what protocols need ("no lock held", "no refund in doubt").

### D. Inside a body and across loop iterations

9. **The state, step by step.** The verifier keeps one **version** of each fact of each binding along each path. A step that may change a fact (an operation, a call, a failure, a loop head, a `together`, a `start`) makes a new version of each fact it changes, and only of those.
   - **Encoding** (`spec/12 §4`). A version that changes one term is a non-recursive `define-fun`: `f₁(x) = ite(x = a, v, f₀(x))`, `v` fresh, constrained by the `ensures`. A version that changes several listed terms tests each. A version that changes a whole fact is a fresh `declare-fun`. Symbols are `i!fact!<binding>!<fact>!<n>`, in the encoder's namespace (ADR 0038 D8).
   - **Loops.** A loop of `spec/17 §5` or `spec/26` is checked once, for any iteration, as today. At the start of the body and after the loop, each fact that the body's frame may change (the union of the change sets and frames of its steps) gets a fresh version: no fact about it survives, as for the bindings the loop updates. Facts that the body cannot change survive unchanged, with every fact about them. This is the rule of `spec/17 §5` rule 3 applied to facts; it adds no loop invariant clause.
   - **What an `unknown` says.** A query whose counterexample depends on a fact a loop forgot is `unknown`, with ADR 0071's form of reason: "the loop forgets `@subscribers.subscribed` because its body may change it; state what one iteration does to it in the `ensures` and `changes` of a helper, and call the helper in the loop". Recursion with contracts carries facts across iterations precisely, since each call is checked from the callee's header (example 5).
   - **Concurrency.** Each branch of a `together` block is verified with the facts known before the block, minus the facts that a sibling branch's frame may change, which are unknown in it; after the block, every fact that some branch may change is fresh (`spec/19 §4`). After `start e`, every fact that the started task's frame may change is fresh in the starting task. Facts change only through operations, so there is no data race on a fact: the rule only removes knowledge that timing could falsify.

### E. Conditions, capabilities, ownership and failures

10. **Conditions may name facts** (ADR 0092). An operation's condition in a header's `uses` MAY name facts of the header's bindings: `uses @ledger: Ledger[write where @locks.held(lock)]`.
    - **Meaning.** Every performance of `write` through `@ledger` happens in a state where `held(lock)` holds. A condition naming a fact is no longer fixed for the call, so the `effect_condition` obligation at each site is evaluated at the site's version of each fact.
    - **Implication at calls** (ADR 0092 rule 3). For a callee's condition `C′` and the caller's `C`, the goal `C′[args] ⇒ C` is proven with every fact term in them at one fresh version, under the parameter facts of Π: the implication must hold in every state the callee may reach, since the callee checks `C′` at each of its own sites.
    - Locks of ADR 0092 rule 8 MAY name facts in the same way.

11. **Capabilities, ownership and `fails`.**
    - **Second-class capabilities** (ADR 0005) are what make facts cheap: since a binding never flows as a value, the binding each step acts on is known by name, and a fact needs no alias analysis of bindings.
    - **Ownership and the memory model.** A fact is keyed by values, not by places. Two copies of a value `conn` denote the same outside resource and share its facts, which is the truth: a copy of a connection id is the same connection. `edit`, `take` and in-place updates (`spec/17`) do not touch facts. Two different variables that may hold equal keys are handled by the solver's equality reasoning: a frame axiom applies only where the keys differ.
    - **No linear types.** A fact does not make a value usable once. "Every opened connection is closed" is a property of the whole run (liveness of a resource), not a precondition; a function can promise it for its own normal returns with `ensures not @db.open(conn)`, and nothing more is claimed. ADR 0123 rule 4 keeps the run-time side for its own capability.
    - **`fails`.** A failure of a call or an operation is a step (rules 4 and 7). A function's `ensures` speaks of normal returns only, as today; a caller that handles a failure with `attempt` or `if error` knows only the frame's unknowns on that path.

12. **Handlers.** A handler implements the capability's operations, and its bodies are verified against their contracts (`spec/05 §5`). A handler has no access to the facts, which describe the outside system the handler stands for.
    - In this record, the fact clauses of an operation are not verified against a handler's body: like a host capability's, they are assumptions, and the build report lists each handler of a capability with facts in the Assumed zone with "fact clauses assumed". The assumption of no interference of rule 6 holds for a handler's outside system as for a host's.
    - A later record MAY let a handler define a fact over its own parameters and bindings, so that its fact clauses become obligations. It is deferred, with its own card.

### F. Diagnostics and repairs

13. **Codes.** Two new codes, free in the registry; following the repository's rule that the specification changes with the implementation, slice 1 (#1325) enters them in `spec/13 §4`:
    - **U2042** (types): a fact in an expression that runs (rule 2).
    - **U2043** (types): a fact's parameter or result type that is not allowed, `starts true`, or `no` over a fact that is not `Bool` (rules 1, 6, 8).
    - Every proof diagnostic stays U5001 (refuted) or U5002 (unknown). The obligation kind `fact_frame` joins `spec/12 §3`. A fact precondition of an operation is a `precondition` obligation, and a fact in a condition an `effect_condition`, as today.
    - U1001 (an unknown fact or variant name) and U1002 (a duplicate) are reused.

14. **What an agent sees.** A refuted precondition names the fact, the step that last changed it and the counterexample:

    ```text
    error[U5001]: `@payments.refund` requires `not in_doubt(charge.id)`, which is refuted
      --> src/orders/cancel.una:21:13
       |
    16 |   let first = attempt @payments.refund(charge.id, amount: charge.amount)
       |               ------- the refund failed with `Timeout` here, so `in_doubt(charge.id)` is unknown
    21 |             let second = attempt @payments.refund(charge.id, amount: charge.amount)
       |                                  ^^^^^^^^^^^^^^^^ a second refund while the first may have been made
       = counterexample: charge.id = "c1", in_doubt("c1") = true
    ```

15. **Ranked repairs** (`spec/13 §3`, ADR 0084). For a fact precondition or condition that is refuted or unknown, in this order:
    1. **Ask.** When the binding's capability has an operation whose `ensures` tells the fact (a query, rule 4), guard the site with it: `if not @payments.is_refunded(charge.id)`. The repair names the operation; it carries an edit only when the header already lists that operation.
    2. **Establish.** When an operation's `ensures` makes the fact hold, call it before the site (`connect` before `query`). Description only: it changes what the program does.
    3. **Require.** Add `requires @b.f(args)` to the enclosing function, which moves the obligation to its callers. Description only.
    4. **For a loop:** move the iteration into a helper with `changes` and `ensures` (rule 9's reason).

    For a refuted `fact_frame`: first, list the term in the function's `changes`; second, narrow the callee's `changes`.

    **Never offered:** removing or weakening an operation's `requires`, adding an `on V keeps` or a `starts` value to a capability, or adding `checked`. Each changes an assumption about the outside system, which is the capability author's decision and shows in the diff; none is a repair.

### G. Cost and the formal model

16. **Cost.**
    - **Run time:** none. Facts are erased; no instruction, word of memory or check is added in either engine.
    - **Tier 0:** checking fact declarations, clause scopes and U2042 is linear in the size of the clauses and bodies.
    - **Queries:** per query, let `V_f` be the number of versions of fact `f` on the paths the query covers and `T_f` the number of distinct argument tuples of `f` in it. Expanded, the `ite` chains and the instances of rule 8 cost O(Σ_f V_f · T_f) terms. Both factors are bounded by the size of the function under check, never by the program, since checking is modular (rule 7). The product is quadratic in the size of one function in the worst case, a function that changes and reads one fact at many distinct keys. The reason this is accepted: the alternative encodings either leave the fragment (the theory of arrays, `QF_AUFLIA`, whose `store` chains cost the same in the solver) or need quantifiers. A package that declares no fact pays nothing. The implementation states the achieved bound and adds a scaling test at growing chain lengths that fails on worse than O(V · T).
    - **Obligations:** one `fact_frame` per step that may reach outside a narrowed frame; preconditions and conditions are the obligations that exist today. All are cached like every obligation (`spec/12 §9`).

17. **The formal model.** The theorem this record needs, *protocol safety*, is a milestone of the formal track: in every run of a checked program whose host honors its capabilities' fact clauses and start values, and in which no other actor changes the resources behind a binding beyond what those clauses state (rule 6), every operation runs in a state where its `requires` holds, every performance of a conditioned operation in a state where its condition holds, and every call changes only the facts of its frame. It rests on two lemmas: the frame lemma (a version outside a step's change set equals the previous one) and the completeness of the instantiation of rule 8. Until the theorem lands, this record is "accepted, not proved", as ADR 0092 rule 7 states for trace safety.

## Worked examples

Each example is a proposal of syntax and an argument on paper: none has been run.

### 1. A database connection: open before a query

```una
pub capability Database
  fact open(conn: Conn)
  op connect(url: Text) -> Conn
    ensures open(result)
    fails DbError
  op query(conn: Conn, sql: Sql) -> Rows
    requires open(conn)
    fails DbError
    on Disconnected ensures not open(conn)
    on Rejected keeps
  op close(conn: Conn) -> Unit
    requires open(conn)
    ensures not open(conn)

pub report(url: Text) -> Rows
  uses @db: Database[connect, query, close]
  fails DbError
=
  let conn = @db.connect(url)
  let rows = @db.query(conn, sql: daily_totals())
  @db.close(conn)
  rows
```

- `query` after `connect`: `open(conn)` holds from `connect`'s `ensures`. Proven.
- A `query` after `close` is refuted: `close` set `open(conn)` false.
- `query` changes no fact on success (its `ensures` names none). On `Disconnected` the connection is closed; on `Rejected` nothing changes; on any other variant `open(conn)` is unchanged too, since the normal change set is empty.
- A helper `run_one(conn: Conn, sql: Sql) -> Rows` with `uses @db: Database[query]` has `open` in its default frame, since a `Disconnected` failure of `query` changes it. Its callers keep `open(conn)` after a normal return only if it says so, `ensures @db.open(conn)`, which its body proves from `query`'s empty normal change set; or it narrows its frame with `changes @db.open(conn)`, so that other connections keep their facts.

### 2. A lock held before use

```una
pub capability Locks
  fact held(lock: LockId) starts false
  op acquire(lock: LockId) -> Unit
    requires not held(lock)
    ensures held(lock)
    fails LockError
    on Busy keeps
  op release(lock: LockId) -> Unit
    requires held(lock)
    ensures not held(lock)

pub capability Ledger
  op write(entry: Entry) -> Unit

pub post(lock: LockId, entry: Entry) -> Unit
  uses @locks: Locks[acquire, release]
  uses @ledger: Ledger[write where @locks.held(lock)]
  requires not @locks.held(lock)
  ensures not @locks.held(lock)
  fails LockError
=
  @locks.acquire(lock)
  @ledger.write(entry)
  @locks.release(lock)
```

- The condition on `write` names a fact of another binding of the header (rule 10). At the site, `held(lock)` holds from `acquire`. Moving `write` after `release` is refuted with the counterexample `held(lock) = false`.
- A failure of `acquire` with `Busy` keeps `held(lock)` false, so the function fails with its `ensures` unconcerned (it speaks of normal returns). Any other failure variant leaves `held(lock)` unknown, and `post` fails out, so the caller knows `held(lock)` is unknown on that path (rule 7).
- A second `acquire` of the same lock is refuted. Re-entrant locks are a different protocol, with `fact depth(lock: LockId) -> Nat`.
- `main` receives `@locks` from the host, so `held` is false everywhere there (`starts false`), and `post`'s `requires` is proven at the call.

### 3. A stream and its phases

```una
pub kind Phase = Open | Ended | Closed

pub capability Streams
  fact phase(stream: StreamId) -> Phase
  op open(path: Text) -> StreamId
    ensures phase(result) is Open
    fails StreamError
  op read(stream: StreamId) -> Option[Chunk]
    requires phase(stream) is Open
    ensures (result is None) implies (phase(stream) is Ended)
    ensures (result is Some) implies (phase(stream) is Open)
    fails StreamError
    on Broken ensures phase(stream) is Ended
  op close(stream: StreamId) -> Unit
    requires not (phase(stream) is Closed)
    ensures phase(stream) is Closed
```

- `read` after it answered `None` is refuted: the phase is `Ended`.
- A program that reads in a loop reads under a fact the body changes, so the loop forgets the phase (rule 9). The iteration goes into a helper, `read_all(stream: StreamId, so_far: List[Chunk]) -> List[Chunk]`, recursive, with `requires @streams.phase(stream) is Open`, `changes @streams.phase(stream)` and `ensures @streams.phase(stream) is Ended`: each recursive call is made only on the `Some` branch, where `phase` is `Open`.
- A `read` that fails with `Broken` ends the stream, so `close` is still allowed. Another failure variant leaves the phase unknown, and a `close` after it is unknown too (`not (phase(stream) is Closed)`): the capability's author decides whether to state that variant's effect with an `on` line, and no repair adds one for them (rule 15).

### 4. Authorization before capture (a test case of the mechanism)

The rule "capture only an authorized payment, and only once" is written with the same three forms, in a capability the application or its provider declares. The language knows nothing of payments.

```una
pub capability Payments
  fact authorized(payment: PaymentId)
  fact captured(payment: PaymentId)
  op authorize(order: Text, amount: Positive) -> PaymentId
    ensures authorized(result) and not captured(result)
    fails PaymentError
  op capture(payment: PaymentId) -> Unit
    requires authorized(payment) and not captured(payment)
    ensures captured(payment)
    fails PaymentError
    on Declined keeps
  op status(payment: PaymentId) -> Bool
    ensures result == old(captured(payment))
```

- `capture` of a `PaymentId` that came from data, not from `authorize`, is unknown: nothing established `authorized`. The repair is rank 3, `requires @payments.authorized(payment)`, which moves the obligation to the caller that authorized it.
- After `capture` fails with `Timeout`, `captured(payment)` is unknown, so a retry is refuted until `status` answers `false` (rank 1, "ask").
- After `Declined`, nothing changed, and a retry is allowed: it is the provider's decline, not an unknown outcome.

### 5. B04: never refund a charge twice

The scaffold's `Payments` (una-bench `tasks/B04-order-cancellation/una/scaffold/src/orders/types.una`) gains one fact and its clauses. The fact is about this run's own effects, so it has a start value:

```una
pub capability Payments
  --- A refund of `charge` may have been made by this run, and its outcome is not known.
  fact in_doubt(charge: Text) starts false
  op refund(charge: Text, amount: Positive) -> Unit
    requires not in_doubt(charge)
    ensures not in_doubt(charge)
    fails RefundError
    on Declined keeps
    on Timeout ensures in_doubt(charge)
  op is_refunded(charge: Text) -> Bool
    ensures not in_doubt(charge)
```

`is_refunded` settles the doubt, so its change set holds `in_doubt(charge)`: after it, the term is `false`, whatever the answer.

The reference's helper and its callers (una-bench `tasks/B04-order-cancellation/una/reference/src/orders/cancel.una`) gain header lines:

| Function | New header lines |
| --- | --- |
| `refund_charge(charge)` | `requires not @payments.in_doubt(charge.id)`, `changes @payments.in_doubt(charge.id)` |
| `refund_from(charges, index, so_far)` | `requires no @payments.in_doubt` |
| `cancel_order(order_id)` (the scaffold's entry) | `requires no @payments.in_doubt` |

- **The violating variant** retries after `Timeout` without asking: the second `refund` requires `not in_doubt(charge.id)`, which `on Timeout` made true. **Refuted**, with the diagnostic of rule 14. The rank-1 repair is the reference's own line, `@payments.is_refunded(charge.id)` before the retry.
- **The secure reference, inside `refund_charge`:** the first refund is under the `requires`; after `Timeout`, `is_refunded` sets the term false, so the retry is proven; `Declined` keeps it false. Proven.
- **Across charges.** After the retry also times out, the reference reports the charge as failed and goes on, leaving `in_doubt(charge.id)` true. The recursive call in `refund_from` then needs `no in_doubt`, which no longer holds: the verifier **refutes** it with the counterexample of two timeouts on one charge. The contract is stronger than the task needs; the harm it guards against is real only when a later charge of the order has the same id, which the reference would then refund while the first refund may have been made. Two repairs build it:
  - ask once more after the second timeout (`is_refunded`), which settles the doubt and also makes the report exact; then `refund_charge` can add `ensures not @payments.in_doubt(charge.id)`, and the frame keeps `no in_doubt` for the recursion (rule 8: the term at `charge.id` is false, every other term unchanged);
  - or state that the charge ids of an order are distinct, a property of every element of a list, which this record does not give (Consequences).
- **What `is_refunded` assumes.** Its `ensures not in_doubt(charge)` says that asking settles the doubt: the answer is final for this run. A refund still in flight at the provider that lands after the question would break it; that is the assumption of no interference of rule 6, stated for this capability, and the Assumed zone lists it.
- **What the fact does not say.** `in_doubt` tracks the doubt a timeout leaves, which is B04's violation. A second refund of a charge whose first refund succeeded is not refused by it: the whole rule "never refund a charge twice" over an order's charges needs their ids to be distinct, an element fact (slice 8).
- `cancel_order` is called by the scaffold's tests with a `given` handler and by a program's `main` with a granted binding: at both, `in_doubt` starts false, so `no @payments.in_doubt` is proven at the call.

### 6. B05: a gap this record does not close

The scaffold's `Subscribers` can say what `add` and `remove` do, with `fact subscribed(newsletter: Text, address: Text)`, and a helper that mails one address can carry the condition `uses @mailer: Mailer[send_batch where @subscribers.subscribed(newsletter.id, address)]` (rule 10). A welcome to one address after its `remove` is then refuted.

B05's rule, though, is about a **batch**: one `send_batch` of a list, after the loop over the requests, whose every element must still be subscribed. That needs a fact about every element of a list, kept through the loop (an address added to `to_welcome`, then removed from the list and from `to_welcome` later in the batch). Element facts are not in this record:
- stated over a list, they are quantified invariants, the work that edition `0.3` reserved the word `all` for (laws);
- carried as element refinements (ADR 0103) that name facts, they would need every state change to re-check or drop the refinements of every live list, a rule of its own.

So **B05 is not covered**, and this record says so instead of stretching rule 8 to it. Slice 8 asks for the record that would.

## Consequences

- **Protocols become contracts.** Order and state rules of a capability are written once, in its declaration, and every function that uses it is checked against them, with counterexamples and ranked repairs.
- **B04's violation fails at build**, and its secure reference builds with header lines and one more query after a second timeout (example 5). The full rule over an order's charges, with repeated charge ids, needs element facts.
- **B05 is not covered** (example 6). Its batch form needs facts over the elements of a list.
- **Agent cost.** New words in clauses only: `fact`, `starts`, `on V keeps`, `on V ensures`, `changes`, `no`, all contextual. Diagnostics stay U5001 and U5002 for proofs, with two new type codes.
- **Cost for existing code: none.** A capability without facts means what it means today.
- **What becomes harder.** A helper that changes a fact must say what it does in its header, or its callers lose the fact (rule 7). That is the price of modular checking, as for `ensures` today. Loops that change a fact need a helper (rule 9).
- **Assumptions grow.** Fact clauses, start values and the absence of interference by other actors (rule 6) are assumptions about the outside system, like `ensures` today, and the Assumed zone lists them.
- **Deferred:** facts over list elements (B05), fact definitions in handlers (rule 12), facts in function types, and the protocol-safety theorem (rule 17).

## Alternatives considered

- **Session types** (a channel's type is the protocol, as a sequence of sends and receives). Rejected:
  - they need linear use of the channel value, a new kind of type that Una does not have and that ADR 0123 declined for its reply rights;
  - a session type describes one channel between two parties; most protocols here are about a resource in an outside system, keyed by an id that is an ordinary value, and are not channels;
  - a failure that leaves the state unknown (B04's timeout) has no natural place in a session type, while it is one line here (rule 4).
- **A separate protocol language** (state machines declared in their own notation, checked by a separate tool). Rejected: a second syntax for agents to learn, a second checker in the trusted base, and a gap between what the machine says and what the code's headers say. Facts are written in the refinement language that `requires` already uses, and checked by the verifier that already runs.
- **Run-time checks only** (a host or a wrapper that refuses an operation in the wrong state, as ADR 0123 does for reply rights). Not chosen as the mechanism: a run-time refusal is a failure in production, found by the user, while the verifier finds the same error at build, with a counterexample. It stays the right tool where the state is the runtime's own, as for ADR 0123, and a host capability MAY still refuse at run time what its facts forbid.
- **Typestate as types** (`Conn[Open]`, `Conn[Closed]`, a value whose type changes). Rejected: a type that changes needs the old value to be unusable (linearity again), and values in Una are copied freely (`spec/17`). Facts keyed by values give the same checks without changing any type.
- **Facts as fields of a ghost record of the binding** (one state value per binding, updated as a whole). Not chosen: every step would rewrite the whole state and every frame would be a whole-record equation, which costs more and says less than per-fact versions.
- **Quantified clauses in general** (`requires all c in charges: ...`). Not chosen here: they leave the fragment unless restricted, and the restriction is the design question of element facts (slice 8). Rule 8's `no` is the one quantified form, chosen because its instances keep the fragment complete.
- **The theory of arrays** for states (`QF_AUFLIA`). Not chosen: it changes the logic of every query that uses a fact, while the `ite` encoding stays in QF_UFLIA at the same asymptotic cost (rule 16).
- **Defaults by name** (a fact named `open` meaning something). Rejected, as ADR 0092 rule 4 rejects conventions.

## Slices

Each slice is a card in Needs spec, to start after this record is accepted:

1. **Spec:** (#1325) `spec/05` (facts, start values, `on` lines), `spec/12` (versions, frames, `fact_frame`, `no`, the encoding), `spec/13` (U2042, U2043, repairs), `spec/02` grammar, conformance cases first.
2. **Parser and checker:** (#1326) fact declarations, clause scopes, U2042, U2043, `changes` and `no` in headers.
3. **Verifier, straight-line:** (#1327) versions and frames of operations and failures inside one body; preconditions and ADR 0092 conditions naming facts; the scaling test of rule 16.
4. **Verifier, calls and loops:** (#1328) function frames, `changes` and `fact_frame`, whole-fact clauses, start values at bindings' beginnings, loops, `together` and `start`.
5. **Diagnostics and repairs:** (#1329) the messages of rule 14, the ranked repairs of rule 15, the `unknown` reasons of rule 9, the Assumed zone entries.
6. **Formal model:** (#1330) the protocol-safety theorem and its two lemmas (rule 17).
7. **Benchmark argument:** (#1331) a new, pre-registered version of B04 (a variant with its own id, never an edit of the frozen task, whose `task.json` protects `scaffold/src/orders/types.una`), with its scaffold and reference under this record, run against the implementation with the hidden oracle (h06 and h09 included), in the benchmark repository (Decisions of acceptance 3).
8. **Record: facts over list elements** (#1332) (B05's batch form), a separate Proposed ADR.

## Open obligations

- The protocol-safety theorem (rule 17).
- B05 (example 6) and element facts (slice 8).
- Fact definitions in handlers (rule 12).
- The places where each contextual word is a word, in `spec/01` and `spec/02` (slice 1, #1325); the words themselves were decided on acceptance.

## Amendment (2026-11-06): the target is edition `0.8` (ADR 0139)

The Target line reads: edition `0.8`, closed by release R0.8, as a preview under the terms of ADR 0121 as its amendment of 2026-11-06 extends them. Editions `0.1` to `0.6` do not change. No other rule changes.
