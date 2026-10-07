# Generates the matrix of the v0.3.1 review: every way a function value can be called or receive
# data in edition 0.1, crossed with the three channels a decision on values of a type parameter
# (`T*`) could leave by: the result, effects and failures.
#
# Each cell is a package `cases/<path>__<channel>[__<variant>]`. Its generic function `g[T]` makes
# the decision `if a == b then p else q` and sends it down the path; `leak(s: secret Int)`
# instantiates `T` with `List[secret Int]`, so a cell that checks clean leaks whether `s == 0`.
# Every cell that is expected to be rejected gets a control `..__control`, the same program with
# `p` in place of the decision, which is expected to check clean unless the cell is marked `rule`
# (rejected by a rule that does not depend on the decision, such as the second-class rule).
#
# Run: python3 -I gen.py; then UNA=<una> ./run-matrix.sh.
import os
import shutil

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.join(HERE, "cases")
D = "if a == b then p else q"

LOCK = """encoding_version = "una-smt-1"
solver           = "z3"
solver_version   = "5.1.0"
logic            = "QF_UFLIA"
step_budget      = 5000000
random_seed      = 0
"""

HEAD = """module sw

pub capability Log
  op write(n: Int) -> Unit

pub kind Bad = | Oops

pub kind Box[T]
  run: fn(T) -> Int

pub kind Hold[T]
  value: T

check_len(xs: List[secret Int]) -> Int
  fails Bad
=
  if xs.len > 0 then fail Oops else 0

"""

# The callback parameter of `g` and its argument in `leak`, per channel.
CB = {
    "result": ("k: fn(T) -> Int", "k: fn(xs) -> xs.len"),
    "effect": ("k: fn(T) -> Unit uses @log: Log[write]", "k: fn(xs) -> @log.write(xs.len)"),
    "failure": ("k: fn(T) -> Int fails Bad", "k: check_len"),
    "pure": ("k: fn(T) -> Int", "k: fn(xs) -> xs.len"),
}


def body(channel, call):
    """The body of `g` for a channel: `call` is an Int expression for result and failure, and a
    Unit statement for effect."""
    if channel == "result":
        return f"  let n = {call}\n  @log.write(n)\n"
    if channel == "effect":
        return f"  {call}\n"
    return f"  let n = {call}\n    if error pass\n  @log.write(1)\n  2\n"


def program(channel, decls, call, cb=None, extra_params="", extra_args=""):
    """A whole module: `decls`, then `g` and `leak`. `cb` selects the callback (defaults to the
    channel's)."""
    param, arg = CB[cb or channel]
    ret = "Int" if channel == "failure" else "Unit"
    fails = "  fails Bad\n" if channel == "failure" else ""
    g = (
        f"g[T](a: T, b: T, p: T, q: T, {param}{extra_params}) -> {ret}\n"
        f"  uses @log: Log[write]\n{fails}=\n{body(channel, call)}"
    )
    invoke = f"g([s], b: [0], p: [], q: [1], {arg}{extra_args})"
    if channel == "failure":
        leak = f"  let r = {invoke}\n    if error use 3\n  @log.write(5)\n"
    else:
        leak = f"  {invoke}\n"
    entry = f"pub leak(s: secret Int) -> Unit\n  uses @log: Log[write]\n=\n{leak}"
    return HEAD + (decls + "\n" if decls else "") + g + "\n" + entry


CELLS = []


def cell(path, channel, expect, src, note, variant=None, control=True, control_expect="accept"):
    """expect: `reject` (the decision must not get through), `rule` (rejected by a rule that
    does not depend on the decision), or `accept`. `control_expect` is `rule` where the control is
    rejected without the decision too: D-137 labels every result of a generic call with the type
    argument's label in edition 0.1, so writing one to a `public` sink is U4003 either way."""
    name = f"{path}__{channel}" + (f"__{variant}" if variant else "")
    CELLS.append((name, expect, note, src))
    if control and expect == "reject":
        assert D in src, name
        CELLS.append((name + "__control", control_expect, "control: " + note, src.replace(D, "p")))


