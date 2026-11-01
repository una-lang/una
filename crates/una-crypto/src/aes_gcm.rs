//! AES-128-GCM and AES-256-GCM authenticated encryption (NIST SP 800-38D) with a 96-bit nonce
//! and a 128-bit tag, the AEADs of the TLS suites `TLS_AES_128_GCM_SHA256` and
//! `TLS_AES_256_GCM_SHA384` (RFC 8446 §5.2).
//!
//! The cipher is the bitsliced AES of this crate and the authenticator is a constant-time
//! GHASH (ADR 0133 rule 14). Open verifies the tag over the whole ciphertext in constant time
//! before it decrypts a byte, and leaves the buffer untouched when the tag is wrong. Only the
//! 96-bit nonce is offered: other nonce lengths need a GHASH of the nonce and TLS never uses
//! them. A nonce must never be used twice with one key; that is the caller's duty (in TLS the
//! record sequence number).
//!
//! Cost: `O(n)` for `n` bytes of data and associated data.

use crate::aes::Aes;
use crate::ct;
use crate::ghash;
use zeroize::Zeroize;

/// The nonce length in bytes.
pub const NONCE_LEN: usize = 12;
/// The tag length in bytes.
pub const TAG_LEN: usize = 16;
/// The longest message one key and nonce may protect: `2^36 - 32` bytes (SP 800-38D §5.2.1.1).
pub const MAX_MESSAGE_LEN: u64 = (1 << 36) - 32;

/// Authentication failed, or the message is longer than [`MAX_MESSAGE_LEN`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Error;

/// An AES-GCM key (128 or 256 bits) with its expanded schedule and hash subkey. Wiped on drop.
pub struct AesGcm {
    aes: Aes,
    ghash: ghash::Key,
}

fn counter_block(nonce: &[u8; NONCE_LEN], counter: u32) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[..12].copy_from_slice(nonce);
    b[12..].copy_from_slice(&counter.to_be_bytes());
    b
}

fn fits(len: usize) -> bool {
    (len as u64) <= MAX_MESSAGE_LEN
}

impl AesGcm {
    fn from_key<const N: usize>(key: &[u8; N]) -> Self {
        let aes = Aes::new(key);
        let mut zero = [0u8; 64];
        let mut out = aes.encrypt4(&zero);
        let mut h = [0u8; 16];
        h.copy_from_slice(&out[..16]);
        let ghash = ghash::Key::new(&h);
        h.zeroize();
        out.zeroize();
        zero.zeroize();
        Self { aes, ghash }
    }

    /// AES-128-GCM with a 16-byte key.
    #[must_use]
    pub fn new_128(key: &[u8; 16]) -> Self {
        Self::from_key(key)
    }

    /// AES-256-GCM with a 32-byte key.
    #[must_use]
    pub fn new_256(key: &[u8; 32]) -> Self {
        Self::from_key(key)
    }

    /// `E_K(J0)`, the mask of the tag.
    fn tag_mask(&self, nonce: &[u8; NONCE_LEN]) -> [u8; 16] {
        let mut blocks = [0u8; 64];
        blocks[..16].copy_from_slice(&counter_block(nonce, 1));
        let mut out = self.aes.encrypt4(&blocks);
        let mut mask = [0u8; 16];
        mask.copy_from_slice(&out[..16]);
        out.zeroize();
        mask
    }

    /// XORs the counter-mode key stream (counters from 2) into `data`. The counter is public
    /// and does not wrap: `fits` bounds the number of blocks below `2^32 - 2`.
    fn ctr_xor(&self, nonce: &[u8; NONCE_LEN], data: &mut [u8]) {
        let mut counter = 2u32;
        for chunk in data.chunks_mut(64) {
            let mut blocks = [0u8; 64];
            for (i, block) in blocks.as_chunks_mut::<16>().0.iter_mut().enumerate() {
                block.copy_from_slice(&counter_block(nonce, counter.wrapping_add(i as u32)));
            }
            let mut stream = self.aes.encrypt4(&blocks);
            for (d, k) in chunk.iter_mut().zip(stream.iter()) {
                *d ^= k;
            }
            stream.zeroize();
            counter = counter.wrapping_add(4);
        }
    }

    fn tag(&self, nonce: &[u8; NONCE_LEN], aad: &[u8], ciphertext: &[u8]) -> [u8; TAG_LEN] {
        let mut s = self.ghash.ghash(aad, ciphertext);
        let mut mask = self.tag_mask(nonce);
        for (a, b) in s.iter_mut().zip(mask.iter()) {
            *a ^= b;
        }
        mask.zeroize();
        s
    }

