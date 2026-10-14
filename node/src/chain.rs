//! Chain state and execution. Consensus (`consensus.rs`) orders blocks; this module executes
//! finalized blocks deterministically, in order, and keeps the mempool, indexes and `blocks.jsonl`.

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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidatorInfo {
    /// `boo1…` account of the validator (same seed as its consensus key).
    pub address: String,
    /// hex ML-DSA-65 public key: consensus votes and p2p identity.
    pub public_key: String,
    /// host:port for authenticated p2p.
    pub p2p_addr: String,
    /// host:port of the validator's HTTP API (used by `join` and the faucet forwarder).
    pub http_addr: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Genesis {
    pub chain_id: String,
    pub token: String,
    pub decimals: u32,
    pub block_time_ms: u64,
    pub validators: Vec<ValidatorInfo>,
    pub faucet: bool,
    pub created_at_ms: u64,
    pub allocations: Vec<Allocation>,
}

impl Genesis {
    pub fn is_validator(&self, addr: &str) -> bool {
        self.validators.iter().any(|v| v.address == addr)
    }

    pub fn initial_state(&self) -> State {
        let mut state = State::default();
        for a in &self.allocations {
            state.credit(&a.address, a.amount);
            state.supply += a.amount;
        }
        state
    }

    pub fn load(path: &Path) -> Result<Self> {
        let bytes = fs::read(path).with_context(|| format!("missing {} — run `blockula summon` first", path.display()))?;
        let v: serde_json::Value = serde_json::from_slice(&bytes)?;
        if v.get("validators").is_none() {
            bail!("{} was created by an older single-validator Blockula; re-summon it (or `blockula dev --reset`)", path.display());
        }
        Ok(serde_json::from_value(v)?)
    }
}

/// Free localhost port, picked by the OS. Used when summoning local testnets.
pub fn free_port() -> Result<u16> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Validator,
    FullNode,
}

pub struct Chain {
    pub home: PathBuf,
    pub genesis: Genesis,
    /// Validator key (also the faucet key on testnets). `None` on full nodes.
    pub keys: Option<Keys>,
    pub role: Role,
    pub state: State,
    pub genesis_root: String,
    /// Hash of the genesis consensus block (parent of block 1).
    pub genesis_hash: String,
    pub blocks: Vec<Block>,
    pub tx_index: HashMap<String, (u64, usize)>,
    pub history: HashMap<String, Vec<String>>,
    pub mempool: Vec<Tx>,
    /// Highest finalized height we know of (may be ahead of `height()` while catching up).
    pub finalized_tip: u64,
    /// Outbound mempool gossip (validators) — every accepted tx is pushed here.
    pub gossip: Option<futures::channel::mpsc::UnboundedSender<Tx>>,
    log: File,
}

/// A finalized block handed to `Chain::execute`, straight from consensus.
pub struct Finalized {
    pub height: u64,
    pub hash: String,
    pub parent: String,
    pub timestamp_ms: u64,
    pub proposer: String,
    pub txs: Vec<Tx>,
    /// hex-encoded finalization certificate, when we saw it.
    pub certificate: String,
}

impl Chain {
    /// Creates a new chain. With one validator, `home` gets genesis.json + validator.key.
    /// With N > 1, `home/v0 … home/v{N-1}` each get the shared genesis and their own key,
    /// with localhost ports picked automatically.
    pub fn summon(home: &Path, chain_id: &str, faucet: bool, block_time_ms: u64, validators: usize, http_port: Option<u16>) -> Result<Vec<PathBuf>> {
        if validators == 0 {
            bail!("need at least one validator");
        }
        let keys: Vec<Keys> = (0..validators).map(|_| Keys::generate()).collect();
        let mut infos = vec![];
        for (i, k) in keys.iter().enumerate() {
            let http = match (i, http_port) {
                (0, Some(p)) => p,
                _ => free_port()?,
            };
            infos.push(ValidatorInfo { address: k.address(), public_key: hex::encode(k.public_key()), p2p_addr: format!("127.0.0.1:{}", free_port()?), http_addr: format!("127.0.0.1:{http}") });
        }
        let per_validator = 1_000_000_000 * DROPS_PER_FANG / validators as u64;
        let mut allocations: Vec<Allocation> = keys.iter().map(|k| Allocation { address: k.address(), amount: per_validator }).collect();
        allocations.push(Allocation { address: treasury(), amount: 100_000_000 * DROPS_PER_FANG });
        let genesis = Genesis { chain_id: chain_id.to_string(), token: "FANG".into(), decimals: DECIMALS, block_time_ms, validators: infos, faucet, created_at_ms: now_ms(), allocations };
        let homes: Vec<PathBuf> = if validators == 1 { vec![home.to_path_buf()] } else { (0..validators).map(|i| home.join(format!("v{i}"))).collect() };
        for (h, k) in homes.iter().zip(&keys) {
            fs::create_dir_all(h)?;
            fs::write(h.join("genesis.json"), serde_json::to_string_pretty(&genesis)?)?;
            fs::write(h.join("validator.key"), k.seed_hex())?;
        }
        if validators > 1 {
            fs::write(home.join("genesis.json"), serde_json::to_string_pretty(&genesis)?)?;
        }
        Ok(homes)
    }

