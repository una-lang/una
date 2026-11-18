"""Family T: trusted modules and policies (`spec/07 §5`, §6, §7 rule 5).

The model assumes every policy is lawful, so these rules rest on the checker alone
(docs/reviews/security-hunt/coverage.md §2). Each operator writes a package with a trusted
module `app.keys` that declares a policy, and usually a caller module `app.caller`, then
breaks one rule. Open programs also get a runnable `app.main`, whose varied literals the
two-run oracle changes.
"""

from dataclasses import dataclass

from ..program import MUST_REJECT, OPEN, Program, rng_for
from ..una import Varied, header, join_decls, label_prefix, package, policy

KEYS = "app.keys"
CALLER = "app.caller"
MAIN = "app.main"
MAIN_HEAD = "pub main() -> Unit\n  uses @console: Console[print]\n="


@dataclass
class Setup:
    """The types of one policy: its `from` and `to`, and the literals of its base type."""

    base: str
    frm_level: str
    cats: tuple
    to_level: str

    @property
    def frm(self):
        return label_prefix(self.frm_level, self.cats) + self.base

    @property
    def to(self):
        return label_prefix(self.to_level) + self.base

    @property
    def from_label(self):
        return label_prefix(self.frm_level, self.cats).strip()

    @property
    def categories(self):
        return list(self.cats)

    def literals(self):
        return {
            "Text": ['"abc"', '""', '"1234"', '"zzzzzzzzzzzz"'],
            "Int": ["0", "1", "-1", "1234"],
            "Bool": ["true", "false"],
        }[self.base]

    def default(self):
        """A value of the `to` type, for the other branch of a conditional."""
        return {"Text": '""', "Int": "0", "Bool": "false"}[self.base]


def setup(rng, to_levels=("public", "internal")):
    base = rng.choice(["Text", "Int", "Bool"])
    frm_level = rng.choice(["confidential", "secret", "restricted"])
    cats = ("PII",) if rng.random() < 0.5 else ()
    return Setup(base, frm_level, cats, rng.choice(list(to_levels)))


def keys_module(s, trusted_header=True, via_pub=False, extra=(), via=None, pol=None,
                imports=()):
    """The trusted module: the policy `mask`, its `via` function `masked`, and `extra`."""
    via_decl = via if via is not None else (
        f"{'pub ' if via_pub else ''}masked(t: {s.frm}) -> {s.to} = t")
    pol_decl = pol if pol is not None else policy("mask", s.frm, s.to, "masked")
    return join_decls(header(KEYS, trusted=trusted_header, imports=imports), pol_decl,
                      via_decl, *extra)


def caller_module(s, *decls, imports=None):
    imps = imports if imports is not None else [(KEYS, ["mask"])]
    return join_decls(header(CALLER, imports=imps), *decls)


def make(op, rule, edition, seed, cls, modules, s, expect=(), note="", variant="",
         trusted=(KEYS,), entry=None, varied=None, categories=None):
    cats = s.categories if categories is None else categories
    files = package("hunt", edition, modules, categories=cats,
                    trusted=None if trusted is None else list(trusted),
                    entry=entry, grants=["console"] if entry else ())
    vary = []
    if varied is not None:
        files, vary = varied.place(files)
    return Program(op=op, rule=rule, edition=edition, seed=seed, cls=cls, files=files,
                   expect=list(expect), note=note, variant=variant, vary=vary)


def runnable_main(s, body_lines, v, imports=None):
    """`app.main`: binds a varied `from` value to `s`, then runs `body_lines`."""
    imps = imports if imports is not None else [(KEYS, ["mask"])]
    lit = v.lit(s.base, s.from_label, s.literals()[0])
    body = "\n".join(["  let s: " + s.frm + " = " + lit] + ["  " + b for b in body_lines])
    return join_decls(header(MAIN, imports=imps), MAIN_HEAD + "\n" + body)


def show(s, expr):
    """A `Console.print` of a public value of the base type."""
    return f'@console.print("{{{expr}}}")' if s.base != "Text" else f"@console.print({expr})"


