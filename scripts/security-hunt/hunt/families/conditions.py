"""Family Q: conditions on allowed effects and contract locks (ADR 0092, `spec/05 §3`).

An operation of a `uses` may carry `where` and a condition over the function's parameters, in the
refinement language: a `Bool` without `checked`, an operation, `result` or `it` (U2012 to U2016).
Each use of the operation must be where the condition holds, an obligation the full check proves
(U5001); a function type's `uses` take no condition (U0101), and a named function with one is not a
function value (U3006). A `[contracts]` lock bounds a function's header: an operation it does not
list is U3023 at tier 0, and a condition that does not imply the lock's is U3023 through the
solver. Both are edition 0.3's: in earlier editions every program of the family is the edition
gate's probe, `where` in a `uses` (U0101) or an unknown table (U7002).
"""

from ..program import MUST_REJECT, OPEN, RESIDUAL, Program, rng_for
from ..una import Varied, header, join_decls, package

MOD = "app.main"
RULE = "ADR 0092, spec/05 §3"
PARAMS = ("dry", "quiet", "off")

LOG = ("capability Log\n  op write(n: Int) -> Unit\n  op count(n: Int) -> Int")


def _gate(edition, cls, expect, note, codes=("U0101",)):
    """Before 0.3, the edition gate's probe: `where` in a `uses` does not parse (U0101), and
    `[contracts]` or `[boundaries]` is an unknown table (U7002)."""
    if edition in ("0.1", "0.2"):
        return MUST_REJECT, list(codes), "edition 0.3 adds conditions on operations"
    return cls, expect, note


# Q1. What tier 0 rejects of a condition: one in a function type (U0101), one with `result`
# (U2012), `it` (U2013), an operation (U2014) or an opaque predicate (U2016), a conditioned
# function used as a value (U3006), a header granting an operation its lock does not list
# (U3023), and a condition in a `[boundaries]` ceiling (U7002).
Q1_FORMS = ("fn-type", "result", "it", "operation", "opaque", "not-a-value", "lock-added",
            "ceiling")


def q1(seed, edition):
    rng = rng_for(seed)
    form = rng.choice(Q1_FORMS)
    p = rng.choice(PARAMS)
    k = rng.choice([0, 1, 3])
    decls = [header(MOD), LOG]
    tables, gate_codes, modules = [], ("U0101",), {}

    def fn(cond, body="  unit", name="update"):
        return (f"pub {name}(n: Int, {p}: Bool) -> Unit\n  uses @log: Log[write where {cond}]\n"
                f"=\n{body}")
    if form == "fn-type":
        decls.append(f"pub apply(f: fn(Int) -> Unit uses @log: Log[write where n > {k}], "
                     "n: Int) -> Unit\n  uses @log: Log[write]\n=\n  f(n)")
        expect = ["U0101"]
    elif form == "result":
        decls.append(fn("result"))
        expect = ["U2012"]
    elif form == "it":
        decls.append(fn(f"it > {k}"))
        expect = ["U2013"]
    elif form == "operation":
        decls.append(fn(f"@log.count(n) > {k}"))
        expect = ["U2014"]
    elif form == "opaque":
        decls.append(f"predicate opaque above(n: Int) -> Bool = n > {k}")
        decls.append(fn("above(n)"))
        expect = ["U2016"]
    elif form == "not-a-value":
        decls.append(f"pub guarded(n: Int) -> Unit\n  uses @log: Log[write where n > {k}]\n=\n"
                     f"  if n > {k}\n    @log.write(n)\n  unit")
        decls.append("pub by_let(n: Int) -> Unit\n  uses @log: Log[write]\n=\n"
                     "  let f = guarded\n  f(n)")
        expect = ["U3006"]
    elif form == "lock-added":
        decls.append(f"pub update(n: Int, {p}: Bool) -> Unit\n"
                     f"  uses @log: Log[write where not {p}, count]\n=\n"
                     f"  if not {p}\n    @log.write(n)\n  unit")
        tables = [("contracts", [(f"{MOD}.update", [f"@log: Log[write where not {p}]"])])]
        expect = ["U3023"]
        gate_codes = ("U0101", "U7002")
    else:
        modules["app.edge"] = join_decls(header("app.edge"), "pub size_of(n: Int) -> Int = n")
        decls.append(f"pub update(n: Int, {p}: Bool) -> Unit\n  uses @log: Log[write]\n=\n"
                     "  @log.write(n)")
        tables = [("boundaries", [("app.edge", [f"Log[write where n > {k}]"])])]
        expect = ["U7002"]
        gate_codes = ("U7002",)
    modules[MOD] = join_decls(*decls)
    cls, expect, note = _gate(edition, MUST_REJECT, expect, "", gate_codes)
    files = package("hunt", edition, modules, tables=tables)
    return Program(op="Q1", rule=RULE, edition=edition, seed=seed, cls=cls, files=files,
                   expect=expect, variant=f"{form}-{p}", note=note)


