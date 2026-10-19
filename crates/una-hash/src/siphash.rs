//! SipHash-2-4 (Aumasson and Bernstein), keyed with 128 bits, with a 64-bit result: two
//! compression rounds per 8-byte block and four finalization rounds. Written here in safe Rust
//! and checked against the reference vectors of its authors (tests/siphash.rs).

/// A 128-bit SipHash key, as two 64-bit words: `k0` from bytes 0 to 7 and `k1` from bytes 8 to
/// 15, little-endian.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SipKey {
    /// The first 64 bits.
    pub k0: u64,
    /// The last 64 bits.
    pub k1: u64,
}

impl SipKey {
    /// The key whose 16 bytes are `bytes`, read as SipHash's reference does.
    pub fn from_bytes(bytes: [u8; 16]) -> SipKey {
        let [a, b, c, d, e, f, g, h, i, j, k, l, m, n, o, p] = bytes;
        SipKey {
            k0: u64::from_le_bytes([a, b, c, d, e, f, g, h]),
            k1: u64::from_le_bytes([i, j, k, l, m, n, o, p]),
        }
    }
}

// No `Debug`: a key never appears in a message, a trace or a report (ADR 0119 rule 9).
impl core::fmt::Debug for SipKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SipKey(..)")
    }
}

/// A SipHash-2-4 computation fed in parts: the result depends only on the concatenation of
/// what was written, never on how it was split.
#[derive(Clone)]
pub struct Sip24 {
    v0: u64,
    v1: u64,
    v2: u64,
    v3: u64,
    /// The bytes written that do not fill a block yet, low byte first.
    tail: u64,
    /// How many bytes of `tail` are filled, 0 to 7.
    ntail: u32,
    /// The total length written, modulo 2^64; its low byte enters the last block.
    length: u64,
}

impl Sip24 {
    /// A computation under `key`.
    pub fn new(key: SipKey) -> Sip24 {
        Sip24 {
            v0: key.k0 ^ 0x736f_6d65_7073_6575,
            v1: key.k1 ^ 0x646f_7261_6e64_6f6d,
            v2: key.k0 ^ 0x6c79_6765_6e65_7261,
            v3: key.k1 ^ 0x7465_6462_7974_6573,
            tail: 0,
            ntail: 0,
            length: 0,
        }
    }

    #[inline]
    fn round(&mut self) {
        self.v0 = self.v0.wrapping_add(self.v1);
        self.v1 = self.v1.rotate_left(13);
        self.v1 ^= self.v0;
        self.v0 = self.v0.rotate_left(32);
        self.v2 = self.v2.wrapping_add(self.v3);
        self.v3 = self.v3.rotate_left(16);
        self.v3 ^= self.v2;
        self.v0 = self.v0.wrapping_add(self.v3);
        self.v3 = self.v3.rotate_left(21);
        self.v3 ^= self.v0;
        self.v2 = self.v2.wrapping_add(self.v1);
        self.v1 = self.v1.rotate_left(17);
        self.v1 ^= self.v2;
        self.v2 = self.v2.rotate_left(32);
    }

    #[inline]
    fn block(&mut self, m: u64) {
        self.v3 ^= m;
        self.round();
        self.round();
        self.v0 ^= m;
    }

    /// Writes `bytes`, in O(|bytes|).
    pub fn write(&mut self, bytes: &[u8]) {
        self.length = self.length.wrapping_add(bytes.len() as u64);
        for &byte in bytes {
            self.tail |= u64::from(byte) << (8 * self.ntail);
            self.ntail += 1;
            if self.ntail == 8 {
                let m = self.tail;
                self.block(m);
                self.tail = 0;
                self.ntail = 0;
            }
        }
    }

    /// The 64-bit result of everything written.
    pub fn finish(mut self) -> u64 {
        let last = self.tail | ((self.length & 0xff) << 56);
        self.block(last);
        self.v2 ^= 0xff;
        self.round();
        self.round();
        self.round();
        self.round();
        self.v0 ^ self.v1 ^ self.v2 ^ self.v3
    }
}

/// SipHash-2-4 of `bytes` under `key`.
pub fn sip24(key: SipKey, bytes: &[u8]) -> u64 {
    let mut state = Sip24::new(key);
    state.write(bytes);
    state.finish()
}