# --- 1. A direct call of a function value parameter (T-App, #253).
for ch, call in [("result", f"k({D})"), ("effect", f"k({D})"), ("failure", f"k({D})")]:
    cell("direct_param", ch, "reject", program(ch, "", call), "k(decided)")

# --- 2. A let-bound lambda that wraps k, annotated (an unannotated one is U2010).
cell("let_lambda", "result", "reject",
     program("result", "", f"w({D})").replace("  let n = w(", "  let w = fn(x: T) -> k(x)\n  let n = w("),
     "let w = fn(x: T) -> k(x); w(decided)")
cell("let_lambda", "effect", "rule",
     program("effect", "", f"w({D})").replace("  w(", "  let w = fn(x: T) -> k(x)\n  w("),
     "an effectful lambda cannot be bound by let (U3006)", control=False)

# --- 3. A function stored in a record field; 4. in a list; 5. returned by a function.
cell("record_field", "result", "reject",
     program("result", "", f"f({D})", cb="pure").replace("  let n = f(", "  let f = box.run\n  let n = f(")
     .replace("k: fn(T) -> Int", "box: Box[T]").replace("k: fn(xs) -> xs.len", "box: Box(run: fn(xs) -> xs.len)"),
     "box.run(decided)")
cell("record_field", "effect", "rule",
     "module sw\n\npub capability Log\n  op write(n: Int) -> Unit\n\npub kind EffBox[T]\n  run: fn(T) -> Unit uses @log: Log[write]\n",
     "an effectful function in a record field (U3006/U3016)", control=False)
cell("list_element", "result", "reject",
     program("result", "", "0", cb="pure").replace("  let n = 0\n", f"  let n = when ks.first()\n    is Some(f) -> f({D})\n    is None -> 0\n")
     .replace("k: fn(T) -> Int", "ks: List[fn(T) -> Int]").replace("k: fn(xs) -> xs.len", "ks: [fn(xs) -> xs.len]"),
     "a function from a list, called with the decision; `leak`'s list of lambdas is U4001 either way (invariant element)",
     control_expect="rule")
cell("list_element", "effect", "rule",
     program("effect", "", "unit").replace("  unit\n", "  let ks = [k]\n  unit\n"),
     "an effectful function in a list (U3006/U3016)", control=False)
cell("returned", "result", "reject",
     program("result", "pass_on[V](f: fn(V) -> Int) -> fn(V) -> Int = f\n", f"h({D})").replace("  let n = h(", "  let h = pass_on(k)\n  let n = h("),
     "a function returned by a generic function, called with the decision", control_expect="rule")
cell("returned", "effect", "rule",
     program("effect", "pass_on[V](f: fn(V) -> Unit uses @log: Log[write]) -> fn(V) -> Unit uses @log: Log[write] = f\n", "unit"),
     "an effectful function returned (U3006)", control=False)

# --- 6. The prelude's higher-order functions with a named callback (#346).
prelude_calls = {
    "map": (f"[{D}].map(each: k).sum()", "k: fn(T) -> Int", "k: fn(xs) -> xs.len"),
    "filter": (f"[{D}].filter(keep: k).len", "k: fn(T) -> Bool", "k: fn(xs) -> xs.len > 0"),
    "first_where": (f"[{D}].first_where(keep: k).or_else(default: p).len", "k: fn(T) -> Bool", "k: fn(xs) -> xs.len > 0"),
    "fold": (f"[{D}].fold(initial: 0, step: k)", "k: fn(Int, T) -> Int", "k: fn(n, xs) -> n + xs.len"),
}
for hof, (call, param, arg) in prelude_calls.items():
    src = program("result", "", call).replace("k: fn(T) -> Int", param).replace("k: fn(xs) -> xs.len", arg)
    if hof == "first_where":
        # `T` is `List[secret Int]`: the element's length is public, its elements are not read.
        src = src.replace(".or_else(default: p).len", ".is_some()").replace("let n = ", "let n = if ").replace(".is_some()\n", ".is_some() then 1 else 0\n")
    cell("prelude_named", "result", "reject", src, f"{hof} with a named callback", variant=hof)
