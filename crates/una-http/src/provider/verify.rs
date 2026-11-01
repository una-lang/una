//! The signature verification algorithms `rustls-webpki` needs for certificate chains and
//! `rustls` for `CertificateVerify`, on `una-crypto`'s verifiers.
//!
//! The list and the identifiers are those of `rustls-webpki`'s default provider; the policy
//! behind each is `una-crypto`'s (strict DER, on-curve keys, subgroup-checked Ed25519 keys,
//! RSA 2048 to 8192 bits with `e >= 65537`, PSS salt as long as the digest).

use rustls::SignatureScheme;
use rustls::crypto::WebPkiSupportedAlgorithms;
use rustls::pki_types::{
    AlgorithmIdentifier, InvalidSignature, SignatureVerificationAlgorithm, alg_id,
};
use una_crypto::ecdsa::{self, Curve};
use una_crypto::{ed25519, rsa};

/// What an algorithm verifies with.
#[derive(Debug, Clone, Copy)]
enum Scheme {
    Ecdsa(Curve, ecdsa::HashAlg),
    Ed25519,
    Pkcs1(rsa::HashAlg),
    Pss(rsa::HashAlg),
}

#[derive(Debug)]
struct Algorithm {
    public_key_alg_id: AlgorithmIdentifier,
    signature_alg_id: AlgorithmIdentifier,
    scheme: Scheme,
}

impl SignatureVerificationAlgorithm for Algorithm {
    fn verify_signature(
        &self,
        public_key: &[u8],
        message: &[u8],
        signature: &[u8],
    ) -> Result<(), InvalidSignature> {
        let verdict = match self.scheme {
            Scheme::Ecdsa(curve, hash) => {
                ecdsa::verify_asn1(curve, hash, public_key, message, signature).map_err(|_| ())
            }
            Scheme::Ed25519 => match (public_key.try_into(), signature.try_into()) {
                (Ok(key), Ok(signature)) => {
                    ed25519::verify(key, message, signature).map_err(|_| ())
                }
                _ => Err(()),
            },
            Scheme::Pkcs1(hash) => {
                rsa::verify_pkcs1(hash, public_key, message, signature).map_err(|_| ())
            }
            Scheme::Pss(hash) => {
                rsa::verify_pss(hash, public_key, message, signature).map_err(|_| ())
            }
        };
        verdict.map_err(|()| InvalidSignature)
    }

    fn public_key_alg_id(&self) -> AlgorithmIdentifier {
        self.public_key_alg_id
    }

    fn signature_alg_id(&self) -> AlgorithmIdentifier {
        self.signature_alg_id
    }
}

/// `sha256WithRSAEncryption` (1.2.840.113549.1.1.11) without the `NULL` parameters.
const RSA_SHA256_ABSENT_PARAMS: AlgorithmIdentifier = AlgorithmIdentifier::from_slice(&[
    0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0b,
]);
/// `sha384WithRSAEncryption` (1.2.840.113549.1.1.12) without the `NULL` parameters.
const RSA_SHA384_ABSENT_PARAMS: AlgorithmIdentifier = AlgorithmIdentifier::from_slice(&[
    0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0c,
]);
/// `sha512WithRSAEncryption` (1.2.840.113549.1.1.13) without the `NULL` parameters.
const RSA_SHA512_ABSENT_PARAMS: AlgorithmIdentifier = AlgorithmIdentifier::from_slice(&[
    0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0d,
]);

