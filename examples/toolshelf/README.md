# toolshelf

A community tool shelf: members check tools out for a number of days and bring them back, under two house rules.

The house rules are business rules of the kind that usually live in a wiki and in scattered `if` statements. Here each rule is a `requirement`, the limits are types, and the functions that apply the rules carry contracts that the solver proves for every input. The example also falls back from one shelf to another, which shows `attempt` and `with` working together.

What it demonstrates:

- two [requirements](../../docs/guide/language/requirements/requirement.md), traced to the functions that apply them;
- rules as types: ranges on a parameter and on a record field ([`where` and ranges](../../docs/guide/language/types/where.md));
- `reflect` [predicates](../../docs/guide/language/types/predicate.md) used in contracts;
- [`requires`](../../docs/guide/language/functions/requires.md) and [`ensures`](../../docs/guide/language/functions/ensures.md) on functions that call a capability;
- a fallback with [`attempt`](../../docs/guide/language/errors/attempt.md) and [`with`](../../docs/guide/language/capabilities/with.md);
- the host capability `Clock`, granted in `una.toml`.

## Run it

From this directory:

```sh
una check .   # copy examples/toolshelf
una test .
una report .
una run .
una build .
./target/una/toolshelf-example
```

`una run .` and the native executable print:

```text
Ana took the ladder for 14 days
returned 20 days later, it is 6 days late
```

The checkout is due 14 days after today, so a return 20 days after today is 6 days late. The output does not depend on the date, although `main` reads the clock. `una test .` runs the examples of both modules:

```text
example toolshelf.desk "the backup shelf has what the main shelf lacks" ... ok
example toolshelf.desk "the main shelf is tried first" ... ok
example toolshelf.tools "a member with room takes a tool for a week" ... ok
example toolshelf.tools "a tool that is not on the shelf cannot be taken" ... ok
example toolshelf.tools "a shelf that is offline stops the checkout" ... ok
example toolshelf.tools "bringing a tool back frees a place" ... ok
example toolshelf.tools "a tool returned three days after its due day is three days late" ... ok

7 examples: 7 passed, 0 failed
```

## How it works

The package has three modules: `toolshelf.tools`, the rules and the shelf; `toolshelf.desk`, the fallback between two shelves; and `toolshelf.main`, the program.

### The rules

```text
requirement SHELF_1
  text "A member holds at most three tools at a time"
  source human
  owner front_desk
  status active

requirement SHELF_2
  text "A checkout lasts from one to twenty-eight days"
  source policy "Tool Shelf Rules 3"
  owner front_desk
  status active
```

Each rule then appears twice in the code: as a type that makes a wrong value impossible to build, and as a contract on the function that changes the value.

```text
pub kind KeepDays = Int in 1..28

pub kind Member
  name: Text
  holding: Int in 0..3
```

`KeepDays` is `SHELF_2`. A `Member` whose `holding` is 4 cannot exist, which is `SHELF_1`.

### Predicates the solver can read

```text
pub predicate reflect can_check_out(m: Member) -> Bool = m.holding < 3

pub predicate reflect overdue(checkout: Checkout, today: Day) -> Bool = today > checkout.due
```

`reflect` makes the body visible to the solver, so it can reason with the predicate. The name still documents the intent: a contract that says `requires can_check_out(member)` reads as the rule, not as arithmetic.

### Checking a tool out

```text
pub check_out(member: Member, name: Text, today: Day, days: KeepDays) -> Checkout
  requires can_check_out(member)
  ensures result.due == today + days
  ensures result.member.holding == member.holding + 1
  uses @shelf: Shelf[pick]
  fails CheckoutError
  derived from SHELF_1, SHELF_2
=
  let tool = @shelf.pick(name)
    if error fail ShelfDown
    if none fail NotOnShelf(tool: name)
  Checkout(tool, member: Member(name: member.name, holding: member.holding + 1), due: today + days)
```

The header is the whole story for a reviewer: what the function needs, what it promises, what it may touch, how it can fail, and which rules it implements. The `requires` is what makes `member.holding + 1` fit in `0..3`. `bring_back` is the mirror image: it `requires checkout.member.holding > 0` and promises one tool less.

`late_days` shows how two `ensures` can pin a function down against a predicate: the result is positive exactly when the checkout is `overdue`, and zero otherwise.

