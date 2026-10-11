"""Family H: handler bodies and the effect floor (`spec/07 §7` rule 4).

A handler operation body is checked for the worst caller: its parameters carry the
operation's `accepts` label, and so does its program counter for effects and calls. The
model has no handler bodies (docs/reviews/security-hunt/coverage.md), so these rules rest
on the checker alone.
"""

from ..program import MUST_REJECT, OPEN, Program, rng_for
from ..una import Varied, header, join_decls, package

MOD = "app.main"


def _make(op, rule, edition, seed, cls, text, expect=(), variant="", note="", entry=False,
          varied=None):
    files = package("hunt", edition, {MOD: text}, entry=MOD if entry else None,
                    grants=["console"] if entry else ())
    vary = []
    if varied is not None:
        files, vary = varied.place(files)
    return Program(op=op, rule=rule, edition=edition, seed=seed, cls=cls, files=files,
                   expect=list(expect), note=note, variant=variant, vary=vary)


def _cap(level, result="Unit"):
    return f"capability Vault\n  op note(x: Int) -> {result}\n    accepts {level}"


BAD = "pub kind Bad = | Nope"


# H1. The body branches on its parameter, which carries `accepts`, then acts publicly.
def h1(seed, edition):
    rng = rng_for(seed)
    level = rng.choice(["internal", "secret"])
    act = rng.choice(["print", "fail-branch", "effect-call", "print-param"])
    decls = [header(MOD), BAD, _cap(level)]
    if act == "print":
        body = '  op note(x) = if x > 0 then @console.print("pos") else unit'
        uses, expect = "  uses @console: Console[print]\n", ["U4003"]
    elif act == "print-param":
        body = '  op note(x) = @console.print("{x}")'
        uses, expect = "  uses @console: Console[print]\n", ["U4003"]
    elif act == "fail-branch":
        decls[2] = ("capability Vault\n  op note(x: Int) -> Unit\n    fails Bad\n"
                    f"    accepts {level}")
        body = "  op note(x) = if x > 0 then fail Nope else unit"
        uses, expect = "", ["U4002"]
    else:
        decls.append('shout() -> Unit\n  uses @console: Console[print]\n=\n'
                     '  @console.print("x")')
        body = "  op note(x) = if x > 0 then shout() else unit"
        uses, expect = "  uses @console: Console[print]\n", ["U4002"]
    decls.append("handler h() for Vault\n" + uses + body)
    return _make("H1", "spec/07 §7 rule 4", edition, seed, MUST_REJECT, join_decls(*decls),
                 expect, variant=f"{level}-{act}")


# H2. The body returns its parameter into a result declared lower than `accepts`.
def h2(seed, edition):
    rng = rng_for(seed)
    level = rng.choice(["internal", "secret"])
    form = rng.choice(["direct", "branch", "arith", "list"])
    result = "List[Int]" if form == "list" else "Int"
    body = {"direct": "x", "branch": "if x > 0 then 1 else 0", "arith": "x + 1",
            "list": "[x]"}[form]
    text = join_decls(header(MOD), _cap(level, result),
                      f"handler h() for Vault\n  op note(x) = {body}")
    return _make("H2", "spec/07 §7 rule 4", edition, seed, MUST_REJECT, text, ["U4001"],
                 variant=f"{level}-{form}")