# Q2. What only the full check decides: a conditioned operation used where its condition may be
# false (U5001), through a lambda, an unconditioned helper, a callee that promises less, or a
# reassigned parameter, and a header whose condition does not imply its lock's (U3023). Tier 0
# accepts them all, so they are residual rows the full check must reject with the expected code
# (`judge_residual`); their twins prove, and are recorded.
Q2_REFUTED = {"unguarded": "U5001", "lambda": "U5001", "helper": "U5001",
              "promise-wrong": "U5001", "reassigned": "U5001", "lock-weakened": "U3023"}
Q2_PROVEN = ("guarded", "required", "promise")


def q2(seed, edition):
    rng = rng_for(seed)
    form = rng.choice(sorted(Q2_REFUTED) + list(Q2_PROVEN))
    p = rng.choice(PARAMS)
    k = rng.choice([0, 2, 5])
    head = f"pub run(n: Int, {p}: Bool) -> Unit\n  uses @log: Log[write where not {p}]\n"
    decls = [header(MOD), LOG]
    tables, gate_codes = [], ("U0101",)
    if form == "unguarded":
        decls.append(head + "=\n  @log.write(n)")
    elif form == "lambda":
        decls.append(f"pub run(xs: List[Int], {p}: Bool) -> List[Unit]\n"
                     f"  uses @log: Log[write where not {p}]\n=\n"
                     "  xs.map(each: fn(n) -> @log.write(n))")
    elif form == "helper":
        decls.append("helper(n: Int) -> Unit = @log.write(n)")
        decls.append(head + "=\n  helper(n)")
    elif form in ("promise-wrong", "promise"):
        decls.append("promises(n: Int, hush: Bool) -> Unit\n  uses @log: Log[write where not hush]"
                     "\n=\n  if not hush\n    @log.write(n)\n  unit")
        arg = "false" if form == "promise-wrong" else p
        decls.append(head + f"=\n  promises(n, hush: {arg})")
    elif form == "reassigned":
        decls.append(f"pub run(n: Int, edit {p}: Bool) -> Unit\n"
                     f"  uses @log: Log[write where not {p}]\n=\n  {p} = false\n  @log.write(n)")
    elif form == "lock-weakened":
        decls.append(f"pub run(n: Int, {p}: Bool) -> Unit\n"
                     f"  uses @log: Log[write where n > {k}]\n=\n"
                     f"  if n > {k}\n    @log.write(n)\n  unit")
        tables = [("contracts", [(f"{MOD}.run", [f"@log: Log[write where not {p}]"])])]
        gate_codes = ("U0101", "U7002")
    elif form == "guarded":
        decls.append(head + f"=\n  if not {p}\n    @log.write(n)\n  unit")
    else:
        decls.append(f"pub run(n: Int, {p}: Bool) -> Unit\n  requires not {p}\n"
                     f"  uses @log: Log[write where not {p}]\n=\n  @log.write(n)")
    expect = [Q2_REFUTED[form]] if form in Q2_REFUTED else []
    note = ("the full check must reject it with " + expect[0] if expect else
            "the full check proves it; recorded")
    cls, expect, note = _gate(edition, RESIDUAL, expect, note, gate_codes)
    files = package("hunt", edition, {MOD: join_decls(*decls)}, tables=tables)
    return Program(op="Q2", rule=RULE + ", spec/12 §3", edition=edition, seed=seed, cls=cls,
                   files=files, expect=expect, variant=f"{form}-{p}", note=note)