# T1. Header and listing disagree.
def t1(seed, edition):
    rng = rng_for(seed)
    s = setup(rng)
    variant = rng.choice(["header-only", "listed-only"])
    header_only = variant == "header-only"
    listing = rng.choice([None, []]) if header_only else [KEYS]
    mods = {
        KEYS: keys_module(s, trusted_header=header_only),
        CALLER: caller_module(s, f"pub use_it(t: {s.frm}) -> {s.to} = mask(t)"),
    }
    expect = ["U7005"] if header_only else ["U7005", "U4004"]
    return make("T1", "spec/07 §6", edition, seed, MUST_REJECT, mods, s, expect,
                variant=variant, trusted=listing)


# T2. Neither header nor listing.
def t2(seed, edition):
    rng = rng_for(seed)
    s = setup(rng)
    mods = {
        KEYS: keys_module(s, trusted_header=False),
        CALLER: caller_module(s, f"pub use_it(t: {s.frm}) -> {s.to} = mask(t)"),
    }
    return make("T2", "spec/07 §6", edition, seed, MUST_REJECT, mods, s, ["U4004"],
                trusted=rng.choice([None, []]))


# T3. The listing spells the module otherwise. The header says `trusted`, so a listing
# that does not name the module exactly must be U7005.
SPELLINGS = [
    ("trailing-space", "app.keys "),
    ("leading-space", " app.keys"),
    ("upper", "App.keys"),
    ("upper-tail", "app.Keys"),
    ("slash", "app/keys"),
    ("file-path", "src/app/keys.una"),
    ("extension", "app.keys.una"),
    ("double-dot", "app..keys"),
    ("fullwidth-dot", "app．keys"),
    ("cyrillic-a", "аpp.keys"),
    ("zero-width", "app.​keys"),
    ("nfd", "app.keyś"),
    ("prefix", "app"),
    ("glob", "app.*"),
    ("empty", ""),
]


def t3(seed, edition):
    rng = rng_for(seed)
    s = setup(rng)
    variant, spelled = rng.choice(SPELLINGS)
    mods = {
        KEYS: keys_module(s),
        CALLER: caller_module(s, f"pub use_it(t: {s.frm}) -> {s.to} = mask(t)"),
    }
    return make("T3", "spec/07 §6", edition, seed, MUST_REJECT, mods, s, ["U7005"],
                variant=variant, trusted=[spelled])


def t4_forms(s, edition):
    """Every way of reaching `masked` other than through `mask`: (name, keys decls,
    caller decls or None). All must be U4005, or a names error for a private import."""
    F, T = s.frm, s.to
    forms = [
        ("direct", [f"pub leak(t: {F}) -> {T} = masked(t)"], None),
        ("value", [f"pub leak() -> fn({F}) -> {T} = masked"], None),
        ("map", [f"pub leak(ts: List[{F}]) -> List[{T}] = ts.map(each: masked)"], None),
        ("lambda-map",
         [f"pub leak(ts: List[{F}]) -> List[{T}] = ts.map(each: fn(t) -> masked(t))"], None),
        ("lambda-value", [f"pub leak() -> fn({F}) -> {T} = fn(t) -> masked(t)"], None),
        ("let-value", [f"pub leak(t: {F}) -> {T}\n=\n  let g = masked\n  g(t)"], None),
        ("generic-id", ["id[X](x: X) -> X = x",
                        f"pub leak(t: {F}) -> {T}\n=\n  let g = id(masked)\n  g(t)"], None),
        ("record", [f"pub kind Box\n  f: fn({F}) -> {T}", "pub leak() -> Box = Box(f: masked)"],
         None),
        ("returned", [f"get() -> fn({F}) -> {T} = masked",
                      f"pub leak(t: {F}) -> {T}\n=\n  let g = get()\n  g(t)"], None),
        ("handler", [f"capability Shower\n  op show(t: {F}) -> {T}\n    accepts {s.from_label}",
                     "handler direct() for Shower\n  op show(t) = masked(t)"], None),
        ("option", [f"pub leak(t: {F}) -> Option[{T}] = Some(masked(t))"], None),
        ("import", [], [f"pub leak(t: {F}) -> {T} = masked(t)"]),
        ("import-value", [], [f"pub leak() -> fn({F}) -> {T} = masked"]),
    ]
    if edition != "0.1":
        forms.append(("together", [
            f"pub leak(t: {F}) -> {T}\n=\n  together\n    let x = masked(t)\n    let y = 1\n  x"],
            None))
    return forms


