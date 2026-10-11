"""Writes formal/UnaFormal/F1/SumImportTests.lean (#512): una.core/1 documents built from helpers.

Usage: python3 formal/gen/sum_import_tests.py formal/UnaFormal/F1/SumImportTests.lean
"""
import json, sys

def L(level="public"):
    return {"level": level, "categories": [], "symbols": []}

def base(k, level="public"):
    return {"kind": "base", "base": {"kind": k}, "label": L(level)}

def named(mod, name, args=(), level="public"):
    return {"kind": "base", "base": {"kind": "named", "module": mod, "name": name, "args": list(args)}, "label": L(level)}

INT, TEXT, UNIT, BOOL = (lambda l="public": base("int", l)), (lambda l="public": base("text", l)), (lambda l="public": base("unit", l)), (lambda l="public": base("bool", l))

def result(t, e, level="public"):
    return named("prelude", "Result", [t, e], level)

def var(x): return {"kind": "var", "value": x}
def int_(n): return {"kind": "int", "value": str(n)}
def text(s): return {"kind": "text", "value": s}
UNITA = {"kind": "unit"}

def atom(a): return {"kind": "atom", "atom": a}
def let_rhs(x, ty, rhs, body): return {"kind": "let", "name": x, "ty": ty, "value": {"kind": "rhs", "value": rhs}, "body": body, "written": False}
def let_expr(x, ty, e, body): return {"kind": "let", "name": x, "ty": ty, "value": {"kind": "expr", "value": e}, "body": body, "written": False}
def ctor(mod, kind, name, index): return {"kind": {"module": mod, "name": kind}, "name": name, "index": index}
def call(f, args): return {"kind": "call", "function": {"module": "m", "name": f}, "type_args": [], "args": args}
def construct(c, args): return {"kind": "construct", "ctor": c, "args": args}
def fail(c, args): return {"kind": "fail", "ctor": c, "args": args, "arg_spans": []}
def attempt(b): return {"kind": "attempt", "body": b}
def handle(b, x, h): return {"kind": "handle", "body": b, "error": x, "handler": h}
def case(sc, arms, otherwise=None):
    return {"kind": "case", "scrutinee": sc, "arms": [{"ctor": c, "binds": bs, "body": b} for (c, bs, b) in arms], "otherwise": otherwise}
def if_(c, t, e): return {"kind": "if", "cond": c, "then": t, "otherwise": e}
def prim(k, args): return {"kind": "prim", "prim": {"kind": k}, "args": args}
def field_(v, i): return {"kind": "field", "value": v, "index": i}
def op_print(a): return {"kind": "op", "cap": "console", "capability": "Console", "capability_path": {"module": "std.host", "name": "Console"}, "op": "print", "args": [a], "labels": [L()]}

PRINT_USES = [{"cap": "console", "capability": "Console", "ops": ["print"]}]

def fun(name, params, res, body, uses=(), fails=None):
    return {"kind": "fun", "value": {
        "name": name, "type_params": [], "constraints": [], "params": [[x, t] for (x, t) in params],
        "result": res, "uses": list(uses),
        "fails": fails, "error": ({"module": "m", "name": fails} if fails else None),
        "body": body, "public": True, "uses_of": [], "fails_of": None, "result_labels": [], "of_raises": []}}

def data(name, variants, record=False, params=()):
    return {"kind": "data", "name": name, "params": list(params),
            "variants": [{"name": n, "fields": [[f, t] for (f, t) in fs]} for (n, fs) in variants], "record": record}

CONSOLE = {"kind": "cap", "name": "Console", "ops": [{"name": "print", "params": [["line", TEXT()]], "result": UNIT(), "fails": None, "error": None, "accepts": L()}]}

def doc(decls):
    return json.dumps({"schema": "una.core/1", "edition": "0.2", "complete": True,
                       "modules": [{"path": "m", "decls": decls}], "library": [], "host": [CONSOLE]},
                      separators=(",", ":"), ensure_ascii=False)