CATALOG = ("capability Catalog\n  op journal(n: Int) -> Unit\n\n"
           "handler book() for Catalog\n  op journal(n) = unit")
VAULT = ("capability Vault\n  op note(x: Int) -> Unit\n    accepts secret\n\n"
         "handler quiet() for Vault\n  op note(x) = unit")
Q3_OPEN = ("guarded-live", "guarded-dry")
# The closed call: the condition is on a secret, so the public operation runs under a secret
# branch (U4003); a checker that took the condition for a guard would let the number of public
# events show the secret.
Q3_CLOSED = ("secret-condition",)


# Q3. A conditioned public operation whose condition is on a public parameter, while a secret goes
# to an operation that accepts it: no observer below the secret sees a difference (the oracle).
def q3(seed, edition):
    rng = rng_for(seed)
    form = rng.choice(Q3_OPEN + Q3_CLOSED)
    p = rng.choice(PARAMS)
    n = rng.choice([1, 4, 9])
    v = Varied()
    if form in Q3_OPEN:
        lit = v.lit("Int", "secret", rng.choice(["5", "7", "12"]))
        work = (f"work(n: Int, {p}: Bool, s: secret Int) -> Unit\n"
                f"  uses @cat: Catalog[journal where not {p}]\n  uses @vault: Vault[note]\n=\n"
                f"  if not {p}\n    @cat.journal(n)\n  @vault.note(s)")
        flag = "false" if form == "guarded-live" else "true"
        body = (f"  let s: secret Int = {lit}\n"
                f"  work({n}, {p}: {flag}, s: s) with @cat = book(), @vault = quiet()\n")
    else:
        lit = v.lit("Bool", "secret", rng.choice(["true", "false"]))
        work = (f"work(n: Int, {p}: secret Bool) -> Unit\n"
                f"  uses @cat: Catalog[journal where not {p}]\n=\n"
                f"  if not {p}\n    @cat.journal(n)\n  unit")
        body = (f"  let d: secret Bool = {lit}\n"
                f"  work({n}, {p}: d) with @cat = book()\n")
    text = join_decls(header(MOD), CATALOG, VAULT, work,
                      "pub main() -> Unit\n  uses @console: Console[print]\n=\n" + body
                      + '  @console.print("done")')
    files, vary = v.place(package("hunt", edition, {MOD: text}, entry=MOD, grants=["console"]))
    closed = form in Q3_CLOSED
    note = ("a closed call: a correct checker rejects it (U4003), and the oracle sets it aside"
            if closed else "expected accepted; the oracle checks that no observer below the "
            "secret sees it")
    cls, expect, note = _gate(edition, OPEN, [], note)
    if cls == MUST_REJECT:
        vary = []
    return Program(op="Q3", rule=RULE + ", spec/07 §3", edition=edition, seed=seed, cls=cls,
                   files=files, expect=expect, note=note, variant=form, vary=vary)


OPERATORS = {"Q1": q1, "Q2": q2, "Q3": q3}
# The editions the fixed run generates for each operator: the last one before conditions, as the
# edition gate's probe, and 0.3.
EDITIONS = {"Q1": ("0.2", "0.3"), "Q2": ("0.2", "0.3"), "Q3": ("0.2", "0.3")}
