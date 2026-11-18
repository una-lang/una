# Generates the cross-feature matrix of the edition 0.2 review (#177): information flow across
# features of edition 0.2, each cell a package `cases/<section>__<name>`.
#
# A cell marked `reject` leaks a `secret` to the public `Log` sink if it checks clean; its
# `__control` twin is the same program with public data where the secret was, and is expected
# to check clean (`accept`), unless marked `rule`. Every accepted cell that runs (no host
# capability but the console) is also run by the interpreter (`una run`) and as a native
# executable (`una build`), and the two outputs must agree.
#
# Run: python3 -I gen.py; then UNA=<una> ./run-matrix.sh.
import os
import shutil

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.join(HERE, "cases")

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

pub handler to_console() for Log
  uses @console: Console[print]
  op write(n) = @console.print("{n}")

pub kind Bad = | Oops

"""

MAIN = """
pub main() -> Unit
  uses @console: Console[print]
=
  run_cell() with @log = to_console()

run_cell() -> Unit
  uses @log: Log[write]
=
{call}
"""

CELLS = []
# Extra files and una.toml lines of a cell, by name.
EXTRA_FILES = {}
EXTRA_TOML = {}
HEADERS = {}


def cell(section, name, expect, code, call, control=None, runs=True, note=""):
    """A cell; `control` is the pair (code, call) of its control, or None for none."""
    CELLS.append((f"{section}__{name}", expect, code, call, runs, note))
    if control is not None:
        ccode, ccall, cexpect = control
        CELLS.append((f"{section}__{name}__control", cexpect, ccode, ccall, runs, "control"))


def sub(text, **values):
    for key, value in values.items():
        text = text.replace("{" + key + "}", value)
    return text


# ----- 1. editable bindings and loops: implicit flows through a rebind -----
LOOP_COUNT = """probe(xs: List[Int], s: {lab} Int) -> Unit
  uses @log: Log[write]
=
  let edit n = 0
  for x in xs
    if x == s
      n = n + 1
  @log.write(n)
"""
cell("edit_loop", "count_under_secret_condition", "reject",
     sub(LOOP_COUNT, lab="secret"), "  probe([1, 2, 3], s: 2)",
     (sub(LOOP_COUNT, lab="public"), "  probe([1, 2, 3], s: 2)", "accept"))

FIELD = """pub kind Tally
  count: Int

probe(s: {lab} Int) -> Unit
  uses @log: Log[write]
=
  let edit t = Tally(count: 0)
  if s > 0
    t.count = 1
  @log.write(t.count)
"""
cell("edit_field", "field_set_under_secret_condition", "reject",
     sub(FIELD, lab="secret"), "  probe(5)",
     (sub(FIELD, lab="public"), "  probe(5)", "accept"))

RANGE = """probe(s: {lab} Int) -> Unit
  uses @log: Log[write]
=
  let edit total = 0
  for i in 0..<s
    total = total + 1
  @log.write(total)
"""
cell("edit_loop", "range_bound_secret", "reject",
     sub(RANGE, lab="secret"), "  probe(3)",
     (sub(RANGE, lab="public"), "  probe(3)", "accept"))

SETLIST = """probe(s: {lab} Int) -> Unit
  uses @log: Log[write]
=
  let edit ys = [0, 0, 0]
  if s > 1
    edit ys.set(index: 1, item: 7)
  @log.write(ys.fold(initial: 0, step: fn(a, y) -> a + y))
"""
cell("edit_list", "set_under_secret_condition", "reject",
     sub(SETLIST, lab="secret"), "  probe(2)",
     (sub(SETLIST, lab="public"), "  probe(2)", "accept"))

EDITPARAM = """bump(edit n: Int, by: Int) -> Unit
=
  n = n + by

probe(s: {lab} Int) -> Unit
  uses @log: Log[write]
=
  let edit n = 0
  if s > 0
    edit n.bump(by: 1)
  @log.write(n)
"""
cell("edit_param", "edit_argument_under_secret_condition", "reject",
     sub(EDITPARAM, lab="secret"), "  probe(4)",
     (sub(EDITPARAM, lab="public"), "  probe(4)", "accept"))

# ----- 2. ADR 0075 label variables of an un-annotated editable binding -----
LV_MAP = """probe(s: {lab} Int) -> Unit
  uses @log: Log[write]
=
  let edit m = empty_map()
  edit m.set(key: 1, value: s)
  let v = m.get(key: 1)
    if none use 0
  @log.write(v)
"""
cell("labelvar", "map_value_from_secret", "reject",
     sub(LV_MAP, lab="secret"), "  probe(9)",
     (sub(LV_MAP, lab="public"), "  probe(9)", "accept"))

LV_KEY = """probe(s: {lab} Int) -> Unit
  uses @log: Log[write]
=
  let edit m = empty_map()
  edit m.set(key: s, value: 1)
  edit m.set(key: 5, value: 1)
  @log.write(m.len)
"""
cell("labelvar", "map_key_from_secret_len", "reject",
     sub(LV_KEY, lab="secret"), "  probe(5)",
     (sub(LV_KEY, lab="public"), "  probe(5)", "accept"))

LV_LIST = """probe(s: {lab} Int) -> Unit
  uses @log: Log[write]
=
  let edit xs = []
  for i in 0..<3
    xs = xs.append(item: i + s)
  @log.write(xs.fold(initial: 0, step: fn(a, x) -> a + x))
"""
cell("labelvar", "list_elements_from_secret", "reject",
     sub(LV_LIST, lab="secret"), "  probe(1)",
     (sub(LV_LIST, lab="public"), "  probe(1)", "accept"))

LV_OPT = """probe(s: {lab} Int) -> Unit
  uses @log: Log[write]
=
  let edit o = None
  if s > 0
    o = Some(1)
  let v = o
    if none use 0
  @log.write(v)
"""
cell("labelvar", "option_set_under_secret_condition", "reject",
     sub(LV_OPT, lab="secret"), "  probe(1)",
     (sub(LV_OPT, lab="public"), "  probe(1)", "accept"))

# ----- 3. together: captures, results, cancellation as a channel -----
TOG_RESULT = """probe(s: {lab} Int) -> Unit
  uses @log: Log[write]
=
  together
    let a = s + 1
    let b = 2
  @log.write(a + b)
"""
cell("together", "branch_result_from_secret", "reject",
     sub(TOG_RESULT, lab="secret"), "  probe(1)",
     (sub(TOG_RESULT, lab="public"), "  probe(1)", "accept"))

TOG_FOR = """probe(s: {lab} Int) -> Unit
  uses @log: Log[write]
=
  let ys = together for i in 0..<3
    if i == s then 1 else 0
  @log.write(ys.fold(initial: 0, step: fn(a, y) -> a + y))
"""
cell("together", "for_body_decides_on_secret", "reject",
     sub(TOG_FOR, lab="secret"), "  probe(1)",
     (sub(TOG_FOR, lab="public"), "  probe(1)", "accept"))

TOG_CANCEL = """check(n: {lab} Int) -> Int
  fails Bad
=
  if n > 0 then fail Oops else n

probe(s: {lab} Int) -> Unit
  uses @log: Log[write]
  fails Bad