# Kinds.
ERR = named("m", "Err")
OOPS, PLAIN = ctor("m", "Err", "Oops", 0), ctor("m", "Err", "Plain", 1)
SHAPE = lambda l="public": named("m", "Shape", level=l)
CIRCLE, SQUARE, TRI = ctor("m", "Shape", "Circle", 0), ctor("m", "Shape", "Square", 1), ctor("m", "Shape", "Tri", 2)
CODE = named("m", "Code")
CA = ctor("m", "Code", "A", 0)
kinds = [
    data("Err", [("Oops", [("info", INT()), ("key", INT("secret"))]), ("Plain", [])]),
    data("Shape", [("Circle", [("r", INT())]), ("Square", []), ("Tri", [("a", INT())])]),
    data("W", [("W", [("x", INT())])], record=False),
    data("Code", [("A", []), ("B", [])]),
    data("Box", [("Full", [("v", {"kind": "base", "base": {"kind": "param", "value": "T"}, "label": L()})]), ("Empty", [])], params=["T"]),
    data("R", [("R", [("x", INT())])], record=True),
]

def print_int(x, k):
    """`print(to_text(x))`, `x` an `Int` at label `lvl` of the text, then `k`."""
    return lambda lvl: let_rhs("%m", TEXT(lvl), prim("to_text", [var(x)]), let_rhs("%u", UNIT(), op_print(var("%m")), k))

def print_lit(s, k):
    return let_rhs("%u", UNIT(), op_print(text(s)), k)

call_boom = let_rhs("v", INT(), call("boom", [var("k")]), atom(var("v")))

def after_attempt(read):
    lvl = "secret" if read == "s" else "public"
    return let_expr("r", result(INT(), ERR), attempt(call_boom),
        case(var("r"), [(ctor("prelude", "Result", "Ok", 0), ["x"], atom(UNITA)),
                        (ctor("prelude", "Result", "Err", 1), ["y"],
                         case(var("y"), [(OOPS, ["i", "s"], print_int(read, atom(UNITA))(lvl)),
                                         (PLAIN, [], atom(UNITA))]))]))

def after_handle(read):
    lvl = "secret" if read == "s" else "public"
    h = case(var("e"), [(OOPS, ["i", "s"], print_int(read, atom(int_(0)))(lvl)), (PLAIN, [], atom(int_(0)))])
    return let_expr("x", INT(), handle(call_boom, "e", h), atom(UNITA))

def shape_case(arms, otherwise=None):
    return case(var("x"), arms, otherwise)

ARM_C = (CIRCLE, [None], atom(UNITA))
ARM_S = (SQUARE, [], atom(UNITA))
ARM_T = (TRI, [None], print_lit("t", atom(UNITA)))

