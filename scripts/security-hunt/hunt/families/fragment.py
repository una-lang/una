"""Family F: the covered fragment, for part (b) (docs/plan/security-hunt.md §2, generators §4 F).

Programs stay inside the importer's vocabulary (one `Console.print`, `Files`, lists with `map`,
`filter`, `fold` and `first_where`, lambdas, generic functions, records, `Option`, `Result`), so the
formal re-check reads them. A secret value travels through a carrier and reaches an ending. Every
program is open: the judge is the comparison of `una check` with the re-check (`differ` driver),
and a declaration `una check` accepts that the re-check rejects is a candidate for the
coordinator, never interpreted before that.
"""

from ..program import OPEN, Program, rng_for
from ..una import header, join_decls, package

MOD = "app.main"
RULE = "part (b): una check and the F1 re-check agree"

# Carriers: statements binding `t`, an `Int` holding the secret `s`, through one construct.
CARRIERS = {
    "direct": "  let t = s + 1\n",
    "map": ("  let ts = [s].map(each: fn(x) -> x + 1)\n"
            "  let t = when ts.first()\n    is Some(x) -> x\n    is None -> 0\n"),
    "filter": ("  let ts = [s, 1].filter(keep: fn(x) -> x > 0)\n"
               "  let t = when ts.first()\n    is Some(x) -> x\n    is None -> 0\n"),
    # The accumulator takes `initial`'s type: a public `0` would make `acc + x` U4001 (seed 5).
    "fold": ("  let z: secret Int = 0\n"
             "  let t = [s, 2].fold(initial: z, step: fn(acc, x) -> acc + x)\n"),
    "first-where": ("  let o = [s, 3].first_where(keep: fn(x) -> x > 1)\n"
                    "  let t = when o\n    is Some(x) -> x\n    is None -> 0\n"),
    "lambda": "  let f = fn(x: secret Int) -> x * 2\n  let t = f(s)\n",
    "generic": "  let t = same(s)\n",
    "record": "  let b = Holder(v: s)\n  let t = b.v\n",
    "option": "  let o = Some(s)\n  let t = when o\n    is Some(x) -> x\n    is None -> 0\n",
    "result": ("  let r: Result[secret Int, Int] = Ok(s)\n"
               "  let t = when r\n    is Ok(x) -> x\n    is Err(e) -> e\n"),
}

SUPPORT = {
    "generic": "same[T](x: T) -> T = x",
    "record": "kind Holder\n  v: secret Int",
}

# Endings: what is done with `t`; `una check` accepts the `secret` ones and rejects the others.
ENDINGS = {
    "secret-result": ("secret Int", "", "  t"),
    "secret-branch": ("secret Int", "", "  if t > 3 then 1 else 0"),
    "public-result": ("Int", "", "  t"),
    "print": ("Unit", "  uses @console: Console[print]\n", '  @console.print("{t}")'),
    "print-under": ("Unit", "  uses @console: Console[print]\n",
                    '  if t > 3\n    @console.print("big")\n  else\n    unit'),
    "write-internal": ("Unit", "  uses @files: Files[write]\n  fails FileError\n",
                       '  @files.write("out.txt", content: "{t}")\n    if error pass'),
}


# F1 to F4: a secret literal, a carrier and an ending.
def f1(seed, edition):
    rng = rng_for(seed)
    carrier = rng.choice(sorted(CARRIERS))
    ending = rng.choice(sorted(ENDINGS))
    result, uses, tail = ENDINGS[ending]
    level = "internal" if ending == "write-internal" else "secret"
    body = CARRIERS[carrier].replace("secret Int", f"{level} Int")
    fn = (f"pub run(s: {level} Int) -> {result}\n" + uses + "=\n" + body + tail)
    decls = [header(MOD)]
    if carrier in SUPPORT:
        decls.append(SUPPORT[carrier].replace("secret Int", f"{level} Int"))
    decls.append(fn)
    files = package("hunt", edition, {MOD: join_decls(*decls)})
    return Program(op="F1", rule=RULE, edition=edition, seed=seed, cls=OPEN, files=files,
                   expect=[], variant=f"{carrier}-{ending}",
                   note="the judge is the comparison of una check and the re-check")


# F7: carriers through the prelude functions F1 reads since #532, #542 and #545. A table of its
# own, so that F1's carriers, and the programs old seeds rebuild to, stay unchanged.
PRELUDE_CARRIERS = {
    "at": ("  let ts = [s, 1]\n"
           "  let t = when ts.at(index: 0)\n    is Some(x) -> x\n    is None -> 0\n"),
    # The index decides whether there is an element (`spec/14 §1`).
    "at-index": ("  let ts = [1, 2]\n"
                 "  let t = when ts.at(index: s)\n    is Some(x) -> x\n    is None -> 0\n"),
    "or-else": "  let o = Some(s)\n  let t = o.or_else(default: 0)\n",
    "or-else-default": "  let o: Option[Int] = None\n  let t = o.or_else(default: s)\n",
    # A call with two arguments labels the second (U2004, `spec/04 §3`).
    "min": "  let t = min(s, b: 5)\n",
    "max": "  let t = max(s, b: 5)\n",
    "abs": "  let t = abs(s)\n",
    # Structural: `is_ok` carries the result's top label only, so `t` is public here.
    "is-ok": ("  let r: Result[secret Int, Int] = Ok(s)\n"
              "  let t = if r.is_ok() then 1 else 0\n"),
    # The coordinator's counterpart: a result chosen under a secret condition, so its top label,
    # and `is_ok`'s `Bool`, are secret.
    "is-ok-top": ("  let r: secret Result[Int, Int] = if s > 0 then Ok(1) else Err(0)\n"
                  "  let t = if r.is_ok() then 1 else 0\n"),
    "slice": '  let tx = "{s}"\n  let t = tx.slice(start: 0, end: 0).len\n',
}

# Edition 0.1 has no `slice` (U1001): its text functions are edition 0.2's (`spec/22`).
EDITION_02_ONLY = {"slice"}


def f7(seed, edition):
    rng = rng_for(seed)
    carriers = [c for c in sorted(PRELUDE_CARRIERS) if edition != "0.1" or c not in EDITION_02_ONLY]
    carrier = rng.choice(carriers)
    ending = rng.choice(sorted(ENDINGS))
    result, uses, tail = ENDINGS[ending]
    level = "internal" if ending == "write-internal" else "secret"
    body = (PRELUDE_CARRIERS[carrier].replace("secret Int", f"{level} Int")
            .replace("secret Result", f"{level} Result"))
    fn = (f"pub run(s: {level} Int) -> {result}\n" + uses + "=\n" + body + tail)
    files = package("hunt", edition, {MOD: join_decls(header(MOD), fn)})
    return Program(op="F7", rule=RULE, edition=edition, seed=seed, cls=OPEN, files=files,
                   expect=[], variant=f"{carrier}-{ending}",
                   note="the judge is the comparison of una check and the re-check")


OPERATORS = {"F1": f1, "F7": f7}