# H3. `with` and a raised program counter: a handler installed under a secret branch, or
# installed publicly and reached from one.
def h3(seed, edition):
    rng = rng_for(seed)
    form = rng.choice(["install-under-pc", "reach-under-pc"])
    v = Varied()
    lit = v.lit("Bool", "secret", "true")
    cap = _cap("secret")
    handler = "handler quiet() for Vault\n  op note(x) = unit"
    work = "work(n: Int) -> Unit\n  uses @vault: Vault[note]\n=\n  @vault.note(n)"
    if form == "install-under-pc":
        main = ("pub main() -> Unit\n  uses @console: Console[print]\n=\n"
                f"  let s: secret Bool = {lit}\n"
                "  guarded(s)\n"
                '  @console.print("done")')
        work += ("\n\nguarded(flag: secret Bool) -> Unit\n=\n"
                 "  if flag\n    work(1) with @vault = quiet()\n  else\n    unit")
    else:
        main = ("pub main() -> Unit\n  uses @console: Console[print]\n=\n"
                f"  let s: secret Bool = {lit}\n"
                "  branch(s) with @vault = quiet()\n"
                '  @console.print("done")')
        work += ("\n\nbranch(flag: secret Bool) -> Unit\n  uses @vault: Vault[note]\n=\n"
                 "  if flag\n    @vault.note(1)\n  else\n    unit")
    text = join_decls(header(MOD), cap, handler, work, main)
    return _make("H3", "spec/07 §7 rule 4", edition, seed, OPEN, text, variant=form,
                 note="expected accepted; the oracle checks that the output does not "
                      "depend on the secret", entry=True, varied=v)


# H4. A user handler of a host capability that breaks the worst caller's rule.
def h4(seed, edition):
    rng = rng_for(seed)
    form = rng.choice(["keys-print-data", "keys-return-data-public", "files-print-content"])
    if form == "keys-print-data":
        text = join_decls(
            header(MOD),
            "handler loud_keys() for Keys\n  uses @console: Console[print]\n"
            "  uses @inner: Keys[hmac]\n  op hmac(key_name, data) =\n"
            "    @console.print(data)\n    @inner.hmac(key_name: key_name, data: data)\n"
            "      if error pass")
        expect = ["U4003"]
    elif form == "keys-return-data-public":
        text = join_decls(
            header(MOD),
            "capability Echo\n  op echo(data: Text) -> Text\n    accepts secret",
            "handler leaky() for Echo\n  op echo(data) = data")
        expect = ["U4001"]
    else:
        text = join_decls(
            header(MOD),
            "handler fake_files() for Files\n  uses @console: Console[print]\n"
            '  op read(path) = ""\n  op write(path, content) = @console.print(content)')
        expect = ["U4003"]
    return _make("H4", "spec/07 §7 rule 4", edition, seed, MUST_REJECT, text, expect,
                 variant=form)


# H5. A handler operation that fails, and the caller's error clauses under each pc.
def h5(seed, edition):
    rng = rng_for(seed)
    pc = rng.choice(["public", "secret"])
    clause = rng.choice(["pass", "use", "attempt"])
    cap = ("capability Door\n  op open(code: Int) -> Int\n    fails Shut\n"
           "    accepts secret")
    kinds = "pub kind Shut = | Closed"
    handler = "handler door() for Door\n  op open(code) = fail Closed"
    call = {"pass": "let v = @door.open(1)\n    if error pass\n  unit",
            "use": "let v = @door.open(1)\n    if error use 0\n  unit",
            "attempt": "let r = attempt @door.open(1)\n  unit"}[clause]
    sig = "-> Unit\n  uses @door: Door[open]" + ("\n  fails Shut" if clause == "pass" else "")
    if pc == "public":
        fn = f"pub knock(n: Int) {sig}\n=\n  {call}"
        cls, expect = OPEN, []
    else:
        body = call.replace("\n", "\n  ")
        fn = (f"pub knock(flag: secret Bool) {sig}\n=\n  if flag\n    {body}\n  else\n    unit")
        # Only `pass` hands the failure to the caller under the secret program counter (U4002); a
        # failure that `attempt` or `if error use` handles inside the branch never leaves it, and
        # the call itself is allowed since the operation accepts `secret` (campaign 2's triage).
        if clause == "pass":
            cls, expect = MUST_REJECT, ["U4002"]
        else:
            cls, expect = OPEN, []
    text = join_decls(header(MOD), kinds, cap, handler, fn)
    return _make("H5", "spec/07 §3 rule 3, §7 rule 4", edition, seed, cls, text, expect,
                 variant=f"pc-{pc}-{clause}")


OPERATORS = {"H1": h1, "H2": h2, "H3": h3, "H4": h4, "H5": h5}
