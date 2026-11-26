# 17. Memory model

Status: Normative, v0.2 (ADR 0056).

Una has mutable value semantics. Values are immutable, and there are no references and no lifetimes. A binding declared editable can be rebound, and a parameter can be passed in and out (`edit`) or consumed (`take`). Every construct of this chapter desugars into Core IR (§4). The UAM, the verifier and the information-flow rules then apply to the desugared program unchanged.

**Editions.** The constructs of this chapter exist only in packages of edition `0.2` and later (`edition = "0.2"` in `una.toml`, `spec/13 §7`).
- In edition `0.1`, the parser keeps the keyword tables of v0.1 (`spec/01 §5`): `old` is an ordinary name, `weak` is not reserved, and `edit`, `take`, `region`, `shared` and `iso` give the U0103 messages of v0.1.
- `for` stays limited to `handler h for K`, so a `for` loop is a syntax error (U0101).
- A single file has the default settings, and so edition `0.1`.
- The benchmark baseline stays on edition `0.1`.

(v0.5, ADR 0137.) **Edition `0.5` writes the binding forms `const` and `var`.** `const x = e` is the plain `let x = e` of edition `0.4` and `var x = e` its `let edit x = e`, with the same typing, desugaring (§4), verification (§5), information flow (§6) and run-time behavior (§7). Every rule of this chapter that names a plain `let` holds of a `const` binding in edition `0.5`, and every rule that names a `let edit` binding holds of a `var` binding. `edit` and `take` stay the markers of parameters, arguments and method receivers, with the meaning of §3; they no longer appear in a binding statement. `let` is U7007 in edition `0.5` (`spec/01 §5`).

## 1. Syntax

These productions extend `spec/02`.

```ebnf
param           = [ "edit" | "take" ] lower_ident ":" type ;
arg             = [ lower_ident ":" ] ( "edit" place | "take" lower_ident | line_expr ) ;
place           = lower_ident { "." lower_ident } ;

statement       = let_stmt | assign_stmt | for_stmt | if_stmt | handled ;
let_stmt        = "let" [ "edit" ] lower_ident [ ":" type ] "=" ( handled | block_expr ) ;
assign_stmt     = place "=" ( handled | block_expr ) ;
for_stmt        = "for" lower_ident "in" ( line_expr | range_expr ) NEWLINE INDENT block DEDENT ;
range_expr      = add_expr ( ".." | "..<" ) add_expr ;
if_stmt         = "if" line_expr NEWLINE INDENT block DEDENT ;      (* no else: a Unit statement *)

postfix         = [ "edit" ] primary { "." lower_ident [ call_args ] | call_args } ;
```

- A method receiver marked `edit` is written before the receiver: `edit total.record(e)`.
- `old` is a keyword. `old(x)` appears only in `ensures` clauses (§3.6).
- An `if_stmt` is a block-form `if` without `else`, and it appears only where a statement can appear. A block whose last element would be an `if_stmt` gets `unit` as its final expression.
- A block whose last element is an `assign_stmt` also gets `unit` as its final expression, as in a `Unit` function that ends by updating an `edit` parameter.
- `for` is already a keyword (`handler h for K`).

(v0.5, ADR 0137 rule 2.) In edition `0.5`, the binding statement is

```ebnf
let_stmt        = ( "const" | "var" ) lower_ident [ ":" type ] "=" ( handled | block_expr ) ;
```

where `var` is the editable form; `const edit x` and `var edit x` are U0101.

## 2. Places and editable bindings

1. An **editable binding** is a binding introduced by `let edit`, or a parameter declared `edit`.
2. A **place** is an editable binding, or a field path of one: `r`, `r.f`, `r.f.g`. Each field in a path MUST be a field of a record kind. A field of a variant payload is not a place.
3. **Assignment.** `p = e` rebinds the place `p` to the value of `e`.
   - `p` MUST be rooted at an editable binding (U2019). A plain `let`, a parameter without `edit`, and a binding captured by a lambda are not editable.
   - There are no compound assignment operators and no assignment to list elements: elements change through `set` (§7).
   - (v0.5, ADR 0137 rule 4.) In edition `0.5`, an editable binding is a `var` binding or a parameter declared `edit`, and a place is rooted at one. An assignment to a `const` binding, to a parameter without `edit`, to a loop variable, to a pattern binding, to a lambda parameter, to a binding captured by a lambda, or to a module constant is U2019, whose message reads `` `x` is not editable; declare it `var x`, or the parameter `edit x` ``. Pattern bindings, loop variables, lambda parameters and handler parameters are introduced without a keyword and behave as `const` bindings (rule 6).