=
  together
    let a = check(s)
      if error pass
    let b = @log.write(1)
  @log.write(a)
"""
cell("together", "cancellation_reveals_failure", "reject",
     sub(TOG_CANCEL, lab="secret"), "  let r = attempt probe(0)\n  @log.write(0)",
     (sub(TOG_CANCEL, lab="public"), "  let r = attempt probe(0)\n  @log.write(0)", "accept"),
     note="a task failing on a secret cancels a later task's public effect")

TOG_CANCEL_TEXT = """probe(s: {lab} Text) -> Unit
  uses @log: Log[write]
  fails ParseError
=
  together
    let a = s.parse_int()
      if error pass
    let b = @log.write(1)
  @log.write(0)
"""
cell("together", "cancellation_by_labeled_parse_failure", "reject",
     sub(TOG_CANCEL_TEXT, lab="secret"), "  let r = attempt probe(\"12\")\n  @log.write(0)",
     (sub(TOG_CANCEL_TEXT, lab="public"), "  let r = attempt probe(\"12\")\n  @log.write(0)", "accept"))

TOG_ATTEMPT = """probe(s: {lab} Text) -> Unit
  uses @log: Log[write]
=
  together
    let a = attempt s.parse_int()
    let b = 3
  @log.write(if a is Ok then 1 else 0)
"""
cell("together", "attempt_in_branch_on_secret_text", "reject",
     sub(TOG_ATTEMPT, lab="secret"), "  probe(\"12\")",
     (sub(TOG_ATTEMPT, lab="public"), "  probe(\"12\")", "accept"))

# ----- 4. uses of / fails of (ADR 0062) -----
OF_COUNT = """probe(xs: List[Int], s: {lab} Int) -> Unit
  uses @log: Log[write]
=
  let kept = xs.filter(keep: fn(x) -> x == s)
  let done = kept.map(each: fn(x) -> @log.write(1))
  @log.write(0)
"""
cell("uses_of", "effect_count_reveals_filter_on_secret", "reject",
     sub(OF_COUNT, lab="secret"), "  probe([1, 2, 2], s: 2)",
     (sub(OF_COUNT, lab="public"), "  probe([1, 2, 2], s: 2)", "accept"))

OF_FAIL = """check(n: Int) -> Int
  fails Bad
=
  if n > 0 then fail Oops else n

probe(xs: List[Int], s: {lab} Int) -> Unit
  uses @log: Log[write]
  fails Bad
=
  let kept = xs.filter(keep: fn(x) -> x == s)
  let done = kept.map(each: check)
    if error pass
  @log.write(0)
"""
cell("uses_of", "failure_of_callback_reveals_filter_on_secret", "reject",
     sub(OF_FAIL, lab="secret"), "  let r = attempt probe([0, 1], s: 0)\n  @log.write(0)",
     (sub(OF_FAIL, lab="public"), "  let r = attempt probe([0, 1], s: 0)\n  @log.write(0)", "accept"))

OF_USER = """each_logged(xs: List[Int], act: fn(Int) -> Unit) -> Int
  uses of act
=
  let done = xs.map(each: act)
  done.len

probe(xs: List[Int], s: {lab} Int) -> Unit
  uses @log: Log[write]
=
  let kept = xs.filter(keep: fn(x) -> x < s)
  let n = each_logged(kept, act: fn(x) -> @log.write(1))
  @log.write(0)
"""
cell("uses_of", "user_of_function_over_secret_filtered_list", "reject",
     sub(OF_USER, lab="secret"), "  probe([1, 5], s: 3)",
     (sub(OF_USER, lab="public"), "  probe([1, 5], s: 3)", "accept"),
     note="the shape of the list decides how many effects run")

OF_FOLD = """probe(xs: List[Int], s: {lab} Int) -> Unit
  uses @log: Log[write]
=
  let total = xs.fold(initial: 0, step: fn(acc, x) -> if x == s then acc + 1 else acc)
  @log.write(total)
"""
cell("uses_of", "fold_step_decides_on_secret", "reject",
     sub(OF_FOLD, lab="secret"), "  probe([1, 2], s: 2)",
     (sub(OF_FOLD, lab="public"), "  probe([1, 2], s: 2)", "accept"))

# ----- 5. maps: keys and iteration order as channels -----
MAP_HAS = """probe(m: {lab} Map[Int, Int]) -> Unit
  uses @log: Log[write]
=
  @log.write(if m.has(key: 3) then 1 else 0)
"""
cell("map", "has_on_secret_map", "reject",
     sub(MAP_HAS, lab="secret"), "  probe(empty_map().with(key: 3, value: 1))",
     (sub(MAP_HAS, lab="public"), "  probe(empty_map().with(key: 3, value: 1))", "accept"))

MAP_ORDER = """probe(s: {lab} Int) -> Unit
  uses @log: Log[write]
=
  let m: {lab} Map[Int, Int] = empty_map()
  let n = m.with(key: s, value: 1).with(key: 5, value: 2)
  let first = n.values().first()
    if none use 0
  @log.write(first)
"""
cell("map", "iteration_order_reveals_secret_key", "reject",
     sub(MAP_ORDER, lab="secret"), "  probe(3)",
     (sub(MAP_ORDER, lab="public"), "  probe(3)", "accept"))

MAP_WITHOUT = """probe(s: {lab} Int) -> Unit
  uses @log: Log[write]
=
  let m: Map[Int, Int] = empty_map()
  let n = m.with(key: 1, value: 1).with(key: 2, value: 2)
  @log.write(n.without(key: s).len)
"""
cell("map", "without_secret_key_len", "reject",
     sub(MAP_WITHOUT, lab="secret"), "  probe(1)",
     (sub(MAP_WITHOUT, lab="public"), "  probe(1)", "accept"))

MAP_GET = """probe(s: {lab} Int) -> Unit
  uses @log: Log[write]
=
  let m: Map[Int, Int] = empty_map()
  let n = m.with(key: 1, value: 10)
  let v = n.get(key: s)
    if none use 0
  @log.write(v)
"""
cell("map", "get_with_secret_key", "reject",
     sub(MAP_GET, lab="secret"), "  probe(1)",
     (sub(MAP_GET, lab="public"), "  probe(1)", "accept"))

MAP_TALLY = """tally[T: Ord](xs: List[T]) -> Map[T, Int]
=
  let edit m = empty_map()
  for x in xs
    let n = m.get(key: x)
      if none use 0
    edit m.set(key: x, value: n + 1)
  m

probe(s: {lab} Int) -> Unit
  uses @log: Log[write]
=
  @log.write(tally([s, 1, 1]).len)
"""
cell("map", "generic_tally_len_over_secret", "reject",
     sub(MAP_TALLY, lab="secret"), "  probe(1)",
     (sub(MAP_TALLY, lab="public"), "  probe(1)", "accept"))

# ----- 6. JSON -----
JSON_PARSE = """probe(s: {lab} Text) -> Unit
  uses @log: Log[write]
=
  let doc = s.parse_json()
    if error use Null
  let n = when doc
    is Number(v) -> 1
    else -> 0
  @log.write(n)
"""
cell("json", "parse_then_branch_on_secret_text", "reject",
     sub(JSON_PARSE, lab="secret"), "  probe(\"12\")",
     (sub(JSON_PARSE, lab="public"), "  probe(\"12\")", "accept"))

JSON_FAIL = """probe(s: {lab} Text) -> Unit
  uses @log: Log[write]