cell("prelude_named", "effect", "rule", program("effect", "", f"let units = [{D}].map(each: k)").replace("  let units", "  let units").replace("  let units = [if a == b then p else q].map(each: k)\n", f"  let units = [{D}].map(each: k)\n  @log.write(units.len)\n"),
     "an effectful callback of the prelude is U3003 in edition 0.1", control=False)
cell("prelude_named", "failure", "rule", program("result", "", f"[{D}].map(each: k).sum()").replace("k: fn(T) -> Int", "k: fn(T) -> Int fails Bad").replace("k: fn(xs) -> xs.len", "k: check_len"),
     "a failing callback of the prelude is U3009 in edition 0.1", control=False)

# --- 7. The prelude with a lambda that calls k: inferred, and annotated.
cell("prelude_lambda", "result", "reject", program("result", "", f"[{D}].map(each: fn(x) -> k(x)).sum()"),
     "map(each: fn(x) -> k(x))", variant="inferred")
cell("prelude_lambda", "result", "reject", program("result", "", f"[{D}].map(each: fn(x: T) -> k(x)).sum()"),
     "map(each: fn(x: T) -> k(x))", variant="annotated")
cell("prelude_lambda", "result", "reject", program("result", "", f"[p].map(each: fn(x) -> k({D})).sum()"),
     "a lambda that captures the decision and calls k", variant="captured")
cell("prelude_lambda", "failure", "reject",
     program("result", "", f"[{D}].map(each: fn(x: T) -> attempt k(x)).len").replace("k: fn(T) -> Int", "k: fn(T) -> Int fails Bad").replace("k: fn(xs) -> xs.len", "k: check_len")
     .replace(".len\n  @log.write(n)", "\n  let m = when n.first()\n    is Some(r) -> if r is Ok then 1 else 0\n    is None -> 2\n  @log.write(m)"),
     "attempt of a failing k inside an annotated lambda, which is pure", variant="attempt")

# --- 8. User-defined generic higher-order functions (#350): the callback's channels, and the
# callee's own effects and failures with a pure callback.
APPLY = "apply[U](x: U, f: fn(U) -> Int) -> Int = f(x)\n"
APPLY_E = "apply_eff[U](x: U, f: fn(U) -> Unit uses @log: Log[write]) -> Unit\n  uses @log: Log[write]\n=\n  f(x)\n"
APPLY_F = "apply_f[U](x: U, f: fn(U) -> Int fails Bad) -> Int\n  fails Bad\n=\n  let r = f(x)\n    if error pass\n  r\n"
LOGGER = "logger[U](x: U, f: fn(U) -> Int) -> Unit\n  uses @log: Log[write]\n=\n  @log.write(f(x))\n"
FAILER = "failer[U](x: U, f: fn(U) -> Int) -> Int\n  fails Bad\n=\n  if f(x) > 0 then fail Oops else 0\n"
for cbname, cbexpr in [("named", "k"), ("inferred", "fn(x) -> k(x)"), ("annotated", "fn(x: T) -> k(x)")]:
    cell("user_generic", "result", "reject", program("result", APPLY, f"apply({D}, f: {cbexpr})"),
         f"apply(decided, f: {cbexpr})", variant=f"callback_{cbname}", control_expect="rule")
    cell("user_generic", "effect", "reject", program("effect", APPLY_E, f"apply_eff({D}, f: {cbexpr})"),
         f"apply_eff(decided, f: {cbexpr}), the callback's effect", variant=f"callback_{cbname}")
    cell("user_generic", "failure", "reject" if cbname == "named" else "rule", program("failure", APPLY_F, f"apply_f({D}, f: {cbexpr})"),
         f"apply_f(decided, f: {cbexpr}), the callback's failure"
         + ("" if cbname == "named" else "; a lambda cannot handle k's failure (U3007)"),
         variant=f"callback_{cbname}", control=cbname == "named")
    cell("user_generic", "effect", "reject", program("effect", LOGGER, f"logger({D}, f: {cbexpr})", cb="pure"),
         f"logger(decided, f: {cbexpr}): the callee writes the pure callback's result", variant=f"callee_{cbname}")
    cell("user_generic", "failure", "reject", program("failure", FAILER, f"failer({D}, f: {cbexpr})", cb="pure"),
         f"failer(decided, f: {cbexpr}): the callee fails on the pure callback's result", variant=f"callee_{cbname}")

