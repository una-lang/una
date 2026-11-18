"""Family B: authority boundaries (ADR 0093, `spec/05 §4`, `spec/07 §6`).

A module named under `[boundaries]` in `una.toml` takes no capability from its callers' context
by name: every call into it, a function of it used as a value, and an install of one of its
handlers is granted with `with` (U3020), within the module's ceiling (U3021), counting a
functional argument's and a handler's own `uses`; and no release may be reachable from it
(U3024). `[boundaries]` is a table of edition 0.3: in editions 0.1 and 0.2 every program of the
family is the edition gate's probe, an unknown table (U7002).
"""

from ..program import MUST_REJECT, OPEN, Program, rng_for
from ..una import Varied, header, join_decls, package, policy

STORE = "app.store"
EDGE = "app.edge"
MAIN = "app.main"
RELEASE = "app.release"
RULE = "ADR 0093, spec/05 §4"
GATE = "edition 0.3 adds `[boundaries]`; earlier editions keep it unknown"

CAPS = ("Store", "Disk", "Shelf")


def _imports(pairs):
    """Imports in canonical form (U7001): by module, and each one's names in order."""
    return sorted((module, sorted(names)) for module, names in pairs)


def _store(cap):
    return join_decls(
        header(STORE),
        f"pub capability {cap}\n  op read(key: Text) -> Text\n"
        "  op write(key: Text, data: Text) -> Unit",
        "pub capability Net\n  op post(body: Text) -> Unit")


def _edge(cap, *extra, imports=None):
    return join_decls(
        header(EDGE, imports=_imports(imports or [(STORE, [cap])])),
        f"pub fetch(key: Text) -> Text\n  uses @store: {cap}[read]\n=\n  @store.read(key)",
        "pub size_of(key: Text) -> Int = 1",
        "pub notify(key: Text, done: fn(Text) -> Unit) -> Unit\n  uses of done\n=\n  done(key)",
        *extra)


def _gated(edition, cls, expect, note, codes=("U7002",)):
    """The class, expected codes and note of a program of `edition`: before 0.3, the probe, an
    unknown table (U7002), and for a narrowed rebinding `@x[ops]` a parse error (U0101)."""
    if edition in ("0.1", "0.2"):
        return MUST_REJECT, list(codes), GATE
    return cls, expect, note


# B1. A call into a boundary that breaks one of its rules: a capability by name (U3020), a grant
# beyond the ceiling, by a binding, a functional argument or a handler (U3021), a handler of the
# boundary installed with its `uses` taken by name (U3020), and a release reachable from it
# (U3024).
B1_FORMS = ("by-name", "ceiling", "functional-argument", "handler-ceiling", "function-value",
            "handler-install", "release")


def b1(seed, edition):
    rng = rng_for(seed)
    cap = rng.choice(CAPS)
    form = rng.choice(B1_FORMS)
    used, caps = ["fetch"], [cap]
    ceiling = [f"{cap}[read]"]
    edge_extra, trusted, modules = [], None, {}
    if form == "by-name":
        decls = [f"pub preview(key: Text) -> Text\n  uses @store: {cap}[read]\n=\n  fetch(key)"]
        expect = ["U3020"]
    elif form == "ceiling":
        decls = [f"pub preview(key: Text) -> Text\n  uses @store: {cap}[read, write]\n=\n"
                 "  fetch(key) with @store = @store"]
        expect = ["U3021"]
    elif form == "functional-argument":
        used, caps = ["notify"], ["Net"]
        decls = ["pub announce(key: Text) -> Unit\n  uses @net: Net[post]\n=\n"
                 "  notify(key, done: fn(p) -> @net.post(p))"]
        expect = ["U3021"]
    elif form == "handler-ceiling":
        caps = [cap, "Net"]
        decls = [f"handler posting() for {cap}\n  uses @net: Net[post]\n  op read(key) = key\n"
                 "  op write(key, data) = @net.post(data)",
                 "pub sent(key: Text) -> Text\n  uses @net: Net[post]\n=\n"
                 "  fetch(key) with @store = posting()"]
        expect = ["U3021"]
    elif form == "function-value":
        decls = [f"apply(key: Text, f: fn(Text) -> Text uses @store: {cap}[read]) -> Text\n"
                 f"  uses @store: {cap}[read]\n=\n  f(key)",
                 f"pub by_value(key: Text) -> Text\n  uses @store: {cap}[read]\n=\n"
                 "  apply(key, f: fetch)"]
        expect = ["U3020"]
    elif form == "handler-install":
        ceiling = [f"{cap}[read]", "Net[post]"]
        edge_extra = [f"pub handler mirrored() for {cap}\n  uses @net: Net[post]\n"
                      "  op read(key) = key\n  op write(key, data) = @net.post(data)"]
        used, caps = ["mirrored"], [cap, "Net"]
        decls = [f"save(key: Text) -> Unit\n  uses @store: {cap}[write]\n=\n"
                 "  @store.write(key, data: key)",
                 "pub header_only(key: Text) -> Unit\n  uses @net: Net[post]\n=\n"
                 "  save(key) with @store = mirrored()"]
        expect = ["U3020"]
    else:
        ceiling = []
        trusted = [RELEASE]
        modules[RELEASE] = join_decls(
            header(RELEASE, trusted=True),
            policy("reveal", "secret Int", "public Int", "open", purpose="count"),
            "open(n: secret Int) -> public Int = n")
        edge_extra = ["pub show(n: secret Int) -> Int = through(n)",
                      "through(n: secret Int) -> Int = reveal(n)"]
        used, caps = ["show"], []
        decls = ["pub leak(n: secret Int) -> Int = show(n)"]
        expect = ["U3024"]
    edge_imports = [(STORE, [cap, "Net"] if form == "handler-install" else [cap])]
    if form == "release":
        edge_imports.append((RELEASE, ["reveal"]))
    modules[STORE] = _store(cap)
    modules[EDGE] = _edge(cap, *edge_extra, imports=edge_imports)
    imports = [(EDGE, used)] + ([(STORE, caps)] if caps else [])
    modules[MAIN] = join_decls(header(MAIN, imports=_imports(imports)), *decls)
    cls, expect, note = _gated(edition, MUST_REJECT, expect, "")
    files = package("hunt", edition, modules, trusted=trusted,
                    tables=[("boundaries", [(EDGE, ceiling)])])
    return Program(op="B1", rule=RULE + (", spec/07 §6" if form == "release" else ""),
                   edition=edition, seed=seed, cls=cls, files=files, expect=expect, note=note,
                   variant=f"{form}-{cap.lower()}")


