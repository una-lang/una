//! SHA-256 and SHA-384 as `rustls` hashes, and HMAC over them (the TLS 1.3 key schedule and the
//! TLS 1.2 PRF are built from HMAC by `rustls` itself: `HkdfUsingHmac`, `PrfUsingHmac`).

use core::marker::PhantomData;

use rustls::crypto::hash::{Context, Hash, HashAlgorithm, Output};
use rustls::crypto::hmac::{Hmac, Key, Tag};
use una_crypto::hmac::Hmac as RawHmac;
use una_crypto::sha2::{Hash as RawHash, Sha256, Sha384};
use zeroize::Zeroize;

/// The widest digest of the hashes here (`Output::MAX_LEN` is 64).
const MAX: usize = 64;

/// Ends `hash` into an [`Output`]. The length is the hash's own, so the call cannot fail; the
/// empty output of the impossible failure would make every handshake MAC mismatch (fail safe).
fn finish<H: RawHash>(hash: H) -> Output {
    const { assert!(H::OUTPUT_LEN <= MAX) };
    let mut buf = [0u8; MAX];
    let Some(out) = buf.get_mut(..H::OUTPUT_LEN) else {
        return Output::new(&[]);
    };
    if hash.finalize_into(out).is_err() {
        return Output::new(&[]);
    }
    let output = Output::new(out);
    buf.zeroize();
    output
}

/// A `rustls` hash over an `una-crypto` one.
pub(crate) struct Digest<H: RawHash> {
    algorithm: HashAlgorithm,
    hash: PhantomData<fn() -> H>,
}

impl<H: RawHash> Digest<H> {
    const fn new(algorithm: HashAlgorithm) -> Self {
        Self {
            algorithm,
            hash: PhantomData,
        }
    }
}

pub(crate) static SHA256: Digest<Sha256> = Digest::new(HashAlgorithm::SHA256);
pub(crate) static SHA384: Digest<Sha384> = Digest::new(HashAlgorithm::SHA384);

impl<H: RawHash + Send + Sync + 'static> Hash for Digest<H> {
    fn start(&self) -> Box<dyn Context> {
        Box::new(Running(H::new()))
    }

    fn hash(&self, bytes: &[u8]) -> Output {
        let mut hash = H::new();
        hash.update(bytes);
        finish(hash)
    }

    fn output_len(&self) -> usize {
        H::OUTPUT_LEN
    }

    fn algorithm(&self) -> HashAlgorithm {
        self.algorithm
    }
}

/// A running hash (the handshake transcript).
struct Running<H: RawHash>(H);

impl<H: RawHash + Send + Sync + 'static> Context for Running<H> {
    fn fork_finish(&self) -> Output {
        finish(self.0.clone())
    }

    fn fork(&self) -> Box<dyn Context> {
        Box::new(Running(self.0.clone()))
    }

    fn finish(self: Box<Self>) -> Output {
        finish(self.0.clone())
    }

    fn update(&mut self, data: &[u8]) {
        self.0.update(data);
    }
}

/// A `rustls` HMAC over an `una-crypto` hash.
pub(crate) struct HmacAlg<H: RawHash>(PhantomData<fn() -> H>);

pub(crate) static HMAC_SHA256: HmacAlg<Sha256> = HmacAlg(PhantomData);
pub(crate) static HMAC_SHA384: HmacAlg<Sha384> = HmacAlg(PhantomData);

impl<H: RawHash + Send + Sync + 'static> Hmac for HmacAlg<H> {
    fn with_key(&self, key: &[u8]) -> Box<dyn Key> {
        Box::new(Keyed(RawHmac::<H>::new(key)))
    }

    fn hash_output_len(&self) -> usize {
        H::OUTPUT_LEN
    }
}

/// An HMAC key, held as the two padded-key hash states (they wipe themselves on drop).
struct Keyed<H: RawHash>(RawHmac<H>);

impl<H: RawHash + Send + Sync + 'static> Key for Keyed<H> {
    fn sign_concat(&self, first: &[u8], middle: &[&[u8]], last: &[u8]) -> Tag {
        const { assert!(H::OUTPUT_LEN <= MAX) };
        let mut mac = self.0.clone();
        mac.update(first);
        for part in middle {
            mac.update(part);
        }
        mac.update(last);
        let mut buf = [0u8; MAX];
        let Some(out) = buf.get_mut(..H::OUTPUT_LEN) else {
            return Tag::new(&[]);
        };
        if mac.finalize_into(out).is_err() {
            return Tag::new(&[]);
        }
        let tag = Tag::new(out);
        buf.zeroize();
        tag
    }

    fn tag_len(&self) -> usize {
        H::OUTPUT_LEN
    }
}