4. **Fixed type.** An editable binding keeps its declared type at every rebind, refinement and label included (§5, §6). Without an annotation, its type is the type of its initial value without its refinement: `let edit n = p` with `p: Pos` gives `n` the type `Int`. An annotation, as in `let edit n: Pos = p`, makes the refinement an invariant. (v0.3, ADR 0103.) Without an annotation it also drops every refinement of a nested position, as `let edit xs = [p]` is a `List[Int]`; an annotation, as `let edit out: List[Pos] = []`, keeps them, and so does the invariant of a loop that updates it (§5 rule 3). (v0.3, ADR 0075.) Without an annotation, every label position the initial value leaves undetermined, at any depth, is a label variable: the top of each position of its type that is still an unsolved type variable when the binding is made, such as the key and value of `empty_map()`, the element of `[]` or the value of `None`, and every label position inside the type that variable comes to stand for. A label variable is the least label above every label that flows into its position (§6 rule 1). The other positions keep the labels the initial value gives them, the binding's top label included. Such a position whose type holds a function type is U4001: the binding needs an annotation.

## 3. Parameter conventions

1. **Declaration.** A parameter declared `edit x: T` is passed in and out: the function receives the caller's value and returns its final value with its result. A parameter declared `take x: T` consumes its argument.
2. **Call-site markers.** A caller writes `edit p` exactly where the parameter is declared `edit`, and `take a` exactly where it is declared `take`. A marker on a plain parameter, or a missing marker, is U2020.
   - **Method receivers** follow the same rule: `edit total.record(e)` passes `total` to a first parameter declared `edit`.
   - **The argument of `edit`** is a place (§2). An argument that is not a place, as in `f(edit g(x))`, is U2020. A place rooted at a binding that is not editable is U2019 (§2.3).
   - **The argument of `take`** is a binding, not a field: `f(take r.f)` is U2020. Any local binding can be taken: a plain `let`, a `let edit` or a parameter. Only an assignment revives it (rule 3), so a plain binding stays taken.
3. **Affine `take`.**
   - After `take a`, every use of `a` is U2017, until an assignment to `a` gives it a new value. Only a `let edit` binding can be reassigned.
   - A binding taken in one branch of an `if` or a `when` counts as taken after the construct.
   - A binding taken in a loop body is taken in every iteration after the first, which is U2017. A binding taken in a loop body is therefore assigned before the end of the body, or is declared inside it.
4. **Exclusivity.** In one call, an `edit p` argument together with any other use of the root binding of `p`, in the same call or as another `edit` or `take` argument, is U2018. This holds even for disjoint fields: `f(edit r.x, y: r.y)` is U2018.
   - (v0.5, ADR 0137 rule 5.) In edition `0.5`, the argument of `edit` is a place rooted at a `var` binding or an `edit` parameter; one rooted elsewhere is U2019. Any local binding can be taken, a `const`, a `var` or a parameter; only a `var` can be assigned, so only a `var` is revived, and a binding taken in a loop body is assigned before the end of the body, which needs a `var`, or is declared inside the body. A module constant is not a local binding (ADR 0126 rule 13): `f(take origin)` is U2020.
5. **One effect per line.** A call with an `edit` or `take` argument is the whole `line_expr` of a statement or a `let` (U3013, `spec/02 §8`). As for any call, a statement call of a non-`Unit` type is U2011, so its result is bound with `let`.
   - A lambda body is not a statement, so a call with a marker there is U3013.
   - **Known limit of v0.2.** A `do` line of an example is not a statement either, and it has no editable binding, so a call with a marker there is U3013. An example exercises a convention through a function that takes a plain argument and edits a local copy, as in `let edit copy = n`, `bump(edit copy)`, `copy`.
6. **`old`.**
   - In an `ensures` clause, an `edit` parameter `x` denotes its final value, and `old(x)` its value on entry.
   - `old(x)` is legal only in `ensures`, and only on an `edit` parameter: U2012 elsewhere, as for `result`.
   - `requires` clauses read entry values, and `old` is not legal there.
7. **Function values.** A function with an `edit` or `take` parameter cannot be used as a value: naming it anywhere but in the callee position of a call is U2021. Function types are unchanged, and have no conventions.
8. **Capabilities.** Operations and handlers cannot declare `edit` or `take` parameters (U2020). Capabilities are second-class and not values, so `edit` and `take` never apply to them.

