//! The record protection of the three AEADs, for TLS 1.3 (RFC 8446 §5.2) and TLS 1.2
//! (RFC 5288 for AES-GCM, RFC 7905 for ChaCha20-Poly1305).
//!
//! Every failure is a generic `rustls` error; an authentication failure says nothing of why.

use rustls::ConnectionTrafficSecrets;
use rustls::Error;
use rustls::crypto::cipher::{
    AeadKey, InboundOpaqueMessage, InboundPlainMessage, Iv, KeyBlockShape, MessageDecrypter,
    MessageEncrypter, NONCE_LEN, Nonce, OutboundOpaqueMessage, OutboundPlainMessage,
    PrefixedPayload, Tls12AeadAlgorithm, Tls13AeadAlgorithm, UnsupportedOperationError,
    make_tls12_aad, make_tls13_aad,
};
use rustls::{ContentType, ProtocolVersion};
use una_crypto::aes_gcm::AesGcm;
use una_crypto::chacha20poly1305;
use zeroize::Zeroizing;

/// The length of an authentication tag, the same for all three.
const TAG_LEN: usize = 16;
/// The largest plaintext fragment of a TLS record (RFC 8446 §5.1, RFC 5246 §6.2.1).
const MAX_FRAGMENT_LEN: usize = 16384;

/// A keyed AEAD. The ChaCha20-Poly1305 key is wiped on drop; `AesGcm` wipes itself.
enum Cipher {
    ChaCha(Zeroizing<[u8; chacha20poly1305::KEY_LEN]>),
    Gcm(Box<AesGcm>),
}

/// Which AEAD.
#[derive(Clone, Copy)]
pub(crate) enum Kind {
    ChaCha20Poly1305,
    Aes128Gcm,
    Aes256Gcm,
}

impl Kind {
    const fn key_len(self) -> usize {
        match self {
            Self::ChaCha20Poly1305 | Self::Aes256Gcm => 32,
            Self::Aes128Gcm => 16,
        }
    }

    /// The keyed AEAD, or `None` when `key` is not of the right length (never, as `rustls`
    /// sizes the key from [`Kind::key_len`]).
    fn keyed(self, key: &AeadKey) -> Option<Cipher> {
        let key = key.as_ref();
        match self {
            Self::ChaCha20Poly1305 => {
                let mut out = Zeroizing::new([0u8; chacha20poly1305::KEY_LEN]);
                if key.len() != out.len() {
                    return None;
                }
                out.copy_from_slice(key);
                Some(Cipher::ChaCha(out))
            }
            Self::Aes128Gcm => {
                let mut array = Zeroizing::new([0u8; 16]);
                if key.len() != array.len() {
                    return None;
                }
                array.copy_from_slice(key);
                Some(Cipher::Gcm(Box::new(AesGcm::new_128(&array))))
            }
            Self::Aes256Gcm => {
                let mut array = Zeroizing::new([0u8; 32]);
                if key.len() != array.len() {
                    return None;
                }
                array.copy_from_slice(key);
                Some(Cipher::Gcm(Box::new(AesGcm::new_256(&array))))
            }
        }
    }
}

impl Cipher {
    /// Encrypts `data` in place and returns the tag.
    fn seal(
        &self,
        nonce: &[u8; NONCE_LEN],
        aad: &[u8],
        data: &mut [u8],
    ) -> Result<[u8; TAG_LEN], Error> {
        match self {
            Self::ChaCha(key) => chacha20poly1305::seal_in_place(key, nonce, aad, data)
                .map_err(|_| Error::EncryptError),
            Self::Gcm(gcm) => gcm
                .seal_in_place(nonce, aad, data)
                .map_err(|_| Error::EncryptError),
        }
    }

