"""Ed25519 signing (RFC 8032 §5.1), in Python with the standard library only.

The hunt's registry builder (`itaty.py`) signs its documents with it, so that a program's
registry, provenance and lock are written without the compiler: an implementation of its own,
which the client in `una` must agree with (`spec/30 §11` rule 2). Only signing and the public
key are here; verification is the client's.

Keys come from a 32-byte seed, as `SigningKey::from_bytes` takes it. Extended coordinates; a
multiple of the base point adds the precomputed doublings `2^i·G` its scalar's bits select, and
a key's expansion and public key are computed once per seed: about a millisecond per signature,
which is enough for a few dozen signatures per program. Nothing here is constant-time: it signs
test documents only, never a secret of anyone.
"""

import functools
import hashlib

P = 2 ** 255 - 19
L = 2 ** 252 + 27742317777372353535851937790883648493
D = (-121665 * pow(121666, P - 2, P)) % P
SQRT_M1 = pow(2, (P - 1) // 4, P)


def _recover_x(y, sign):
    x2 = (y * y - 1) * pow(D * y * y + 1, P - 2, P) % P
    if x2 == 0:
        return 0 if not sign else None
    x = pow(x2, (P + 3) // 8, P)
    if (x * x - x2) % P:
        x = x * SQRT_M1 % P
    if (x * x - x2) % P:
        return None
    if x & 1 != sign:
        x = P - x
    return x


_GY = 4 * pow(5, P - 2, P) % P
_GX = _recover_x(_GY, 0)
G = (_GX, _GY, 1, _GX * _GY % P)
IDENTITY = (0, 1, 1, 0)


def _add(a, b):
    x1, y1, z1, t1 = a
    x2, y2, z2, t2 = b
    aa = (y1 - x1) * (y2 - x2) % P
    bb = (y1 + x1) * (y2 + x2) % P
    cc = 2 * t1 * t2 * D % P
    dd = 2 * z1 * z2 % P
    e, f, g, h = bb - aa, dd - cc, dd + cc, bb + aa
    return (e * f % P, g * h % P, f * g % P, e * h % P)


def _doublings():
    table, point = [], G
    for _ in range(256):
        table.append(point)
        point = _add(point, point)
    return table


_G_TABLE = _doublings()


def _mul_base(s):
    """`s·G`, from the table of the base point's doublings."""
    q = IDENTITY
    i = 0
    while s > 0:
        if s & 1:
            q = _add(q, _G_TABLE[i])
        s >>= 1
        i += 1
    return q


def _encode(point):
    x, y, z, _ = point
    zi = pow(z, P - 2, P)
    x, y = x * zi % P, y * zi % P
    return int.to_bytes(y | ((x & 1) << 255), 32, "little")


def _sha512_int(*parts):
    h = hashlib.sha512()
    for part in parts:
        h.update(part)
    return int.from_bytes(h.digest(), "little")


@functools.lru_cache(maxsize=None)
def _expand(seed):
    if len(seed) != 32:
        raise ValueError("an Ed25519 seed is 32 bytes")
    h = hashlib.sha512(seed).digest()
    a = int.from_bytes(h[:32], "little")
    a &= (1 << 254) - 8
    a |= 1 << 254
    return a, h[32:]


@functools.lru_cache(maxsize=None)
def public_key(seed):
    """The 32-byte public key of the key whose seed is `seed`."""
    a, _ = _expand(seed)
    return _encode(_mul_base(a))


def sign(seed, message):
    """The 64-byte signature of `message` by the key whose seed is `seed`."""
    a, prefix = _expand(seed)
    public = public_key(seed)
    r = _sha512_int(prefix, message) % L
    big_r = _encode(_mul_base(r))
    k = _sha512_int(big_r, public, message) % L
    s = (r + k * a) % L
    return big_r + int.to_bytes(s, 32, "little")