## 4. Desugaring to Core IR

Core IR gains tuple types `(τ, ...)`, tuple construction `(a, ...)`, and the binder `let (x, ...) = r in e`. Neither is legal surface syntax.

| Surface | Core IR |
| --- | --- |
| `fun f(edit x: T, y: U) -> R` | `fun f(x: T, y: U) : (T, R)`, where each normal return `e` becomes `(x', e)` with `x'` the final value of `x` |
| `let r = f(edit a, y: b)` | `let (%a1, r) = f(a, b) in ...`, with `a` renamed to `%a1` afterwards |
| `f(edit r.g, y: b)` | `let %t = r.g in let (%t1, %v) = f(%t, b) in let %r1 = C(r.f1, ..., %t1, ...) in ...`, rebuilding the record of `r` with `g` changed |
| `let edit x = e` | `let x: T = e in ...` |
| `x = e` | `let %x1: T = e in ...`, with `x` renamed to `%x1` afterwards (SSA) |
| `r.g = e` | `let %v = e in let %r1: T = C(r.f1, ..., %v, ...) in ...` |
| `f(take a)` | `f(a)`; `take` is checked statically and leaves no trace |
| `if c` block without `else` | `if c then block else unit`, with the updates of the block joined at the end of each branch |
| `for x in xs` body | a call of a generated recursive function `%loop(xs, b1, ..., bn) : (T1, ..., Tn)`, over the bindings `b1` to `bn` that the body updates (§5) |

**Joins.** At the end of an `if`, a `when` or a loop, an editable binding updated in some branch is the value the branch that ran left it. In Core IR, each branch returns the updated bindings, and the construct binds them afresh.

**Loops.**
- The generated function takes the list, or the range bounds and the current index, and the current values of the updated bindings.
- It returns their final values.
- Its capability context and its error kind are those of the enclosing function, restricted to what the body uses.
- A loop over a list visits the elements in order, and a loop over `a..<b` or `a..b` visits the integers in increasing order. The bounds are evaluated once, before the first iteration.
- There is no `break` and no `continue`.
- (ADR 0056, amendment of 2026-10-02.) Core IR represents the generated function as a `Loop` node with the same meaning, whose iterations are not calls: they do not count toward the call depth of `spec/11` (U6003). An inclusive range ends at its upper bound without computing its successor, so `a..b` up to the greatest value of a fixed-width type does not overflow.
- **The loop variable** is a plain binding (U2019 on assignment). It has no `edit` form, so the message of U2019 on it says that it is read-only and suggests a separate `let edit` binding in the body, as `let edit next_x = x`; the variable of a `together for` (`spec/19 §1`) gets the same message. It carries the labels of the elements and the top label of the list, or the labels of the bounds. A range's bounds are integers of one type, `Int` or a fixed-width type, and a literal bound takes the type of the other.
- **As a statement**, a loop has the value `unit`, also as the last line of a block.

## 5. Static semantics

