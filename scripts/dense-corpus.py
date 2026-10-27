#!/usr/bin/env python3
"""The bit-for-bit corpus of the dense kernels (ADR 0120 rule 53, issue #988).

Writes crates/una-core/tests/dense_corpus.txt: cases of the chains of ADR 0120 rules 45 to 50
and of the kernels' `fma` and `ldexp`, each with its inputs and the bits of its expected
outputs. The test crates/una-core/tests/dense_corpus.rs checks that the reference chains of
`una_core::dense` and the blocked kernels of `una_core::dense::blocked` reproduce every case.

The expected bits come from this script, an implementation of the chains independent of the
Rust code: every operation is IEEE 754 binary64 in roundTiesToEven, Python's `+`, `-`, `*`, `/`
and `math.sqrt` (with the IEEE results of a division by zero and of a negative square root
written out, where Python raises), and `fma` and `ldexp` computed exactly on integers and
rounded once by `fractions.Fraction`'s correctly rounded conversion. No platform `fma` is
used. Every NaN written is the canonical one, 0x7FF8000000000000 (ADR 0114 rule 6.1).

Inputs are either listed as bits (`bits:h1,h2,...`) or drawn from a named generator with a
seed, which the Rust test implements too (xorshift64 with shifts 13, 7, 17):
  u:SEED    uniform values j * 2^-52 - 1, j the top 53 bits of a draw: exact, in [-1, 1)
  w:SEED    values of random significand and sign, exponents from -100 to 99
  s:SEED    special values: NaN, both infinities, both zeros, subnormals, the extremes,
            values whose products overflow or underflow, raw bits, and uniform values
  dd:SEED   an n x n matrix of u values with n added to each diagonal element: regular
  spd:SEED  an n x n symmetric matrix of u values with n + |u| on the diagonal: positive
            definite by Gershgorin's theorem
  zero      +0.0 everywhere
A field `patch=P:HEX,...` then stores the value of bits HEX at row-major position P of the
first input (a matrix made singular or indefinite in a chosen place).
Each case records an FNV-1a 64 digest of its input bits (`in=`), so a generator that differs
is reported as such. Outputs of at most 64 values are listed (`out=`); longer ones are given
as the FNV-1a 64 digest of their bits, little-endian, and their count (`digest=HEX:COUNT`).

Usage: scripts/dense-corpus.py [OUT]   (default: crates/una-core/tests/dense_corpus.txt)
Regenerating must give the checked-in file byte for byte; a change of the corpus is a change
of the language's results and needs a record (ADR 0120 rule 46).
"""
import math
import os
import struct
import sys
from fractions import Fraction

MASK = (1 << 64) - 1
SIGN = 1 << 63
CANONICAL_NAN = 0x7FF8000000000000
KB = 64
LANES = 32


def bits(x):
    """The bits of x, a NaN made canonical."""
    if x != x:
        return CANONICAL_NAN
    return struct.unpack("<Q", struct.pack("<d", x))[0]


def from_bits(b):
    return struct.unpack("<d", struct.pack("<Q", b & MASK))[0]


def canon(x):
    return from_bits(CANONICAL_NAN) if x != x else x


NAN = from_bits(CANONICAL_NAN)
INF = math.inf


def sign_of(x):
    return bits(x) >> 63 if x == x else 0


# --- IEEE operations, roundTiesToEven -----------------------------------------------------


def add(a, b):
    return canon(a + b)


def div(a, b):
    if a != a or b != b:
        return NAN
    if b == 0.0:
        if a == 0.0:
            return NAN
        negative = sign_of(a) ^ sign_of(b)
        return -INF if negative else INF
    return canon(a / b)


def sqrt(x):
    if x != x or (x < 0.0):
        return NAN
    return math.sqrt(x)


def neg(x):
    return NAN if x != x else from_bits(bits(x) ^ SIGN)


def exact(x):
    """A finite x as an exact Fraction."""
    return Fraction(x)


def round_fraction(q, negative_zero):
    """The binary64 value nearest the exact rational q, ties to even; an exact zero takes the
    sign `negative_zero` decides."""
    if q == 0:
        return -0.0 if negative_zero else 0.0
    try:
        return float(q)
    except OverflowError:
        return INF if q > 0 else -INF


