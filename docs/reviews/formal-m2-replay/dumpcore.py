"""Reads the text of `una check --dump-core` (spec/09) into a `una.core/1` document (ADR 0083).

The replay of the formal track's M2 runs the Lean checker on the Core that a release before each
fix of S-01 to S-12 printed for the finding's repro. That release accepted the repro, so its Core
holds the labels its checker inferred, the bug included. Releases before v0.3.4 have no
`una core`, so this script reads their debug text, for the forms the repros use: functions,
capabilities, `let`, `if`, `case`, lambdas, calls, operations, primitives and lists. A handler
or `with` is written as a node the checker does not know, so it is reported as not re-checked.

Usage: python3 -I dumpcore.py <dump.txt> <module> <edition> > <doc.json>
"""

import json
import re
import sys

LEVELS = {"public", "internal", "confidential", "secret", "restricted"}
PRIMS = {"==": "eq", "!=": "ne", "<": "lt", ">": "gt", "<=": "le", ">=": "ge", "+": "add",
         "-": "sub", "*": "mul", "not": "not", "len": "len", "concat": "concat",
         "to_text": "to_text"}

TOKEN = re.compile(r'\s*(?:(?P<str>"(?:[^"\\]|\\.)*")|(?P<arrow>->)|(?P<op>==|!=|<=|>=)'
                   r'|(?P<num>-?\d+)|(?P<name>[A-Za-z_%][A-Za-z0-9_%]*(?:\.[A-Za-z_][A-Za-z0-9_]*)*\*?)'
                   r'|(?P<sym>[()\[\]{},:=.+<>*\-λ⇒↦]))')


def tokens(text):
    out, i = [], 0
    while i < len(text):
        if text[i:].strip() == "":
            break
        m = TOKEN.match(text, i)
        if not m:
            raise SyntaxError(f"cannot read {text[i:i + 30]!r}")
        out.append(m.group(m.lastgroup))
        i = m.end()
    return out


def label(level="public", cats=(), syms=()):
    return {"level": level, "categories": sorted(cats), "symbols": sorted(syms)}


def unknown(what):
    return {"kind": what}


