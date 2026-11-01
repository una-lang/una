//! RSA signature verification: RSASSA-PKCS1-v1_5 and RSASSA-PSS (RFC 8017) with SHA-256, SHA-384
//! and SHA-512, the `RSA_PKCS1_2048_8192_*` and `RSA_PSS_2048_8192_*` algorithms `rustls-webpki`
//! asks of a provider.
//!
//! Verification handles public data only (the public key, the message and the signature), so
//! the exponentiation is variable time in the public exponent (ADR 0133 rule 8); signing and
//! private keys are not part of this crate. The arithmetic is the Montgomery arithmetic of
//! [`crate::mont::DynMont`].
//!
//! **Policy.** A signature verifies only when all of these hold:
//!
//! 1. the key is a strict DER `RSAPublicKey` (`SEQUENCE { INTEGER n, INTEGER e }`, the contents
//!    of the `subjectPublicKey` bit string that `rustls-webpki` hands a verifier) or a strict DER
//!    `SubjectPublicKeyInfo` with the `rsaEncryption` algorithm and its `NULL` parameters (the
//!    `RSASSA-PSS` key type is not accepted). Strict DER: minimal definite lengths (the long form
//!    only for 128 and above, one or two length bytes), no trailing bytes, positive minimal
//!    integers, no BER leniencies;
//! 2. the modulus `n` is odd and between 2048 and 8192 bits (exact bit length, not rounded up to
//!    bytes as `ring` does), and the exponent `e` is odd with `65537 <= e < 2^33`. This is
//!    `rustls-webpki`'s modulus policy with the exponent floor of the CA/Browser Forum and NIST
//!    SP 800-56B (`ring` still accepts `e = 3` when verifying; this module does not);
//! 3. the signature is exactly as long as the modulus in bytes and, as an integer, below `n`
//!    (RFC 8017 §5.2.2 and §8.2.2 step 1);
//! 4. **PKCS#1 v1.5 (§8.2.2, §9.2):** the recovered message `EM = s^e mod n`, as `k` bytes,
//!    is byte for byte equal to the encoding this module builds from the message:
//!    `0x00 0x01 0xff... 0x00 || DigestInfo`. There is no parsing of `EM`: no lenient or
//!    alternative `DigestInfo` (BER, a missing `NULL`, trailing data) can be accepted because
//!    only one byte string is accepted;
//! 5. **PSS (§8.1.2, §9.1.2):** the hash of the message and the mask generation function MGF1
//!    use the same hash, and **the salt is exactly as long as the digest** (as TLS 1.3 requires,
//!    RFC 8446 §4.2.3, and as `ring` verifies).
//!    The encoded message is checked per RFC 8017 §9.1.2 in full: the length condition, the
//!    trailer byte `0xbc`, the zero bits at the top of the masked data block (when the modulus
//!    has `8 k + 1` bits the encoded message is one byte shorter than the signature and the
//!    leading byte of the recovered value must be zero), the zero padding string and the `0x01`
//!    separator, and finally the recomputed hash. `RSA_PSS_*` in webpki means PSS with a
//!    plain `rsaEncryption` key, which is the key form accepted here.
//!
//! **Cost.** `O(|M|)` for the hash and one modular exponentiation whose multiplications are the
//! `O(k^2)` Montgomery product of `k = ceil(bits / 64)` limbs: 16 squarings and one product for
//! `e = 65537`, at most `bits(e)` squarings plus `popcount(e) - 1` products for any permitted
//! `e` (at most 33 and 32). Setup is `O(k^2 log k)` (once per key). Memory is a few kibibytes on
//! the stack; nothing is allocated.
// Indices are loop counters over fixed arrays or bounds computed from public lengths
// (ADR 0133 rule 6).
#![allow(clippy::indexing_slicing)]

use crate::mont::{DynMont, MAX_LIMBS, Wide};
use crate::sha2::{Hash, Sha256, Sha384, Sha512};

/// The hash of the message (and, for PSS, of the mask generation function).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAlg {
    /// SHA-256.
    Sha256,
    /// SHA-384.
    Sha384,
    /// SHA-512.
    Sha512,
}