    /// Opens a chain home and replays `blocks.jsonl`, re-executing every block and checking
    /// state roots and hash links. `validator.key` is optional (full nodes don't have one).
    pub fn open(home: &Path) -> Result<Self> {
        let genesis = Genesis::load(&home.join("genesis.json"))?;
        let key_path = home.join("validator.key");
        let keys = if key_path.exists() {
            let seed = hex::decode(fs::read_to_string(&key_path)?.trim())?;
            let keys = Keys::from_seed(seed.try_into().map_err(|_| anyhow!("validator.key must be 32 bytes hex"))?);
            if !genesis.is_validator(&keys.address()) {
                bail!("validator.key is not in this genesis' validator set");
            }
            Some(keys)
        } else {
            None
        };
        let role = if keys.is_some() { Role::Validator } else { Role::FullNode };
        let state = genesis.initial_state();
        let genesis_root = state.root();
        let genesis_hash = hex::encode(crate::consensus::genesis_block(&genesis).digest_bytes());
        let path = home.join("blocks.jsonl");
        let log = OpenOptions::new().create(true).append(true).open(&path)?;
        let mut chain = Chain { home: home.into(), genesis, keys, role, state, genesis_root, genesis_hash, blocks: vec![], tx_index: HashMap::new(), history: HashMap::new(), mempool: vec![], finalized_tip: 0, gossip: None, log };
        for line in BufReader::new(File::open(&path)?).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let block: Block = serde_json::from_str(&line)?;
            chain.replay(block)?;
        }
        chain.finalized_tip = chain.height();
        Ok(chain)
    }

    /// The validator's own address (fee recipient, faucet account).
    pub fn me(&self) -> Option<String> {
        self.keys.as_ref().map(|k| k.address())
    }

    /// State root after executing `height` (0 = genesis).
    pub fn root_at(&self, height: u64) -> Option<&str> {
        if height == 0 {
            return Some(&self.genesis_root);
        }
        self.blocks.get(height as usize - 1).map(|b| b.state_root.as_str())
    }

    pub fn last_hash(&self) -> &str {
        self.blocks.last().map(|b| b.hash.as_str()).unwrap_or(&self.genesis_hash)
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
        if block.height != self.height() + 1 || block.prev_hash != self.last_hash() {
            bail!("blocks.jsonl is not contiguous at height {}", block.height);
        }
        for tx in &block.txs {
            self.state.apply(tx, &self.genesis.chain_id, &block.proposer, block.height).with_context(|| format!("replaying block {}", block.height))?;
        }
        if self.state.root() != block.state_root {
            bail!("state root mismatch at height {}", block.height);
        }
        self.index(&block);
        self.blocks.push(block);
        Ok(())
    }

    /// Executes a finalized block: applies its txs in order, skipping invalid ones (exactly like
    /// the old single-proposer loop), appends it durably to `blocks.jsonl`, then prunes the mempool.
    pub fn execute(&mut self, f: Finalized) -> Result<()> {
        if f.height != self.height() + 1 {
            bail!("expected block {}, got {}", self.height() + 1, f.height);
        }
        if f.parent != self.last_hash() {
            bail!("block {} does not extend our chain", f.height);
        }
        let mut txs = vec![];
        for tx in f.txs {
            if self.state.apply(&tx, &self.genesis.chain_id, &f.proposer, f.height).is_ok() {
                txs.push(tx);
            }
        }
        let block = Block { height: f.height, prev_hash: f.parent, timestamp_ms: f.timestamp_ms, proposer: f.proposer, state_root: self.state.root(), txs, hash: f.hash, signature: f.certificate };
        writeln!(self.log, "{}", serde_json::to_string(&block)?)?;
        self.log.sync_data()?;
        self.index(&block);
        self.blocks.push(block);
        self.finalized_tip = self.finalized_tip.max(f.height);
        self.revalidate_mempool();
        Ok(())
    }

    /// Drops mempool txs that were included or can no longer apply (bad nonce, spent balance…).
    fn revalidate_mempool(&mut self) {
        let mut scratch = self.state.clone();
        let next = self.height() + 1;
        let proposer = self.fee_sink();
        let chain_id = self.genesis.chain_id.clone();
        let tx_index = &self.tx_index;
        self.mempool.retain(|tx| !tx_index.contains_key(&tx.id()) && scratch.apply(tx, &chain_id, &proposer, next).is_ok());
    }

    fn fee_sink(&self) -> String {
        self.me().unwrap_or_else(|| self.genesis.validators[0].address.clone())
    }

    /// Checks a tx against current state plus whatever is already queued, then queues it.
    /// Returns the id and whether it was new (already-known txs are accepted idempotently).
    pub fn submit_tx(&mut self, tx: Tx) -> Result<(String, bool)> {
        let id = tx.id();
        if self.tx_index.contains_key(&id) || self.mempool.iter().any(|t| t.id() == id) {
            return Ok((id, false));
        }
        let mut scratch = self.state.clone();
        let next = self.height() + 1;
        let proposer = self.fee_sink();
        for queued in &self.mempool {
            let _ = scratch.apply(queued, &self.genesis.chain_id, &proposer, next);
        }
        scratch.apply(&tx, &self.genesis.chain_id, &proposer, next)?;
        if self.mempool.len() >= MAX_MEMPOOL {
            bail!("mempool is full, try again in a few seconds");
        }
        if let Some(g) = &self.gossip {
            let _ = g.unbounded_send(tx.clone());
        }
        self.mempool.push(tx);
        Ok((id, true))
    }

    pub fn submit(&mut self, tx: Tx) -> Result<String> {
        self.submit_tx(tx).map(|(id, _)| id)
    }

    /// Adds a tx that another node already accepted (full nodes track what they forwarded).
    pub fn track_pending(&mut self, tx: Tx) {
        let id = tx.id();
        if !self.tx_index.contains_key(&id) && !self.mempool.iter().any(|t| t.id() == id) {
            self.mempool.push(tx);
        }
    }

    /// Next nonce for an address, counting txs already waiting in the mempool.
    pub fn next_nonce(&self, addr: &str) -> u64 {
        self.state.account(addr).nonce + self.mempool.iter().filter(|t| t.sender().ok().as_deref() == Some(addr)).count() as u64
    }

    /// Testnet faucet: the validator's own genesis-funded account pays. Full nodes forward the
    /// request to a validator instead (see api.rs).
    pub fn faucet(&mut self, to: &str, amount: u64) -> Result<Tx> {
        if !self.genesis.faucet {
            bail!("this chain has no faucet");
        }
        let keys = self.keys.as_ref().ok_or_else(|| anyhow!("this node has no faucet key"))?;
        let to = self.state.resolve(to)?;
        let from = keys.address();
        let mut tx = Tx {
            chain_id: self.genesis.chain_id.clone(),
            nonce: self.next_nonce(&from),
            from,
            fee: 0,
            action: Action::Transfer { to, amount },
            memo: "🦇 faucet drip".into(),
            public_key: hex::encode(keys.public_key()),
            signature: "00".repeat(crypto::SIGNATURE_LEN),
        };
        tx.fee = min_fee(tx.size());
        let tx = tx.sign(keys);
        self.submit(tx.clone())?;
        Ok(tx)
    }

    /// Test helper: execute the mempool as the next block (what a lone validator would finalize).
    #[cfg(test)]
    pub fn produce_block(&mut self) -> Result<()> {
        let f = Finalized { height: self.height() + 1, hash: hex::encode(crypto::hash(format!("test-{}", self.height()).as_bytes())), parent: self.last_hash().to_string(), timestamp_ms: now_ms(), proposer: self.fee_sink(), txs: self.mempool.clone(), certificate: String::new() };
        self.execute(f)
    }
}