def t4(seed, edition):
    rng = rng_for(seed)
    s = setup(rng)
    name, keys_extra, caller_decls = rng.choice(t4_forms(s, edition))
    mods = {KEYS: keys_module(s, extra=keys_extra)}
    if caller_decls is not None:
        mods[CALLER] = caller_module(s, *caller_decls, imports=[(KEYS, ["masked"])])
    return make("T4", "spec/07 §5 rule 3, §7 rule 5", edition, seed, MUST_REJECT, mods, s,
                ["U4005"], variant=name)


# T5. The `via` function is public, and may be imported.
def t5(seed, edition):
    rng = rng_for(seed)
    s = setup(rng)
    imported = rng.random() < 0.5
    mods = {KEYS: keys_module(s, via_pub=True)}
    if imported:
        mods[CALLER] = caller_module(s, f"pub leak(t: {s.frm}) -> {s.to} = masked(t)",
                                     imports=[(KEYS, ["masked"])])
    return make("T5", "spec/07 §5 rule 3", edition, seed, MUST_REJECT, mods, s, ["U4005"],
                variant="imported" if imported else "declared")


# T6. The `via` function lives in another module.
def t6(seed, edition):
    rng = rng_for(seed)
    s = setup(rng)
    helpers_trusted = rng.random() < 0.5
    helpers = join_decls(header("app.helpers", trusted=helpers_trusted),
                         f"pub masked(t: {s.frm}) -> {s.to} = t")
    keys = join_decls(header(KEYS, trusted=True, imports=[("app.helpers", ["masked"])]),
                      policy("mask", s.frm, s.to, "masked"))
    listing = [KEYS, "app.helpers"] if helpers_trusted else [KEYS]
    return make("T6", "spec/07 §7 rule 5", edition, seed, MUST_REJECT,
                {KEYS: keys, "app.helpers": helpers}, s, ["U4005"],
                variant="helpers-trusted" if helpers_trusted else "helpers-plain",
                trusted=listing)


# T7. The `via` function's types differ from the policy's beyond labels.
def t7(seed, edition):
    rng = rng_for(seed)
    s = setup(rng)
    F, T, lab = s.frm, s.to, s.from_label
    other = {"Text": "Int", "Int": "Text", "Bool": "Int"}[s.base]
    odefault = {"Text": '""', "Int": "0", "Bool": "false"}[other]
    cases = {
        "param-type": (None, f"masked(t: {label_prefix(s.frm_level, s.cats)}{other}) -> {T} = "
                       + s.default()),
        "result-type": (None, f"masked(t: {F}) -> {label_prefix(s.to_level)}{other} = "
                        + odefault),
        "requires": (None, f"masked(t: {F}) -> {T}\n  requires t == t\n=\n  t"),
        "generic": (None, "masked[X](t: X) -> X = t"),
        "list-param": (None, f"masked(t: List[{F}]) -> {T} = " + s.default()),
        "two-params": (None, f"masked(t: {F}, u: {F}) -> {T} = t"),
    }
    if s.base == "Int":
        cases["refined-from"] = (
            policy("mask", f"{lab} Pos".strip(), T, "masked"),
            f"masked(t: {F}) -> {T} = t")
    variant = rng.choice(sorted(cases))
    pol, via = cases[variant]
    extra = ["pub kind Pos = Int where it > 0"] if variant == "refined-from" else []
    mods = {KEYS: keys_module(s, via=via, pol=pol, extra=extra)}
    return make("T7", "spec/07 §5 rule 4", edition, seed, MUST_REJECT, mods, s, ["U4005"],
                variant=variant)


