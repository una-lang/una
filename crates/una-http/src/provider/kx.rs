//! X25519 key exchange (RFC 7748, RFC 8446 §7.4.2).

use rustls::crypto::{ActiveKeyExchange, GetRandomFailed, SharedSecret, SupportedKxGroup};
use rustls::{Error, NamedGroup, PeerMisbehaved};
use una_crypto::x25519;
use zeroize::{Zeroize, Zeroizing};

/// The X25519 group.
pub(crate) static X25519: &dyn SupportedKxGroup = &Group;

#[derive(Debug)]
struct Group;

impl SupportedKxGroup for Group {
    fn start(&self) -> Result<Box<dyn ActiveKeyExchange>, Error> {
        let mut private = Zeroizing::new([0u8; x25519::KEY_LEN]);
        getrandom::fill(private.as_mut_slice()).map_err(|_| GetRandomFailed)?;
        let public = x25519::public_key(&private);
        Ok(Box::new(Active { private, public }))
    }

    fn name(&self) -> NamedGroup {
        NamedGroup::X25519
    }
}

/// An exchange in progress: the ephemeral private key (wiped on drop) and its public value.
struct Active {
    private: Zeroizing<[u8; x25519::KEY_LEN]>,
    public: [u8; x25519::KEY_LEN],
}

impl ActiveKeyExchange for Active {
    fn complete(self: Box<Self>, peer_pub_key: &[u8]) -> Result<SharedSecret, Error> {
        let peer: [u8; x25519::KEY_LEN] = peer_pub_key
            .try_into()
            .map_err(|_| PeerMisbehaved::InvalidKeyShare)?;
        let mut secret = x25519::shared_secret(&self.private, &peer)
            .map_err(|_| PeerMisbehaved::InvalidKeyShare)?;
        let shared = SharedSecret::from(secret.as_slice());
        secret.zeroize();
        Ok(shared)
    }

    fn pub_key(&self) -> &[u8] {
        &self.public
    }

    fn group(&self) -> NamedGroup {
        NamedGroup::X25519
    }
}
