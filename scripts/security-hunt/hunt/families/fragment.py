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
    "fold": "  let t = [s, 2].fold(initial: 0, step: fn(acc, x) -> acc + x)\n",
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


OPERATORS = {"F1": f1}
