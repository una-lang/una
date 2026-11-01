//! The cipher suites: the hash, the key derivation (HKDF, PRF over HMAC) and the AEAD of each.

use rustls::crypto::CipherSuiteCommon;
use rustls::crypto::KeyExchangeAlgorithm;
use rustls::crypto::tls12::PrfUsingHmac;
use rustls::crypto::tls13::HkdfUsingHmac;
use rustls::{
    CipherSuite, SignatureScheme, SupportedCipherSuite, Tls12CipherSuite, Tls13CipherSuite,
};

use super::aead::{Kind, Tls12, Tls13};
use super::hash::{HMAC_SHA256, HMAC_SHA384, SHA256, SHA384};

/// The confidentiality limit of AES-GCM records under one key (RFC 8446 §5.5 allows `2^24.5` full-size records; the value here is `1 << 24`, below it).
const AES_LIMIT: u64 = 1 << 24;

pub(crate) static TLS13_CHACHA20_POLY1305_SHA256: SupportedCipherSuite =
    SupportedCipherSuite::Tls13(&Tls13CipherSuite {
        common: CipherSuiteCommon {
            suite: CipherSuite::TLS13_CHACHA20_POLY1305_SHA256,
            hash_provider: &SHA256,
            confidentiality_limit: u64::MAX,
        },
        hkdf_provider: &HkdfUsingHmac(&HMAC_SHA256),
        aead_alg: &Tls13(Kind::ChaCha20Poly1305),
        quic: None,
    });

pub(crate) static TLS13_AES_256_GCM_SHA384: SupportedCipherSuite =
    SupportedCipherSuite::Tls13(&Tls13CipherSuite {
        common: CipherSuiteCommon {
            suite: CipherSuite::TLS13_AES_256_GCM_SHA384,
            hash_provider: &SHA384,
            confidentiality_limit: AES_LIMIT,
        },
        hkdf_provider: &HkdfUsingHmac(&HMAC_SHA384),
        aead_alg: &Tls13(Kind::Aes256Gcm),
        quic: None,
    });

pub(crate) static TLS13_AES_128_GCM_SHA256: SupportedCipherSuite =
    SupportedCipherSuite::Tls13(&Tls13CipherSuite {
        common: CipherSuiteCommon {
            suite: CipherSuite::TLS13_AES_128_GCM_SHA256,
            hash_provider: &SHA256,
            confidentiality_limit: AES_LIMIT,
        },
        hkdf_provider: &HkdfUsingHmac(&HMAC_SHA256),
        aead_alg: &Tls13(Kind::Aes128Gcm),
        quic: None,
    });

/// The signature schemes a TLS 1.2 ECDSA suite accepts from the server.
static ECDSA_SCHEMES: &[SignatureScheme] = &[
    SignatureScheme::ED25519,
    SignatureScheme::ECDSA_NISTP384_SHA384,
    SignatureScheme::ECDSA_NISTP256_SHA256,
];

/// The signature schemes a TLS 1.2 RSA suite accepts from the server.
static RSA_SCHEMES: &[SignatureScheme] = &[
    SignatureScheme::RSA_PSS_SHA512,
    SignatureScheme::RSA_PSS_SHA384,
    SignatureScheme::RSA_PSS_SHA256,
    SignatureScheme::RSA_PKCS1_SHA512,
    SignatureScheme::RSA_PKCS1_SHA384,
    SignatureScheme::RSA_PKCS1_SHA256,
];

macro_rules! tls12 {
    ($name:ident, $suite:ident, $hash:expr, $limit:expr, $schemes:expr, $kind:expr, $hmac:expr) => {
        pub(crate) static $name: SupportedCipherSuite =
            SupportedCipherSuite::Tls12(&Tls12CipherSuite {
                common: CipherSuiteCommon {
                    suite: CipherSuite::$suite,
                    hash_provider: &$hash,
                    confidentiality_limit: $limit,
                },
                kx: KeyExchangeAlgorithm::ECDHE,
                sign: $schemes,
                aead_alg: &Tls12($kind),
                prf_provider: &PrfUsingHmac(&$hmac),
            });
    };
}

tls12!(
    TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256,
    TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256,
    SHA256,
    u64::MAX,
    ECDSA_SCHEMES,
    Kind::ChaCha20Poly1305,
    HMAC_SHA256
);
tls12!(
    TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
    TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
    SHA384,
    AES_LIMIT,
    ECDSA_SCHEMES,
    Kind::Aes256Gcm,
    HMAC_SHA384
);
tls12!(
    TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
    TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
    SHA256,
    AES_LIMIT,
    ECDSA_SCHEMES,
    Kind::Aes128Gcm,
    HMAC_SHA256
);
tls12!(
    TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256,
    TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256,
    SHA256,
    u64::MAX,
    RSA_SCHEMES,
    Kind::ChaCha20Poly1305,
    HMAC_SHA256
);
tls12!(
    TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
    TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
    SHA384,
    AES_LIMIT,
    RSA_SCHEMES,
    Kind::Aes256Gcm,
    HMAC_SHA384
);
tls12!(
    TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
    TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
    SHA256,
    AES_LIMIT,
    RSA_SCHEMES,
    Kind::Aes128Gcm,
    HMAC_SHA256
);