macro_rules! algorithm {
    ($(#[$doc:meta])* $name:ident, $key:expr, $signature:expr, $scheme:expr) => {
        $(#[$doc])*
        pub static $name: &dyn SignatureVerificationAlgorithm = &Algorithm {
            public_key_alg_id: $key,
            signature_alg_id: $signature,
            scheme: $scheme,
        };
    };
}

algorithm!(
    /// ECDSA with a P-256 key and SHA-256.
    ECDSA_P256_SHA256,
    alg_id::ECDSA_P256,
    alg_id::ECDSA_SHA256,
    Scheme::Ecdsa(Curve::P256, ecdsa::HashAlg::Sha256)
);
algorithm!(
    /// ECDSA with a P-256 key and SHA-384.
    ECDSA_P256_SHA384,
    alg_id::ECDSA_P256,
    alg_id::ECDSA_SHA384,
    Scheme::Ecdsa(Curve::P256, ecdsa::HashAlg::Sha384)
);
algorithm!(
    /// ECDSA with a P-384 key and SHA-256.
    ECDSA_P384_SHA256,
    alg_id::ECDSA_P384,
    alg_id::ECDSA_SHA256,
    Scheme::Ecdsa(Curve::P384, ecdsa::HashAlg::Sha256)
);
algorithm!(
    /// ECDSA with a P-384 key and SHA-384.
    ECDSA_P384_SHA384,
    alg_id::ECDSA_P384,
    alg_id::ECDSA_SHA384,
    Scheme::Ecdsa(Curve::P384, ecdsa::HashAlg::Sha384)
);
algorithm!(
    /// Ed25519 (RFC 8410).
    ED25519,
    alg_id::ED25519,
    alg_id::ED25519,
    Scheme::Ed25519
);
algorithm!(
    /// RSASSA-PKCS1-v1_5 with SHA-256, a 2048 to 8192-bit key.
    RSA_PKCS1_SHA256,
    alg_id::RSA_ENCRYPTION,
    alg_id::RSA_PKCS1_SHA256,
    Scheme::Pkcs1(rsa::HashAlg::Sha256)
);
algorithm!(
    /// RSASSA-PKCS1-v1_5 with SHA-384, a 2048 to 8192-bit key.
    RSA_PKCS1_SHA384,
    alg_id::RSA_ENCRYPTION,
    alg_id::RSA_PKCS1_SHA384,
    Scheme::Pkcs1(rsa::HashAlg::Sha384)
);
algorithm!(
    /// RSASSA-PKCS1-v1_5 with SHA-512, a 2048 to 8192-bit key.
    RSA_PKCS1_SHA512,
    alg_id::RSA_ENCRYPTION,
    alg_id::RSA_PKCS1_SHA512,
    Scheme::Pkcs1(rsa::HashAlg::Sha512)
);
algorithm!(
    /// As [`RSA_PKCS1_SHA256`], for certificates whose algorithm identifier omits the `NULL`.
    RSA_PKCS1_SHA256_ABSENT_PARAMS,
    alg_id::RSA_ENCRYPTION,
    RSA_SHA256_ABSENT_PARAMS,
    Scheme::Pkcs1(rsa::HashAlg::Sha256)
);
algorithm!(
    /// As [`RSA_PKCS1_SHA384`], for certificates whose algorithm identifier omits the `NULL`.
    RSA_PKCS1_SHA384_ABSENT_PARAMS,
    alg_id::RSA_ENCRYPTION,
    RSA_SHA384_ABSENT_PARAMS,
    Scheme::Pkcs1(rsa::HashAlg::Sha384)
);
algorithm!(
    /// As [`RSA_PKCS1_SHA512`], for certificates whose algorithm identifier omits the `NULL`.
    RSA_PKCS1_SHA512_ABSENT_PARAMS,
    alg_id::RSA_ENCRYPTION,
    RSA_SHA512_ABSENT_PARAMS,
    Scheme::Pkcs1(rsa::HashAlg::Sha512)
);
algorithm!(
    /// RSASSA-PSS with SHA-256, MGF1 and a 32-byte salt, a key of type `rsaEncryption`.
    RSA_PSS_SHA256,
    alg_id::RSA_ENCRYPTION,
    alg_id::RSA_PSS_SHA256,
    Scheme::Pss(rsa::HashAlg::Sha256)
);
algorithm!(
    /// RSASSA-PSS with SHA-384, MGF1 and a 48-byte salt, a key of type `rsaEncryption`.
    RSA_PSS_SHA384,
    alg_id::RSA_ENCRYPTION,
    alg_id::RSA_PSS_SHA384,
    Scheme::Pss(rsa::HashAlg::Sha384)
);
algorithm!(
    /// RSASSA-PSS with SHA-512, MGF1 and a 64-byte salt, a key of type `rsaEncryption`.
    RSA_PSS_SHA512,
    alg_id::RSA_ENCRYPTION,
    alg_id::RSA_PSS_SHA512,
    Scheme::Pss(rsa::HashAlg::Sha512)
);

/// The algorithms the provider supports and the mapping from TLS signature schemes.
pub(crate) static SUPPORTED_SIG_ALGS: WebPkiSupportedAlgorithms = WebPkiSupportedAlgorithms {
    all: &[
        ECDSA_P256_SHA256,
        ECDSA_P256_SHA384,
        ECDSA_P384_SHA256,
        ECDSA_P384_SHA384,
        ED25519,
        RSA_PSS_SHA256,
        RSA_PSS_SHA384,
        RSA_PSS_SHA512,
        RSA_PKCS1_SHA256,
        RSA_PKCS1_SHA384,
        RSA_PKCS1_SHA512,
        RSA_PKCS1_SHA256_ABSENT_PARAMS,
        RSA_PKCS1_SHA384_ABSENT_PARAMS,
        RSA_PKCS1_SHA512_ABSENT_PARAMS,
    ],
    mapping: &[
        // For TLS 1.2 the curve is not fixed by the scheme; for TLS 1.3 it is.
        (
            SignatureScheme::ECDSA_NISTP384_SHA384,
            &[ECDSA_P384_SHA384, ECDSA_P256_SHA384],
        ),
        (
            SignatureScheme::ECDSA_NISTP256_SHA256,
            &[ECDSA_P256_SHA256, ECDSA_P384_SHA256],
        ),
        (SignatureScheme::ED25519, &[ED25519]),
        (SignatureScheme::RSA_PSS_SHA512, &[RSA_PSS_SHA512]),
        (SignatureScheme::RSA_PSS_SHA384, &[RSA_PSS_SHA384]),
        (SignatureScheme::RSA_PSS_SHA256, &[RSA_PSS_SHA256]),
        (SignatureScheme::RSA_PKCS1_SHA512, &[RSA_PKCS1_SHA512]),
        (SignatureScheme::RSA_PKCS1_SHA384, &[RSA_PKCS1_SHA384]),
        (SignatureScheme::RSA_PKCS1_SHA256, &[RSA_PKCS1_SHA256]),
    ],
};