1. **Types.** The checker checks the desugared program by the rules of `spec/10`. An assignment `p = e` checks `e` against the fixed type of `p` (§2.4), as an annotated `let` does (`spec/10 §5`).
2. **Refinements at every rebind (V2-11).** Each assignment, each `edit` call and each join carries the refinement obligation of an annotated `let` (ADR 0037). So an editable binding never holds a value outside its refinement.
3. **Loop invariant.**
   - The generated function of a loop has, as `requires` and `ensures`, the refinements of the bindings it updates. These are the loop invariant.
   - The verifier checks the body once, for any iteration, with the updated bindings fresh at their declared types, and keeps the facts of the enclosing context, since the bindings the loop does not update keep their values. After the loop, the updated bindings are fresh again at their declared types: no fact about their values survives it (ADR 0056, amendment of 2026-10-02).
   - Inside the body, the loop variable of a range loop carries the fact `a <= i < b`, or `a <= i <= b` for an inclusive range, as a hypothesis, not as its type. Inside the body of a loop over a list, the list has at least one element (#278).
   - (v0.2, ADR 0071.) **The length frame.** A list binding that the loop updates is length-preserving when every path of the body that completes an iteration gives it a value that keeps the iteration's starting length. That holds through an alias, through the component of an `edit` argument whose callee ensures exactly `p.len == old(p).len` (as `set` does, `spec/14 §1`), through an `if` or `case` whose completing branches all keep it, and through a nested loop that keeps it. Anything else breaks it: `append`, a `take` argument, any other call, a literal or another list. For a length-preserving binding `x` with initial value `x₀`, the verifier assumes `x.len == x₀.len` in the body and of its value after the loop.
   - There is no `invariant` clause in v0.2.
4. **Lambdas.** A lambda captures the value an editable binding has when the lambda is created (V2-10). Inside the lambda the binding is not editable (U2019), and later updates outside are not seen.
5. **Termination** is not checked, as for recursion (`spec/04 §2` rule 3).

## 6. Information flow

1. **Fixed labels (V2-16).** An editable binding's declared label is fixed. (v0.3, ADR 0075.) The label variables of an un-annotated binding (§2 rule 4) are solved first, over every assignment, `edit` argument and join of the binding, and then fixed: every read sees the solution, and every flow is checked against it.
   - An assignment, an `edit` call or a join flows the new value into the declared type (`spec/07 §7` rule 2).
   - A rebind under a program counter above the binding's label is U4001 at the value, never a silent raise.
2. **`edit` parameters.** The desugar returns the parameter's final value at its declared type, which flows back into the caller's place. In effect, an `edit` argument and its parameter carry the same labels.
3. **Loops.**
   - A `for` over a list runs its body under the top label of the list, and a range loop under the labels of its bounds.
   - In that body, `fail` and calls that can fail are U4002, and every sink must accept the program counter (`spec/07 §7` rule 4).

## 7. Dynamic semantics

1. **Transactional values (V2-5).** A call that fails leaves every `edit` argument unchanged for the caller, since the desugared call returns the new values only on success. A loop whose body fails leaves the bindings it updates at their values from before the loop.
2. **Effects are not rolled back.** Operations performed before the failure stay performed, and the trace records them (`spec/11 §5`), as in v0.1 when a function fails after an operation:

   ```una
   notify(edit sent: Int, to: Text) -> Unit
     uses @mail: Mail[send]
     fails MailError
   =
     @mail.send(to)
       if error pass
     sent = sent + 1
     check_quota(sent)
       if error pass
   ```

   If `check_quota` fails, the caller's `sent` keeps its old value, but the mail was sent and the trace shows it.
3. **Backend freedom.** The meaning of every construct is that of its desugared form in the UAM (`spec/11 §1`). A backend MAY update a uniquely held value in place, reuse the memory of a value passed by `take`, move an argument into a call, a call through a function value included, at its last use (ADR 0072), and choose where values live. None of these is observable.
   - (v0.2, ADR 0072.) A binding passed to a call gives its value up to the callee when nothing that runs after the call reads it, on success or on failure. The callee may then update that value in place. A binding that the code after a failure reads keeps its value, as rule 1 requires (`spec/16 §4` rule 7).

## 8. Standard library

The prelude (`spec/14 §1`) gains one function:

| Function | Contract |
| --- | --- |
| `set[T](edit xs: List[T], index: Int, item: T) -> Unit` | `requires 0 <= index and index < xs.len`; `ensures xs.len == old(xs).len` |

It is written `edit xs.set(index: i, item: v)`. Graphs and cyclic structures use arenas and indices (V2-3, issue #82).

## 9. Reserved words

`edit`, `take` and `old` become keywords. The table of `spec/01 §5` changes as follows:

| Keywords | Message |
| --- | --- |
| `shared`, `iso`, `weak` | reserved for a later edition (aliasing) |
| `region` | not a keyword in Una; the backend chooses where values live |
| `var` | not a keyword in Una; use `let edit` |
| `while`, `loop` | not a keyword in Una; use `for` or recursion |

(v0.2, ADR 0065.) `together` also becomes a keyword, with the rows of `spec/19 §9`.

(v0.5, ADR 0137 rule 1.) In edition `0.5`, `const` and `var` are keywords, so the row of `var` is removed; `let` is reserved and is U7007 (`spec/01 §5`). Every other row is that of edition `0.4`.

## 10. Diagnostic codes

These codes join the registry of `spec/13 §4` with their implementation:

| Code | Domain | Meaning |
| --- | --- | --- |
| U2017 | types | Use of a binding after it was passed by `take` |
| U2018 | types | An `edit` argument together with another use of its binding in the same call |
| U2019 | types | Assignment to a binding that is not editable |
| U2020 | types | A parameter convention (`edit`, `take`) not matched at the call site, or declared where none is allowed |
| U2021 | types | A function with `edit` or `take` parameters used as a value |
