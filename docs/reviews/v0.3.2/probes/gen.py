# Sound generic programs that #354's rule (an annotation keeps `T*` apart from `T` in edition 0.1
# too) might reject, each in edition 0.1 and 0.2. Run: python3 -I gen.py; then ./run-probes.sh.
import os
root = os.path.dirname(os.path.abspath(__file__)) + "/cases"
LOCK = """encoding_version = "una-smt-1"
solver           = "z3"
solver_version   = "5.1.0"
logic            = "QF_UFLIA"
step_budget      = 5000000
random_seed      = 0
"""
HEAD = "module sw\n\npub kind Hold[T]\n  value: T\n\n"
USE = """
pub use_it(s: secret Int) -> secret Int = {call}

pub use_public(n: Int) -> Int = {pcall}
"""
cases = {
 "return_decided": ("choose[T](a: T, b: T, p: T, q: T) -> T = if a == b then p else q\n",
                    "choose(s, b: 0, p: 1, q: 2)", "choose(n, b: 0, p: 1, q: 2)"),
 "annotated_local_returned": ("choose[T](a: T, b: T, p: T, q: T) -> T\n=\n  let y: T = if a == b then p else q\n  y\n",
                    "choose(s, b: 0, p: 1, q: 2)", "choose(n, b: 0, p: 1, q: 2)"),
 "unannotated_local_returned": ("choose[T](a: T, b: T, p: T, q: T) -> T\n=\n  let y = if a == b then p else q\n  y\n",
                    "choose(s, b: 0, p: 1, q: 2)", "choose(n, b: 0, p: 1, q: 2)"),
 "stored_by_generic_param": ("keep[U](x: U) -> List[U] = [x]\n\nchoose[T](a: T, b: T, p: T, q: T) -> List[T] = keep(if a == b then p else q)\n",
                    "choose(s, b: 0, p: 1, q: 2).len", "choose(n, b: 0, p: 1, q: 2).len"),
 "annotated_list": ("choose[T](a: T, b: T, p: T, q: T) -> List[T]\n=\n  let xs: List[T] = [if a == b then p else q]\n  xs\n",
                    "choose(s, b: 0, p: 1, q: 2).len", "choose(n, b: 0, p: 1, q: 2).len"),
 "annotated_record": ("choose[T](a: T, b: T, p: T, q: T) -> Hold[T]\n=\n  let h: Hold[T] = Hold(value: if a == b then p else q)\n  h\n",
                    "choose(s, b: 0, p: 1, q: 2).value", "choose(n, b: 0, p: 1, q: 2).value"),
 "record_returned_unannotated": ("choose[T](a: T, b: T, p: T, q: T) -> Hold[T] = Hold(value: if a == b then p else q)\n",
                    "choose(s, b: 0, p: 1, q: 2).value", "choose(n, b: 0, p: 1, q: 2).value"),
 "append_to_param_list": ("add[T](a: T, b: T, p: T, xs: List[T]) -> List[T] = xs.append(item: if a == b then p else a)\n",
                    "add(s, b: 0, p: 1, xs: [s]).len", "add(n, b: 0, p: 1, xs: [n]).len"),
 "option_returned": ("pick[T](a: T, b: T, p: T) -> Option[T] = if a == b then Some(p) else None\n",
                    "pick(s, b: 0, p: 1).or_else(default: 0)", "pick(n, b: 0, p: 1).or_else(default: 0)"),
 "decided_bool_returned": ("same[T](a: T, b: T) -> Bool = a == b\n",
                    "if same(s, b: 0) then 1 else 0", "if same(n, b: 0) then 1 else 0"),
 "nested_generic_return": ("choose[T](a: T, b: T, p: T, q: T) -> T = if a == b then p else q\n\ntwice[V](a: V, b: V, p: V, q: V) -> V = choose(a, b: b, p: choose(a, b: b, p: p, q: q), q: q)\n",
                    "twice(s, b: 0, p: 1, q: 2)", "twice(n, b: 0, p: 1, q: 2)"),
}
for name, (decl, call, pcall) in cases.items():
    for ed in ("0.1", "0.2"):
        d = f"{root}/{name}__{ed.replace('.', '_')}"
        os.makedirs(d + "/src", exist_ok=True)
        open(d + "/una.toml", "w").write(f'[package]\nname = "{name}"\nversion = "0.1.0"\nedition = "{ed}"\n')
        open(d + "/verification.lock", "w").write(LOCK)
        open(d + "/src/sw.una", "w").write(HEAD + decl + USE.format(call=call, pcall=pcall))
print(len(cases) * 2)
