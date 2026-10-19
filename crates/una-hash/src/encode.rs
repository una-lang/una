//! The hash encoding of an `Eq` value (ADR 0119 rule 9): the one definition, by value, that both
//! engines feed to SipHash-2-4. Each engine walks its own representation of a value and calls
//! these functions in the rule's order; this module writes the bytes.
//!
//! Its two properties, which the tests pin:
//! - equal values encode equally: every integer has one form, chosen by its mathematical value,
//!   whatever its type or representation;
//! - distinct values of one type encode differently: every integer is self-delimiting, and every
//!   variable-length part is preceded by its length.

use crate::siphash::Sip24;

/// Where an encoding goes: a SipHash computation, or, for tests, a byte vector.
pub trait Sink {
    /// Appends `bytes`.
    fn put(&mut self, bytes: &[u8]);
}

impl Sink for Sip24 {
    fn put(&mut self, bytes: &[u8]) {
        self.write(bytes);
    }
}

impl Sink for Vec<u8> {
    fn put(&mut self, bytes: &[u8]) {
        self.extend_from_slice(bytes);
    }
}

/// The encoder of one value, over a sink.
pub struct Encoder<'a, S: Sink> {
    sink: &'a mut S,
}

/// The first byte of an integer in -2^63 to 2^63 - 1.
const SMALL: u8 = 0;
/// The first byte of an integer outside that range.
const BIG: u8 = 1;
/// The first byte of an `F64`.
const FLOAT: u8 = 2;

impl<'a, S: Sink> Encoder<'a, S> {
    /// An encoder that writes to `sink`.
    pub fn new(sink: &'a mut S) -> Encoder<'a, S> {
        Encoder { sink }
    }

    /// An integer in -2^63 to 2^63 - 1: the byte 0 and 8 bytes of two's complement,
    /// little-endian.
    pub fn int(&mut self, value: i64) {
        self.sink.put(&[SMALL]);
        self.sink.put(&value.to_le_bytes());
    }

    /// An unsigned integer: in the small form up to 2^63 - 1, and past it in the big form, as an
    /// `Int` of the same value takes (a `U64` of 2^63 or more).
    pub fn uint(&mut self, value: u64) {
        match i64::try_from(value) {
            Ok(small) => self.int(small),
            Err(_) => self.big(false, &value.to_le_bytes()),
        }
    }

    /// An integer given by its sign and its magnitude, little-endian, of any length and with any
    /// number of zero bytes at its end. A value that lies in -2^63 to 2^63 - 1 takes the small
    /// form, so a promoted `Int` encodes as a tagged one of the same value. Otherwise: the byte
    /// 1, a sign byte (0 for positive, 1 for negative), the length of the magnitude in bytes as
    /// 8 bytes little-endian, and the magnitude, with no zero byte at its end.
    pub fn big(&mut self, negative: bool, magnitude: &[u8]) {
        let mut len = magnitude.len();
        while len > 0 && magnitude.get(len - 1) == Some(&0) {
            len -= 1;
        }
        let magnitude = magnitude.get(..len).unwrap_or_default();
        if len <= 8 {
            let mut word = [0u8; 8];
            for (to, from) in word.iter_mut().zip(magnitude) {
                *to = *from;
            }
            let m = u64::from_le_bytes(word);
            // -2^63 to 2^63 - 1: a magnitude up to 2^63 - 1, or 2^63 when negative.
            if m <= i64::MAX as u64 {
                let v = m as i64;
                self.int(if negative { -v } else { v });
                return;
            }
            if negative && m == 1u64 << 63 {
                self.int(i64::MIN);
                return;
            }
        }
        // Zero has no sign: an empty magnitude took the small form above.
        self.sink.put(&[BIG, u8::from(negative)]);
        self.sink.put(&(len as u64).to_le_bytes());
        self.sink.put(magnitude);
    }

    /// A `Bool`, as the integer 0 or 1.
    pub fn bool(&mut self, value: bool) {
        self.int(i64::from(value));
    }

    /// `unit`, as the integer 0.
    pub fn unit(&mut self) {
        self.int(0);
    }

    /// A variant's declaration index, or an `Id`'s value, as an integer.
    pub fn index(&mut self, index: u64) {
        self.uint(index);
    }

    /// An `F64`, by its bits: the byte 2 and the 8 bytes of the bits, little-endian. The caller
    /// passes the canonical bits (ADR 0114 rule 6.1), so every NaN encodes alike, and `-0.0` and
    /// `0.0` differ, as their `==` does.
    pub fn f64_bits(&mut self, bits: u64) {
        self.sink.put(&[FLOAT]);
        self.sink.put(&bits.to_le_bytes());
    }

    /// An `F32`, as the integer whose value is its 32 canonical bits.
    pub fn f32_bits(&mut self, bits: u32) {
        self.int(i64::from(bits));
    }

    /// A length or a count, as an integer.
    pub fn len(&mut self, n: usize) {
        self.uint(n as u64);
    }

    /// A `Text` (its UTF-8 bytes) or a `Hashed` (its digest bytes): the length in bytes as an
    /// integer, then the bytes.
    pub fn bytes(&mut self, bytes: &[u8]) {
        self.len(bytes.len());
        self.sink.put(bytes);
    }
}