# Nested and recursive generic callees, and one that calls the prelude.
NESTED_E = APPLY_E + "\ntwice[V](x: V, f: fn(V) -> Unit uses @log: Log[write]) -> Unit\n  uses @log: Log[write]\n=\n  apply_eff(x, f: f)\n"
NESTED_L = LOGGER + "\nrelay[V](x: V, f: fn(V) -> Int) -> Unit\n  uses @log: Log[write]\n=\n  logger(x, f: f)\n"
REC_E = ("repeat[U](n: Int, x: U, f: fn(U) -> Unit uses @log: Log[write]) -> Unit\n  requires n >= 0\n  uses @log: Log[write]\n=\n"
         "  if n > 0\n    f(x)\n    repeat(n - 1, x: x, f: f)\n  else\n    unit\n")
PRELUDE_IN = "summed[U](xs: List[U], f: fn(U) -> Int) -> Unit\n  uses @log: Log[write]\n=\n  @log.write(xs.map(each: f).sum())\n"
cell("user_nested", "effect", "reject", program("effect", NESTED_E, f"twice({D}, f: k)"), "twice -> apply_eff, the callback's effect", variant="callback")
cell("user_nested", "effect", "reject", program("effect", NESTED_L, f"relay({D}, f: k)", cb="pure"), "relay -> logger, the callee's effect", variant="callee")
cell("user_recursive", "effect", "reject", program("effect", REC_E, f"repeat(2, x: {D}, f: k)"), "a recursive generic callee calls the callback")
cell("user_calls_prelude", "effect", "reject", program("effect", PRELUDE_IN, f"summed([{D}], f: k)", cb="pure"), "the callee maps the callback and writes the sum")

# Non-generic callees: their parameters are public, so the decided data cannot enter.
cell("user_non_generic", "effect", "reject",
     program("effect", "log_int(n: Int, f: fn(Int) -> Unit uses @log: Log[write]) -> Unit\n  uses @log: Log[write]\n=\n  f(n)\n",
             "log_int(if a == b then 1 else 0, f: h)", extra_params=", h: fn(Int) -> Unit uses @log: Log[write]", extra_args=", h: fn(n) -> @log.write(n)")
     .replace("log_int(if a == b then 1 else 0", f"log_int(if a == b then 1 else 0"),
     "a non-generic callee with an Int parameter", control=False)

# Decided data that is not of type T, passed through a type parameter.
cell("user_generic_int", "effect", "reject",
     program("effect", APPLY_E.replace("apply_eff", "apply_any"), "apply_any(if a == b then 1 else 0, f: h)",
             extra_params=", h: fn(Int) -> Unit uses @log: Log[write]", extra_args=", h: fn(n) -> @log.write(n)"),
     "a decided Int through U, to an Int callback", control=False)

# The decision inside a generic kind, an Option, or a list's shape.
cell("user_generic_wrapped", "effect", "reject",
     program("effect", APPLY_E, f"apply_eff(Hold(value: {D}), f: hk)", extra_params=", hk: fn(Hold[T]) -> Unit uses @log: Log[write]",
             extra_args=", hk: fn(h) -> @log.write(h.value.len)"),
     "the decision in a record of a generic kind", variant="record")