# T8. Policies the spec allows but B1 does not, observed through public sinks.
def t8(seed, edition):
    rng = rng_for(seed)
    variant = rng.choice(["list-len", "option-branch", "record-field", "effect-under-pc",
                          "effect-public", "via-calls-policy"])
    frm_level = rng.choice(["secret", "confidential"])
    s = Setup("Int", frm_level, (), "internal")
    lab = s.from_label
    v = Varied()
    if variant == "list-len":
        pol = policy("mask", f"{lab} List[Int]", "internal List[Int]", "masked")
        via = f"masked(t: {lab} List[Int]) -> internal List[Int] = t"
        caller = f'pub look(t: {lab} List[Int]) -> Unit\n  uses @console: Console[print]\n=\n  @console.print("{{mask(t).len}}")'
        return make("T8", "spec/07 §5, §7 rule 3", edition, seed, MUST_REJECT,
                    {KEYS: keys_module(s, via=via, pol=pol), CALLER: caller_module(s, caller)},
                    s, ["U4003"], variant=variant)
    if variant == "option-branch":
        pol = policy("mask", f"{lab} Option[Int]", "internal Option[Int]", "masked")
        via = f"masked(t: {lab} Option[Int]) -> internal Option[Int] = t"
        caller = (f"pub look(t: {lab} Option[Int]) -> Unit\n  uses @console: Console[print]\n=\n"
                  "  when mask(t)\n    is Some(v) -> @console.print(\"some\")\n"
                  "    is None -> @console.print(\"none\")")
        return make("T8", "spec/07 §5, §3 rule 2", edition, seed, MUST_REJECT,
                    {KEYS: keys_module(s, via=via, pol=pol), CALLER: caller_module(s, caller)},
                    s, ["U4003"], variant=variant)
    if variant == "record-field":
        kind = "pub kind Card\n  last: Int"
        pol = policy("mask", f"{lab} Card", "internal Card", "masked")
        via = f"masked(t: {lab} Card) -> internal Card = t"
        caller = (f'pub look(t: {lab} Card) -> Unit\n  uses @console: Console[print]\n=\n'
                  '  @console.print("{mask(t).last}")')
        return make("T8", "spec/07 §5, §7 rule 3", edition, seed, MUST_REJECT,
                    {KEYS: keys_module(s, via=via, pol=pol, extra=[kind]),
                     CALLER: caller_module(s, caller, imports=[(KEYS, ["mask", "Card"])])},
                    s, ["U4003"], variant=variant)
    effect_via = (f"masked(t: {s.frm}) -> {s.to}\n  uses @console: Console[print]\n=\n"
                  '  @console.print("released")\n  t')
    if variant == "effect-under-pc":
        # The flag is `internal`, as `to` is, so rule 2 holds and only the `via` function's
        # effect, which the policy inherits (rule 1), forbids the call under it.
        caller = (f"pub look(flag: internal Bool, t: {s.frm}) -> {s.to}\n"
                  "  uses @console: Console[print]\n=\n"
                  f"  if flag then mask(t) else 0")
        return make("T8", "spec/07 §3 rule 4, §5 rule 1", edition, seed, MUST_REJECT,
                    {KEYS: keys_module(s, via=effect_via), CALLER: caller_module(s, caller)},
                    s, ["U4002"], variant=variant)
    if variant == "effect-public":
        main = runnable_main(s, ["let r = mask(s)", '@console.print("done")'], v)
        return make("T8", "spec/07 §5 rule 1", edition, seed, OPEN,
                    {KEYS: keys_module(s, via=effect_via), MAIN: main}, s,
                    note="a via with an effect, called at a public pc; its output must not "
                         "depend on the secret", variant=variant, entry=MAIN, varied=v)
    # via-calls-policy: `masked` calls a second policy of the same module.
    inner = policy("inner", s.frm, s.to, "inner_via")
    inner_via = f"inner_via(t: {s.frm}) -> {s.to} = t"
    via = f"masked(t: {s.frm}) -> {s.to} = inner(t)"
    main = runnable_main(s, ["let r = mask(s)", '@console.print("done")'], v)
    return make("T8", "spec/07 §5 rule 3", edition, seed, OPEN,
                {KEYS: keys_module(s, via=via, extra=[inner, inner_via]), MAIN: main}, s,
                note="a via that calls another policy: report the verdict for triage",
                variant=variant, entry=MAIN, varied=v)