class Parser:
    def __init__(self, toks, module, tparams=()):
        self.t, self.i, self.module, self.tparams = toks, 0, module, set(tparams)

    def peek(self, k=0):
        return self.t[self.i + k] if self.i + k < len(self.t) else None

    def take(self, want=None):
        tok = self.peek()
        if want is not None and tok != want:
            raise SyntaxError(f"expected {want!r}, found {tok!r} at {self.t[self.i:self.i + 8]}")
        self.i += 1
        return tok

    # Types: `secret + label(T*) List[Int]`, `fn(T) -> Int uses {log ↦ Log[write]}`.
    def ty(self):
        level, cats, syms = "public", [], []
        while True:
            tok = self.peek()
            if tok in LEVELS:
                level = self.take()
                if self.peek() == "[" and self.peek(1) not in (None,) and self.peek(2) in (",", "]"):
                    self.take("[")
                    while self.peek() != "]":
                        cats.append(self.take())
                        if self.peek() == ",":
                            self.take()
                    self.take("]")
            elif tok == "label":
                self.take()
                self.take("(")
                syms.append(self.take())
                self.take(")")
            else:
                break
            if self.peek() == "+":
                self.take()
        lab = label(level, cats, syms)
        if self.peek() == "fn":
            self.take()
            self.take("(")
            params = []
            while self.peek() != ")":
                params.append(self.ty())
                if self.peek() == ",":
                    self.take()
            self.take(")")
            self.take("->")
            result = self.ty()
            uses = self.uses()
            fails = None
            if self.peek() == "fails":
                self.take()
                fails = self.take()
            return {"kind": "fn", "params": params, "result": result, "uses": uses,
                    "fails": fails, "label": lab}
        name = self.take()
        args = []
        if self.peek() == "[":
            self.take()
            while self.peek() != "]":
                args.append(self.ty())
                if self.peek() == ",":
                    self.take()
            self.take("]")
        scalars = {"Int": "int", "Bool": "bool", "Text": "text", "Unit": "unit"}
        if name in scalars:
            base = {"kind": scalars[name]}
        elif name in self.tparams:
            base = {"kind": "param", "value": name}
        elif name in ("List", "Option", "Result"):
            base = {"kind": "named", "module": "prelude", "name": name, "args": args}
        else:
            mod, _, short = name.rpartition(".")
            base = {"kind": "named", "module": mod or self.module, "name": short, "args": args}
        return {"kind": "base", "base": base, "label": lab}

    def uses(self):
        out = []
        if self.peek() != "uses":
            return out
        self.take()
        self.take("{")
        while self.peek() != "}":
            cap = self.take()
            self.take("↦")
            capability = self.take()
            self.take("[")
            ops = []
            while self.peek() != "]":
                ops.append(self.take())
                if self.peek() == ",":
                    self.take()
            self.take("]")
            out.append({"cap": cap, "capability": capability, "ops": ops})
            if self.peek() == ",":
                self.take()
        self.take("}")
        return out

    def atom(self):
        tok = self.take()
        if tok.startswith('"'):
            return {"kind": "text", "value": json.loads(tok)}
        if re.fullmatch(r"-?\d+", tok):
            return {"kind": "int", "value": tok}
        if tok in ("true", "false"):
            return {"kind": "bool", "value": tok == "true"}
        if tok == "unit":
            return {"kind": "unit"}
        return {"kind": "var", "value": tok}

    def args(self):
        self.take("(")
        out = []
        while self.peek() != ")":
            out.append(self.atom())
            if self.peek() == ",":
                self.take()
        self.take(")")
        return out

    def targs(self):
        out = []
        if self.peek() == "[":
            self.take()
            while self.peek() != "]":
                out.append(self.ty())
                if self.peek() == ",":
                    self.take()
            self.take("]")
        return out

    def params(self):
        self.take("(")
        out = []
        while self.peek() != ")":
            x = self.take()
            self.take(":")
            out.append([x, self.ty()])
            if self.peek() == ",":
                self.take()
        self.take(")")
        return out

    def bound(self, caps):
        """What a `let` binds: an expression or a right-hand side."""
        tok = self.peek()
        if tok in ("if", "case", "let", "with"):
            return {"kind": "expr", "value": self.expr(caps)}
        return {"kind": "rhs", "value": self.rhs(caps)}

    def rhs(self, caps):
        tok = self.peek()
        if tok == "λ":
            self.take()
            ps = self.params()
            self.take(".")
            return {"kind": "lambda", "params": ps, "body": self.expr(caps), "function": None}
        if tok == "[":
            self.take()
            items = []
            while self.peek() != "]":
                items.append(self.atom())
                if self.peek() == ",":
                    self.take()
            self.take("]")
            return {"kind": "list", "value": items}
        if self.peek(1) in ("(", "[") and tok not in ("Some",):
            name = self.take()
            if name in PRIMS:
                return {"kind": "prim", "prim": {"kind": PRIMS[name]}, "args": self.args()}
            head, _, tail = name.partition(".")
            if head in caps and tail:
                cap = caps[head]
                return {"kind": "op", "cap": head, "capability": cap,
                        "capability_path": {"module": self.module, "name": cap}, "op": tail,
                        "args": self.args()}
            if "." in name:
                mod, _, short = name.rpartition(".")
                targs = self.targs()
                return {"kind": "call", "function": {"module": mod, "name": short},
                        "type_args": targs, "args": self.args()}
            return {"kind": "apply", "callee": {"kind": "var", "value": name},
                    "args": self.args()}
        if tok in ("Some", "None"):
            self.take()
            args = self.args() if tok == "Some" else []
            return {"kind": "construct",
                    "ctor": {"kind": {"module": "prelude", "name": "Option"}, "name": tok,
                             "index": 0 if tok == "Some" else 1}, "args": args}
        return {"kind": "atom", "value": self.atom()}

    def expr(self, caps):
        tok = self.peek()
        if tok == "let":
            self.take()
            x = self.take()
            self.take(":")
            t = self.ty()
            self.take("=")
            v = self.bound(caps)
            self.take("in")
            return {"kind": "let", "name": x, "ty": t, "value": v, "body": self.expr(caps)}
        if tok == "if":
            self.take()
            c = self.atom()
            self.take("then")
            th = self.expr(caps)
            self.take("else")
            return {"kind": "if", "cond": c, "then": th, "otherwise": self.expr(caps)}
        if tok == "case":
            self.take()
            sc = self.atom()
            self.take("of")
            arms, other = [], None
            while True:
                if self.peek() == "else":
                    self.take()
                    self.take("⇒")
                    other = self.expr(caps)
                    break
                ctor = self.take()
                binds = []
                if self.peek() == "(":
                    self.take()
                    while self.peek() != ")":
                        binds.append(self.take())
                        if self.peek() == ",":
                            self.take()
                    self.take(")")
                self.take("⇒")
                kind = "Result" if ctor in ("Ok", "Err") else "Option"
                index = {"Some": 0, "None": 1, "Ok": 0, "Err": 1}.get(ctor, 0)
                arms.append({"ctor": {"kind": {"module": "prelude", "name": kind},
                                      "name": ctor, "index": index},
                             "binds": binds, "body": self.expr(caps)})
                if self.peek() not in ("Some", "None", "Ok", "Err", "else"):
                    break
            return {"kind": "case", "scrutinee": sc, "arms": arms, "otherwise": other}
        if tok == "fail":
            self.take()
            ctor = self.take()
            args = self.args() if self.peek() == "(" else []
            return {"kind": "fail", "ctor": {"kind": {"module": self.module, "name": "?"},
                                             "name": ctor, "index": 0}, "args": args}
        if tok == "with":
            # Outside F1 (F2): the declaration is reported as not re-checked.
            while self.peek() is not None:
                self.take()
            return unknown("with")
        return {"kind": "atom", "atom": self.atom()}