funs = [
    fun("boom", [("k", INT("secret"))], INT(), fail(OOPS, [int_(1), var("k")]), fails="Err"),
    fun("att_leak", [("k", INT("secret"))], UNIT(), after_attempt("s"), uses=PRINT_USES),
    fun("att_ok", [("k", INT("secret"))], UNIT(), after_attempt("i"), uses=PRINT_USES),
    fun("h_leak", [("k", INT("secret"))], UNIT(), after_handle("s"), uses=PRINT_USES),
    fun("h_ok", [("k", INT("secret"))], UNIT(), after_handle("i"), uses=PRINT_USES),
    fun("choice", [("b", BOOL("secret"))], UNIT(), if_(var("b"), fail(PLAIN, []), atom(UNITA)), fails="Err"),
    fun("choice_pub", [("b", BOOL())], UNIT(), if_(var("b"), fail(PLAIN, []), atom(UNITA)), fails="Err"),
    fun("third", [("x", SHAPE("secret"))], UNIT(), shape_case([ARM_C, ARM_S, ARM_T]), uses=PRINT_USES),
    fun("third_pub", [("x", SHAPE())], UNIT(), shape_case([ARM_C, ARM_S, ARM_T]), uses=PRINT_USES),
    fun("missing", [("x", SHAPE())], UNIT(), shape_case([ARM_C, ARM_S]), uses=PRINT_USES),
    fun("dup", [("x", SHAPE())], UNIT(), shape_case([ARM_C, ARM_C, ARM_S, ARM_T]), uses=PRINT_USES),
    fun("other_arm", [("x", SHAPE())], UNIT(), shape_case([ARM_C], atom(UNITA)), uses=PRINT_USES),
    fun("as_record", [("x", SHAPE())], INT(), let_rhs("v", INT(), field_(var("x"), 0), atom(var("v")))),
    fun("generic", [("b", named("m", "Box", [INT()]))], UNIT(), atom(UNITA)),
    fun("eq_sum", [("x", SHAPE()), ("y", SHAPE())], BOOL(), let_rhs("q", BOOL(), prim("eq", [var("x"), var("y")]), atom(var("q")))),
    fun("w_use", [], INT(), let_rhs("w", named("m", "W"), construct(ctor("m", "W", "W", 0), [int_(1)]),
        case(var("w"), [(ctor("m", "W", "W", 0), ["z"], atom(var("z")))]))),
    fun("c_fail", [], UNIT(), fail(CA, []), fails="Code"),
    fun("c_attempt", [], UNIT(), let_expr("r", result(UNIT(), CODE), attempt(let_rhs("u", UNIT(), call("c_fail", []), atom(var("u")))), atom(UNITA))),
    fun("c_handle", [], UNIT(), handle(let_rhs("u", UNIT(), call("c_fail", []), atom(var("u"))), "e", atom(UNITA))),
    fun("c_handle_read", [], UNIT(), handle(let_rhs("u", UNIT(), call("c_fail", []), atom(var("u"))), "e",
        let_rhs("q", BOOL(), prim("eq", [var("e"), var("e")]), atom(UNITA)))),
    fun("p_fail", [], UNIT(), fail(PLAIN, []), fails="Err"),
    fun("p_attempt", [], UNIT(), let_expr("r", result(UNIT(), ERR), attempt(let_rhs("u", UNIT(), call("p_fail", []), atom(var("u")))), atom(UNITA))),
    fun("p_handle", [], UNIT(), handle(let_rhs("u", UNIT(), call("p_fail", []), atom(var("u"))), "e",
        case(var("e"), [(OOPS, [None, None], atom(UNITA)), (PLAIN, [], atom(UNITA))]))),
]
sums_doc = doc(kinds + funs)

# A policy whose `via` fails with a payload kind.
via_doc = doc([
    data("Err", [("Oops", [("info", INT())]), ("Plain", [])]),
    fun("check", [("x", INT("secret"))], BOOL(),
        let_rhs("c", BOOL("secret"), prim("eq", [var("x"), int_(1)]),
                if_(var("c"), fail(PLAIN, []), atom({"kind": "bool", "value": True}))), fails="Err"),
    {"kind": "policy", "name": "reveal", "from": INT("secret"), "to": BOOL(), "via": {"module": "m", "name": "check"}},
    fun("rev", [("s", INT("secret"))], BOOL(), let_rhs("b", BOOL(), {"kind": "declassify", "policy": {"module": "m", "name": "reveal"}, "via": {"module": "m", "name": "check"}, "arg": var("s")}, atom(var("b")))),
    fun("other", [], INT(), atom(int_(1))),
])