=
  let r = attempt s.parse_json()
  @log.write(if r is Ok then 1 else 0)
"""
cell("json", "whether_secret_text_parses", "reject",
     sub(JSON_FAIL, lab="secret"), "  probe(\"[1\")",
     (sub(JSON_FAIL, lab="public"), "  probe(\"[1\")", "accept"))

JSON_QUOTE = """probe(s: {lab} Text) -> Unit
  uses @log: Log[write]
=
  let text = "[{s.quote_json()}]"
  let doc = text.parse_json()
    if error use Null
  @log.write(text.len)
"""
cell("json", "quote_json_length", "reject",
     sub(JSON_QUOTE, lab="secret"), "  probe(\"ab\")",
     (sub(JSON_QUOTE, lab="public"), "  probe(\"ab\")", "accept"))

JSON_FIELDS = """probe(s: {lab} Text) -> Unit
  uses @log: Log[write]
=
  let doc = s.parse_json()
    if error use Null
  let n = when doc
    is Array(items) -> items.len
    else -> 0
  @log.write(n)
"""
cell("json", "array_item_count_of_secret_text", "reject",
     sub(JSON_FIELDS, lab="secret"), "  probe(\"[1, 2, 3]\")",
     (sub(JSON_FIELDS, lab="public"), "  probe(\"[1, 2, 3]\")", "accept"))

JSON_TOJSON = """probe(s: {lab} Text) -> Unit
  uses @log: Log[write]
=
  let doc = s.parse_json()
    if error use Null
  @log.write(doc.to_json().len)
"""
cell("json", "to_json_of_parsed_secret", "reject",
     sub(JSON_TOJSON, lab="secret"), "  probe(\"[1, 2]\")",
     (sub(JSON_TOJSON, lab="public"), "  probe(\"[1, 2]\")", "accept"))

# ----- 7. text -----
for name, expr, arg in [
    ("find", "if s.find(text: \"x\") is Some then 1 else 0", "\"axb\""),
    ("split_len", "s.split(on: \",\").len", "\"a,b\""),
    ("replace_len", "s.replace(pattern: \"a\", by: \"bb\").len", "\"aa\""),
    ("chars_len", "s.chars().len", "\"abc\""),
    ("bytes_len", "s.bytes().len", "\"abc\""),
    ("ends_with", "if s.ends_with(suffix: \"c\") then 1 else 0", "\"abc\""),
    ("upper_len", "s.to_upper().len", "\"abc\""),
    ("trim_len", "s.trim_start().len", "\" ab\""),
]:
    code = """probe(s: {lab} Text) -> Unit
  uses @log: Log[write]