pub const MAX_MEMPOOL: usize = 5_000;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_and_names_survive_restart() {
        let home = std::env::temp_dir().join(format!("blockula-test-{}", now_ms()));
        Chain::summon(&home, "test-1", true, 1000, 1, None).unwrap();
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

    fn signed(c: &Chain, from: &str, keys: &Keys, nonce: u64, action: Action) -> Tx {
        let mut tx = Tx { chain_id: c.genesis.chain_id.clone(), from: from.into(), nonce, fee: 0, action, memo: String::new(), public_key: hex::encode(keys.public_key()), signature: "00".repeat(crypto::SIGNATURE_LEN) };
        tx.fee = min_fee(tx.size());
        tx.sign(keys)
    }

    #[test]
    fn owner_and_session_keys() {
        let home = std::env::temp_dir().join(format!("blockula-aa-{}", now_ms()));
        Chain::summon(&home, "test-aa", true, 1000, 1, None).unwrap();
        let mut c = Chain::open(&home).unwrap();
        let (owner, laptop, agent) = (Keys::from_seed([1u8; 32]), Keys::from_seed([2u8; 32]), Keys::from_seed([3u8; 32]));
        let acct = owner.address();
        c.faucet(&acct, 100 * DROPS_PER_FANG).unwrap();
        c.produce_block().unwrap();

        // A stranger's key can't act for the account.
        assert!(c.submit(signed(&c, &acct, &agent, 0, Action::Transfer { to: agent.address(), amount: 1 })).unwrap_err().to_string().contains("isn't authorized"));

        // Owner adds a second device and an agent session key: 5 FANG per 10 blocks.
        let session = KeyRole::Session { limit: 5 * DROPS_PER_FANG, period_blocks: 10, expires_at: 1_000 };
        c.submit(signed(&c, &acct, &owner, 0, Action::AddKey { key_id: key_id(&laptop.public_key()), label: "laptop".into(), role: KeyRole::Owner })).unwrap();
        c.submit(signed(&c, &acct, &owner, 1, Action::AddKey { key_id: key_id(&agent.public_key()), label: "claude".into(), role: session })).unwrap();
        c.produce_block().unwrap();
        assert_eq!(c.state.keys_of(&acct).len(), 3);

        // The agent can spend within its limit, from the owner's account…
        c.submit(signed(&c, &acct, &agent, 2, Action::Transfer { to: laptop.address(), amount: 3 * DROPS_PER_FANG })).unwrap();
        c.produce_block().unwrap();
        // …but not past it, and never anything other than transfers.
        let over = c.submit(signed(&c, &acct, &agent, 3, Action::Transfer { to: laptop.address(), amount: 2 * DROPS_PER_FANG }));
        assert!(over.unwrap_err().to_string().contains("limit reached"));
        let reg = c.submit(signed(&c, &acct, &agent, 3, Action::RegisterName { name: "sneaky".into() }));
        assert!(reg.unwrap_err().to_string().contains("only send transfers"));

        // The laptop (owner) revokes the agent; the original key can't be the last owner removed.
        c.submit(signed(&c, &acct, &laptop, 3, Action::RemoveKey { key_id: key_id(&agent.public_key()) })).unwrap();
        c.submit(signed(&c, &acct, &laptop, 4, Action::RemoveKey { key_id: key_id(&owner.public_key()) })).unwrap();
        c.produce_block().unwrap();
        let last = c.submit(signed(&c, &acct, &laptop, 5, Action::RemoveKey { key_id: key_id(&laptop.public_key()) }));
        assert!(last.unwrap_err().to_string().contains("last owner"));
        assert!(c.submit(signed(&c, &acct, &agent, 5, Action::Transfer { to: laptop.address(), amount: 1 })).is_err());

        // Everything replays identically from disk.
        let root = c.state.root();
        drop(c);
        assert_eq!(Chain::open(&home).unwrap().state.root(), root);
        fs::remove_dir_all(home).ok();
    }
}
