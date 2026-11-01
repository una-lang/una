//! ChaCha20-Poly1305 authenticated encryption (RFC 8439 §2.8), the AEAD of the TLS 1.3 suite
//! `TLS_CHACHA20_POLY1305_SHA256` (RFC 8446 §5.2, with the 12-byte nonce of RFC 8446 §5.3).
//!
//! Open verifies the tag over the ciphertext in constant time before it decrypts a byte, and
//! leaves the buffer untouched when the tag is wrong. Cost: `O(n)` for `n` bytes.

// Indices are constants over fixed-size buffers (ADR 0133 rule 6).
#![allow(clippy::indexing_slicing)]

use crate::chacha20;
use crate::ct;
use crate::poly1305::Poly1305;
use zeroize::Zeroize;

/// The key length in bytes.
pub const KEY_LEN: usize = 32;
/// The nonce length in bytes.
pub const NONCE_LEN: usize = 12;
/// The tag length in bytes.
pub const TAG_LEN: usize = 16;
/// The longest message one key and nonce may protect: `2^38 - 64` bytes (RFC 8439 §2.8).
pub const MAX_MESSAGE_LEN: u64 = (1 << 38) - 64;

/// Authentication failed, or the message or the associated data is longer than the limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Error;

fn tag(key: &[u8; KEY_LEN], nonce: &[u8; NONCE_LEN], aad: &[u8], ciphertext: &[u8]) -> [u8; 16] {
    let mut block = [0u8; 64];
    chacha20::block(key, nonce, 0, &mut block);
    let mut one_time = [0u8; 32];
    one_time.copy_from_slice(&block[..32]);
    block.zeroize();
    let mut mac = Poly1305::new(&one_time);
    one_time.zeroize();
    let pad = [0u8; 16];
    mac.update(aad);
    mac.update(&pad[..(16 - aad.len() % 16) % 16]);
    mac.update(ciphertext);
    mac.update(&pad[..(16 - ciphertext.len() % 16) % 16]);
    mac.update(&(aad.len() as u64).to_le_bytes());
    mac.update(&(ciphertext.len() as u64).to_le_bytes());
    mac.finalize()
}

fn fits(len: usize) -> bool {
    (len as u64) <= MAX_MESSAGE_LEN
}

/// Encrypts `data` in place and returns the tag.
///
/// # Errors
/// [`Error`] when `data` is longer than [`MAX_MESSAGE_LEN`] (nothing is changed).
pub fn seal_in_place(
    key: &[u8; KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    data: &mut [u8],
) -> Result<[u8; TAG_LEN], Error> {
    if !fits(data.len()) {
        return Err(Error);
    }
    chacha20::apply(key, nonce, 1, data);
    Ok(tag(key, nonce, aad, data))
}

/// Verifies `received_tag` over `aad` and the ciphertext in `data`, then decrypts `data` in
/// place.
///
/// # Errors
/// [`Error`] when the tag does not match or `data` is too long; `data` is left unchanged.
pub fn open_in_place(
    key: &[u8; KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    data: &mut [u8],
    received_tag: &[u8; TAG_LEN],
) -> Result<(), Error> {
    if !fits(data.len()) {
        return Err(Error);
    }
    let expected = tag(key, nonce, aad, data);
    if !ct::eq(&expected, received_tag) {
        return Err(Error);
    }
    chacha20::apply(key, nonce, 1, data);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc8439_section_2_8_2() {
        let key: [u8; 32] = core::array::from_fn(|i| 0x80 + i as u8);
        let nonce = [7, 0, 0, 0, 0x40, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47];
        let aad = [
            0x50, 0x51, 0x52, 0x53, 0xc0, 0xc1, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7,
        ];
        let mut data = *b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
        let t = seal_in_place(&key, &nonce, &aad, &mut data).unwrap();
        assert_eq!(
            t,
            [
                0x1a, 0xe1, 0x0b, 0x59, 0x4f, 0x09, 0xe2, 0x6a, 0x7e, 0x90, 0x2e, 0xcb, 0xd0, 0x60,
                0x06, 0x91
            ]
        );
        assert_eq!(&data[..4], &[0xd3, 0x1a, 0x8d, 0x34]);
        open_in_place(&key, &nonce, &aad, &mut data, &t).unwrap();
        assert_eq!(&data[..6], b"Ladies");
    }
}