impl HashAlg {
    /// The digest length in bytes.
    #[must_use]
    pub const fn output_len(self) -> usize {
        match self {
            Self::Sha256 => 32,
            Self::Sha384 => 48,
            Self::Sha512 => 64,
        }
    }

    /// The DER `DigestInfo` prefix of RFC 8017 §9.2 note 1: `SEQUENCE { SEQUENCE { OID, NULL },
    /// OCTET STRING (digest length) }` up to the digest.
    const fn digest_info_prefix(self) -> &'static [u8] {
        match self {
            Self::Sha256 => &[
                0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02,
                0x01, 0x05, 0x00, 0x04, 0x20,
            ],
            Self::Sha384 => &[
                0x30, 0x41, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02,
                0x02, 0x05, 0x00, 0x04, 0x30,
            ],
            Self::Sha512 => &[
                0x30, 0x51, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02,
                0x03, 0x05, 0x00, 0x04, 0x40,
            ],
        }
    }
}

/// The signature did not verify, or the key is not acceptable (the reason is deliberately not
/// distinguished).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Error;

/// The smallest and largest modulus, in bits.
const MIN_MODULUS_BITS: usize = 2048;
const MAX_MODULUS_BITS: usize = 8192;
/// The smallest and largest public exponent.
const MIN_EXPONENT: u64 = 65537;
const MAX_EXPONENT: u64 = (1 << 33) - 1;

/// An RSA public key that passed the policy, with its Montgomery constants.
pub struct PublicKey {
    mont: DynMont,
    e: u64,
    /// The modulus length in bytes (the signature length).
    len: usize,
    /// The modulus length in bits.
    bits: usize,
}

impl PublicKey {
    /// Builds a key from the big-endian bytes of `n` and `e`, without leading zero bytes.
    ///
    /// # Errors
    /// [`Error`] when the modulus or the exponent is outside the policy above.
    pub fn from_components(n: &[u8], e: &[u8]) -> Result<Self, Error> {
        let first = *n.first().ok_or(Error)?;
        if first == 0 || e.first().is_none_or(|&b| b == 0) || e.len() > 5 {
            return Err(Error);
        }
        let bits = 8 * n.len() - first.leading_zeros() as usize;
        if !(MIN_MODULUS_BITS..=MAX_MODULUS_BITS).contains(&bits) {
            return Err(Error);
        }
        let e = e.iter().fold(0u64, |acc, &b| acc << 8 | u64::from(b));
        if !(MIN_EXPONENT..=MAX_EXPONENT).contains(&e) || e & 1 == 0 {
            return Err(Error);
        }
        // `DynMont::new` refuses an even modulus.
        let mont = DynMont::new(n).ok_or(Error)?;
        Ok(Self {
            mont,
            e,
            len: n.len(),
            bits,
        })
    }

    /// Parses a strict DER `RSAPublicKey` (RFC 8017 Appendix A.1.1).
    ///
    /// # Errors
    /// [`Error`] for any malformed encoding or a key outside the policy.
    pub fn from_rsa_public_key_der(der: &[u8]) -> Result<Self, Error> {
        let (body, rest) = der::tlv(0x30, der).ok_or(Error)?;
        if !rest.is_empty() {
            return Err(Error);
        }
        let (n, body) = der::unsigned(body).ok_or(Error)?;
        let (e, body) = der::unsigned(body).ok_or(Error)?;
        if !body.is_empty() {
            return Err(Error);
        }
        Self::from_components(n, e)
    }

    /// Parses a strict DER `SubjectPublicKeyInfo` holding an `rsaEncryption` key.
    ///
    /// # Errors
    /// [`Error`] for any malformed encoding, another algorithm or a key outside the policy.
    pub fn from_spki_der(der: &[u8]) -> Result<Self, Error> {
        /// `SEQUENCE { OID 1.2.840.113549.1.1.1, NULL }`.
        const RSA_ENCRYPTION: [u8; 15] = [
            0x30, 0x0d, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01, 0x05,
            0x00,
        ];
        let (body, rest) = der::tlv(0x30, der).ok_or(Error)?;
        if !rest.is_empty() {
            return Err(Error);
        }
        let body = body.strip_prefix(&RSA_ENCRYPTION[..]).ok_or(Error)?;
        let (bits, rest) = der::tlv(0x03, body).ok_or(Error)?;
        if !rest.is_empty() {
            return Err(Error);
        }
        // No unused bits in the bit string.
        let key = bits.strip_prefix(&[0u8]).ok_or(Error)?;
        Self::from_rsa_public_key_der(key)
    }