cell("user_generic_wrapped", "effect", "reject",
     program("effect", APPLY_E, f"apply_eff(if a == b then [p] else [p, q], f: lk)", extra_params=", lk: fn(List[T]) -> Unit uses @log: Log[write]",
             extra_args=", lk: fn(l) -> @log.write(l.len)"),
     "a decided list shape", variant="shape", control=False)

# Callbacks carried inside data arguments: a record field and a list.
LOGGER_BOX = "logger_box[U](x: U, box: Box[U]) -> Unit\n  uses @log: Log[write]\n=\n  let f = box.run\n  @log.write(f(x))\n"
LOGGER_LIST = ("logger_list[U](x: U, fs: List[fn(U) -> Int]) -> Unit\n  uses @log: Log[write]\n=\n"
               "  let n = when fs.first()\n    is Some(f) -> f(x)\n    is None -> 0\n  @log.write(n)\n")
cell("callback_in_data", "effect", "reject", program("effect", LOGGER_BOX, f"logger_box({D}, box: Box(run: k))", cb="pure"),
     "the callee calls a pure callback from a record argument and writes it", variant="record")
cell("callback_in_data", "effect", "rule", program("effect", LOGGER_LIST, f"logger_list({D}, fs: [k])", cb="pure"),
     "the callee calls a pure callback from a list argument and writes it; `[k]` does not flow into `List[fn(U) -> Int]` (U4001, also without the decision)",
     variant="list", control=False)

# A decided function value.
cell("decided_function", "result", "reject",
     program("result", "", f"(if a == b then k else j)(p)", extra_params=", j: fn(T) -> Int", extra_args=", j: fn(xs) -> 0"),
     "(if decided then k else j)(p)", variant="direct", control=False)
cell("decided_function", "result", "reject",
     program("result", APPLY, f"apply(p, f: if a == b then k else j)", extra_params=", j: fn(T) -> Int", extra_args=", j: fn(xs) -> 0"),
     "apply(p, f: decided function)", variant="user_generic", control=False)
cell("decided_function", "effect", "reject",
     program("effect", LOGGER, f"logger(p, f: if a == b then k else j)", cb="pure", extra_params=", j: fn(T) -> Int", extra_args=", j: fn(xs) -> 0"),
     "logger(p, f: decided function): the callee writes its result", variant="user_generic_callee", control=False)

# A lambda that captures the decision, given to a generic callee.
cell("captured_decision", "effect", "reject", program("effect", APPLY_E, f"apply_eff(p, f: fn(x) -> k({D}))"),
     "apply_eff(p, f: fn(x) -> k(decided))")
cell("captured_decision", "effect", "reject", program("effect", LOGGER, f"logger(p, f: fn(x) -> if a == b then 1 else 0)", cb="pure"),
     "logger(p, f: a lambda whose result is the decision)", variant="callee", control=False)

# --- 9. Handlers: an operation called with the decision, and a handler installed with it.
HANDLER = ("handler counting(n: Int) for Log\n  uses @log: Log[write]\n  op write(m) = @log.write(n)\n")
cell("handler_op", "effect", "reject", program("effect", "", f"@log.write(k({D}))", cb="pure"), "an operation's argument is k(decided)")
cell("handler_install", "effect", "reject",
     program("effect", HANDLER, "inner() with @log = counting(if a == b then 1 else 0)", cb="pure")
     .replace("g[T]", "inner() -> Unit\n  uses @log: Log[write]\n=\n  @log.write(0)\n\ng[T]"),
     "a handler installed with a decided argument, whose operation writes it", control=False)

# --- 10. attempt and if error around a generic callee.
cell("attempt", "failure", "reject",
     program("result", APPLY_F, f"attempt apply_f({D}, f: k)").replace("k: fn(T) -> Int", "k: fn(T) -> Int fails Bad").replace("k: fn(xs) -> xs.len", "k: check_len")
     .replace("  @log.write(n)", "  let m = if n is Ok then 1 else 0\n  @log.write(m)"),
     "attempt apply_f(decided, f: k), then its tag")