### Falling back to a second shelf

```text
pub check_out_anywhere(member: Member, name: Text, today: Day, days: KeepDays) -> Checkout
  requires can_check_out(member)
  ensures result.due == today + days
  uses @main_shelf: Shelf[pick]
  uses @backup_shelf: Shelf[pick]
  fails CheckoutError
=
  let first_try = attempt check_out(member, name, today, days) with @shelf = @main_shelf
  when first_try
    is Ok(checkout) -> checkout
    is Err ->
      check_out(member, name, today, days) with @shelf = @backup_shelf
        if error pass
```

`check_out` uses a capability called `@shelf`. The desk holds two shelves under other names, and binds `@shelf` to one of them for each call with `with`. `attempt` keeps the first outcome as a `Result`, so a failure of the main shelf leads to the backup instead of out of the function. `if error pass` lets a failure of the backup leave the function unchanged, as a `CheckoutError`. The `ensures` follows from `check_out`'s own, on both paths.

### The program

`main` reads the clock and turns milliseconds into days with `now / 86_400_000`. It is the only function that touches the host, and `una.toml` grants it the console and the clock: `grants = ["console", "clock"]`. The shelf in `main` is the `shelf_with` handler over a fixed catalog.

## What `una check` proves

`una obligations .` lists 42 obligations, all proven:

- every `ensures` of `check_out`, `bring_back`, `late_days` and `check_out_anywhere`;
- a member never holds more than three tools: each call to `check_out` meets `can_check_out`, and the new count stays in `0..3`;
- every checkout lasts from 1 to 28 days at every call.

`una trace SHELF_1` shows the rule with what implements it:

```text
requirement SHELF_1 (active)
  text "A member holds at most three tools at a time"
  source human
  owner front_desk
toolshelf.tools.bring_back
  requires checkout.member.holding > 0
  ensures result.holding == checkout.member.holding - 1
  proven   ob-2ecf6008288c postcondition result.holding == checkout.member.holding - 1
  proven   ob-938f9eb8fced refinement    (checkout.member.holding - 1) in 0..3
toolshelf.tools.check_out
  requires can_check_out(member)
  ensures result.due == today + days
  ensures result.member.holding == member.holding + 1
  proven   ob-46cea04ba61e postcondition result.due == today + days
  proven   ob-f7b9f3eae75c postcondition result.member.holding == member.holding + 1
  proven   ob-c04f1a76c228 refinement    (member.holding + 1) in 0..3
```

## Common mistakes

### Applying a rule without checking it

Delete the `requires can_check_out(member)` line of `check_out`. A member who already holds three tools could take a fourth:

```text
[U5001] Error: refinement `(member.holding + 1) in 0..3` is refuted
    ╭─[ src/toolshelf/tools.una:62:61 ]
    │
 27 │   holding: Int in 0..3
    │                   ──┬─
    │                     ╰─── range
    │
 62 │   Checkout(tool, member: Member(name: member.name, holding: member.holding + 1), due: today + days)
    │                                                             ─────────┬────────
    │                                                                      ╰────────── refinement `(member.holding + 1) in 0..3` is refuted
    │
    │ Help 1: repair 1: add a guard (`if` or a `when` arm) that handles the values where `(member.holding + 1) in 0..3` does not hold
    │
    │ Help 2: repair 2 (escape: checked; a runtime test replaces the static proof: the program fails at run time where the proof would have rejected it; it counts against [escape_budget] and is listed in the build report): check the value with `checked(value, (member.holding + 1) in 0..3) else fail <Variant>`
    │
    │ Note 1: obligation ob-c04f1a76c228 (refinement) is refuted: (member.holding + 1) in 0..3
    │
    │ Note 2: counterexample: days = 1, member = Member#0, member.holding = 3, member.name = Text#0, tool = Tool#0
    │
    │ Note 3: run `una explain U5001` for an explanation
────╯
```

The counterexample names the case: `member.holding = 3`.

### A caller that breaks the rule

Put the `requires` back, and in the first example of `tools.una` give Ana three tools, `Member(name: "Ana", holding: 3)`. The call in the example is now refuted:

