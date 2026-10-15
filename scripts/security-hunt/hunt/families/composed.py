"""Composed families: two blind rows at once, where each rule has to hold inside the other's
construct.

- CS, `together` over user sum types (`spec/19 §3` with `spec/07 §3` rule 2, §7 rule 3): a task
  matches a secret sum value, or reads a secret variant field, or a secret `is` decides how many
  tasks run. Edition 0.2; in 0.1 every program is U0103.
- PG, failure payloads holding generic kinds (`spec/07 §7` rules 2 and 3, ADR 0046): a payload
  field of a covariant `Box[secret Int]`, of an invariant `Sink[Int]`, or nested in `Option`,
  recovered after `attempt` and sent where its labels do not allow.

Each must-reject shape has a twin on public data.
"""

from ..program import MUST_REJECT, OPEN, Program, rng_for
from ..una import header, join_decls, package

MOD = "app.main"

GUESS = "pub kind Guess = Hit(pin: Int) | Miss"
VAULT = "pub kind Vault = Open(code: secret Int) | Shut"
PUB_VAULT = "pub kind Vault = Open(code: Int) | Shut"
LOG = "pub capability Log\n  op write(line: Text) -> Unit"
USES_LOG = "  uses @log: Log[write]\n"
BOX = "pub kind Box[T]\n  value: T"
SINK = "pub kind Sink[T]\n  eat: fn(T) -> Int"


def _make(op, rule, edition, seed, cls, text, expect=(), variant="", gate_0_1=False):
    if gate_0_1 and edition == "0.1":
        cls, expect = MUST_REJECT, ["U0103"]
    files = package("hunt", edition, {MOD: text})
    return Program(op=op, rule=rule, edition=edition, seed=seed, cls=cls, files=files,
                   expect=list(expect), variant=variant,
                   note="edition 0.1 reserves `together`" if gate_0_1 and edition == "0.1" else "")


# CS1. Tasks over a list of sum values: the list's elements are secret (each task matches a secret
# value) or a variant's field is secret (each task reads it), with a public effect in the task.
def cs1(seed, edition):
    rng = rng_for(seed)
    form = rng.choice(["match-secret-element", "secret-field"])
    twin = rng.random() < 0.25
    if form == "match-secret-element":
        kinds = GUESS
        param = "gs: List[Guess]" if twin else "gs: List[secret Guess]"
        body = ('  together for g in gs\n    when g\n      is Hit(pin) -> @log.write("hit")\n'
                "      is Miss -> unit")
    else:
        kinds = PUB_VAULT if twin else VAULT
        param = "gs: List[Vault]"
        body = ('  together for g in gs\n    when g\n      is Open(code) -> @log.write("{code}")\n'
                "      is Shut -> unit")
    fn = f"pub run({param}) -> List[Unit]\n" + USES_LOG + "=\n" + body
    text = join_decls(header(MOD), kinds, LOG, fn)
    return _make("CS1", "spec/19 §3, spec/07 §7 rule 3", edition, seed,
                 OPEN if twin else MUST_REJECT, text, [] if twin else ["U4003"],
                 variant=form + ("-twin" if twin else ""), gate_0_1=True)


# CS2. A branch binds a value read from a secret sum, which keeps its label after the block, and a
# secret `is` decides how many tasks run.
def cs2(seed, edition):
    rng = rng_for(seed)
    form = rng.choice(["branch-keeps-match", "count-by-is"])
    twin = rng.random() < 0.25
    lab = "" if twin else "secret "
    if form == "branch-keeps-match":
        fn = (f"pub run(g: {lab}Guess, n: Int) -> Unit\n" + USES_LOG + "=\n"
              "  together\n    let a = when g\n      is Hit(pin) -> pin\n      is Miss -> 0\n"
              '    let b = n + 1\n  @log.write("{a}")')
        expect = ["U4003"]
    else:
        fn = (f"pub run(g: {lab}Guess) -> List[Unit]\n" + USES_LOG + "=\n"
              "  let k = if g is Miss then 1 else 2\n"
              '  together for i in 0..<k\n    @log.write("one")')
        expect = ["U4003"]
    text = join_decls(header(MOD), GUESS, LOG, fn)
    return _make("CS2", "spec/19 §3, spec/07 §7 rule 3", edition, seed,
                 OPEN if twin else MUST_REJECT, text, [] if twin else expect,
                 variant=form + ("-twin" if twin else ""), gate_0_1=True)


# PG1. A payload field of a generic kind, recovered after `attempt` and sent to a public result or
# print: a covariant `Box[secret Int]`, the same nested in `Option`, or an invariant `Sink[Int]`
# taken as a `Sink[secret Int]`.
def pg1(seed, edition):
    rng = rng_for(seed)
    form = rng.choice(["box-return", "box-print", "option-box-return", "sink-as-secret"])
    twin = rng.random() < 0.25
    lab = "" if twin else "secret "
    decls = [header(MOD), BOX, SINK]
    if form.startswith("box"):
        decls.append(f"pub kind Bad = | Oops(b: Box[{lab}Int])")
        inner = f"inner(pin: {lab}Int) -> Int\n  fails Bad\n=\n  fail Oops(b: Box(value: pin))"
        arm = "is Oops(b) -> b.value"
    elif form == "option-box-return":
        decls.append(f"pub kind Bad = | Oops(o: Option[Box[{lab}Int]])")
        inner = (f"inner(pin: {lab}Int) -> Int\n  fails Bad\n=\n"
                 "  fail Oops(o: Some(Box(value: pin)))")
        arm = ("is Oops(o) ->\n          when o\n            is Some(b) -> b.value\n"
               "            is None -> 0")
    else:
        decls.append("pub kind Bad = | Oops(s: Sink[Int])")
        inner = ("inner(pin: Int) -> Int\n  fails Bad\n=\n"
                 "  fail Oops(s: Sink(eat: fn(x: Int) -> x))")
        # The recovered sink, built at `public`, is used where a secret is handed to it.
        target = "Sink[Int]" if twin else "Sink[secret Int]"
        pin = "pin: Int" if twin else "pin: secret Int"
        fn = (f"pub run({pin}) -> Int\n=\n  let r = attempt inner(0)\n  when r\n"
              "    is Ok(k) -> k\n    is Err(e) ->\n      when e\n        is Oops(s) ->\n"
              f"          let t: {target} = s\n          t.eat(pin)")
        decls += [inner, fn]
        return _make("PG1", "spec/07 §7 rule 2 (ADR 0046)", edition, seed,
                     OPEN if twin else MUST_REJECT, join_decls(*decls), [] if twin else ["U4001"],
                     variant=form + ("-twin" if twin else ""))
    arm_line = (f"        {arm}" if "\n" not in arm else f"        {arm}")
    recover = (f"  let r = attempt inner(pin)\n  let n = when r\n    is Ok(k) -> k\n"
               f"    is Err(e) ->\n      when e\n{arm_line}\n")
    pin = f"pin: {lab}Int"
    if form == "box-print":
        fn = (f"pub run({pin}) -> Unit\n  uses @console: Console[print]\n=\n" + recover +
              '  @console.print("{n}")')
        expect = ["U4003"]
    else:
        fn = f"pub run({pin}) -> Int\n=\n" + recover + "  n"
        expect = ["U4001"]
    decls += [inner, fn]
    return _make("PG1", "spec/07 §7 rules 2 and 3", edition, seed,
                 OPEN if twin else MUST_REJECT, join_decls(*decls), [] if twin else expect,
                 variant=form + ("-twin" if twin else ""))


OPERATORS = {"CS1": cs1, "CS2": cs2, "PG1": pg1}