cell("attempt", "failure", "reject",
     program("result", FAILER, f"attempt failer({D}, f: k)").replace("  @log.write(n)", "  let m = if n is Ok then 1 else 0\n  @log.write(m)"),
     "attempt failer(decided, f: pure k), then its tag", variant="callee")
cell("if_error_use", "failure", "reject",
     program("result", FAILER, f"failer({D}, f: k)").replace("  @log.write(n)", "    if error use 7\n  @log.write(1)"),
     "failer(decided, f: k) if error use 7, then a write", variant="callee")

# --- 11. An example: it calls a generic function on concrete values, so no type parameter, and
# no T*, is in scope. (`checked` on a decided value is T-Checked's U4002, its own conformance.)
cell("example", "result", "accept",
     HEAD + APPLY + "\nexample \"apply on lists\"\n  do apply([1], f: fn(xs) -> xs.len)\n  expect result == 1\n",
     "an example of a generic higher-order function")


# --- 12. Aimed at #350's rule (rerun at 92d1b6e): deeper data, callbacks in Option and
# Result, two type parameters, a callback stored and called later, a handler around the call,
# and a lambda checked before the argument that carries the decision.
LOGGER_OPT = ("logger_opt[U](x: U, f: Option[fn(U) -> Int]) -> Unit\n  uses @log: Log[write]\n=\n"
              "  let n = when f\n    is Some(h) -> h(x)\n    is None -> 0\n  @log.write(n)\n")
LOGGER_RES = ("logger_res[U](x: U, f: Result[fn(U) -> Int, Int]) -> Unit\n  uses @log: Log[write]\n=\n"
              "  let n = when f\n    is Ok(h) -> h(x)\n    is Err(e) -> e\n  @log.write(n)\n")
cell("rule_350", "effect", "reject", program("effect", LOGGER_OPT, f"logger_opt({D}, f: Some(k))", cb="pure"),
     "the callee calls a pure callback from an Option argument and writes it", variant="option_callback")
cell("rule_350", "effect", "reject", program("effect", LOGGER_RES, f"logger_res({D}, f: Ok(k))", cb="pure"),
     "the callee calls a pure callback from a Result argument and writes it", variant="result_callback")
DEEP = ("deep[U](xs: List[Hold[List[U]]], f: fn(Hold[List[U]]) -> Int) -> Unit\n  uses @log: Log[write]\n=\n"
        "  @log.write(xs.map(each: f).sum())\n")
cell("rule_350", "effect", "reject",
     program("effect", DEEP, f"deep([Hold(value: [{D}])], f: hk)", extra_params=", hk: fn(Hold[List[T]]) -> Int",
             extra_args=", hk: fn(h) -> h.value.len"),
     "the decision in a list of records of lists", variant="deep_data")
cell("rule_350", "effect", "reject",
     program("effect", DEEP, f"deep([Hold(value: [p])], f: fn(h) -> if a == b then 1 else 0)", cb="pure"),
     "a lambda that decides itself, in a list of records of lists", variant="deep_lambda_decides", control=False)
TWO = ("two[U, W](x: U, y: W, f: fn(U) -> Int) -> Unit\n  uses @log: Log[write]\n=\n  @log.write(f(x))\n")
cell("rule_350", "effect", "reject",
     "module sw\n\npub capability Log\n  op write(n: Int) -> Unit\n\n" + TWO
     + "\ng[T, V](a: T, b: T, c: V, e: V, k: fn(V) -> Int) -> Unit\n  uses @log: Log[write]\n=\n"
     + "  two(if a == b then c else e, y: a, f: k)\n\npub leak(s: secret Int) -> Unit\n  uses @log: Log[write]\n=\n"
     + "  g(s, b: 0, c: [], e: [1], k: fn(xs) -> xs.len)\n",
     "two type parameters: a decision on T selects a V value given to a V callback", variant="two_params", control=False)
