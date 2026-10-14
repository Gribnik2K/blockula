//! Post-quantum primitives: ML-DSA-65 (FIPS 204) signatures, BLAKE3 hashing,
//! bech32m `boo1…` addresses. Nothing in here relies on elliptic curves.

use anyhow::{anyhow, bail, Result};
use bech32::{Bech32m, Hrp};
use ml_dsa::{
    signature::{Keypair, Signer, Verifier},
    EncodedSignature, EncodedVerifyingKey, MlDsa65, Seed, Signature, SigningKey, VerifyingKey,
};

pub const ADDRESS_HRP: &str = "boo";
pub const PUBLIC_KEY_LEN: usize = 1952;
pub const SIGNATURE_LEN: usize = 3309;

pub fn hash(data: &[u8]) -> [u8; 32] {
    *blake3::hash(data).as_bytes()
}

/// An address is bech32m("boo", BLAKE3(public_key)). A full 256-bit hash keeps
/// collision resistance at ~128 bits even against quantum (BHT) attacks.
pub fn address_from_public_key(public_key: &[u8]) -> String {
    address_from_bytes(&hash(public_key))
}

pub fn address_from_bytes(bytes: &[u8; 32]) -> String {
    bech32::encode::<Bech32m>(Hrp::parse(ADDRESS_HRP).unwrap(), bytes).unwrap()
}

/// Well-known protocol accounts that no one holds a key for.
pub fn system_address(label: &str) -> String {
    address_from_bytes(&hash(format!("blockula/system/{label}").as_bytes()))
}

pub fn address_bytes(s: &str) -> Option<[u8; 32]> {
    if !is_address(s) {
        return None;
    }
    match bech32::decode(s) {
        Ok((hrp, data)) if hrp.as_str() == ADDRESS_HRP => data.try_into().ok(),
        _ => None,
    }
}

/// Canonical (lowercase) `boo1…` address. Bech32 also accepts all-uppercase strings, which
/// would alias accounts, so they're rejected.
pub fn is_address(s: &str) -> bool {
    s.len() <= 90 && !s.bytes().any(|b| b.is_ascii_uppercase()) && matches!(bech32::decode(s), Ok((hrp, data)) if hrp.as_str() == ADDRESS_HRP && data.len() == 32)
}

/// Lowercase hex only (no uppercase aliases of the same bytes).
pub fn is_lower_hex(s: &str) -> bool {
    s.len() % 2 == 0 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub struct Keys {
    signing: SigningKey<MlDsa65>,
    seed: [u8; 32],
}

impl Keys {
    pub fn from_seed(seed: [u8; 32]) -> Self {
        let signing = SigningKey::<MlDsa65>::from_seed(&Seed::from(seed));
        Self { signing, seed }
    }

    pub fn generate() -> Self {
        let mut seed = [0u8; 32];
        getrandom::fill(&mut seed).expect("os randomness");
        Self::from_seed(seed)
    }

    /// A second handle on the same key (re-derived from the seed).
    #[cfg(test)]
    pub fn clone_keys(&self) -> Self {
        Self::from_seed(self.seed)
    }

    pub fn seed(&self) -> [u8; 32] {
        self.seed
    }

    pub fn seed_hex(&self) -> String {
        hex::encode(self.seed)
    }

    pub fn public_key(&self) -> Vec<u8> {
        self.signing.verifying_key().encode().to_vec()
    }

    pub fn address(&self) -> String {
        address_from_public_key(&self.public_key())
    }

    pub fn sign(&self, msg: &[u8]) -> Vec<u8> {
        self.signing.sign(msg).encode().to_vec()
    }
}

pub fn verify(public_key: &[u8], msg: &[u8], signature: &[u8]) -> Result<()> {
    if public_key.len() != PUBLIC_KEY_LEN {
        bail!("public key must be {PUBLIC_KEY_LEN} bytes (ML-DSA-65)");
    }
    if signature.len() != SIGNATURE_LEN {
        bail!("signature must be {SIGNATURE_LEN} bytes (ML-DSA-65)");
    }
    let vk = VerifyingKey::<MlDsa65>::decode(
        &EncodedVerifyingKey::<MlDsa65>::try_from(public_key).map_err(|_| anyhow!("bad public key"))?,
    );
    let sig = Signature::<MlDsa65>::decode(
        &EncodedSignature::<MlDsa65>::try_from(signature).map_err(|_| anyhow!("bad signature"))?,
    )
    .ok_or_else(|| anyhow!("malformed signature"))?;
    vk.verify(msg, &sig).map_err(|_| anyhow!("invalid signature"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_verify_round_trip() {
        let k = Keys::from_seed([7u8; 32]);
        let sig = k.sign(b"hello crypt");
        verify(&k.public_key(), b"hello crypt", &sig).unwrap();
        assert!(verify(&k.public_key(), b"tampered", &sig).is_err());
        assert!(k.address().starts_with("boo1"));
        assert!(is_address(&k.address()));
    }
}
