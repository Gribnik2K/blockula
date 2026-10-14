use crate::crypto;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// 1 FANG = 10^9 drops 🩸
pub const DECIMALS: u32 = 9;
pub const DROPS_PER_FANG: u64 = 1_000_000_000;
pub const NAME_SUFFIX: &str = ".boo";

/// u64 amounts travel as JSON strings so JavaScript never loses precision.
pub mod u64_str {
    use serde::{de::Error, Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &u64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum N {
            S(String),
            N(u64),
        }
        match N::deserialize(d)? {
            N::S(s) => s.parse().map_err(D::Error::custom),
            N::N(n) => Ok(n),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    /// `to` is a `boo1…` address or a `.boo` name.
    Transfer {
        to: String,
        #[serde(with = "u64_str")]
        amount: u64,
    },
    RegisterName { name: String },
    /// Authorize another key on this account (owner keys only).
    AddKey { key_id: String, label: String, role: KeyRole },
    /// Revoke a key (owner keys only). The last owner key can't be removed.
    RemoveKey { key_id: String },
    /// Offer to validate: bonds `bond` drops from the sender and creates a *pending* validator
    /// owned by the sender. `consensus_key` is the validator's hex ML-DSA-65 public key.
    RegisterValidator {
        consensus_key: String,
        p2p_addr: String,
        http_addr: String,
        #[serde(with = "u64_str")]
        bond: u64,
        /// Proof of possession: hex ML-DSA-65 signature by `consensus_key` over
        /// `pop_message(chain_id, from, p2p_addr, http_addr)` with context `POP_CONTEXT`.
        /// Stops anyone registering (squatting) a key they don't hold.
        #[serde(default)]
        pop: String,
    },
    /// Season 0 allowlist: an admin approves a pending validator (active from the next epoch).
    ApproveValidator { validator: String },
    /// Admin removes a validator (it leaves the set at the next epoch boundary).
    RemoveValidator { validator: String },
    /// Owner reclaims the bond of a removed validator, one full epoch after it left.
    Unbond { validator: String },
}

/// What a key is allowed to do on an account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum KeyRole {
    /// Full control: another device's passkey, or a cold recovery key.
    Owner,
    /// Agent/app key: transfers only, at most `limit` drops (amount + fee) per `period_blocks`,
    /// and nothing at all after block `expires_at`.
    Session {
        #[serde(with = "u64_str")]
        limit: u64,
        period_blocks: u64,
        expires_at: u64,
    },
}

/// A key's id is BLAKE3(public key) as hex, the same 32 bytes inside a `boo1…` address.
pub fn key_id(public_key: &[u8]) -> String {
    hex::encode(crypto::hash(public_key))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tx {
    pub chain_id: String,
    /// The account acting. The signing key must be authorized on it.
    pub from: String,
    pub nonce: u64,
    #[serde(with = "u64_str")]
    pub fee: u64,
    pub action: Action,
    #[serde(default)]
    pub memo: String,
    /// hex ML-DSA-65 public key (1952 bytes)
    pub public_key: String,
    /// hex ML-DSA-65 signature over `signing_bytes()` (3309 bytes)
    pub signature: String,
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u16).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

impl Tx {
    /// Canonical encoding that gets signed. Mirrored exactly in sdk-ts/src/tx.ts.
    pub fn signing_bytes(&self) -> Vec<u8> {
        let mut out = b"BLOCKULA_TX_V2".to_vec();
        put_str(&mut out, &self.chain_id);
        put_str(&mut out, &self.from);
        out.extend_from_slice(&self.nonce.to_le_bytes());
        out.extend_from_slice(&self.fee.to_le_bytes());
        match &self.action {
            Action::Transfer { to, amount } => {
                out.push(0);
                put_str(&mut out, to);
                out.extend_from_slice(&amount.to_le_bytes());
            }
            Action::RegisterName { name } => {
                out.push(1);
                put_str(&mut out, name);
            }
            Action::AddKey { key_id, label, role } => {
                out.push(2);
                put_str(&mut out, key_id);
                put_str(&mut out, label);
                match role {
                    KeyRole::Owner => out.push(0),
                    KeyRole::Session { limit, period_blocks, expires_at } => {
                        out.push(1);
                        out.extend_from_slice(&limit.to_le_bytes());
                        out.extend_from_slice(&period_blocks.to_le_bytes());
                        out.extend_from_slice(&expires_at.to_le_bytes());
                    }
                }
            }
            Action::RemoveKey { key_id } => {
                out.push(3);
                put_str(&mut out, key_id);
            }
            Action::RegisterValidator { consensus_key, p2p_addr, http_addr, bond, pop } => {
                out.push(4);
                put_str(&mut out, consensus_key);
                put_str(&mut out, p2p_addr);
                put_str(&mut out, http_addr);
                out.extend_from_slice(&bond.to_le_bytes());
                put_str(&mut out, pop);
            }
            Action::ApproveValidator { validator } => {
                out.push(5);
                put_str(&mut out, validator);
            }
            Action::RemoveValidator { validator } => {
                out.push(6);
                put_str(&mut out, validator);
            }
            Action::Unbond { validator } => {
                out.push(7);
                put_str(&mut out, validator);
            }
        }
        put_str(&mut out, &self.memo);
        out
    }

    /// BLAKE3 over the signed bytes and the *decoded* signature, so re-encoding the hex
    /// (e.g. uppercase) can't produce a different id for the same tx.
    pub fn id(&self) -> String {
        let mut b = self.signing_bytes();
        match hex::decode(&self.signature) {
            Ok(sig) => b.extend_from_slice(&sig),
            Err(_) => b.extend_from_slice(self.signature.as_bytes()),
        }
        hex::encode(crypto::hash(&b))
    }

    /// Size/format rules every tx must meet before anything else looks at it: canonical
    /// lowercase hex and addresses, capped string fields, and the serialized size limit.
    pub fn check_format(&self) -> Result<()> {
        let json_len = serde_json::to_vec(self).map(|v| v.len()).unwrap_or(usize::MAX);
        if json_len > MAX_TX_BYTES {
            bail!("transaction too large ({json_len} bytes, max {MAX_TX_BYTES})");
        }
        if self.chain_id.len() > 64 {
            bail!("chain_id too long");
        }
        if !crypto::is_address(&self.from) {
            bail!("`from` must be a lowercase boo1… address");
        }
        if self.memo.len() > MAX_MEMO {
            bail!("memo too long (max {MAX_MEMO} bytes)");
        }
        if !crypto::is_lower_hex(&self.public_key) || self.public_key.len() != crypto::PUBLIC_KEY_LEN * 2 {
            bail!("public_key must be {} bytes of lowercase hex", crypto::PUBLIC_KEY_LEN);
        }
        if !crypto::is_lower_hex(&self.signature) || self.signature.len() != crypto::SIGNATURE_LEN * 2 {
            bail!("signature must be {} bytes of lowercase hex", crypto::SIGNATURE_LEN);
        }
        let key_id_ok = |k: &str| k.len() == 64 && crypto::is_lower_hex(k);
        let addr = |field: &str, v: &str| -> Result<()> {
            if v.len() > MAX_ADDR {
                bail!("{field} too long (max {MAX_ADDR} bytes)");
            }
            crate::chain::check_host_port(v).with_context(|| field.to_string())
        };
        match &self.action {
            Action::Transfer { to, .. } => {
                if to.len() > MAX_NAME_FIELD {
                    bail!("`to` too long (max {MAX_NAME_FIELD} bytes)");
                }
                if !crypto::is_address(to) {
                    normalize_name(to).context("`to` must be a lowercase boo1… address or a .boo name")?;
                }
            }
            Action::RegisterName { name } => {
                if name.len() > MAX_NAME_FIELD {
                    bail!("name too long");
                }
                normalize_name(name)?;
            }
            Action::AddKey { key_id, label, role } => {
                if !key_id_ok(key_id) {
                    bail!("key_id must be 32 bytes of lowercase hex (BLAKE3 of the public key)");
                }
                if label.len() > MAX_LABEL {
                    bail!("key label too long (max {MAX_LABEL} bytes)");
                }
                if let KeyRole::Session { period_blocks, .. } = role {
                    if *period_blocks == 0 {
                        bail!("session period_blocks must be at least 1");
                    }
                }
            }
            Action::RemoveKey { key_id } => {
                if !key_id_ok(key_id) {
                    bail!("key_id must be 32 bytes of lowercase hex");
                }
            }
            Action::RegisterValidator { consensus_key, p2p_addr, http_addr, pop, .. } => {
                if !crypto::is_lower_hex(consensus_key) || consensus_key.len() != crypto::PUBLIC_KEY_LEN * 2 {
                    bail!("consensus_key must be an ML-DSA-65 public key ({} bytes of lowercase hex)", crypto::PUBLIC_KEY_LEN);
                }
                if !crypto::is_lower_hex(pop) || pop.len() != crypto::SIGNATURE_LEN * 2 {
                    bail!("pop must be an ML-DSA-65 signature by the consensus key (see `blockula validator register`)");
                }
                addr("p2p_addr", p2p_addr)?;
                if !(http_addr.starts_with("http://") || http_addr.starts_with("https://")) {
                    addr("http_addr", http_addr)?;
                } else if http_addr.len() > MAX_ADDR {
                    bail!("http_addr too long (max {MAX_ADDR} bytes)");
                }
            }
            Action::ApproveValidator { validator } | Action::RemoveValidator { validator } | Action::Unbond { validator } => {
                if !crypto::is_address(validator) {
                    bail!("validator must be a lowercase boo1… address");
                }
            }
        }
        Ok(())
    }

    pub fn sender(&self) -> Result<String> {
        if !crypto::is_address(&self.from) {
            bail!("`from` must be a boo1… address");
        }
        Ok(self.from.clone())
    }

    pub fn signer_key_id(&self) -> Result<String> {
        Ok(key_id(&hex::decode(&self.public_key).context("public_key hex")?))
    }

    pub fn size(&self) -> u64 {
        (self.signing_bytes().len() + self.public_key.len() / 2 + self.signature.len() / 2) as u64
    }

    pub fn verify_signature(&self) -> Result<()> {
        if self.memo.len() > MAX_MEMO {
            bail!("memo too long (max {MAX_MEMO} bytes)");
        }
        let pk = hex::decode(&self.public_key).context("public_key hex")?;
        let sig = hex::decode(&self.signature).context("signature hex")?;
        crypto::verify(&pk, &self.signing_bytes(), &sig)
    }

    pub fn sign(mut self, keys: &crypto::Keys) -> Self {
        self.public_key = hex::encode(keys.public_key());
        self.signature = hex::encode(keys.sign(&self.signing_bytes()));
        self
    }
}

/// Base fee is priced by bytes because post-quantum signatures are big. It's burned 🔥.
pub fn min_fee(tx_size: u64) -> u64 {
    10_000 + tx_size * 10
}

/// Short names are scarce, so they cost more. Paid to the treasury.
pub fn name_price(name: &str) -> u64 {
    match name.len() {
        3 => 100 * DROPS_PER_FANG,
        4 => 25 * DROPS_PER_FANG,
        _ => 5 * DROPS_PER_FANG,
    }
}

pub const MAX_TX_BYTES: usize = 64 * 1024;
pub const MAX_MEMO: usize = 256;
pub const MAX_LABEL: usize = 64;
pub const MAX_NAME_FIELD: usize = 64;
pub const MAX_ADDR: usize = 100;
/// Transfers that create an account must carry at least this much (makes account spam costly).
pub const MIN_NEW_ACCOUNT: u64 = DROPS_PER_FANG / 100;

/// Domain separation for validator proofs of possession (FIPS 204 context string).
pub const POP_CONTEXT: &[u8] = b"blockula/validator-pop/v1";

/// What a validator's consensus key signs to prove possession when registering.
pub fn pop_message(chain_id: &str, owner: &str, p2p_addr: &str, http_addr: &str) -> Vec<u8> {
    let mut out = b"BLOCKULA_VALIDATOR_POP_V1".to_vec();
    for s in [chain_id, owner, p2p_addr, http_addr] {
        put_str(&mut out, s);
    }
    out
}

/// Accepts "jack" or "jack.boo" (ASCII letters in any case), returns the bare label "jack".
/// Whitespace and non-ASCII are rejected, not trimmed or folded (e.g. the Kelvin sign 'K'
/// lowercases to 'k', which would alias names).
pub fn normalize_name(input: &str) -> Result<String> {
    if !input.is_ascii() {
        bail!("names may only use a-z, 0-9 and '-'");
    }
    if input.len() > MAX_NAME_FIELD || input.trim() != input {
        bail!("names can't contain spaces");
    }
    let n = input.to_ascii_lowercase();
    let n = n.strip_suffix(NAME_SUFFIX).unwrap_or(&n).to_string();
    if !(3..=32).contains(&n.len()) {
        bail!("names must be 3–32 characters");
    }
    if !n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        bail!("names may only use a-z, 0-9 and '-'");
    }
    if n.starts_with('-') || n.ends_with('-') {
        bail!("names can't start or end with '-'");
    }
    Ok(n)
}

pub fn fmt_fang(drops: u64) -> String {
    let whole = drops / DROPS_PER_FANG;
    let frac = drops % DROPS_PER_FANG;
    if frac == 0 {
        format!("{whole} FANG")
    } else {
        let f = format!("{frac:09}");
        format!("{whole}.{} FANG", f.trim_end_matches('0'))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Block {
    pub height: u64,
    pub prev_hash: String,
    pub timestamp_ms: u64,
    pub proposer: String,
    pub state_root: String,
    pub txs: Vec<Tx>,
    pub hash: String,
    /// Kept for compatibility; certificates live in `certs.bin` now (see chain.rs).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub signature: String,
    /// Txs (by index into `txs`) that were included and charged their fee, but whose action
    /// failed at execution (e.g. the balance was spent by an earlier tx in the block).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub failed: std::collections::BTreeMap<usize, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_amounts() {
        assert_eq!(normalize_name("Jack.boo").unwrap(), "jack");
        assert!(normalize_name("ab").is_err());
        assert!(normalize_name("-ab").is_err());
        assert_eq!(fmt_fang(1_500_000_000), "1.5 FANG");
        assert_eq!(fmt_fang(DROPS_PER_FANG), "1 FANG");
    }
}