def fma(a, b, c):
    """IEEE fusedMultiplyAdd, a * b + c rounded once, computed exactly."""
    if a != a or b != b or c != c:
        return NAN
    if math.isinf(a) or math.isinf(b):
        if a == 0.0 or b == 0.0:
            return NAN
        product_negative = sign_of(a) ^ sign_of(b)
        if math.isinf(c) and sign_of(c) != product_negative:
            return NAN
        return -INF if product_negative else INF
    if math.isinf(c):
        return c
    q = exact(a) * exact(b) + exact(c)
    if q == 0:
        product_zero = a == 0.0 or b == 0.0
        if product_zero and c == 0.0:
            # A sum of two zeros: negative only when both are.
            return round_fraction(q, (sign_of(a) ^ sign_of(b)) == 1 and sign_of(c) == 1)
        return 0.0
    return round_fraction(q, False)


def ldexp(y, k):
    """y * 2^k, rounded once to nearest, ties to even."""
    if y != y:
        return NAN
    if math.isinf(y) or y == 0.0:
        return y
    q = exact(y) * (Fraction(2) ** k)
    return round_fraction(q, sign_of(y) == 1) if q != 0 else (-0.0 if y < 0 else 0.0)


def total_key(x):
    """The key of totalOrder of a non-negative value or the canonical NaN: its bits."""
    return bits(x)


def abs_(x):
    return NAN if x != x else from_bits(bits(x) & ~SIGN)


# --- The reference chains of ADR 0120 rules 45 to 50 ----------------------------------------


def lane_sum(items):
    lanes = [0.0] * LANES
    for k, x in enumerate(items):
        lanes[k % LANES] = add(lanes[k % LANES], x)
    w = LANES // 2
    while w >= 1:
        for l in range(w):
            lanes[l] = add(lanes[l], lanes[l + w])
        w //= 2
    return lanes[0]


def dot_from(start, pairs):
    s = start
    p = 0.0
    count = 0
    for x, y in pairs:
        p = fma(x, y, p)
        count += 1
        if count == KB:
            s = add(s, p)
            p = 0.0
            count = 0
    if count > 0:
        s = add(s, p)
    return s


def exponent(m):
    """The integer e with 2^(e-1) <= m < 2^e, for a finite positive m."""
    return math.frexp(m)[1]


def norm2(items):
    largest = 0.0
    for x in items:
        a = abs_(x)
        if total_key(a) > total_key(largest):
            largest = a
    if largest == 0.0:
        return 0.0
    if largest != largest or math.isinf(largest):
        return canon(largest)
    e = exponent(largest)
    scaled = [ldexp(x, -e) for x in items]
    return ldexp(sqrt(dot_from(0.0, [(x, x) for x in scaled])), e)


def multiply(a, b, c, m, k, n):
    """c + a b, element (i, j) a chain of rule 46 from c_ij."""
    out = []
    for i in range(m):
        for j in range(n):
            pairs = [(a[i * k + t], b[t * n + j]) for t in range(k)]
            out.append(dot_from(c[i * n + j], pairs))
    return out


def multiply_array(a, v, m, k):
    return [dot_from(0.0, [(a[i * k + t], v[t]) for t in range(k)]) for i in range(m)]


def lu(a, n):
    a = list(a)
    pivots = [0] * n
    singular = False
    b = 0
    while b < n:
        e = min(b + KB, n)
        for k in range(b, e):
            p = k
            best = total_key(abs_(a[k * n + k]))
            for i in range(k + 1, n):
                candidate = total_key(abs_(a[i * n + k]))
                if candidate > best:
                    p, best = i, candidate
            pivots[k] = p
            if a[p * n + k] == 0.0:
                singular = True
            if p != k:
                for j in range(n):
                    a[p * n + j], a[k * n + j] = a[k * n + j], a[p * n + j]
            pivot = a[k * n + k]
            for i in range(k + 1, n):
                l = div(a[i * n + k], pivot)
                a[i * n + k] = l
                for j in range(k + 1, e):
                    a[i * n + j] = fma(neg(l), a[k * n + j], a[i * n + j])
        for k in range(b, e):
            for i in range(k + 1, e):
                l = a[i * n + k]
                for j in range(e, n):
                    a[i * n + j] = fma(neg(l), a[k * n + j], a[i * n + j])
        for i in range(e, n):
            for j in range(e, n):
                p = 0.0
                for k in range(b, e):
                    p = fma(neg(a[i * n + k]), a[k * n + j], p)
                a[i * n + j] = add(a[i * n + j], p)
        b = e
    return not singular, a, pivots