cell("rule_350", "effect", "reject",
     "module sw\n\npub capability Log\n  op write(n: Int) -> Unit\n\n" + TWO
     + "\ng[T, V](a: T, b: T, c: V, e: V, k: fn(V) -> Int) -> Unit\n  uses @log: Log[write]\n=\n"
     + "  two(c, y: if a == b then a else b, f: k)\n\npub leak(s: secret Int) -> Unit\n  uses @log: Log[write]\n=\n"
     + "  g(s, b: 0, c: [], e: [1], k: fn(xs) -> xs.len)\n",
     "two type parameters: the decided argument goes to a parameter the callback never sees (conservative)", variant="two_params_unused", control=False)
CAPTURE = "capture[U](x: U, f: fn(U) -> Int) -> fn(Int) -> Int = fn(n) -> f(x) + n\n"
cell("rule_350", "result", "reject",
     program("result", CAPTURE, f"later(0)").replace("  let n = later(0)", f"  let later = capture({D}, f: k)\n  let n = later(0)"),
     "the callee returns a closure over the decided value and the callback, called later", variant="stored_closure",
     control_expect="rule")
cell("rule_350", "effect", "reject",
     program("effect", APPLY_E, f"apply_eff({D}, f: k) with @log = @log"),
     "the call under a with clause that rebinds the capability", variant="with_rebind")
HANDLED = ("handler forward() for Log\n  uses @out: Log[write]\n  op write(n) = @out.write(n)\n")
cell("rule_350", "effect", "reject",
     program("effect", APPLY_E + "\n" + HANDLED, f"apply_eff({D}, f: k) with @log = forward()")
     .replace("  uses @log: Log[write]\n=\n  apply_eff(", "  uses @out: Log[write]\n=\n  apply_eff(")
     .replace("k: fn(T) -> Unit uses @log: Log[write]) -> Unit\n  uses @out", "k: fn(T) -> Unit uses @log: Log[write]) -> Unit\n  uses @out")
     .replace("pub leak(s: secret Int) -> Unit\n  uses @log: Log[write]\n=\n  g(", "pub leak(s: secret Int) -> Unit\n  uses @log: Log[write]\n=\n  g(")
     .replace("  g([s]", "  g([s]").replace("=\n  g([s], b: [0], p: [], q: [1], k: fn(xs) -> @log.write(xs.len))", "=\n  g([s], b: [0], p: [], q: [1], k: fn(xs) -> @log.write(xs.len)) with @out = @log"),
     "a handler installed around the call", variant="handler_around")
APPLY_REV = ("apply_rev[U](f: fn(U) -> Unit uses @log: Log[write], x: U) -> Unit\n  uses @log: Log[write]\n=\n  f(x)\n")
cell("rule_350", "effect", "reject", program("effect", APPLY_REV, f"apply_rev(fn(y) -> k(y), x: {D})"),
     "an inferred lambda checked before the argument that carries the decision", variant="lambda_first")
APPLY_REV_P = ("apply_rev[U](f: fn(U) -> Int, x: U) -> Unit\n  uses @log: Log[write]\n=\n  @log.write(f(x))\n")
cell("rule_350", "effect", "reject", program("effect", APPLY_REV_P, f"apply_rev(fn(y) -> k(y), x: {D})", cb="pure"),
     "an inferred pure lambda checked before the decided argument; the callee writes its result", variant="lambda_first_callee")
cell("rule_350", "effect", "reject", program("effect", APPLY_E, f"apply_eff(x: {D}, f: fn(y) -> k(y))").replace("apply_eff(x: ", "apply_eff("),
     "control shape: an inferred lambda after the decided argument", variant="lambda_after")


# --- 13. An annotated lambda that is not itself the argument (the coordinator's probe of
# `annotated_lambdas`, keyed by the lambda's span): inside a list, an Option, a record, an `if`,
# and parentheses.
LAM = "fn(x: T) -> k(x)"
cell("annotated_nested", "effect", "reject", program("effect", LOGGER_LIST, f"logger_list({D}, fs: [{LAM}])", cb="pure"),
     "the callee calls an annotated lambda from a list argument and writes it", variant="list", control_expect="rule")