expect = [
    ("boom", "accepted", "a failure with a payload, its secret field from a secret argument"),
    ("att_leak", "rejected", "P2: a secret field of the failure read into `print` after `attempt`"),
    ("att_ok", "accepted", "its public twin: the public field"),
    ("h_leak", "rejected", "P2: a secret field read into `print` by a handler"),
    ("h_ok", "accepted", "its public twin"),
    ("choice", "rejected", "P3: a variant chosen by a secret, by T-Fail's public program counter"),
    ("choice_pub", "accepted", "its public twin"),
    ("third", "rejected", "S1: a `when` of three arms on a secret sum, the print in the third arm"),
    ("third_pub", "accepted", "its public twin: exhaustive, so the chain ends in `unreachable`"),
    ("missing", "not re-checked", "an arm missing, with no `otherwise`: the importer's own check"),
    ("dup", "not re-checked", "a variant twice"),
    ("other_arm", "accepted", "one arm and an `otherwise`"),
    ("as_record", "not re-checked", "a sum where a record is expected: a lookup miss, never a default"),
    ("generic", "not re-checked", "a generic sum"),
    ("eq_sum", "not re-checked", "`==` on a sum, which F1 does not model"),
    ("w_use", "accepted", "a kind with a field, flagged `record: false`: a sum of one variant"),
    ("c_fail", "accepted", "a code kind at `fail`"),
    ("c_attempt", "accepted", "the same kind at `attempt`, as a code"),
    ("c_handle", "accepted", "and at a handler, as a code"),
    ("c_handle_read", "not re-checked", "a handler of a code that reads it, as before #512"),
    ("p_fail", "accepted", "a payload kind's variant without fields at `fail`: a `raise`"),
    ("p_attempt", "accepted", "the same kind at `attempt`, as a payload kind"),
    ("p_handle", "accepted", "and at a handler, as a payload kind"),
]

out = []
out.append('''/-
Tests of the reading of sum kinds and failures with a payload (#512), checked when the library
builds. The documents are written by `formal/gen/sum_import_tests.py`, which regenerates this file
(one module `m`, edition 0.2, `Console.print` accepting `public`); each verdict has its reason.
-/
import UnaFormal.F1.ImportTests

namespace Una.F1.SumImportTests

open Import ImportTests

/-- `Err = Oops(info: Int, key: secret Int) | Plain`, `Shape = Circle(r: Int) | Square |
Tri(a: Int)`, `W = W(x: Int)` flagged `record: false`, the code kind `Code = A | B`, the generic
`Box[T]` and the record `R`, with the functions below. -/
def sums : String := r#"''' + sums_doc + '''"#

/-- A policy whose `via` function fails with a payload kind (`spec/07 §5` rule 1 allows it; F1's
option B1 does not): every declaration of the document is not re-checked. -/
def viaPayload : String := r#"''' + via_doc + '''"#
''')
for (f, v, why) in expect:
    out.append(f'-- {why}.\n#guard run sums "m.{f}" == "{v}"')
third_body = json.dumps(shape_case([ARM_C, ARM_S, ARM_T]), separators=(",", ":"))
out.append('''
#guard ["m.check", "m.rev", "m.other"].all (run viaPayload · == "not re-checked")

/-- The body of `third_pub`, as the export writes it. -/
def thirdBody : String := r#"''' + third_body + '''"#

/-- `Shape`, as the reading gives its shape: one field, none, one. -/
def shapeCtx : Ctx := { records := [], funs := [], sums := [("m.Shape", [1, 0, 1])] }

def shapeProg : Program :=
  mkProgram [] Label.bot {} [] [] [] [⟨"m.Shape", [[.int .bot], [], [.int .bot]], Label.bot⟩]

/-- The reading of `third_pub`'s body: a chain of three tests, the last `else` `unreachable`. -/
def thirdRead : Option Expr :=
  match Lean.Json.parse thirdBody with
  | .ok j => (plainRun (expr shapeCtx j)).toOption
  | .error _ => none

#guard match thirdRead with
  | some (.caseSum _ 0 _ _ (.caseSum _ 1 _ _ (.caseSum _ 2 _ _ .unreachable))) => true
  | _ => false

/-- **The exhaustive chain runs to a value on every variant, never `aborted`**. -/
def runsOn (v : Val) : Bool :=
  match thirdRead with
  | some e =>
    match (eval Host.unit 50 shapeProg ⟨(), (), fun _ => none⟩ [("x", v)] e).2.1 with
    | .ok _ => true
    | _ => false
  | none => false

#guard [.variant "m.Shape" 0 [.int 1], .variant "m.Shape" 1 [], .variant "m.Shape" 2 [.int 7]].all runsOn

end Una.F1.SumImportTests
''')
open(sys.argv[1], "w").write("\n".join(out))