def blocks(text):
    """The top-level blocks of the dump: each starts at column 0."""
    out, cur = [], []
    for line in text.splitlines():
        if line.startswith("exit ") or line.startswith("module "):
            continue
        if line and not line[0].isspace() and line[0] != "}":
            if cur:
                out.append("\n".join(cur))
            cur = [line]
        elif line.strip():
            cur.append(line)
    if cur:
        out.append("\n".join(cur))
    return out


def convert(text, module, edition):
    caps, decls = {}, []
    for block in blocks(text):
        head = block.split()[0]
        if head == "cap":
            name = block.split()[1]
            ops = []
            for line in block.splitlines()[1:]:
                m = re.match(r"\s*op (\w+)(\(.*\)) : (.*) accepts (.*)$", line)
                if not m:
                    continue
                p = Parser(tokens(m.group(2)), module)
                ps = p.params()
                res = Parser(tokens(m.group(3)), module).ty()
                acc = Parser(tokens(m.group(4) + " Unit"), module).ty()["label"]
                ops.append({"name": m.group(1), "params": ps, "result": res, "fails": None,
                            "error": None, "accepts": acc})
            decls.append({"kind": "cap", "name": name, "ops": ops})
    capnames = {d["name"] for d in decls if d["kind"] == "cap"}
    for block in blocks(text):
        head = block.split()[0]
        if head == "handler":
            decls.append({"kind": "handler", "value": {"name": block.split()[1].split("(")[0]}})
        elif head == "fun":
            toks = tokens(block)
            p = Parser(toks, module)
            p.take("fun")
            name = p.take()
            tps, cons = [], []
            if p.peek() == "[":
                p.take()
                while p.peek() != "]":
                    tps.append(p.take())
                    con = "none"
                    if p.peek() == ":":
                        p.take()
                        con = p.take().lower()
                    cons.append({"kind": con})
                    if p.peek() == ",":
                        p.take()
                p.take("]")
            p.tparams = set(tps)
            ps = p.params()
            p.take(":")
            result = p.ty()
            uses = p.uses()
            fails = None
            if p.peek() == "fails":
                p.take()
                fails = p.take()
            p.take("=")
            caprefs = {u["cap"]: u["capability"] for u in uses}
            body = p.expr(caprefs)
            if p.peek() is not None:
                raise SyntaxError(f"{name}: unread {p.t[p.i:p.i + 10]}")
            decls.append({"kind": "fun", "value": {
                "name": name, "type_params": tps, "constraints": cons, "params": ps,
                "result": result, "uses": uses, "fails": fails, "error": None,
                "body": body}})
    del capnames
    return {"schema": "una.core/1", "edition": edition, "complete": True,
            "modules": [{"path": module, "decls": decls}], "library": [], "host": []}


if __name__ == "__main__":
    path, module, edition = sys.argv[1:4]
    with open(path, encoding="utf-8") as f:
        print(json.dumps(convert(f.read(), module, edition), ensure_ascii=False, indent=1))