CAPS_MODULE = "app.caps"
VAULT = ("pub capability Vault\n  op note(x: Int) -> Unit\n    accepts secret\n\n"
         "pub handler quiet() for Vault\n  op note(x) = unit")
B2_OPEN = ("grant-handler", "narrowed", "function-value")
# The closed calls: a correct checker rejects them, so the oracle sets them aside; one it accepted
# would show the oracle a public event that the secret decides.
B2_CLOSED = ("under-secret-pc",)


# B2. Calls into a boundary granted within its ceiling, while the caller holds a secret: the
# secret reaches only an operation that accepts it, so no observer below it sees a difference
# (the oracle). Its closed call grants a public operation into the boundary, from a function called
# under a secret branch (U4002).
def b2(seed, edition):
    rng = rng_for(seed)
    form = rng.choice(B2_OPEN + B2_CLOSED)
    v = Varied()
    lit = v.lit("Int", "secret", rng.choice(["5", "7", "12"]))
    k = rng.choice([2, 3, 4])
    edge = join_decls(
        header(EDGE, imports=[(CAPS_MODULE, ["Vault"])]),
        "pub stash(n: secret Int) -> Unit\n  uses @vault: Vault[note]\n=\n  @vault.note(n)",
        "pub banner(k: Int) -> Unit\n  uses @console: Console[print]\n=\n"
        '  @console.print("banner {k}")')
    decls = []
    if form == "grant-handler":
        body = (f"  stash(s) with @vault = quiet()\n"
                f"  banner({k}) with @console = @console[print]\n")
    elif form == "narrowed":
        body = (f"  banner({k}) with @console = @console[print]\n"
                "  stash(s + 1) with @vault = quiet()\n")
    elif form == "function-value":
        decls.append("apply(n: secret Int, f: fn(secret Int) -> Unit uses @vault: Vault[note])"
                     " -> Unit\n  uses @vault: Vault[note]\n=\n  f(n)")
        body = "  apply(s, f: stash) with @vault = quiet()\n"
    else:
        decls.append("loud() -> Unit\n  uses @console: Console[print]\n=\n"
                     "  banner(1) with @console = @console[print]")
        body = f"  if s > {k}\n    loud()\n"
    main = join_decls(
        header(MAIN, imports=[(CAPS_MODULE, ["Vault", "quiet"]), (EDGE, ["banner", "stash"])]),
        *decls,
        "pub main() -> Unit\n  uses @console: Console[print]\n=\n"
        f"  let s: secret Int = {lit}\n" + body + '  @console.print("done")')
    caps = join_decls(header(CAPS_MODULE), VAULT)
    files = package("hunt", edition, {CAPS_MODULE: caps, EDGE: edge, MAIN: main}, entry=MAIN,
                    grants=["console"],
                    tables=[("boundaries", [(EDGE, ["Vault[note]", "Console[print]"])])])
    files, vary = v.place(files)
    closed = form in B2_CLOSED
    note = ("a closed call: a correct checker rejects it (U4002), and the oracle sets it aside"
            if closed else "expected accepted; the oracle checks that no observer below the "
            "secret sees it")
    cls, expect, note = _gated(edition, OPEN, ["U4002"] if closed else [], note, ("U7002", "U0101"))
    if cls == MUST_REJECT:
        # The probe is a check only: nothing of it runs.
        vary = []
    return Program(op="B2", rule=RULE + ", spec/07 §3", edition=edition, seed=seed, cls=cls,
                   files=files, expect=expect, note=note, variant=form, vary=vary)


OPERATORS = {"B1": b1, "B2": b2}
# The editions the fixed run generates for each operator: the last one before `[boundaries]`, as
# the edition gate's probe, and 0.3.
EDITIONS = {"B1": ("0.2", "0.3"), "B2": ("0.2", "0.3")}