def cholesky(a, n):
    a = list(a)
    definite = True
    b = 0
    while b < n:
        e = min(b + KB, n)
        for j in range(b, e):
            s = a[j * n + j]
            for k in range(b, j):
                l = a[j * n + k]
                s = fma(neg(l), l, s)
            if not (s > 0.0):
                definite = False
            d = sqrt(s)
            a[j * n + j] = d
            for i in range(j + 1, n):
                t = a[i * n + j]
                for k in range(b, j):
                    t = fma(neg(a[i * n + k]), a[j * n + k], t)
                a[i * n + j] = div(t, d)
        for i in range(e, n):
            for j in range(e, i + 1):
                p = 0.0
                for k in range(b, e):
                    p = fma(neg(a[i * n + k]), a[j * n + k], p)
                a[i * n + j] = add(a[i * n + j], p)
        b = e
    for i in range(n):
        for j in range(i + 1, n):
            a[i * n + j] = 0.0
    return definite, a


def solve_triangular(t, n, b, r, lower, unit):
    b = list(b)
    row = (lambda q: q) if lower else (lambda q: n - 1 - q)
    for c in range(r):
        start = 0
        while start < n:
            end = min(start + KB, n)
            for q in range(start, end):
                i = row(q)
                v = b[i * r + c]
                for earlier in range(start, q):
                    k = row(earlier)
                    v = fma(neg(t[i * n + k]), b[k * r + c], v)
                if not unit:
                    v = div(v, t[i * n + i])
                b[i * r + c] = canon(v)
            for later in range(end, n):
                i = row(later)
                p = 0.0
                for q in range(start, end):
                    k = row(q)
                    p = fma(neg(t[i * n + k]), b[k * r + c], p)
                b[i * r + c] = add(b[i * r + c], p)
            start = end
    return b


def lu_solve(factors, pivots, n, b, r):
    b = list(b)
    for k, p in enumerate(pivots):
        if p != k:
            for c in range(r):
                b[p * r + c], b[k * r + c] = b[k * r + c], b[p * r + c]
    b = solve_triangular(factors, n, b, r, True, True)
    return solve_triangular(factors, n, b, r, False, False)


# --- Generators -----------------------------------------------------------------------------


class Draws:
    """xorshift64 with shifts 13, 7, 17, from a nonzero state made of the seed."""

    def __init__(self, seed):
        self.state = (seed * 0x9E3779B97F4A7C15 + 0x2545F4914F6CDD1D) & MASK or 1

    def next(self):
        s = self.state
        s ^= (s << 13) & MASK
        s ^= s >> 7
        s ^= (s << 17) & MASK
        self.state = s
        return s


def uniform(r):
    return (r >> 11) * 2.0**-52 - 1.0


SPECIALS = [
    NAN,
    INF,
    -INF,
    0.0,
    -0.0,
    5e-324,
    -(2.0**-1022),
    1.7976931348623157e308,
    -1.7976931348623157e308,
    1e300,
    -1e-300,
    3 * 5e-324,
    1.0,
    -1.0,
]


def value(kind, draws):
    r = draws.next()
    if kind == "u":
        return uniform(r)
    if kind == "w":
        exponent_ = (r >> 3) % 200 - 100
        return from_bits(((r >> 63) << 63) | ((1023 + exponent_) << 52) | (r & ((1 << 52) - 1)))
    if kind == "s":
        pick = r % 16
        if pick < len(SPECIALS):
            return SPECIALS[pick]
        if pick == len(SPECIALS):
            return canon(from_bits(draws.next()))
        return uniform(draws.next())
    raise ValueError(kind)


def generate(spec, count, n=0):
    if spec == "zero":
        return [0.0] * count
    if spec.startswith("bits:"):
        out = [canon(from_bits(int(h, 16))) for h in spec[5:].split(",") if h]
        assert len(out) == count, spec
        return out
    kind, seed = spec.split(":")
    draws = Draws(int(seed))
    if kind in ("u", "w", "s"):
        return [value(kind, draws) for _ in range(count)]
    us = [uniform(draws.next()) for _ in range(count)]
    if kind == "dd":
        return [add(us[i * n + j], float(n)) if i == j else us[i * n + j]
                for i in range(n) for j in range(n)]
    if kind == "spd":
        out = [0.0] * count
        for i in range(n):
            for j in range(n):
                if i == j:
                    out[i * n + j] = add(float(n), abs_(us[i * n + i]))
                elif i > j:
                    out[i * n + j] = us[i * n + j]
                    out[j * n + i] = us[i * n + j]
        return out
    raise ValueError(spec)