=
  @log.write(""" + expr + ")\n"
    cell("text", name, "reject", sub(code, lab="secret"), f"  probe({arg})",
         (sub(code, lab="public"), f"  probe({arg})", "accept"))

TEXT_PARSE = """probe(s: {lab} Text) -> Unit
  uses @log: Log[write]
=
  let n = s.parse_int()
    if error use 0
  @log.write(n)
"""
cell("text", "parse_int_default", "reject",
     sub(TEXT_PARSE, lab="secret"), "  probe(\"x\")",
     (sub(TEXT_PARSE, lab="public"), "  probe(\"x\")", "accept"))

TEXT_SLICE = """probe(s: {lab} Text) -> Unit
  uses @log: Log[write]
=
  let at = s.find(text: ",")
    if none use 0
  @log.write(s.slice(start: 0, end: at).len)
"""
cell("text", "find_then_slice", "reject",
     sub(TEXT_SLICE, lab="secret"), "  probe(\"ab,c\")",
     (sub(TEXT_SLICE, lab="public"), "  probe(\"ab,c\")", "accept"))

TEXT_JOIN = """probe(s: {lab} Text) -> Unit
  uses @log: Log[write]
=
  let parts = ["a", s]
  @log.write(parts.join(separator: ",").len)
"""
cell("text", "join_with_secret_part", "reject",
     sub(TEXT_JOIN, lab="secret"), "  probe(\"bc\")",
     (sub(TEXT_JOIN, lab="public"), "  probe(\"bc\")", "accept"))

# ----- 8. HTTP: request arguments as sinks, responses and failures as sources -----
HTTP_HEAD = """pub capability Out
  op write(text: Text) -> Unit

"""
HTTP_URL = """probe(s: secret Text) -> Unit
  uses @http: Http[get]
=
  let r = attempt @http.get("https://example.com/{s}", headers: [], key_name: None)
  unit
"""
cell("http", "secret_in_url", "reject", HTTP_URL, "  unit", runs=False)
HTTP_HEADER = """probe(s: secret Text) -> Unit
  uses @http: Http[get]
=
  let h = Header(name: "X", value: s)
  let r = attempt @http.get("https://example.com/", headers: [h], key_name: None)
  unit
"""
cell("http", "secret_in_header_value", "reject", HTTP_HEADER, "  unit", runs=False)
HTTP_BODY = """probe(s: secret Text) -> Unit
  uses @http: Http[post]
=
  let r = attempt @http.post("https://example.com/", headers: [], body: s, key_name: None)
  unit
"""
cell("http", "secret_in_body", "reject", HTTP_BODY, "  unit", runs=False)
HTTP_RESPONSE = """probe() -> Unit
  uses @http: Http[get]
  uses @log: Log[write]
=
  let r = attempt @http.get("https://example.com/", headers: [], key_name: None)
  let n = when r
    is Ok(response) -> response.status
    is Err(e) -> 0
  @log.write(n)
"""
cell("http", "response_status_to_public_sink", "reject", HTTP_RESPONSE, "  unit", runs=False)
HTTP_ERRORKIND = """probe(u: internal Text) -> Unit
  uses @http: Http[get]
  uses @log: Log[write]
=
  let r = attempt @http.get(u, headers: [], key_name: None)
  @log.write(if r is Ok then 1 else 0)
"""
cell("http", "whether_internal_url_request_fails", "reject", HTTP_ERRORKIND, "  unit", runs=False)
HTTP_SECRET_KEYNAME = """probe(k: secret Text) -> Unit
  uses @http: Http[get]
=
  let r = attempt @http.get("https://example.com/", headers: [], key_name: Some(k))
  unit
"""
cell("http", "secret_key_name", "reject", HTTP_SECRET_KEYNAME, "  unit", runs=False)

# ----- 9. fixed-width integers: overflow obligations, conversions, named arithmetic -----
FW_WRAP = """probe(s: {lab} U8) -> Unit
  uses @log: Log[write]
=
  @log.write(s.wrapping_add(by: 200).to_int())
"""
cell("fixed", "wrapping_add_of_secret", "reject",
     sub(FW_WRAP, lab="secret"), "  probe(100)",
     (sub(FW_WRAP, lab="public"), "  probe(100)", "accept"))
FW_GUARD = """probe(s: {lab} U8) -> Unit
  uses @log: Log[write]
=
  let r = if s.to_int() + 1 <= 255 then s + 1 else 0
  @log.write(r.to_int())
"""
cell("fixed", "guarded_add_of_secret", "reject",
     sub(FW_GUARD, lab="secret"), "  probe(100)",
     (sub(FW_GUARD, lab="public"), "  probe(100)", "accept"))
FW_NARROW = """probe(s: {lab} Int) -> Unit
  uses @log: Log[write]
=
  let small = if s >= 0 and s <= 255 then s.to_u8() else 0
  @log.write(small.to_int())
"""
cell("fixed", "narrowing_under_secret_guard", "reject",
     sub(FW_NARROW, lab="secret"), "  probe(7)",
     (sub(FW_NARROW, lab="public"), "  probe(7)", "accept"))
FW_SAT = """probe(s: {lab} U8) -> Unit
  uses @log: Log[write]
=
  @log.write(if s.saturating_sub(by: 10) == 0 then 1 else 0)
"""
cell("fixed", "saturating_sub_decides_on_secret", "reject",
     sub(FW_SAT, lab="secret"), "  probe(5)",
     (sub(FW_SAT, lab="public"), "  probe(5)", "accept"))

# ----- 10. generics with constraints: Ord and Eq comparisons as decisions, sort -----
GEN_MAX = """largest[T: Ord](xs: List[T], floor: T) -> T = xs.fold(initial: floor, step: fn(best, x) -> max(best, b: x))

probe(s: {lab} Int) -> Unit
  uses @log: Log[write]
=
  @log.write(largest([1, s], floor: 0))
"""
cell("generic", "largest_with_secret", "reject",
     sub(GEN_MAX, lab="secret"), "  probe(4)",
     (sub(GEN_MAX, lab="public"), "  probe(4)", "accept"))
GEN_SORT = """probe(s: {lab} Int) -> Unit
  uses @log: Log[write]
=
  let sorted = [5, s].sort()
  let first = sorted.first()
    if none use 0
  @log.write(if first == 5 then 1 else 0)
"""
cell("generic", "sort_position_reveals_secret", "reject",
     sub(GEN_SORT, lab="secret"), "  probe(3)",
     (sub(GEN_SORT, lab="public"), "  probe(3)", "accept"))
GEN_SORTLEN = """probe(xs: {lab} List[Int]) -> Unit
  uses @log: Log[write]
=
  @log.write(xs.sort().len)
"""
cell("generic", "sort_of_secret_list_len", "reject",
     sub(GEN_SORTLEN, lab="secret"), "  probe([3, 1])",
     (sub(GEN_SORTLEN, lab="public"), "  probe([3, 1])", "accept"))
GEN_EQ = """index_of[T: Eq](xs: List[T], item: T) -> Int
=
  let edit at = 0 - 1
  let edit i = 0
  for x in xs
    if x == item and at < 0
      at = i
    i = i + 1
  at

probe(s: {lab} Int) -> Unit
  uses @log: Log[write]
=
  @log.write(index_of([1, 2, 3], item: s))
"""
cell("generic", "index_of_secret_item", "reject",
     sub(GEN_EQ, lab="secret"), "  probe(2)",
     (sub(GEN_EQ, lab="public"), "  probe(2)", "rule"),
     note="the decision in the generic body is assigned to a public editable binding (#40)")
GEN_MAPKEY = """firsts[K: Ord](m: Map[K, Int]) -> Int
=
  let vs = m.values()
  let v = vs.first()
    if none use 0
  v

probe(s: {lab} Int) -> Unit
  uses @log: Log[write]
=
  let m: Map[Int, Int] = empty_map()
  @log.write(firsts(m.with(key: s, value: 1).with(key: 4, value: 2)))
"""
cell("generic", "generic_over_map_with_secret_key", "reject",
     sub(GEN_MAPKEY, lab="secret"), "  probe(3)",
     (sub(GEN_MAPKEY, lab="public"), "  probe(3)", "accept"))
GEN_TOGETHER = """pick[T: Ord](a: T, b: T) -> T
=
  together
    let x = max(a, b: b)
    let y = min(a, b: b)
  x

probe(s: {lab} Int) -> Unit
  uses @log: Log[write]
=
  @log.write(pick(s, b: 4))
"""
cell("generic", "together_inside_generic_decision", "reject",
     sub(GEN_TOGETHER, lab="secret"), "  probe(3)",
     (sub(GEN_TOGETHER, lab="public"), "  probe(3)", "accept"))


# ----- 11. effects whose number depends on a secret shape -----
FOR_EFFECT = """probe(xs: {lab} List[Int]) -> Unit
  uses @log: Log[write]
=
  for x in xs
    @log.write(1)
"""
cell("effects", "for_over_secret_list_counts_effects", "reject",
     sub(FOR_EFFECT, lab="secret"), "  probe([1, 2])",
     (sub(FOR_EFFECT, lab="public"), "  probe([1, 2])", "accept"))
RANGE_EFFECT = """probe(s: {lab} Int) -> Unit
  uses @log: Log[write]
=
  for i in 0..<s
    @log.write(1)
"""
cell("effects", "range_with_secret_bound_counts_effects", "reject",
     sub(RANGE_EFFECT, lab="secret"), "  probe(2)",
     (sub(RANGE_EFFECT, lab="public"), "  probe(2)", "accept"))
ELEM_EFFECT = """probe(xs: List[{lab} Int]) -> Unit
  uses @log: Log[write]
=
  for x in xs
    @log.write(1)
"""
cell("effects", "for_over_list_of_secret_elements", "accept",
     sub(ELEM_EFFECT, lab="secret"), "  probe([1, 2])",
     note="only the elements are secret: the number of writes is public")
ELEM_COND = """probe(xs: List[{lab} Int]) -> Unit
  uses @log: Log[write]
=
  for x in xs
    if x > 0
      @log.write(1)
"""
cell("effects", "effect_under_secret_element_condition", "reject",
     sub(ELEM_COND, lab="secret"), "  probe([1, 0])",
     (sub(ELEM_COND, lab="public"), "  probe([1, 0])", "accept"))
TOG_FOR_EFFECT = """probe(s: {lab} Int) -> Unit
  uses @log: Log[write]
=
  let ys = together for i in 0..<s
    @log.write(1)
  @log.write(0)
"""
cell("effects", "together_for_with_secret_bound", "reject",
     sub(TOG_FOR_EFFECT, lab="secret"), "  probe(2)",
     (sub(TOG_FOR_EFFECT, lab="public"), "  probe(2)", "accept"))
MAP_FOR = """probe(m: Map[{lab} Int, Int]) -> Unit
  uses @log: Log[write]
=
  for k in m.keys()
    @log.write(1)
"""
cell("effects", "for_over_keys_of_map_with_secret_keys", "reject",
     sub(MAP_FOR, lab="secret"), "  probe(empty_map().with(key: 1, value: 1))",
     (sub(MAP_FOR, lab="public"), "  probe(empty_map().with(key: 1, value: 1))", "accept"),
     note="which keys a map holds is its shape, labeled by its keys")

# ----- 12. maps whose key type is labeled, with a public top -----
MAP_KEYLAB = """probe(m: Map[{lab} Int, Int]) -> Unit
  uses @log: Log[write]
=
  @log.write(m.len)
"""
cell("map", "len_of_map_with_secret_key_type", "reject",
     sub(MAP_KEYLAB, lab="secret"), "  probe(empty_map().with(key: 1, value: 1))",
     (sub(MAP_KEYLAB, lab="public"), "  probe(empty_map().with(key: 1, value: 1))", "accept"))
MAP_VALLAB = """probe(m: Map[Int, {lab} Int]) -> Unit
  uses @log: Log[write]
=
  @log.write(m.len)
"""
cell("map", "len_of_map_with_secret_values_only", "accept",
     sub(MAP_VALLAB, lab="secret"), "  probe(empty_map().with(key: 1, value: 1))",
     note="only the values are secret: the keys, and so the length, are public")
MAP_HAS_KEYLAB = """probe(m: Map[{lab} Int, Int]) -> Unit
  uses @log: Log[write]
=
  @log.write(if m.has(key: 1) then 1 else 0)
"""
cell("map", "has_on_map_with_secret_key_type", "reject",
     sub(MAP_HAS_KEYLAB, lab="secret"), "  probe(empty_map().with(key: 1, value: 1))",
     (sub(MAP_HAS_KEYLAB, lab="public"), "  probe(empty_map().with(key: 1, value: 1))", "accept"))
MAP_ENTRIES_VAL = """probe(m: Map[{lab} Int, Int]) -> Unit
  uses @log: Log[write]
=
  let firsts = m.entries().map(each: fn(e) -> e.value)
  let v = firsts.first()
    if none use 0
  @log.write(v)
"""
cell("map", "first_value_in_key_order_of_secret_keys", "reject",
     sub(MAP_ENTRIES_VAL, lab="secret"), "  probe(empty_map().with(key: 2, value: 7).with(key: 1, value: 8))",
     (sub(MAP_ENTRIES_VAL, lab="public"), "  probe(empty_map().with(key: 2, value: 7).with(key: 1, value: 8))", "accept"),
     note="the order of public values depends on the secret keys")

# ----- 13. JSON constructors and canonical output -----
JSON_KEY = """probe(s: {lab} Text) -> Unit
  uses @log: Log[write]
=
  let fields: Map[Text, Json] = empty_map()
  let doc = Object(fields.with(key: s, value: Null))
  @log.write(doc.to_json().len)
"""
cell("json", "secret_key_in_object", "reject",
     sub(JSON_KEY, lab="secret"), "  probe(\"k\")",
     (sub(JSON_KEY, lab="public"), "  probe(\"k\")", "accept"))
JSON_ORDER = r"""probe(s: {lab} Text) -> Unit
  uses @log: Log[write]
=
  let text = "\{\"b\": 1, {s.quote_json()}: 2\}"
  let doc = text.parse_json()
    if error use Null
  let n = when doc
    is Object(fields) ->
      let vs = fields.values()
      let v = vs.first()
        if none use Null
      when v
        is Number(k) -> k
        else -> 0
    else -> 0
  @log.write(n)
"""
cell("json", "object_key_order_reveals_secret_key", "reject",
     sub(JSON_ORDER, lab="secret"), "  probe(\"a\")",
     (sub(JSON_ORDER, lab="public"), "  probe(\"a\")", "accept"))

# ----- 14. failure payloads through fails of and together -----
PAYLOAD = """pub kind Got = | Value(n: Int)

fails_with(x: Int) -> Int
  fails Got
=
  fail Value(n: x)

probe(xs: List[{lab} Int]) -> Unit
  uses @log: Log[write]
=
  let r = attempt xs.map(each: fails_with)
  let n = when r
    is Ok(v) -> 0
    is Err(e) ->
      when e
        is Value(n) -> n
  @log.write(n)
"""
cell("fails_of", "payload_of_callback_failure_from_secret_element", "reject",
     sub(PAYLOAD, lab="secret"), "  probe([4])",
     (sub(PAYLOAD, lab="public"), "  probe([4])", "accept"))
TOG_FIRST = """pub kind Which = First | Second

one(t: {lab} Text) -> Int
  fails Which
=
  let n = t.parse_int()
    if error use 0
  if n > 0 then fail First else 0

probe(t: {lab} Text) -> Unit
  uses @log: Log[write]
=
  let r = attempt run_both(t)
  let n = when r
    is Ok(v) -> 0
    is Err(e) ->
      when e
        is First -> 1
        is Second -> 2
  @log.write(n)

run_both(t: {lab} Text) -> Int
  fails Which
=
  together
    let a = one(t)
      if error pass
    let b = fail_second()
      if error pass
  a + b

fail_second() -> Int
  fails Which
=
  fail Second
"""
cell("together", "which_task_failed_first", "reject",
     sub(TOG_FIRST, lab="secret"), "  probe(\"1\")",
     (sub(TOG_FIRST, lab="public"), "  probe(\"1\")", "accept"))

# ----- 15. interpreter and native agree on edge cases (public values only) -----
NATIVE = [
    ("wrapping_mul_u64", r"18446744073709551615.to_u64().wrapping_mul(by: 3).to_int()"),
    ("wrapping_add_i8", r"127.to_i8().wrapping_add(by: 1).to_int()"),
    ("saturating_add_u8", r"250.to_u8().saturating_add(by: 10).to_int()"),
    ("shift_right_u32", r"4294967295.to_u32().shift_right(by: 31).to_int()"),
    ("bit_and_u16", r"65535.to_u16().bit_and(mask: 255).to_int()"),
    ("int_division_negative", r"(0 - 7) / 2"),
    ("int_mod_negative", r"(0 - 7) mod 2"),
    ("big_int_product", r"123456789012345678 * 987654321098765432 mod 1000000007"),
    ("text_unicode_len", r'"a\u{e9}\u{20ac}\u{1f600}".len'),
    ("text_unicode_bytes", r'"a\u{e9}\u{20ac}".bytes().len'),
    ("text_split_empty_parts", r'"a,,b,".split(on: ",").len'),
    ("text_to_upper_non_ascii", r'"\u{e9}a".to_upper().bytes().len'),
    ("sort_texts_first_len", r'["b", "B", "a", "\u{e9}"].sort().join(separator: "|").bytes().len'),
    ("map_keys_order", r'position(empty_map().with(key: "b", value: 2).with(key: "a", value: 1).keys().join(separator: ""), part: "b")'),
    ("json_roundtrip_len", r'json_len("\{\"b\": [1, 2.50, true, -0, 1e3], \"a\": null, \"c\": \"\\u00e9\"\}")'),
    ("parse_int_leading_zeros", r'parse_or_zero("-007")'),
    ("parse_int_huge", r'parse_or_zero("123456789012345678901234567890") mod 1000'),
    ("find_unicode_position", r'position("\u{e9}\u{e9}x", part: "x")'),
    ("replace_overlapping", r'"aaaa".replace(pattern: "aa", by: "b").len'),
    ("sum_of_squares_loop", r"squares(40)"),
]
NATIVE_HELPERS = r"""json_len(t: Text) -> Int
=
  let doc = t.parse_json()
    if error use Null
  doc.to_json().bytes().len

parse_or_zero(t: Text) -> Int
=
  let n = t.parse_int()
    if error use 0
  n

position(t: Text, part: Text) -> Int
=
  let at = t.find(text: part)
    if none use 0 - 1
  at

squares(n: Int) -> Int
=
  let edit total = 0
  for i in 0..<n
    total = total + i * i
  total

"""
for name, expr in NATIVE:
    code = NATIVE_HELPERS + "probe() -> Unit\n  uses @log: Log[write]\n=\n  @log.write(" + expr + ")\n"
    cell("native", name, "accept", code, "  probe()")


# ----- 16. traces: labeled arguments of operations that accept them are redacted alike -----
VAULT = """pub capability Vault
  op keep(n: Int) -> Unit
    accepts secret
  op keep_text(t: Text) -> Unit
    accepts secret
  op keep_list(xs: List[Int]) -> Unit
    accepts secret
  op keep_map(m: Map[Text, Int]) -> Unit
    accepts secret

pub handler drop() for Vault
  op keep(n) = unit
  op keep_text(t) = unit
  op keep_list(xs) = unit
  op keep_map(m) = unit

"""
TRACES = [
    ("secret_int", "probe(s: secret Int) -> Unit\n  uses @vault: Vault[keep]\n=\n  @vault.keep(s)\n", "probe(41)"),
    ("secret_text", "probe(s: secret Text) -> Unit\n  uses @vault: Vault[keep_text]\n=\n  @vault.keep_text(\"x{s}\")\n", 'probe("ab")'),
    ("list_of_secret", "probe(xs: List[secret Int]) -> Unit\n  uses @vault: Vault[keep_list]\n=\n  @vault.keep_list(xs)\n", "probe([1, 2])"),
    ("secret_list", "probe(xs: secret List[Int]) -> Unit\n  uses @vault: Vault[keep_list]\n=\n  @vault.keep_list(xs)\n", "probe([1, 2])"),
    ("map_secret_values", "probe(m: Map[Text, secret Int]) -> Unit\n  uses @vault: Vault[keep_map]\n=\n  @vault.keep_map(m)\n", 'probe(empty_map().with(key: "a", value: 1))'),
    ("map_secret_keys", "probe(m: Map[secret Text, Int]) -> Unit\n  uses @vault: Vault[keep_map]\n=\n  @vault.keep_map(m)\n", 'probe(empty_map().with(key: "a", value: 1))'),
    ("together_secret", "probe(s: secret Int) -> Unit\n  uses @vault: Vault[keep]\n=\n  together\n    let a = @vault.keep(s)\n    let b = @vault.keep(s + 1)\n  unit\n", "probe(7)"),
    ("loop_secret", "probe(s: secret Int) -> Unit\n  uses @vault: Vault[keep]\n=\n  for i in 0..<2\n    @vault.keep(s + i)\n", "probe(7)"),
]
for name, code, call in TRACES:
    CELLS.append((f"traces__{name}", "accept", VAULT + code,
                  f"  {call} with @vault = drop()", True, "traces must redact alike"))

# ----- 17. edit parameters, receivers and recursive kinds -----
GEN_EDIT = """push[T](edit xs: List[T], item: T) -> Unit
=
  xs = xs.append(item: item)

probe(s: {lab} Int) -> Unit
  uses @log: Log[write]
=
  let edit xs: List[Int] = []
  edit xs.push(item: s)
  let v = xs.first()
    if none use 0
  @log.write(v)
"""
cell("edit_param", "generic_edit_parameter_takes_secret_item", "reject",
     sub(GEN_EDIT, lab="secret"), "  probe(3)",
     (sub(GEN_EDIT, lab="public"), "  probe(3)", "accept"))
GEN_EDIT_LEN = """push_if[T: Eq](edit xs: List[T], item: T, other: T) -> Unit
=
  if item == other
    xs = xs.append(item: item)

probe(s: {lab} Int) -> Unit
  uses @log: Log[write]
=
  let edit xs: List[Int] = []
  edit xs.push_if(item: s, other: 3)
  @log.write(xs.len)
"""
cell("edit_param", "generic_edit_parameter_decides_on_secret", "reject",
     sub(GEN_EDIT_LEN, lab="secret"), "  probe(3)",
     (sub(GEN_EDIT_LEN, lab="public"), "  probe(3)", "rule"),
     note="the decision is in the generic body; the editable list's label is public (#40)")
RECEIVER = """pub kind Tally
  count: Int

record(edit t: Tally, n: Int) -> Unit
=
  t.count = t.count + n

probe(s: {lab} Int) -> Unit
  uses @log: Log[write]
=
  let edit t = Tally(count: 0)
  edit t.record(n: s)
  @log.write(t.count)
"""
cell("edit_param", "receiver_takes_secret_amount", "reject",
     sub(RECEIVER, lab="secret"), "  probe(2)",
     (sub(RECEIVER, lab="public"), "  probe(2)", "accept"))
TAKE = """consume(take xs: List[Int]) -> Int = xs.len

probe(s: {lab} Int) -> Unit
  uses @log: Log[write]
=
  let edit xs = [1, 2]
  if s > 0
    xs = xs.append(item: 3)
  let n = consume(take xs)
  @log.write(n)
"""
cell("edit_param", "take_after_secret_append", "reject",
     sub(TAKE, lab="secret"), "  probe(1)",
     (sub(TAKE, lab="public"), "  probe(1)", "accept"))
TREE = """pub kind Tree = Leaf | Node(left: Tree, value: {lab} Int, right: Tree)

size(t: Tree) -> Int
=
  when t
    is Leaf -> 0
    is Node(left, value, right) -> size(left) + 1 + size(right)

total(t: Tree) -> {lab} Int
=
  when t
    is Leaf -> 0
    is Node(left, value, right) -> total(left) + value + total(right)

probe(t: Tree) -> Unit
  uses @log: Log[write]
=
  @log.write(size(t))
"""
cell("recursive", "size_of_tree_with_secret_values", "accept",
     sub(TREE, lab="secret"), "  probe(Node(left: Leaf, value: 3, right: Leaf))",
     note="the shape is public; only the values are secret")
TREE_SUM = TREE.replace("@log.write(size(t))", "@log.write(total(t))")
cell("recursive", "total_of_tree_with_secret_values", "reject",
     sub(TREE_SUM, lab="secret"), "  probe(Node(left: Leaf, value: 3, right: Leaf))",
     (sub(TREE_SUM, lab="public"), "  probe(Node(left: Leaf, value: 3, right: Leaf))", "accept"))
TREE_SHAPE = """pub kind Tree = Leaf | Node(left: Tree, value: {lab} Int, right: Tree)

insert(t: Tree, x: {lab} Int) -> Tree
=
  when t
    is Leaf -> Node(left: Leaf, value: x, right: Leaf)
    is Node(left, value, right) -> if x < value then Node(left: insert(left, x: x), value: value, right: right) else Node(left: left, value: value, right: insert(right, x: x))

depth(t: Tree) -> Int
=
  when t
    is Leaf -> 0
    is Node(left, value, right) -> 1 + max(depth(left), b: depth(right))

probe(a: {lab} Int, b: {lab} Int) -> Unit
  uses @log: Log[write]
=
  let t = insert(insert(insert(Leaf, x: 2), x: a), x: b)
  @log.write(depth(t))
"""
cell("recursive", "search_tree_shape_decided_by_secrets", "reject",
     sub(TREE_SHAPE, lab="secret"), "  probe(1, b: 0)",
     (sub(TREE_SHAPE, lab="public"), "  probe(1, b: 0)", "accept"),
     note="comparisons on secret values decide the shape of a tree whose shape is public")


# ----- 18. printing and filtering labeled structures -----
for name, code, arg, expect_secret in [
    ("interpolate_map_with_secret_keys",
     "probe(m: Map[{lab} Text, Int]) -> Unit\n  uses @log: Log[write]\n=\n  @log.write(\"{m}\".len)\n",
     'empty_map().with(key: "ab", value: 1)', "reject"),
    ("interpolate_map_with_secret_values",
     "probe(m: Map[Text, {lab} Int]) -> Unit\n  uses @log: Log[write]\n=\n  @log.write(\"{m}\".len)\n",
     'empty_map().with(key: "ab", value: 10)', "reject"),
    ("interpolate_list_of_secret",
     "probe(xs: List[{lab} Int]) -> Unit\n  uses @log: Log[write]\n=\n  @log.write(\"{xs}\".len)\n",
     "[10, 2]", "reject"),
    ("filter_map_entries_on_secret_values",
     "probe(m: Map[Int, {lab} Int]) -> Unit\n  uses @log: Log[write]\n=\n  @log.write(m.entries().filter(keep: fn(e) -> e.value > 0).len)\n",
     "empty_map().with(key: 1, value: 5).with(key: 2, value: 0)", "reject"),
    ("loop_appends_on_secret_values",
     "probe(m: Map[Int, {lab} Int]) -> Unit\n  uses @log: Log[write]\n=\n  let edit kept: List[Int] = []\n  for e in m.entries()\n    if e.value > 0\n      kept = kept.append(item: e.key)\n  @log.write(kept.len)\n",
     "empty_map().with(key: 1, value: 5).with(key: 2, value: 0)", "reject"),
    ("default_of_missing_key_in_secret_map",
     "probe(m: {lab} Map[Int, Int]) -> Unit\n  uses @log: Log[write]\n=\n  let v = m.get(key: 1)\n    if none use 7\n  @log.write(v)\n",
     "empty_map()", "reject"),
    ("integer_constraint_to_int",
     "widen[T: Integer](x: T) -> Int = x.to_int()\n\nprobe(s: {lab} U8) -> Unit\n  uses @log: Log[write]\n=\n  @log.write(widen(s))\n",
     "200", "reject"),
    ("sum_of_secret_bytes",
     "probe(s: {lab} Text) -> Unit\n  uses @log: Log[write]\n=\n  @log.write(s.bytes().fold(initial: 0, step: fn(a, b) -> a + b.to_int()))\n",
     '"ab"', "reject"),
    ("kind_with_secret_field_interpolated",
     "pub kind Card\n  number: {lab} Text\n  holder: Text\n\nprobe(c: Card) -> Unit\n  uses @log: Log[write]\n=\n  @log.write(\"{c}\".len)\n",
     'Card(number: "1234", holder: "a")', "reject"),
]:
    cell("print", name, expect_secret, sub(code, lab="secret"), f"  probe({arg})",
         (sub(code, lab="public"), f"  probe({arg})", "accept"))


# ----- 19. edition 0.2 decisions on values of T given to functions named by `uses of` -----
OF_HEAD = """g[T: Eq](a: T, b: T, p: T, q: T, k: fn(T) -> Unit) -> Unit
  uses of k
=
{body}

"""
OF_CELLS = [
    ("direct", "  k({d})"),
    ("map", "  let done = [{d}].map(each: k)\n  unit"),
    ("together", "  together\n    let x = k({d})\n    let y = 1\n  unit"),
    ("let_bound", "  let chosen = {d}\n  k(chosen)"),
    ("map_key", "  let m = empty_map().with(key: 1, value: {d})\n  let v = m.get(key: 1)\n    if none use p\n  k(v)"),
    ("option", "  let o = Some({d})\n  let v = o\n    if none use p\n  k(v)"),
    ("for_loop", "  for x in [{d}]\n    k(x)"),
    ("fold", "  let n = [{d}].fold(initial: 0, step: fn(acc, x) -> acc + 1)\n  k({d})"),
]
for name, body in OF_CELLS:
    for decided, suffix, expect in (("if a == b then p else q", "", "reject"), ("p", "__control", "accept")):
        code = OF_HEAD.replace("{body}", body.replace("{d}", decided)) + """leak(s: secret Int) -> Unit
  uses @log: Log[write]
=
  g([s], b: [0], p: [], q: [1, 2], k: fn(xs) -> @log.write(xs.len))
"""
        CELLS.append((f"of_raise__{name}{suffix}", expect, code, "  leak(0)", True,
                      "edition 0.2: a decision on T given to a function named by `uses of`"))


# ----- 20. handlers that capture labeled arguments -----
for name, code, call, expect, ctl in [
    ("handler_effect_on_captured_secret",
     "pub handler gated(limit: {lab} Int) for Log\n  uses @console: Console[print]\n  op write(n) = report(n, limit: limit)\n\nreport(n: Int, limit: {lab} Int) -> Unit\n  uses @console: Console[print]\n=\n  if n > limit\n    @console.print(\"big\")\n\nprobe() -> Unit\n  uses @log: Log[write]\n=\n  @log.write(5)\n",
     "MAIN:  probe() with @log = gated(3)", "reject", "accept"),
    ("handler_result_is_captured_secret",
     "pub capability Source\n  op read() -> Int\n\npub handler fixed(value: {lab} Int) for Source\n  op read() = value\n\nprobe() -> Unit\n  uses @src: Source[read]\n  uses @log: Log[write]\n=\n  let n = @src.read()\n  @log.write(n)\n",
     "  probe() with @src = fixed(3)", "reject", "accept"),
    ("handler_failure_on_captured_secret",
     "pub capability Gate\n  op allow(n: Int) -> Int\n    fails Bad\n\npub handler strict(limit: {lab} Int) for Gate\n  op allow(n) = if n > limit then fail Oops else n\n\nprobe() -> Unit\n  uses @gate: Gate[allow]\n  uses @log: Log[write]\n=\n  let r = attempt @gate.allow(5)\n  @log.write(if r is Ok then 1 else 0)\n",
     "  probe() with @gate = strict(3)", "reject", "accept"),
    ("handler_argument_from_secret_in_with",
     "pub capability Source\n  op read() -> Int\n\npub handler fixed(value: Int) for Source\n  op read() = value\n\nprobe(s: {lab} Int) -> Unit\n  uses @log: Log[write]\n=\n  let n = read_it() with @src = fixed(s)\n  @log.write(n)\n\nread_it() -> Int\n  uses @src: Source[read]\n=\n  @src.read()\n",
     "  probe(3)", "reject", "accept"),
]:
    cell("handler", name, expect, sub(code, lab="secret"), call,
         (sub(code, lab="public"), call, ctl))

# ----- 21. policies and trusted modules -----
POLICY_TOML = '\n[labels]\ncategories = ["Pin"]\n\n[trusted]\nmodules = ["release"]\n'
RELEASE = """trusted module release

import helpers (digits_of)

pub policy digit_count
  from secret[Pin] Text
  to public Nat
  via count_digits
  purpose "how long a pin is, not what it is"
  owner review

count_digits(t: secret[Pin] Text) -> public Nat = digits_of(t)
"""
HELPERS_LEAK = """module helpers

pub digits_of(t: Text) -> Nat = t.len
"""
HELPERS_OK = """module helpers

pub digits_of(t: secret[Pin] Text) -> secret[Pin] Nat = t.len
"""
for name, helpers, expect in [("via_calls_untrusted_helper_with_public_parameter", HELPERS_LEAK, "accept"),
                              ("via_calls_untrusted_helper_with_labeled_parameter", HELPERS_OK, "accept")]:
    code = """probe(pin: secret[Pin] Text) -> Unit
  uses @log: Log[write]
=
  @log.write(digit_count(pin))
"""
    full = f"cell_{name}"
    CELLS.append((f"policy__{name}", expect, code, '  probe("1234")', True,
                  "a policy's via function calls a module that is not trusted; via bodies are not checked for labels (spec/07 §6 rule 6)"))
    EXTRA_TOML[f"policy__{name}"] = POLICY_TOML
    EXTRA_FILES[f"policy__{name}"] = {"release.una": RELEASE, "helpers.una": helpers}
    HEADERS[f"policy__{name}"] = HEAD.replace("module sw\n", "module sw\n\nimport release (digit_count)\n")
VIA_LOOP = """trusted module release

pub policy first_digit
  from secret[Pin] Text
  to public Nat
  via scan
  purpose "the first digit only"
  owner review

scan(t: secret[Pin] Text) -> public Nat
=
  let edit total = 0
  for c in t.chars()
    let d = c.parse_int()
      if error use 0
    total = total * 10 + d
  total
"""
CELLS.append(("policy__via_with_loop_computes_more_than_its_purpose", "accept", """probe(pin: secret[Pin] Text) -> Unit
  uses @log: Log[write]
=
  @log.write(first_digit(pin))
""", '  probe("1234")', True, "via bodies are not checked for labels (spec/07 §6 rule 6), edition 0.2 constructs included"))
EXTRA_TOML["policy__via_with_loop_computes_more_than_its_purpose"] = POLICY_TOML
EXTRA_FILES["policy__via_with_loop_computes_more_than_its_purpose"] = {"release.una": VIA_LOOP}
HEADERS["policy__via_with_loop_computes_more_than_its_purpose"] = HEAD.replace("module sw\n", "module sw\n\nimport release (first_digit)\n")
TRUSTED_DIRECT = """trusted module release

pub peek(t: secret[Pin] Text) -> Nat = t.len
"""
CELLS.append(("policy__trusted_function_outside_a_policy", "reject", """probe(pin: secret[Pin] Text) -> Unit
  uses @log: Log[write]
=
  @log.write(peek(pin))
""", '  probe("1234")', True, "a trusted module lowers labels only through a policy"))
EXTRA_TOML["policy__trusted_function_outside_a_policy"] = POLICY_TOML
EXTRA_FILES["policy__trusted_function_outside_a_policy"] = {"release.una": TRUSTED_DIRECT}
HEADERS["policy__trusted_function_outside_a_policy"] = HEAD.replace("module sw\n", "module sw\n\nimport release (peek)\n")

# ----- 22. checked and opaque predicates on labeled values -----
CHECKED = """pub predicate opaque small(n: Int) -> Bool = n < 10

pub kind Small = Int where small(it)

narrow(n: {lab} Int) -> Small
  fails Bad
=
  checked(n, small(it)) else fail Oops

probe(s: {lab} Int) -> Unit
  uses @log: Log[write]
=
  let r = attempt narrow(s)
  @log.write(if r is Ok then 1 else 0)
"""
cell("checked", "whether_checked_fails_on_secret", "reject",
     sub(CHECKED, lab="secret"), "  probe(3)",
     (sub(CHECKED, lab="public"), "  probe(3)", "accept"))
for nm in ("checked__whether_checked_fails_on_secret", "checked__whether_checked_fails_on_secret__control"):
    EXTRA_TOML[nm] = "\n[escape_budget]\nchecked_per_module = 1\n"


# ----- 23. via bodies are not checked for labels (spec/07 §6 rule 6): edition 0.2 rules too -----
for name, via in [
    ("edit_under_labeled_condition", "v(s: secret[Pin] Int) -> public Int\n=\n  let edit n = 0\n  if s > 0\n    n = 1\n  n\n"),
    ("map_key_from_labeled_value", "v(s: secret[Pin] Int) -> public Int\n=\n  let edit m: Map[Int, Int] = empty_map()\n  edit m.set(key: s, value: 1)\n  m.len\n"),
    ("loop_bound_from_labeled_value", "v(s: secret[Pin] Int) -> public Int\n=\n  let edit xs: List[Int] = []\n  for i in 0..<s\n    xs = xs.append(item: i)\n  xs.len\n"),
    ("plain_if_on_labeled_value", "v(s: secret[Pin] Int) -> public Int = if s > 0 then 1 else 0\n"),
    ("together_on_labeled_value", "v(s: secret[Pin] Int) -> public Int\n=\n  together\n    let a = s + 1\n    let b = 2\n  a + b\n"),
]:
    nm = f"via_02__{name}"
    release = ("trusted module release\n\npub policy p\n  from secret[Pin] Int\n  to public Int\n  via v\n"
               "  purpose \"a review cell\"\n  owner review\n\n" + via)
    CELLS.append((nm, "accept", "probe(s: secret[Pin] Int) -> Unit\n  uses @log: Log[write]\n=\n  @log.write(p(s))\n",
                  "  probe(2)", True, "via bodies are exempt from label checks, spec/07 §6 rule 6"))
    EXTRA_TOML[nm] = POLICY_TOML
    EXTRA_FILES[nm] = {"release.una": release}
    HEADERS[nm] = HEAD.replace("module sw\n", "module sw\n\nimport release (p)\n")


def write_cell(name, expect, code, call, runs, note):
    path = os.path.join(ROOT, name)
    os.makedirs(os.path.join(path, "src"))
    grants = '["console"]'
    with open(os.path.join(path, "una.toml"), "w") as f:
        f.write(f'[package]\nname = "{name[:60]}"\nversion = "0.1.0"\nedition = "0.2"\n\n'
                f'[run]\nentry = "sw"\ngrants = {grants}\n' + EXTRA_TOML.get(name, ""))
    for rel, text in EXTRA_FILES.get(name, {}).items():
        with open(os.path.join(path, "src", rel), "w") as f:
            f.write(text)
    with open(os.path.join(path, "verification.lock"), "w") as f:
        f.write(LOCK)
    header = f"-- expect: {expect}: {name}" + (f" ({note})" if note else "") + (
        "" if runs else " [check only]") + "\n"
    with open(os.path.join(path, "src", "sw.una"), "w") as f:
        head = HEADERS.get(name, HEAD)
        if call.startswith("MAIN:"):
            main = "\npub main() -> Unit\n  uses @console: Console[print]\n=\n" + call[len("MAIN:"):] + "\n"
        else:
            main = MAIN.replace("{call}", call)
        f.write(header + head + code + main)


if os.path.isdir(ROOT):
    shutil.rmtree(ROOT)
for c in CELLS:
    write_cell(*c)
print(f"{len(CELLS)} cells")