    /// Encrypts `data` in place and returns the tag.
    ///
    /// # Errors
    /// [`Error`] when `data` is longer than [`MAX_MESSAGE_LEN`] (nothing is changed).
    pub fn seal_in_place(
        &self,
        nonce: &[u8; NONCE_LEN],
        aad: &[u8],
        data: &mut [u8],
    ) -> Result<[u8; TAG_LEN], Error> {
        if !fits(data.len()) {
            return Err(Error);
        }
        self.ctr_xor(nonce, data);
        Ok(self.tag(nonce, aad, data))
    }

    /// Verifies `received_tag` over `aad` and the ciphertext in `data`, then decrypts `data` in
    /// place.
    ///
    /// # Errors
    /// [`Error`] when the tag does not match or `data` is too long; `data` is left unchanged.
    pub fn open_in_place(
        &self,
        nonce: &[u8; NONCE_LEN],
        aad: &[u8],
        data: &mut [u8],
        received_tag: &[u8; TAG_LEN],
    ) -> Result<(), Error> {
        if !fits(data.len()) {
            return Err(Error);
        }
        let expected = self.tag(nonce, aad, data);
        if !ct::eq(&expected, received_tag) {
            return Err(Error);
        }
        self.ctr_xor(nonce, data);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
            .collect()
    }

    /// GCM specification test cases 1 to 4 (key 0 and the "feffe9..." key).
    #[test]
    fn gcm_spec_test_cases() {
        let zero_key: [u8; 16] = [0; 16];
        let zero_nonce = [0u8; 12];
        let g = AesGcm::new_128(&zero_key);
        let mut empty: [u8; 0] = [];
        assert_eq!(
            g.seal_in_place(&zero_nonce, &[], &mut empty)
                .unwrap()
                .to_vec(),
            hex("58e2fccefa7e3061367f1d57a4e7455a")
        );
        let mut one = [0u8; 16];
        let t = g.seal_in_place(&zero_nonce, &[], &mut one).unwrap();
        assert_eq!(one.to_vec(), hex("0388dace60b6a392f328c2b971b2fe78"));
        assert_eq!(t.to_vec(), hex("ab6e47d42cec13bdf53a67b21257bddf"));

        let key: [u8; 16] = hex("feffe9928665731c6d6a8f9467308308").try_into().unwrap();
        let nonce: [u8; 12] = hex("cafebabefacedbaddecaf888").try_into().unwrap();
        let g = AesGcm::new_128(&key);
        let pt = hex(
            "d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a72\
             1c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b391aafd255",
        );
        let mut buf = pt.clone();
        let t = g.seal_in_place(&nonce, &[], &mut buf).unwrap();
        assert_eq!(
            buf,
            hex(
                "42831ec2217774244b7221b784d0d49ce3aa212f2c02a4e035c17e2329aca12e\
                 21d514b25466931c7d8f6a5aac84aa051ba30b396a0aac973d58e091473f5985"
            )
        );
        assert_eq!(t.to_vec(), hex("4d5c2af327cd64a62cf35abd2ba6fab4"));

        // Test case 4: with associated data and a 60-byte message (partial last block).
        let aad = hex("feedfacedeadbeeffeedfacedeadbeefabaddad2");
        let mut buf = pt[..60].to_vec();
        let t = g.seal_in_place(&nonce, &aad, &mut buf).unwrap();
        assert_eq!(t.to_vec(), hex("5bc94fbc3221a5db94fae95ae7121a47"));
        g.open_in_place(&nonce, &aad, &mut buf, &t).unwrap();
        assert_eq!(buf, pt[..60]);
    }

    #[test]
    fn aes256_gcm_spec_test_case_16() {
        let key: [u8; 32] = hex("feffe9928665731c6d6a8f9467308308feffe9928665731c6d6a8f9467308308")
            .try_into()
            .unwrap();
        let nonce: [u8; 12] = hex("cafebabefacedbaddecaf888").try_into().unwrap();
        let aad = hex("feedfacedeadbeeffeedfacedeadbeefabaddad2");
        let mut buf = hex(
            "d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a72\
             1c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b39",
        );
        let t = AesGcm::new_256(&key)
            .seal_in_place(&nonce, &aad, &mut buf)
            .unwrap();
        assert_eq!(t.to_vec(), hex("76fc6ece0f4e1768cddf8853bb2d551b"));
    }

    #[test]
    fn a_wrong_tag_leaves_the_buffer_untouched() {
        let g = AesGcm::new_128(&[1; 16]);
        let nonce = [2u8; 12];
        let mut buf = vec![3u8; 100];
        let mut t = g.seal_in_place(&nonce, b"aad", &mut buf).unwrap();
        t[0] ^= 1;
        let before = buf.clone();
        assert!(g.open_in_place(&nonce, b"aad", &mut buf, &t).is_err());
        assert_eq!(buf, before);
    }
}
