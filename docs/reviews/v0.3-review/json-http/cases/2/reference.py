"""A reference for spec/23 §3 and §4, written from the specification alone, for the
cross-review of JSON (#308): parse a text into a value or the first failure with its position
in scalar values, and write a value in canonical form.

Values: None, True/False, ("num", int), ("dec", text), ("str", text), ("arr", [..]),
("obj", {key: value}).
"""

import sys

sys.setrecursionlimit(20000)

WS = " \t\n\r"
MAX_DEPTH = 512
MAX_DIGITS = 4300


class Fail(Exception):
    def __init__(self, kind, at):
        super().__init__(f"{kind} {at}")
        self.kind = kind
        self.at = at


def parse(t):
    p = Parser(t)
    p.ws()
    v = p.value(0)
    p.ws()
    if p.i < len(t):
        raise Fail("syntax", p.i)
    return v


class Parser:
    def __init__(self, t):
        self.t = t
        self.i = 0

    def peek(self):
        return self.t[self.i] if self.i < len(self.t) else None

    def ws(self):
        while self.peek() is not None and self.peek() in WS:
            self.i += 1

    def expect_word(self, word, value):
        for ch in word:
            if self.peek() != ch:
                raise Fail("syntax", self.i)
            self.i += 1
        return value

    def value(self, depth):
        c = self.peek()
        if c is None:
            raise Fail("syntax", self.i)
        if c == "n":
            return self.expect_word("null", None)
        if c == "t":
            return self.expect_word("true", True)
        if c == "f":
            return self.expect_word("false", False)
        if c == '"':
            return ("str", self.string())
        if c == "[" or c == "{":
            if depth + 1 > MAX_DEPTH:
                raise Fail("deep", self.i)
            return self.array(depth + 1) if c == "[" else self.obj(depth + 1)
        if c == "-" or c.isdigit() and c in "0123456789":
            return self.number()
        raise Fail("syntax", self.i)

    def digits(self):
        start = self.i
        while self.peek() is not None and self.peek() in "0123456789":
            self.i += 1
        return self.i - start

    def number(self):
        start = self.i
        if self.peek() == "-":
            self.i += 1
        c = self.peek()
        if c is None or c not in "0123456789":
            raise Fail("syntax", self.i)
        int_start = self.i
        if c == "0":
            self.i += 1
            n_int = 1
        else:
            n_int = self.digits()
        exact = True
        if self.peek() == ".":
            exact = False
            self.i += 1
            if self.digits() == 0:
                raise Fail("syntax", self.i)
        if self.peek() in ("e", "E"):
            exact = False
            self.i += 1
            if self.peek() in ("+", "-"):
                self.i += 1
            if self.digits() == 0:
                raise Fail("syntax", self.i)
        text = self.t[start : self.i]
        if exact:
            if n_int > MAX_DIGITS:
                self.too_long = (start, int_start)
                raise Fail("long", start)
            return ("num", int(text))
        return ("dec", text)

    def string(self):
        self.i += 1  # the opening quote
        out = []
        while True:
            c = self.peek()
            if c is None:
                raise Fail("syntax", self.i)
            if c == '"':
                self.i += 1
                return "".join(out)
            if ord(c) < 0x20:
                raise Fail("syntax", self.i)
            if c != "\\":
                out.append(c)
                self.i += 1
                continue
            self.i += 1
            e = self.peek()
            if e is None:
                raise Fail("syntax", self.i)
            simple = {'"': '"', "\\": "\\", "/": "/", "b": "\b", "f": "\f", "n": "\n", "r": "\r", "t": "\t"}
            if e in simple:
                out.append(simple[e])
                self.i += 1
                continue
            if e != "u":
                raise Fail("syntax", self.i)
            self.i += 1
            unit = self.hex4(first_low=None)
            if 0xD800 <= unit <= 0xDBFF:
                # A high surrogate: `\u` and a low surrogate must follow.
                if self.peek() != "\\":
                    raise Fail("syntax", self.i)
                self.i += 1
                if self.peek() != "u":
                    raise Fail("syntax", self.i)
                self.i += 1
                low = self.hex4(first_low=True)
                out.append(chr(0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00)))
            else:
                out.append(chr(unit))

    def hex4(self, first_low):
        """Four hex digits. `first_low=True`: they must make a low surrogate; None: anything but
        a lone low surrogate. Fails at the first digit that cannot continue."""
        value = 0
        for k in range(4):
            c = self.peek()
            if c is None or c not in "0123456789abcdefABCDEF":
                raise Fail("syntax", self.i)
            d = int(c, 16)
            # Positions 0 and 1 decide whether the unit is a surrogate.
            if first_low is True:
                if k == 0 and d != 0xD:
                    raise Fail("syntax", self.i)
                if k == 1 and d < 0xC:
                    raise Fail("syntax", self.i)
            else:
                if k == 1 and value == 0xD and d >= 0xC:
                    raise Fail("syntax", self.i)  # a lone low surrogate
            value = value * 16 + d
            self.i += 1
        return value

    def array(self, depth):
        self.i += 1
        items = []
        self.ws()
        if self.peek() == "]":
            self.i += 1
            return ("arr", items)
        while True:
            self.ws()
            items.append(self.value(depth))
            self.ws()
            c = self.peek()
            if c == ",":
                self.i += 1
                continue
            if c == "]":
                self.i += 1
                return ("arr", items)
            raise Fail("syntax", self.i)

    def obj(self, depth):
        self.i += 1
        fields = {}
        self.ws()
        if self.peek() == "}":
            self.i += 1
            return ("obj", fields)
        while True:
            self.ws()
            if self.peek() != '"':
                raise Fail("syntax", self.i)
            at = self.i
            key = self.string()
            # The failure is the first error in reading order (spec/23 §3 rule 5): a duplicate
            # key is known once the key is read, before its value.
            if key in fields:
                raise Fail("duplicate", at)
            self.ws()
            if self.peek() != ":":
                raise Fail("syntax", self.i)
            self.i += 1
            self.ws()
            fields[key] = self.value(depth)
            self.ws()
            c = self.peek()
            if c == ",":
                self.i += 1
                continue
            if c == "}":
                self.i += 1
                return ("obj", fields)
            raise Fail("syntax", self.i)


ESCAPED = {"\b": "\\b", "\f": "\\f", "\n": "\\n", "\r": "\\r", "\t": "\\t", '"': '\\"', "\\": "\\\\"}


def quote(s):
    out = ['"']
    for ch in s:
        o = ord(ch)
        if ch in ESCAPED:
            out.append(ESCAPED[ch])
        elif o < 0x20 or o == 0x7F or o in (0x2028, 0x2029) or 0x202A <= o <= 0x202E or 0x2066 <= o <= 0x2069:
            out.append("\\u%04x" % o)
        else:
            out.append(ch)
    out.append('"')
    return "".join(out)


def write(v):
    if v is None:
        return "null"
    if v is True:
        return "true"
    if v is False:
        return "false"
    kind, x = v
    if kind == "num":
        return str(x)
    if kind == "dec":
        return x
    if kind == "str":
        return quote(x)
    if kind == "arr":
        return "[" + ",".join(write(i) for i in x) + "]"
    # Keys in the order of Text (spec/21 §1 rule 3, spec/03): by scalar values.
    return "{" + ",".join(quote(k) + ":" + write(x[k]) for k in sorted(x)) + "}"


def outcome(t):
    try:
        return "ok " + write(parse(t))
    except Fail as f:
        return f"{f.kind} {f.at}"
    except RecursionError:
        return "REFERENCE RECURSION"