    /// Verifies `tag`, then decrypts `data` in place.
    fn open(
        &self,
        nonce: &[u8; NONCE_LEN],
        aad: &[u8],
        data: &mut [u8],
        tag: &[u8; TAG_LEN],
    ) -> Result<(), Error> {
        match self {
            Self::ChaCha(key) => chacha20poly1305::open_in_place(key, nonce, aad, data, tag)
                .map_err(|_| Error::DecryptError),
            Self::Gcm(gcm) => gcm
                .open_in_place(nonce, aad, data, tag)
                .map_err(|_| Error::DecryptError),
        }
    }
}

/// Splits a record body into ciphertext and tag.
fn split_tag(body: &mut [u8]) -> Result<(&mut [u8], [u8; TAG_LEN]), Error> {
    let at = body.len().checked_sub(TAG_LEN).ok_or(Error::DecryptError)?;
    let (data, tag) = body.split_at_mut(at);
    let tag: [u8; TAG_LEN] = (&*tag).try_into().map_err(|_| Error::DecryptError)?;
    Ok((data, tag))
}

// TLS 1.3 ------------------------------------------------------------------------------------

/// The TLS 1.3 AEAD of a cipher suite.
pub(crate) struct Tls13(pub(crate) Kind);

impl Tls13AeadAlgorithm for Tls13 {
    fn encrypter(&self, key: AeadKey, iv: Iv) -> Box<dyn MessageEncrypter> {
        Box::new(Tls13Encrypter {
            cipher: self.0.keyed(&key),
            iv,
        })
    }

    fn decrypter(&self, key: AeadKey, iv: Iv) -> Box<dyn MessageDecrypter> {
        Box::new(Tls13Decrypter {
            cipher: self.0.keyed(&key),
            iv,
        })
    }

    fn key_len(&self) -> usize {
        self.0.key_len()
    }

    fn extract_keys(
        &self,
        key: AeadKey,
        iv: Iv,
    ) -> Result<ConnectionTrafficSecrets, UnsupportedOperationError> {
        Ok(match self.0 {
            Kind::ChaCha20Poly1305 => ConnectionTrafficSecrets::Chacha20Poly1305 { key, iv },
            Kind::Aes128Gcm => ConnectionTrafficSecrets::Aes128Gcm { key, iv },
            Kind::Aes256Gcm => ConnectionTrafficSecrets::Aes256Gcm { key, iv },
        })
    }
}

struct Tls13Encrypter {
    cipher: Option<Cipher>,
    iv: Iv,
}

struct Tls13Decrypter {
    cipher: Option<Cipher>,
    iv: Iv,
}

impl MessageEncrypter for Tls13Encrypter {
    fn encrypt(
        &mut self,
        msg: OutboundPlainMessage<'_>,
        seq: u64,
    ) -> Result<OutboundOpaqueMessage, Error> {
        let cipher = self.cipher.as_ref().ok_or(Error::EncryptError)?;
        let total_len = self.encrypted_payload_len(msg.payload.len());
        let mut payload = PrefixedPayload::with_capacity(total_len);
        let nonce = Nonce::new(&self.iv, seq).0;
        let aad = make_tls13_aad(total_len);
        payload.extend_from_chunks(&msg.payload);
        payload.extend_from_slice(&msg.typ.to_array());
        let tag = cipher.seal(&nonce, &aad, payload.as_mut())?;
        payload.extend_from_slice(&tag);
        Ok(OutboundOpaqueMessage::new(
            ContentType::ApplicationData,
            ProtocolVersion::TLSv1_2,
            payload,
        ))
    }

    fn encrypted_payload_len(&self, payload_len: usize) -> usize {
        payload_len + 1 + TAG_LEN
    }
}

impl MessageDecrypter for Tls13Decrypter {
    fn decrypt<'a>(
        &mut self,
        mut msg: InboundOpaqueMessage<'a>,
        seq: u64,
    ) -> Result<InboundPlainMessage<'a>, Error> {
        let cipher = self.cipher.as_ref().ok_or(Error::DecryptError)?;
        let payload = &mut msg.payload;
        let nonce = Nonce::new(&self.iv, seq).0;
        let aad = make_tls13_aad(payload.len());
        let (data, tag) = split_tag(payload)?;
        cipher.open(&nonce, &aad, data, &tag)?;
        let plain_len = data.len();
        payload.truncate(plain_len);
        msg.into_tls13_unpadded_message()
    }
}