# T9. Ambiguous sharing: two policies with one `via`; one name in two trusted modules.
def t9(seed, edition):
    rng = rng_for(seed)
    s = setup(rng)
    variant = rng.choice(["shared-via", "same-name-two-modules"])
    if variant == "shared-via":
        keys = keys_module(s, extra=[policy("mask2", s.frm, s.to, "masked")])
        mods = {KEYS: keys,
                CALLER: caller_module(s, f"pub a(t: {s.frm}) -> {s.to} = mask(t)",
                                      f"pub b(t: {s.frm}) -> {s.to} = mask2(t)",
                                      imports=[(KEYS, ["mask", "mask2"])])}
        listing = [KEYS]
    else:
        other = join_decls(header("app.other", trusted=True),
                           policy("mask", s.frm, s.to, "masked"),
                           f"masked(t: {s.frm}) -> {s.to} = t")
        mods = {KEYS: keys_module(s), "app.other": other,
                CALLER: caller_module(s, f"pub a(t: {s.frm}) -> {s.to} = mask(t)")}
        listing = [KEYS, "app.other"]
    return make("T9", "spec/07 §5 rule 3", edition, seed, OPEN, mods, s,
                note="ambiguity probe: report the verdict for triage", variant=variant,
                trusted=listing)


# T10. A policy call under a program counter above its `to`.
def t10(seed, edition):
    rng = rng_for(seed)
    s = setup(rng, to_levels=("public", "internal"))
    F, T, d = s.frm, s.to, s.default()
    flag = rng.choice(["secret", "confidential"])
    forms = {
        "if": f"pub pick(flag: {flag} Bool, t: {F}) -> {flag} {s.base}\n=\n"
              f"  if flag then mask(t) else {d}",
        "when-option": f"pub pick(o: {flag} Option[Int], t: {F}) -> {flag} {s.base}\n=\n"
                       f"  when o\n    is Some(v) -> mask(t)\n    is None -> {d}",
        "lambda": f"pub pick(flag: {flag} Bool, ts: List[{F}]) -> List[{flag} {s.base}] = "
                  f"ts.map(each: fn(t) -> if flag then mask(t) else {d})",
        "generic": f"pub pick[X](a: X, b: X, t: {F}) -> {T}\n=\n"
                   f"  if a == b then mask(t) else {d}",
        "handler": (f"capability Probe\n  op probe(flag: Bool, t: {F}) -> {flag} {s.base}\n"
                    f"    accepts {flag}\n\n"
                    f"handler probing() for Probe\n  op probe(flag, t) = "
                    f"if flag then mask(t) else {d}"),
    }
    if edition != "0.1":
        forms["together"] = (f"pub pick(flag: {flag} Bool, t: {F}) -> {flag} {s.base}\n=\n"
                             f"  together\n    let x = if flag then mask(t) else {d}\n"
                             "    let y = 1\n  x")
    variant = rng.choice(sorted(forms))
    return make("T10", "spec/07 §5 rule 2", edition, seed, MUST_REJECT,
                {KEYS: keys_module(s), CALLER: caller_module(s, forms[variant])}, s, ["U4002"],
                variant=variant)