```text
[U5001] Error: precondition `can_check_out(Member(name: "Ana", holding: 3))` of this call is refuted
    ╭─[ src/toolshelf/tools.una:96:6 ]
    │
 53 │   requires can_check_out(member)
    │            ──────────┬──────────
    │                      ╰──────────── requires clause
    │
 96 │   do check_out(Member(name: "Ana", holding: 3), name: "drill", today: 100, days: 7)
    │      ───────────────────────────────────────┬──────────────────────────────────────
    │                                             ╰──────────────────────────────────────── precondition `can_check_out(Member(name: "Ana", holding: 3))` of this call is refuted
    │
    │ Help 1: repair 1: add a guard (`if` or a `when` arm) that handles the values where `can_check_out(Member(name: "Ana", holding: 3))` does not hold
    │
    │ Help 2: repair 2 (escape: checked; a runtime test replaces the static proof: the program fails at run time where the proof would have rejected it; it counts against [escape_budget] and is listed in the build report): check the value with `checked(value, can_check_out(Member(name: "Ana", holding: 3))) else fail <Variant>`
    │
    │ Note 1: obligation ob-1ce6f94f8309 (precondition) is refuted: can_check_out(Member(name: "Ana", holding: 3))
    │
    │ Note 2: run `una explain U5001` for an explanation
────╯
```

### A checkout that is too long

In `main`, ask for the ladder for 30 days, `days: 30`:

```text
[U5001] Error: refinement `30 in 1..28` is refuted
    ╭─[ src/toolshelf/main.una:15:90 ]
    │
 15 │   let checkout = check_out(Member(name: "Ana", holding: 0), name: "ladder", today, days: 30) with @shelf = shelf_with(tools: catalog())
    │                                                                                          ─┬
    │                                                                                           ╰── refinement `30 in 1..28` is refuted
    │
    ├─[ src/toolshelf/tools.una:16:28 ]
    │
 16 │ pub kind KeepDays = Int in 1..28
    │                            ──┬──
    │                              ╰──── range
    │
    │ Note 1: obligation ob-6d88845711a7 (refinement) is refuted: 30 in 1..28
    │
    │ Note 2: run `una explain U5001` for an explanation
────╯
```

The rule is enforced where the value enters, in another module, with a pointer back to the type that states it.

## Extend it

1. **Both shelves down.** Add an example to `desk.una` where both shelves are `offline_shelf()`, and expect `fails ShelfDown`. *Hint:* import `offline_shelf` from `toolshelf.tools`.
2. **A late fee.** Add `pub late_fee(checkout: Checkout, today: Day) -> Nat`, three units for each late day, with `ensures not overdue(checkout, today) implies result == 0`. *Hint:* reuse `late_days`. Add an example: a checkout due on day 10 and returned on day 16 costs 18.
3. **A fee the solver cannot prove.** Make the fee a tenth of the tool's deposit for each late day: `late_days(checkout, today) * (checkout.tool.deposit / 10)`. `una check .` now reports U5002, not U5001: the obligations are *unknown*, because the solver leaves a product of two values uninterpreted. Unknown is not refuted. It means that the proof needs a different shape, such as a fee per day that is a constant, or a fee table chosen with `when`.

## Learn more

- Construct pages: [`requirement`](../../docs/guide/language/requirements/requirement.md), [predicates](../../docs/guide/language/types/predicate.md), [`where` and ranges](../../docs/guide/language/types/where.md), [`requires`](../../docs/guide/language/functions/requires.md), [`ensures`](../../docs/guide/language/functions/ensures.md), [`attempt`](../../docs/guide/language/errors/attempt.md), [`with`](../../docs/guide/language/capabilities/with.md), [`if error`](../../docs/guide/language/errors/if-error.md), [host capabilities](../../docs/guide/language/capabilities/host.md).
- In the Learn section of the website: chapter 6, "Failure", chapter 7, "Contracts and the verifier", and chapter 10, "Testing".
- Specification: [refinements](../../docs/spec/03-types.md#s3), [predicates](../../docs/spec/03-types.md#s4), [clauses](../../docs/spec/04-functions-and-contracts.md#s2), [passing capabilities](../../docs/spec/05-capabilities-and-effects.md#s4), [`attempt`](../../docs/spec/06-errors.md#s5), [requirements](../../docs/spec/08-requirements-and-examples.md#s1).

## Edition

Edition `0.1` ([edition](../../docs/guide/configure/package/edition.md)).