cell("annotated_nested", "effect", "reject", program("effect", LOGGER_OPT, f"logger_opt({D}, f: Some({LAM}))", cb="pure"),
     "the callee calls an annotated lambda from an Option argument and writes it", variant="option")
cell("annotated_nested", "effect", "reject", program("effect", LOGGER_BOX, f"logger_box({D}, box: Box(run: {LAM}))", cb="pure"),
     "the callee calls an annotated lambda from a record argument and writes it", variant="record")
cell("annotated_nested", "effect", "reject", program("effect", LOGGER, f"logger({D}, f: ({LAM}))", cb="pure"),
     "the callee calls a parenthesized annotated lambda and writes it", variant="parens")
cell("annotated_nested", "effect", "reject",
     program("effect", LOGGER, f"logger({D}, f: if c then {LAM} else fn(x: T) -> 0)", cb="pure", extra_params=", c: Bool", extra_args=", c: true"),
     "the callee calls one of two annotated lambdas chosen by a public condition", variant="if")
cell("annotated_nested", "effect", "reject", program("effect", APPLY_E, f"apply_eff({D}, f: (fn(x: T) -> k(x)))"),
     "an effectful parenthesized annotated lambda", variant="parens_effect")
cell("annotated_nested", "result", "reject", program("result", "", f"[{D}].map(each: ({LAM})).sum()"),
     "map with a parenthesized annotated lambda", variant="prelude_parens")
cell("annotated_nested", "result", "reject",
     program("result", "", f"[{D}].map(each: if c then {LAM} else fn(x: T) -> 0).sum()", extra_params=", c: Bool", extra_args=", c: true"),
     "map with one of two annotated lambdas chosen by a public condition", variant="prelude_if")
cell("annotated_nested", "result", "reject",
     program("result", "", f"[{D}].fold(initial: 0, step: fn(acc, x: T) -> acc + k(x))"),
     "fold with a lambda whose second parameter alone is annotated", variant="prelude_partial_annotation")
cell("annotated_nested", "result", "reject",
     program("result", "", f"[{D}].map(each: fn(x: T) -> [x].map(each: fn(y) -> k(y)).sum()).sum()"),
     "an annotated lambda whose body maps an inferred lambda over its parameter", variant="prelude_inner_inferred")


cell("annotated_nested", "result", "reject",
     program("result", "", f"[{D}].map(each: w).sum()").replace("  let n = [", f"  let w = {LAM}\n  let n = ["),
     "map with an annotated lambda bound by let", variant="prelude_let")
cell("annotated_nested", "effect", "reject",
     program("effect", LOGGER, f"logger({D}, f: w)", cb="pure").replace("  logger(", f"  let w = {LAM}\n  logger("),
     "the callee calls an annotated lambda bound by let and writes it", variant="let")
cell("annotated_nested", "result", "reject",
     program("result", "pass_on[V](f: fn(V) -> Int) -> fn(V) -> Int = f\n", f"[{D}].map(each: pass_on({LAM})).sum()"),
     "map with an annotated lambda returned by a generic function", variant="prelude_returned", control_expect="rule")


def write():
    if os.path.isdir(ROOT):
        shutil.rmtree(ROOT)
    for name, expect, note, src in CELLS:
        d = os.path.join(ROOT, name)
        os.makedirs(os.path.join(d, "src"))
        with open(os.path.join(d, "una.toml"), "w") as f:
            f.write(f'[package]\nname = "{name}"\nversion = "0.1.0"\nedition = "0.1"\n')
        with open(os.path.join(d, "verification.lock"), "w") as f:
            f.write(LOCK)
        with open(os.path.join(d, "src", "sw.una"), "w") as f:
            f.write(f"-- expect: {expect}: {note}\n{src}")
    print(len(CELLS))


write()