# T11. A failing `via` (S-18, ADR 0090): the failure is labeled pc ⊔ ℓ(to).
def t11(seed, edition):
    rng = rng_for(seed)
    to_level = rng.choice(["public", "internal", "secret"])
    pc = rng.choice(["public", "internal"])
    wrapper = rng.choice(["attempt-when", "error-pass", "error-use", "error-fail"])
    s = Setup("Int", "secret", (), to_level)
    T = s.to
    gate_kind = "pub kind Gate = | TooBig"
    via = f"gate(x: secret Int) -> {T}\n  fails Gate\n=\n  if x > 3 then fail TooBig else x"
    pol = policy("checkpin", "secret Int", T, "gate")
    keys = join_decls(header(KEYS, trusted=True), gate_kind, pol, via)
    fail_label_public = to_level == "public" and pc == "public"
    if wrapper == "attempt-when":
        call = ("let r = attempt checkpin(n)\n  when r\n    is Ok(v) -> @console.print(\"ok\")\n"
                "    is Err(e) -> @console.print(\"err\")")
        sig, expect = "-> Unit\n  uses @console: Console[print]", ["U4003"]
    elif wrapper == "error-pass":
        call, sig, expect = "let v = checkpin(n)\n    if error pass\n  unit", \
            "-> Unit\n  fails Gate", ["U4002"]
    elif wrapper == "error-use":
        call = ("let v = checkpin(n)\n    if error use 0\n  @console.print(\"{v}\")")
        sig, expect = "-> Unit\n  uses @console: Console[print]", ["U4003"]
    else:
        call, sig, expect = ("let v = checkpin(n)\n    if error fail Other\n  unit",
                             "-> Unit\n  fails Mine", ["U4002"])
    pre = []
    if wrapper == "error-fail":
        pre.append("pub kind Mine = | Other")
    if pc == "public":
        fn = f"pub probe(n: secret Int) {sig}\n=\n  {call}"
    else:
        body = call.replace("\n", "\n  ")
        fn = (f"pub probe(n: secret Int, flag: internal Bool) {sig}\n=\n"
              f"  if flag\n    {body}\n  else\n    unit")
    caller = caller_module(s, *pre, fn, imports=[(KEYS, ["Gate", "checkpin"])])
    if pc == "internal":
        # Under an internal program counter the policy call can fail, so it is U4002 first, at
        # the call, whatever `to` and the wrapper are (from the review of #517): these variants
        # test that rule, and the failure's own label only adds to it.
        expect = ["U4002"]
    cls = OPEN if fail_label_public else MUST_REJECT
    return make("T11", "spec/07 §5 rule 6", edition, seed, cls, {KEYS: keys, CALLER: caller},
                s, [] if fail_label_public else expect,
                variant=f"to-{to_level}-pc-{pc}-{wrapper}",
                note="a public failure label: expected accepted" if fail_label_public else "")


# T12. Two policies composed.
def t12(seed, edition):
    rng = rng_for(seed)
    variant = rng.choice(["chain-to-public", "chain-to-internal-printed", "chain-reversed"])
    s = Setup("Int", "secret", (), "internal")
    p1 = policy("lower", "secret Int", "internal Int", "lower_via")
    p1v = "lower_via(t: secret Int) -> internal Int = t"
    if variant == "chain-reversed":
        p2 = policy("lowest", "secret Int", "Int", "lowest_via")
        p2v = "lowest_via(t: secret Int) -> Int = t"
    else:
        p2 = policy("lowest", "internal Int", "internal Int" if variant ==
                    "chain-to-internal-printed" else "Int", "lowest_via")
        p2v = ("lowest_via(t: internal Int) -> " +
               ("internal Int" if variant == "chain-to-internal-printed" else "Int") + " = t")
    keys = join_decls(header(KEYS, trusted=True), p1, p1v, p2, p2v)
    v = Varied()
    main = runnable_main(s, ['@console.print("{lowest(lower(s))}")'], v,
                         imports=[(KEYS, ["lower", "lowest"])])
    if variant == "chain-to-internal-printed":
        return make("T12", "spec/07 §5, §4", edition, seed, MUST_REJECT, {KEYS: keys, MAIN: main},
                    s, ["U4003"], variant=variant, entry=MAIN, varied=v)
    return make("T12", "spec/07 §5", edition, seed, OPEN, {KEYS: keys, MAIN: main}, s,
                note="two releases in a row: a difference is explained by the policies",
                variant=variant, entry=MAIN, varied=v)


OPERATORS = {
    "T1": t1, "T2": t2, "T3": t3, "T4": t4, "T5": t5, "T6": t6,
    "T7": t7, "T8": t8, "T9": t9, "T10": t10, "T11": t11, "T12": t12,
}