    /// The modulus length in bytes, which is the length of every valid signature.
    #[must_use]
    pub fn modulus_len(&self) -> usize {
        self.len
    }

    /// The modulus length in bits.
    #[must_use]
    pub fn modulus_bits(&self) -> usize {
        self.bits
    }

    /// RSASSA-PKCS1-v1_5 verification (RFC 8017 §8.2.2) with the given hash of `message`.
    ///
    /// # Errors
    /// [`Error`] unless the signature is valid under the policy above.
    pub fn verify_pkcs1(
        &self,
        hash: HashAlg,
        message: &[u8],
        signature: &[u8],
    ) -> Result<(), Error> {
        let mut em = [0u8; MAX_LIMBS * 8];
        let em = self.recover(signature, &mut em)?;
        let digest = Digest::of(hash, message);
        let prefix = hash.digest_info_prefix();
        let t_len = prefix.len() + hash.output_len();
        // 0x00 0x01 PS 0x00 T with at least 8 bytes of PS; always true for 2048 bits and more.
        if em.len() < t_len + 11 {
            return Err(Error);
        }
        let ps_end = em.len() - t_len - 1;
        let ok = em[0] == 0
            && em[1] == 1
            && em[2..ps_end].iter().all(|&b| b == 0xff)
            && em[ps_end] == 0
            && em[ps_end + 1..ps_end + 1 + prefix.len()] == *prefix
            && em[ps_end + 1 + prefix.len()..] == *digest.as_slice();
        if ok { Ok(()) } else { Err(Error) }
    }

    /// RSASSA-PSS verification (RFC 8017 §8.1.2) with MGF1 over the same hash and a salt as long
    /// as the digest.
    ///
    /// # Errors
    /// [`Error`] unless the signature is valid under the policy above.
    pub fn verify_pss(&self, hash: HashAlg, message: &[u8], signature: &[u8]) -> Result<(), Error> {
        let mut buf = [0u8; MAX_LIMBS * 8];
        let value = self.recover(signature, &mut buf)?;
        let h_len = hash.output_len();
        let s_len = h_len;
        // emBits = modBits - 1; emLen = ceil(emBits / 8), which is one byte less than the
        // modulus when modBits = 8 k + 1 (the recovered value then has a leading zero byte).
        let em_bits = self.bits - 1;
        let em_len = em_bits.div_ceil(8);
        let em = if em_len == value.len() {
            value
        } else {
            let (lead, rest) = value.split_first().ok_or(Error)?;
            if *lead != 0 {
                return Err(Error);
            }
            rest
        };
        if em.len() != em_len || em_len < h_len + s_len + 2 {
            return Err(Error);
        }
        let digest = Digest::of(hash, message);
        // EM = maskedDB || H || 0xbc
        let (&trailer, rest) = em.split_last().ok_or(Error)?;
        if trailer != 0xbc {
            return Err(Error);
        }
        let (masked_db, h) = rest.split_at(em_len - h_len - 1);
        let unused_bits = 8 * em_len - em_bits;
        let top_mask = 0xffu8 >> unused_bits;
        if masked_db[0] & !top_mask != 0 {
            return Err(Error);
        }
        // DB = maskedDB xor MGF1(H, emLen - hLen - 1), with the unused bits cleared.
        let mut db = [0u8; MAX_LIMBS * 8];
        let db = &mut db[..masked_db.len()];
        mgf1(hash, h, db);
        for (d, m) in db.iter_mut().zip(masked_db) {
            *d ^= m;
        }
        db[0] &= top_mask;
        // PS (zeros) || 0x01 || salt
        let pad = db.len() - s_len - 1;
        if db[..pad].iter().any(|&b| b != 0) || db[pad] != 1 {
            return Err(Error);
        }
        let salt = &db[pad + 1..];
        // H' = Hash(0x00 * 8 || mHash || salt)
        let mut check = Hasher::new(hash);
        check.update(&[0u8; 8]);
        check.update(digest.as_slice());
        check.update(salt);
        let mut h_prime = [0u8; 64];
        check.finish(&mut h_prime);
        if h_prime[..h_len] == *h {
            Ok(())
        } else {
            Err(Error)
        }
    }