def fnv(values):
    h = 0xCBF29CE484222325
    for x in values:
        for byte in struct.pack("<Q", bits(x)):
            h ^= byte
            h = (h * 0x100000001B3) & MASK
    return h


def outputs(values):
    if len(values) <= 64:
        return "out=" + ",".join(f"{bits(x):016x}" for x in values)
    return f"digest={fnv(values):016x}:{len(values)}"


# --- The cases ------------------------------------------------------------------------------

LINES = []


def case(fields, inputs, result):
    LINES.append(" ".join(fields + [f"in={fnv(inputs):016x}", result]))


def patched(values, patch):
    """`values` with each (position, value) of `patch` stored."""
    out = list(values)
    for position, x in patch:
        out[position] = x
    return out


def patches(patch):
    return "patch=" + ",".join(f"{position}:{bits(x):016x}" for position, x in patch)


def hexes(values):
    return "bits:" + ",".join(f"{bits(x):016x}" for x in values)


def sizes_around(points):
    out = []
    for p in points:
        out.extend([p - 1, p, p + 1])
    return sorted(set(x for x in out if x >= 0))


def main():
    out_path = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
        os.path.dirname(os.path.abspath(__file__)), "..", "crates", "una-core", "tests",
        "dense_corpus.txt")
    seed = 1

    # Sums (rule 45): empty, one, around L and its multiples, and special values.
    for n in [0, 1, 2, 7] + sizes_around([32, 64, 96]) + [257, 1000]:
        for kind in ("u", "w", "s"):
            spec = f"{kind}:{seed}"
            seed += 1
            x = generate(spec, n)
            case(["sum", f"n={n}", f"x={spec}"], x, outputs([lane_sum(x)]))
    for x in ([-0.0], [-0.0, -0.0], [INF, -INF], [1e308, 1e308, -1e308], [5e-324] * 40):
        case(["sum", f"n={len(x)}", f"x={hexes(x)}"], x, outputs([lane_sum(x)]))

    # Inner products (rule 46): around KB and its multiples.
    for n in [0, 1, 2, 13] + sizes_around([64, 128, 192]) + [1000]:
        for kx, ky in (("u", "u"), ("w", "u"), ("s", "w"), ("s", "s")):
            sx, sy = f"{kx}:{seed}", f"{ky}:{seed + 1}"
            seed += 2
            x, y = generate(sx, n), generate(sy, n)
            case(["dot", f"n={n}", f"x={sx}", f"y={sy}"], x + y,
                 outputs([dot_from(0.0, list(zip(x, y)))]))

    # norm2 (rule 47), scaled; its conformance points: the largest finite value, 2^1023, the
    # smallest subnormal, and tiny values whose squares underflow.
    for n in [0, 1, 3, 64, 65, 200]:
        for kind in ("u", "w", "s"):
            spec = f"{kind}:{seed}"
            seed += 1
            x = generate(spec, n)
            case(["norm2", f"n={n}", f"x={spec}"], x, outputs([norm2(x)]))
    for x in ([1.7976931348623157e308], [2.0**1023], [5e-324], [1e-200], [3.0, 4.0],
              [1.7976931348623157e308] * 2, [1e-300, 1e-310, 5e-324], [-0.0, 0.0],
              [NAN, 1.0], [-INF, 1.0], [2.0**-1060] * 70):
        case(["norm2", f"n={len(x)}", f"x={hexes(x)}"], x, outputs([norm2(x)]))

    # Products (rule 46 and rule 37).
    for m, k, n in [(0, 0, 0), (1, 0, 3), (1, 1, 1), (2, 3, 4), (3, 64, 2), (3, 65, 2),
                    (2, 129, 3), (5, 7, 70), (17, 31, 13), (66, 66, 66)]:
        for kind in ("u", "s"):
            sa, sb = f"{kind}:{seed}", f"u:{seed + 1}"
            seed += 2
            a, b = generate(sa, m * k), generate(sb, k * n)
            case(["multiply", f"m={m}", f"k={k}", f"n={n}", f"a={sa}", f"b={sb}", "c=zero"],
                 a + b, outputs(multiply(a, b, [0.0] * (m * n), m, k, n)))
            sc = f"w:{seed}"
            seed += 1
            c = generate(sc, m * n)
            case(["multiply", f"m={m}", f"k={k}", f"n={n}", f"a={sa}", f"b={sb}", f"c={sc}"],
                 a + b + c, outputs(multiply(a, b, c, m, k, n)))
        sa, sv = f"u:{seed}", f"w:{seed + 1}"
        seed += 2
        a, v = generate(sa, m * k), generate(sv, k)
        case(["multiply_array", f"m={m}", f"k={k}", f"a={sa}", f"v={sv}"], a + v,
             outputs(multiply_array(a, v, m, k)))

    # LU (rule 48): one panel, two, three; singular, NaN, and special values.
    for n in [0, 1, 2, 3, 7, 63, 64, 65, 67, 130]:
        for spec_kind in ("dd", "u"):
            spec = f"{spec_kind}:{seed}"
            seed += 1
            a = generate(spec, n * n, n)
            ok, f, pivots = lu(a, n)
            case(["lu", f"n={n}", f"a={spec}", f"ok={int(ok)}",
                  "pivots=" + ",".join(str(p) for p in pivots)], a, outputs(f))
    for n in [3, 66]:
        spec = f"s:{seed}"
        seed += 1
        a = generate(spec, n * n, n)
        ok, f, pivots = lu(a, n)
        case(["lu", f"n={n}", f"a={spec}", f"ok={int(ok)}",
              "pivots=" + ",".join(str(p) for p in pivots)], a, outputs(f))
    for a in ([1.0, 2.0, 2.0, 4.0], [0.0] * 9, [NAN, 1.0, 1.0, 1.0], [1.0, 2.0, -3.0, 4.0],
              [0.0, 1.0, 1.0, 0.0], [-0.0, 0.0, 0.0, -0.0], [INF, 1.0, 1.0, 1.0]):
        n = math.isqrt(len(a))
        ok, f, pivots = lu(a, n)
        case(["lu", f"n={n}", f"a={hexes(a)}", f"ok={int(ok)}",
              "pivots=" + ",".join(str(p) for p in pivots)], a, outputs(f))
    # A matrix singular only in its second panel: its last row is zero, so the last pivot is
    # an exact zero, found after the whole factorization ran.
    n = 66
    spec = f"dd:{seed}"
    seed += 1
    patch = [(n * (n - 1) + j, 0.0) for j in range(n)]
    a = patched(generate(spec, n * n, n), patch)
    ok, f, pivots = lu(a, n)
    assert not ok
    case(["lu", f"n={n}", f"a={spec}", patches(patch), f"ok={int(ok)}",
          "pivots=" + ",".join(str(p) for p in pivots)], a, outputs(f))

    # The LU solve (rule 48): exchanges, unit lower, upper.
    for n, r in [(1, 1), (3, 2), (64, 1), (65, 3), (130, 2)]:
        sa, sb = f"dd:{seed}", f"u:{seed + 1}"
        seed += 2
        a, b = generate(sa, n * n, n), generate(sb, n * r)
        ok, f, pivots = lu(a, n)
        assert ok
        case(["lu_solve", f"n={n}", f"r={r}", f"a={sa}", f"b={sb}"], a + b,
             outputs(lu_solve(f, pivots, n, b, r)))

    # Cholesky (rule 49): definite, indefinite, NaN, zero pivots.
    for n in [0, 1, 2, 5, 63, 64, 65, 130]:
        spec = f"spd:{seed}"
        seed += 1
        a = generate(spec, n * n, n)
        ok, l = cholesky(a, n)
        case(["cholesky", f"n={n}", f"a={spec}", f"ok={int(ok)}"], a, outputs(l))
    for n in [4, 70]:
        spec = f"u:{seed}"
        seed += 1
        a = generate(spec, n * n, n)
        ok, l = cholesky(a, n)
        case(["cholesky", f"n={n}", f"a={spec}", f"ok={int(ok)}"], a, outputs(l))
    for a in ([1.0, 2.0, 2.0, 1.0], [4.0, 99.0, 2.0, 5.0], [0.0, 0.0, 0.0, 0.0],
              [NAN, 0.0, 0.0, 1.0], [-1.0, 0.0, 0.0, 4.0], [4.0, 0.0, INF, 1.0]):
        n = math.isqrt(len(a))
        ok, l = cholesky(a, n)
        case(["cholesky", f"n={n}", f"a={hexes(a)}", f"ok={int(ok)}"], a, outputs(l))
    # Not positive definite only in the second panel.
    n = 70
    spec = f"spd:{seed}"
    seed += 1
    patch = [(68 * n + 68, -1.0)]
    a = patched(generate(spec, n * n, n), patch)
    ok, l = cholesky(a, n)
    assert not ok
    case(["cholesky", f"n={n}", f"a={spec}", patches(patch), f"ok={int(ok)}"], a, outputs(l))

    # Triangular solves (rule 50), both triangles, with and without a unit diagonal.
    for n, r in [(0, 1), (1, 1), (2, 3), (63, 1), (64, 2), (65, 1), (129, 2), (130, 1)]:
        for lower in (True, False):
            for unit in (False, True):
                st, sb = f"dd:{seed}", f"u:{seed + 1}"
                seed += 2
                t, b = generate(st, n * n, n), generate(sb, n * r)
                name = "lower" if lower else "upper"
                case([name, f"n={n}", f"r={r}", f"unit={int(unit)}", f"t={st}", f"b={sb}"],
                     t + b, outputs(solve_triangular(t, n, b, r, lower, unit)))
    # A zero on the diagonal gives infinities or NaN; special right-hand sides.
    for lower in (True, False):
        t = [0.0, 0.0, 1.0, 2.0] if lower else [2.0, 1.0, 0.0, 0.0]
        b = [1.0, 0.0, 0.0, NAN]
        name = "lower" if lower else "upper"
        case([name, "n=2", "r=2", "unit=0", f"t={hexes(t)}", f"b={hexes(b)}"], t + b,
             outputs(solve_triangular(t, 2, b, 2, lower, False)))

    # The kernels' `fma`: operands and results near overflow, near the underflow threshold and
    # subnormal, cancellations and halfway cases, as the emulation's range test sees them.
    triples = []
    edges = [2.0**995, 2.0**994 * 1.5, 2.0**1020, 2.0**-480, 2.0**-481 * 1.25, 2.0**-1022,
             2.0**-968, 2.0**-969 * 1.75, 1.7976931348623157e308, 5e-324, 2.0**510,
             2.0**-510 * 1.1, 1.0 + 2.0**-52, 1.0 - 2.0**-53, 3.0, 0.1]
    for x in edges:
        for y in edges[::3]:
            for z in (0.0, -0.0, 1.0, -(x * y), 2.0**-1074, -(2.0**1022), 2.0**-1000):
                triples.append((x, -y, z))
    draws = Draws(988)
    for _ in range(400):
        x = from_bits((draws.next() & ((1 << 52) - 1)) | (((draws.next() % 140) + 953) << 52))
        y = from_bits((draws.next() & ((1 << 52) - 1)) | (((draws.next() % 140) + 953) << 52))
        p = x * y
        shift = (draws.next() % 9) - 4
        for z in (-p, -p + math.ulp(p) * shift / 4, math.ulp(p) * 2.0**-((draws.next() % 60))):
            triples.append((x, y, z))
    for _ in range(200):
        triples.append(tuple(canon(from_bits(draws.next())) for _ in range(3)))
    for x, y, z in ((INF, 0.0, 1.0), (-0.0, 1.0, -0.0), (0.0, -1.0, 0.0), (1.0, 1.0, -1.0),
                    (1e308, 10.0, -INF), (1e308, 10.0, -1e308), (1e-200, 1e-200, -0.0),
                    (1.0 + 2.0**-52, 1.0 - 2.0**-52, -1.0), (NAN, 1.0, 1.0)):
        triples.append((x, y, z))
    for x, y, z in triples:
        LINES.append(f"fma {bits(x):016x} {bits(y):016x} {bits(z):016x} "
                     f"out={bits(fma(x, y, z)):016x}")

    # The kernels' `ldexp` (rule 47).
    for y in (1.0, -1.5, 5e-324, 2.0**-1022, 1.7976931348623157e308, 0.3, -0.0, INF):
        for k in (-2000, -1100, -1074, -1023, -60, -1, 0, 1, 60, 1023, 1074, 2000):
            LINES.append(f"ldexp {bits(y):016x} {k} out={bits(ldexp(y, k)):016x}")

    header = [
        "# The bit-for-bit corpus of the dense kernels (ADR 0120 rule 53, issue #988).",
        "# Written by scripts/dense-corpus.py, which documents the format; do not edit.",
    ]
    with open(out_path, "w", encoding="ascii", newline="\n") as out:
        out.write("\n".join(header + LINES) + "\n")


if __name__ == "__main__":
    main()
