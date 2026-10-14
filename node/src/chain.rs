//! Single-proposer chain for Phase 0 (`blockula dev` / `blockula summon`).
//! Consensus is deliberately simple here; multi-validator BFT comes in Phase 1.

use crate::crypto::{self, Keys};
use crate::types::*;
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn treasury() -> String {
    crypto::system_address("treasury")
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Allocation {
    pub address: String,
    #[serde(with = "u64_str")]
    pub amount: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Genesis {
    pub chain_id: String,
    pub token: String,
    pub decimals: u32,
    pub block_time_ms: u64,
    pub validator: String,
    pub faucet: bool,
    pub created_at_ms: u64,
    pub allocations: Vec<Allocation>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Account {
    #[serde(with = "u64_str")]
    pub balance: u64,
    pub nonce: u64,
    /// Authorized keys by key id. Empty means "only the key the address was derived from".
    pub keys: BTreeMap<String, KeyRecord>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KeyRecord {
    pub label: String,
    pub role: KeyRole,
    pub added_at: u64,
    /// Session keys: drops spent in the current window, and when that window started.
    #[serde(with = "u64_str")]
    pub spent: u64,
    pub window_start: u64,
}

pub const MAX_KEYS: usize = 16;

#[derive(Clone, Debug, Serialize)]
pub struct NameRecord {
    pub name: String,
    pub owner: String,
    pub registered_at: u64,
}

#[derive(Clone, Default)]
pub struct State {
    pub accounts: BTreeMap<String, Account>,
    pub names: BTreeMap<String, NameRecord>,
    pub supply: u64,
    pub burned: u64,
}

impl State {
    pub fn account(&self, addr: &str) -> Account {
        self.accounts.get(addr).cloned().unwrap_or_default()
    }

    pub fn resolve(&self, to: &str) -> Result<String> {
        if crypto::is_address(to) {
            return Ok(to.to_string());
        }
        let n = normalize_name(to)?;
        self.names.get(&n).map(|r| r.owner.clone()).ok_or_else(|| anyhow!("no one has claimed {n}{NAME_SUFFIX} yet"))
    }

    /// First name an address registered — shown everywhere instead of the raw address.
    pub fn primary_name(&self, addr: &str) -> Option<String> {
        self.names.values().filter(|r| r.owner == addr).min_by_key(|r| r.registered_at).map(|r| format!("{}{NAME_SUFFIX}", r.name))
    }

    /// Keys on an account, including the implicit original key for accounts that never added one.
    pub fn keys_of(&self, addr: &str) -> BTreeMap<String, KeyRecord> {
        let acct = self.account(addr);
        if !acct.keys.is_empty() {
            return acct.keys;
        }
        let original = crypto::address_bytes(addr).map(hex::encode).unwrap_or_default();
        BTreeMap::from([(original, KeyRecord { label: "original key".into(), role: KeyRole::Owner, added_at: 0, spent: 0, window_start: 0 })])
    }

    fn debit(&mut self, addr: &str, amount: u64) -> Result<()> {
        let acct = self.accounts.entry(addr.to_string()).or_default();
        if acct.balance < amount {
            bail!("insufficient balance: has {}, needs {}", fmt_fang(acct.balance), fmt_fang(amount));
        }
        acct.balance -= amount;
        Ok(())
    }

    fn credit(&mut self, addr: &str, amount: u64) {
        self.accounts.entry(addr.to_string()).or_default().balance += amount;
    }

    /// Validates and applies a tx. On error the state is left untouched.
    pub fn apply(&mut self, tx: &Tx, chain_id: &str, proposer: &str, height: u64) -> Result<()> {
        if tx.chain_id != chain_id {
            bail!("wrong chain_id: expected {chain_id}");
        }
        tx.verify_signature()?;
        let sender = tx.sender()?;
        let signer = tx.signer_key_id()?;
        let keys = self.keys_of(&sender);
        let key = keys.get(&signer).ok_or_else(|| anyhow!("this key isn't authorized on {}", short(&sender)))?;
        let min = min_fee(tx.size());
        if tx.fee < min {
            bail!("fee too low: minimum is {} drops", min);
        }
        let acct = self.account(&sender);
        if tx.nonce != acct.nonce {
            bail!("bad nonce: expected {}, got {}", acct.nonce, tx.nonce);
        }
        let mut next = self.clone();
        if let KeyRole::Session { limit, period_blocks, expires_at } = key.role {
            if height > expires_at {
                bail!("session key expired at block {expires_at}");
            }
            let Action::Transfer { amount, .. } = &tx.action else { bail!("session keys can only send transfers") };
            let mut rec = key.clone();
            if height >= rec.window_start.saturating_add(period_blocks) {
                rec.spent = 0;
                rec.window_start = height;
            }
            rec.spent = rec.spent.saturating_add(*amount).saturating_add(tx.fee);
            if rec.spent > limit {
                bail!("session key limit reached: {} per {period_blocks} blocks", fmt_fang(limit));
            }
            let acct = next.accounts.entry(sender.clone()).or_default();
            acct.keys = keys.clone();
            acct.keys.insert(signer.clone(), rec);
        }
        next.debit(&sender, tx.fee)?;
        next.burned += min;
        next.supply -= min;
        next.credit(proposer, tx.fee - min);
        match &tx.action {
            Action::Transfer { to, amount } => {
                let to = next.resolve(to)?;
                next.debit(&sender, *amount)?;
                next.credit(&to, *amount);
            }
            Action::RegisterName { name } => {
                let n = normalize_name(name)?;
                if next.names.contains_key(&n) {
                    bail!("{n}{NAME_SUFFIX} is already taken");
                }
                next.debit(&sender, name_price(&n))?;
                next.credit(&treasury(), name_price(&n));
                next.names.insert(n.clone(), NameRecord { name: n, owner: sender.clone(), registered_at: height });
            }
            Action::AddKey { key_id, label, role } => {
                if key.role != KeyRole::Owner {
                    bail!("only owner keys can add keys");
                }
                if key_id.len() != 64 || hex::decode(key_id).is_err() {
                    bail!("key_id must be 32 bytes hex (BLAKE3 of the public key)");
                }
                if label.len() > 64 {
                    bail!("key label too long (max 64 bytes)");
                }
                let mut keys = keys.clone();
                if keys.contains_key(key_id) {
                    bail!("that key is already on this account");
                }
                if keys.len() >= MAX_KEYS {
                    bail!("accounts can hold at most {MAX_KEYS} keys");
                }
                keys.insert(key_id.clone(), KeyRecord { label: label.clone(), role: role.clone(), added_at: height, spent: 0, window_start: height });
                next.accounts.entry(sender.clone()).or_default().keys = keys;
            }
            Action::RemoveKey { key_id } => {
                if key.role != KeyRole::Owner {
                    bail!("only owner keys can remove keys");
                }
                let mut keys = keys.clone();
                let removed = keys.remove(key_id).ok_or_else(|| anyhow!("no such key on this account"))?;
                if removed.role == KeyRole::Owner && !keys.values().any(|k| k.role == KeyRole::Owner) {
                    bail!("can't remove the last owner key — you'd lock yourself out");
                }
                next.accounts.entry(sender.clone()).or_default().keys = keys;
            }
        }
        next.accounts.get_mut(&sender).unwrap().nonce += 1;
        *self = next;
        Ok(())
    }

    pub fn root(&self) -> String {
        let mut h = blake3::Hasher::new();
        for (a, acct) in &self.accounts {
            h.update(a.as_bytes());
            h.update(&acct.balance.to_le_bytes());
            h.update(&acct.nonce.to_le_bytes());
            for (id, k) in &acct.keys {
                h.update(id.as_bytes());
                h.update(serde_json::to_string(k).unwrap().as_bytes());
            }
        }
        for (n, r) in &self.names {
            h.update(n.as_bytes());
            h.update(r.owner.as_bytes());
        }
        h.finalize().to_hex().to_string()
    }

    pub fn describe(&self, tx: &Tx) -> String {
        let who = |a: &str| self.primary_name(a).unwrap_or_else(|| short(a));
        let from = tx.sender().map(|s| who(&s)).unwrap_or_default();
        match &tx.action {
            Action::Transfer { to, amount } => {
                let to = if crypto::is_address(to) { who(to) } else { format!("{}{NAME_SUFFIX}", normalize_name(to).unwrap_or(to.clone())) };
                format!("{from} sent {} to {to}", fmt_fang(*amount))
            }
            Action::RegisterName { name } => {
                let sender = tx.sender().map(|s| short(&s)).unwrap_or_default();
                format!("{sender} claimed the name {}{NAME_SUFFIX}", normalize_name(name).unwrap_or(name.clone()))
            }
            Action::AddKey { label, role: KeyRole::Owner, .. } => format!("{from} added an owner key “{label}”"),
            Action::AddKey { label, role: KeyRole::Session { limit, period_blocks, .. }, .. } => {
                format!("{from} let “{label}” spend up to {} every {period_blocks} blocks", fmt_fang(*limit))
            }
            Action::RemoveKey { key_id } => format!("{from} revoked key {}…", &key_id[..key_id.len().min(10)]),
        }
    }
}

pub fn short(addr: &str) -> String {
    if addr.len() > 16 { format!("{}…{}", &addr[..10], &addr[addr.len() - 6..]) } else { addr.to_string() }
}

pub struct Chain {
    pub home: PathBuf,
    pub genesis: Genesis,
    pub keys: Keys,
    pub state: State,
    pub blocks: Vec<Block>,
    pub tx_index: HashMap<String, (u64, usize)>,
    pub history: HashMap<String, Vec<String>>,
    pub mempool: Vec<Tx>,
    log: File,
}

impl Chain {
    /// Creates a new chain home: genesis.json + validator.key. Used by `dev` and `summon`.
    pub fn summon(home: &Path, chain_id: &str, faucet: bool, block_time_ms: u64) -> Result<()> {
        fs::create_dir_all(home)?;
        let keys = Keys::generate();
        let genesis = Genesis {
            chain_id: chain_id.to_string(),
            token: "FANG".into(),
            decimals: DECIMALS,
            block_time_ms,
            validator: keys.address(),
            faucet,
            created_at_ms: now_ms(),
            allocations: vec![
                Allocation { address: keys.address(), amount: 1_000_000_000 * DROPS_PER_FANG },
                Allocation { address: treasury(), amount: 100_000_000 * DROPS_PER_FANG },
            ],
        };
        fs::write(home.join("genesis.json"), serde_json::to_string_pretty(&genesis)?)?;
        fs::write(home.join("validator.key"), keys.seed_hex())?;
        Ok(())
    }

    pub fn open(home: &Path) -> Result<Self> {
        let genesis: Genesis = serde_json::from_slice(&fs::read(home.join("genesis.json")).context("missing genesis.json — run `blockula summon` first")?)?;
        let seed = hex::decode(fs::read_to_string(home.join("validator.key"))?.trim())?;
        let keys = Keys::from_seed(seed.try_into().map_err(|_| anyhow!("validator.key must be 32 bytes hex"))?);
        if keys.address() != genesis.validator {
            bail!("validator.key does not match genesis validator");
        }
        let mut state = State::default();
        for a in &genesis.allocations {
            state.credit(&a.address, a.amount);
            state.supply += a.amount;
        }
        let path = home.join("blocks.jsonl");
        let log = OpenOptions::new().create(true).append(true).open(&path)?;
        let mut chain = Chain { home: home.into(), genesis, keys, state, blocks: vec![], tx_index: HashMap::new(), history: HashMap::new(), mempool: vec![], log };
        for line in BufReader::new(File::open(&path)?).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let block: Block = serde_json::from_str(&line)?;
            chain.replay(block)?;
        }
        Ok(chain)
    }

    pub fn height(&self) -> u64 {
        self.blocks.last().map(|b| b.height).unwrap_or(0)
    }

    fn index(&mut self, block: &Block) {
        for (i, tx) in block.txs.iter().enumerate() {
            let id = tx.id();
            self.tx_index.insert(id.clone(), (block.height, i));
            let sender = tx.sender().ok();
            if let Some(s) = &sender {
                self.history.entry(s.clone()).or_default().push(id.clone());
            }
            if let Action::Transfer { to, .. } = &tx.action {
                if let Ok(to) = self.state.resolve(to) {
                    if sender.as_ref() != Some(&to) {
                        self.history.entry(to).or_default().push(id.clone());
                    }
                }
            }
        }
    }

    fn replay(&mut self, block: Block) -> Result<()> {
        let pk = self.keys.public_key();
        crypto::verify(&pk, &block.header_bytes(), &hex::decode(&block.signature)?).context("block signature")?;
        for tx in &block.txs {
            self.state.apply(tx, &self.genesis.chain_id, &block.proposer, block.height)?;
        }
        if self.state.root() != block.state_root {
            bail!("state root mismatch at height {}", block.height);
        }
        self.index(&block);
        self.blocks.push(block);
        Ok(())
    }

    /// Checks a tx against current state plus whatever is already queued, then queues it.
    pub fn submit(&mut self, tx: Tx) -> Result<String> {
        let mut scratch = self.state.clone();
        let next = self.height() + 1;
        for queued in &self.mempool {
            let _ = scratch.apply(queued, &self.genesis.chain_id, &self.genesis.validator, next);
        }
        scratch.apply(&tx, &self.genesis.chain_id, &self.genesis.validator, next)?;
        let id = tx.id();
        self.mempool.push(tx);
        Ok(id)
    }

    pub fn produce_block(&mut self) -> Result<&Block> {
        let height = self.height() + 1;
        let proposer = self.genesis.validator.clone();
        let mut txs = vec![];
        for tx in std::mem::take(&mut self.mempool) {
            if self.state.apply(&tx, &self.genesis.chain_id, &proposer, height).is_ok() {
                txs.push(tx);
            }
        }
        let mut block = Block {
            height,
            prev_hash: self.blocks.last().map(|b| b.hash.clone()).unwrap_or_else(|| "0".repeat(64)),
            timestamp_ms: now_ms(),
            proposer,
            state_root: self.state.root(),
            txs,
            hash: String::new(),
            signature: String::new(),
        };
        let header = block.header_bytes();
        block.hash = hex::encode(crypto::hash(&header));
        block.signature = hex::encode(self.keys.sign(&header));
        writeln!(self.log, "{}", serde_json::to_string(&block)?)?;
        self.index(&block);
        self.blocks.push(block);
        Ok(self.blocks.last().unwrap())
    }

    /// Next nonce for an address, counting txs already waiting in the mempool.
    pub fn next_nonce(&self, addr: &str) -> u64 {
        self.state.account(addr).nonce + self.mempool.iter().filter(|t| t.sender().ok().as_deref() == Some(addr)).count() as u64
    }

    pub fn faucet(&mut self, to: &str, amount: u64) -> Result<String> {
        if !self.genesis.faucet {
            bail!("this chain has no faucet");
        }
        let to = self.state.resolve(to)?;
        let mut tx = Tx {
            chain_id: self.genesis.chain_id.clone(),
            from: self.genesis.validator.clone(),
            nonce: self.next_nonce(&self.genesis.validator),
            fee: 0,
            action: Action::Transfer { to, amount },
            memo: "🦇 faucet drip".into(),
            public_key: hex::encode(self.keys.public_key()),
            signature: "00".repeat(crypto::SIGNATURE_LEN),
        };
        tx.fee = min_fee(tx.size());
        let tx = tx.sign(&self.keys);
        self.submit(tx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_and_names_survive_restart() {
        let home = std::env::temp_dir().join(format!("blockula-test-{}", now_ms()));
        Chain::summon(&home, "test-1", true, 1000).unwrap();
        let alice = Keys::from_seed([1u8; 32]);
        {
            let mut c = Chain::open(&home).unwrap();
            c.faucet(&alice.address(), 50 * DROPS_PER_FANG).unwrap();
            c.produce_block().unwrap();
            let mut tx = Tx { chain_id: "test-1".into(), from: alice.address(), nonce: 0, fee: 0, action: Action::RegisterName { name: "alice".into() }, memo: String::new(), public_key: String::new(), signature: "00".repeat(crypto::SIGNATURE_LEN) };
            tx.public_key = hex::encode(alice.public_key());
            tx.fee = min_fee(tx.size());
            c.submit(tx.sign(&alice)).unwrap();
            c.produce_block().unwrap();
            assert_eq!(c.state.resolve("alice.boo").unwrap(), alice.address());
        }
        let c = Chain::open(&home).unwrap();
        assert_eq!(c.height(), 2);
        assert_eq!(c.state.primary_name(&alice.address()).unwrap(), "alice.boo");
        fs::remove_dir_all(home).ok();
    }
}