    /// RSAVP1 (RFC 8017 §5.2.2): `s^e mod n` as `modulus_len` bytes, for a signature of exactly
    /// that length and below the modulus.
    fn recover<'a>(
        &self,
        signature: &[u8],
        out: &'a mut [u8; MAX_LIMBS * 8],
    ) -> Result<&'a [u8], Error> {
        if signature.len() != self.len {
            return Err(Error);
        }
        let s = DynMont::limbs_from_be(signature).ok_or(Error)?;
        if !self.mont.is_below_modulus(&s) {
            return Err(Error);
        }
        let m: Wide = self.mont.pow_public(&s, self.e);
        let out = &mut out[..self.len];
        DynMont::to_be(&m, out);
        Ok(out)
    }
}

/// Verifies an RSASSA-PKCS1-v1_5 signature with a key in `RSAPublicKey` DER form.
///
/// # Errors
/// [`Error`] for a rejected key or a signature that does not verify.
pub fn verify_pkcs1(
    hash: HashAlg,
    rsa_public_key_der: &[u8],
    message: &[u8],
    signature: &[u8],
) -> Result<(), Error> {
    PublicKey::from_rsa_public_key_der(rsa_public_key_der)?.verify_pkcs1(hash, message, signature)
}

/// Verifies an RSASSA-PSS signature (MGF1 over the same hash, salt as long as the digest) with a
/// key in `RSAPublicKey` DER form.
///
/// # Errors
/// [`Error`] for a rejected key or a signature that does not verify.
pub fn verify_pss(
    hash: HashAlg,
    rsa_public_key_der: &[u8],
    message: &[u8],
    signature: &[u8],
) -> Result<(), Error> {
    PublicKey::from_rsa_public_key_der(rsa_public_key_der)?.verify_pss(hash, message, signature)
}

/// A hash chosen at run time.
enum Hasher {
    S256(Sha256),
    S384(Sha384),
    S512(Sha512),
}

impl Hasher {
    fn new(alg: HashAlg) -> Self {
        match alg {
            HashAlg::Sha256 => Self::S256(Sha256::new()),
            HashAlg::Sha384 => Self::S384(Sha384::new()),
            HashAlg::Sha512 => Self::S512(Sha512::new()),
        }
    }

    fn update(&mut self, data: &[u8]) {
        match self {
            Self::S256(h) => h.update(data),
            Self::S384(h) => h.update(data),
            Self::S512(h) => h.update(data),
        }
    }

    /// Writes the digest to the front of `out` (at least 64 bytes).
    fn finish(self, out: &mut [u8; 64]) {
        let _ = match self {
            Self::S256(h) => h.finalize_into(&mut out[..32]),
            Self::S384(h) => h.finalize_into(&mut out[..48]),
            Self::S512(h) => h.finalize_into(&mut out[..64]),
        };
    }
}

/// A digest and its length.
struct Digest {
    bytes: [u8; 64],
    len: usize,
}

impl Digest {
    fn of(alg: HashAlg, message: &[u8]) -> Self {
        let mut h = Hasher::new(alg);
        h.update(message);
        let mut bytes = [0u8; 64];
        h.finish(&mut bytes);
        Self {
            bytes,
            len: alg.output_len(),
        }
    }

    fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

/// MGF1 (RFC 8017 §B.2.1): fills `out` with `Hash(seed || 0) || Hash(seed || 1) || ...`.
fn mgf1(alg: HashAlg, seed: &[u8], out: &mut [u8]) {
    let h_len = alg.output_len();
    for (counter, chunk) in out.chunks_mut(h_len).enumerate() {
        let mut h = Hasher::new(alg);
        h.update(seed);
        h.update(&(counter as u32).to_be_bytes());
        let mut block = [0u8; 64];
        h.finish(&mut block);
        chunk.copy_from_slice(&block[..chunk.len()]);
    }
}

/// Strict DER reading, as much as an RSA key needs.
mod der {
    /// A DER length: short form below 128, otherwise one or two length bytes, minimal. (A
    /// modulus of 8192 bits is 1,024 bytes, so three-byte lengths never occur in a key.)
    fn length(b: &[u8]) -> Option<(usize, &[u8])> {
        let (&first, rest) = b.split_first()?;
        match first {
            0..=0x7f => Some((usize::from(first), rest)),
            0x81 => {
                let (&len, rest) = rest.split_first()?;
                (len >= 0x80).then_some((usize::from(len), rest))
            }
            0x82 => {
                let (hi, rest) = rest.split_first()?;
                let (lo, rest) = rest.split_first()?;
                let len = usize::from(*hi) << 8 | usize::from(*lo);
                (len >= 0x100).then_some((len, rest))
            }
            _ => None,
        }
    }

    /// Reads one element with the given tag: its content and what follows it.
    pub(super) fn tlv(tag: u8, b: &[u8]) -> Option<(&[u8], &[u8])> {
        let (&t, rest) = b.split_first()?;
        if t != tag {
            return None;
        }
        let (len, rest) = length(rest)?;
        (len <= rest.len()).then(|| rest.split_at(len))
    }

    /// Reads a positive, minimally encoded `INTEGER` and returns its magnitude (the sign byte
    /// removed); the first byte of the magnitude is never zero.
    pub(super) fn unsigned(b: &[u8]) -> Option<(&[u8], &[u8])> {
        let (value, rest) = tlv(0x02, b)?;
        let (&first, tail) = value.split_first()?;
        let magnitude = match first {
            0x00 => {
                // Only a sign byte before a high bit.
                if tail.first().is_none_or(|&second| second & 0x80 == 0) {
                    return None;
                }
                tail
            }
            0x01..=0x7f => value,
            _ => return None, // negative
        };
        Some((magnitude, rest))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn der_integers_are_strict() {
        assert_eq!(der::unsigned(&[2, 1, 5]), Some((&[5u8][..], &[][..])));
        assert_eq!(
            der::unsigned(&[2, 2, 0, 0x80]),
            Some((&[0x80u8][..], &[][..]))
        );
        assert_eq!(der::unsigned(&[2, 1, 5, 9]), Some((&[5u8][..], &[9u8][..])));
        for bad in [
            &[2u8, 0][..],        // empty
            &[2, 1, 0][..],       // zero
            &[2, 2, 0, 5][..],    // redundant sign byte
            &[2, 2, 0, 0][..],    // redundant sign byte
            &[2, 1, 0x80][..],    // negative
            &[2, 3, 5, 5][..],    // content shorter than its length
            &[2, 0x81, 1, 5][..], // long form for a short length
            &[2, 0x80][..],       // indefinite length
            &[3, 1, 5][..],       // wrong tag
            &[][..],
        ] {
            assert_eq!(der::unsigned(bad), None, "{bad:02x?}");
        }
        // Lengths: one byte after 0x81 from 128, two after 0x82 from 256, nothing longer.
        assert!(der::tlv(0x30, &[0x30, 0x81, 0x7f]).is_none());
        let mut one = vec![0x30, 0x81, 0x80];
        one.extend_from_slice(&[0; 128]);
        assert!(der::tlv(0x30, &one).is_some());
        let mut two = vec![0x30, 0x82, 0x00, 0xff];
        two.extend_from_slice(&[0; 255]);
        assert!(der::tlv(0x30, &two).is_none());
        let mut two = vec![0x30, 0x82, 0x01, 0x00];
        two.extend_from_slice(&[0; 256]);
        assert!(der::tlv(0x30, &two).is_some());
        assert!(der::tlv(0x30, &[0x30, 0x83, 0x00, 0x01, 0x00]).is_none());
    }

    #[test]
    fn digest_info_prefixes_have_the_right_lengths() {
        for h in [HashAlg::Sha256, HashAlg::Sha384, HashAlg::Sha512] {
            let p = h.digest_info_prefix();
            // SEQUENCE length covers everything after the two header bytes.
            assert_eq!(usize::from(p[1]), p.len() - 2 + h.output_len());
            assert_eq!(usize::from(*p.last().unwrap()), h.output_len());
        }
    }
}