// TLS 1.2 ------------------------------------------------------------------------------------

/// The TLS 1.2 AEAD of a cipher suite.
pub(crate) struct Tls12(pub(crate) Kind);

/// The explicit part of the AES-GCM nonce, sent in every record (RFC 5288 §3).
const GCM_EXPLICIT_NONCE_LEN: usize = 8;
const GCM_OVERHEAD: usize = GCM_EXPLICIT_NONCE_LEN + TAG_LEN;

fn gcm_iv(write_iv: &[u8], explicit: &[u8]) -> Option<Iv> {
    let mut iv = [0u8; NONCE_LEN];
    let (salt, rest) = iv.split_at_mut(4);
    if write_iv.len() != salt.len() || explicit.len() != rest.len() {
        return None;
    }
    salt.copy_from_slice(write_iv);
    rest.copy_from_slice(explicit);
    Some(Iv::new(iv))
}

impl Tls12AeadAlgorithm for Tls12 {
    fn encrypter(&self, key: AeadKey, iv: &[u8], extra: &[u8]) -> Box<dyn MessageEncrypter> {
        let iv = match self.0 {
            Kind::ChaCha20Poly1305 => (iv.len() == NONCE_LEN).then(|| Iv::copy(iv)),
            Kind::Aes128Gcm | Kind::Aes256Gcm => gcm_iv(iv, extra),
        };
        Box::new(Tls12Encrypter {
            cipher: self.0.keyed(&key),
            iv,
            gcm: !matches!(self.0, Kind::ChaCha20Poly1305),
        })
    }

    fn decrypter(&self, key: AeadKey, iv: &[u8]) -> Box<dyn MessageDecrypter> {
        let gcm = !matches!(self.0, Kind::ChaCha20Poly1305);
        let iv = if gcm {
            // The nonce is the 4-byte salt and the 8 explicit bytes of each record.
            gcm_iv(iv, &[0u8; GCM_EXPLICIT_NONCE_LEN])
        } else {
            (iv.len() == NONCE_LEN).then(|| Iv::copy(iv))
        };
        Box::new(Tls12Decrypter {
            cipher: self.0.keyed(&key),
            iv,
            gcm,
        })
    }

    fn key_block_shape(&self) -> KeyBlockShape {
        match self.0 {
            Kind::ChaCha20Poly1305 => KeyBlockShape {
                enc_key_len: 32,
                fixed_iv_len: 12,
                explicit_nonce_len: 0,
            },
            Kind::Aes128Gcm | Kind::Aes256Gcm => KeyBlockShape {
                enc_key_len: self.0.key_len(),
                fixed_iv_len: 4,
                explicit_nonce_len: GCM_EXPLICIT_NONCE_LEN,
            },
        }
    }

    fn extract_keys(
        &self,
        key: AeadKey,
        iv: &[u8],
        explicit: &[u8],
    ) -> Result<ConnectionTrafficSecrets, UnsupportedOperationError> {
        Ok(match self.0 {
            Kind::ChaCha20Poly1305 => {
                let iv: [u8; NONCE_LEN] = iv.try_into().map_err(|_| UnsupportedOperationError)?;
                ConnectionTrafficSecrets::Chacha20Poly1305 {
                    key,
                    iv: Iv::new(iv),
                }
            }
            Kind::Aes128Gcm => ConnectionTrafficSecrets::Aes128Gcm {
                key,
                iv: gcm_iv(iv, explicit).ok_or(UnsupportedOperationError)?,
            },
            Kind::Aes256Gcm => ConnectionTrafficSecrets::Aes256Gcm {
                key,
                iv: gcm_iv(iv, explicit).ok_or(UnsupportedOperationError)?,
            },
        })
    }
}

struct Tls12Encrypter {
    cipher: Option<Cipher>,
    iv: Option<Iv>,
    gcm: bool,
}

struct Tls12Decrypter {
    cipher: Option<Cipher>,
    iv: Option<Iv>,
    gcm: bool,
}

impl MessageEncrypter for Tls12Encrypter {
    fn encrypt(
        &mut self,
        msg: OutboundPlainMessage<'_>,
        seq: u64,
    ) -> Result<OutboundOpaqueMessage, Error> {
        let cipher = self.cipher.as_ref().ok_or(Error::EncryptError)?;
        let iv = self.iv.as_ref().ok_or(Error::EncryptError)?;
        let total_len = self.encrypted_payload_len(msg.payload.len());
        let mut payload = PrefixedPayload::with_capacity(total_len);
        let nonce = Nonce::new(iv, seq).0;
        let aad = make_tls12_aad(seq, msg.typ, msg.version, msg.payload.len());
        if self.gcm {
            // explicit nonce || ciphertext || tag
            payload.extend_from_slice(nonce.get(4..).ok_or(Error::EncryptError)?);
            payload.extend_from_chunks(&msg.payload);
            let body = payload
                .as_mut()
                .get_mut(GCM_EXPLICIT_NONCE_LEN..)
                .ok_or(Error::EncryptError)?;
            let tag = cipher.seal(&nonce, &aad, body)?;
            payload.extend_from_slice(&tag);
        } else {
            payload.extend_from_chunks(&msg.payload);
            let tag = cipher.seal(&nonce, &aad, payload.as_mut())?;
            payload.extend_from_slice(&tag);
        }
        Ok(OutboundOpaqueMessage::new(msg.typ, msg.version, payload))
    }

    fn encrypted_payload_len(&self, payload_len: usize) -> usize {
        if self.gcm {
            payload_len + GCM_OVERHEAD
        } else {
            payload_len + TAG_LEN
        }
    }
}

impl MessageDecrypter for Tls12Decrypter {
    fn decrypt<'a>(
        &mut self,
        mut msg: InboundOpaqueMessage<'a>,
        seq: u64,
    ) -> Result<InboundPlainMessage<'a>, Error> {
        let cipher = self.cipher.as_ref().ok_or(Error::DecryptError)?;
        let iv = self.iv.as_ref().ok_or(Error::DecryptError)?;
        let overhead = if self.gcm { GCM_OVERHEAD } else { TAG_LEN };
        let typ = msg.typ;
        let version = msg.version;
        let payload = &mut msg.payload;
        let plain_len = payload
            .len()
            .checked_sub(overhead)
            .ok_or(Error::DecryptError)?;
        if plain_len > MAX_FRAGMENT_LEN {
            return Err(Error::PeerSentOversizedRecord);
        }
        let aad = make_tls12_aad(seq, typ, version, plain_len);
        if self.gcm {
            // The nonce is the salt and the explicit bytes the record carries, not the counter.
            let mut nonce = Nonce::new(iv, 0).0;
            let explicit = payload
                .get(..GCM_EXPLICIT_NONCE_LEN)
                .ok_or(Error::DecryptError)?;
            nonce
                .get_mut(4..)
                .ok_or(Error::DecryptError)?
                .copy_from_slice(explicit);
            let body = payload
                .get_mut(GCM_EXPLICIT_NONCE_LEN..)
                .ok_or(Error::DecryptError)?;
            let (data, tag) = split_tag(body)?;
            cipher.open(&nonce, &aad, data, &tag)?;
            payload.copy_within(
                GCM_EXPLICIT_NONCE_LEN..GCM_EXPLICIT_NONCE_LEN + plain_len,
                0,
            );
        } else {
            let nonce = Nonce::new(iv, seq).0;
            let (data, tag) = split_tag(payload)?;
            cipher.open(&nonce, &aad, data, &tag)?;
        }
        payload.truncate(plain_len);
        Ok(msg.into_plain_message())
    }
}
