//! Multi-validator BFT: Commonware simplex orders blocks, marshal disseminates and backfills them,
//! and finalized blocks are executed in order by `Chain::execute` (deferred execution).
//!
//! Block flow:
//! * **propose** (leader): wait until `parent.timestamp + block_time`, pick mempool txs that apply
//!   on top of our executed state plus any not-yet-executed ancestors, and attach the latest state
//!   root we've executed (`exec_height`, `exec_root`).
//! * **verify** (everyone, before voting): structure, size limits, timestamps, proposer == leader,
//!   stateless tx checks (chain id, ML-DSA signature, minimum fee), and the attached state root if
//!   we've executed that height ourselves.
//! * **finalize**: marshal delivers finalized blocks in order to [`Executor`], which executes them
//!   (invalid txs are skipped) and acknowledges only after `blocks.jsonl` is synced to disk.

use crate::chain::{Chain, Finalized, Genesis};
use crate::pq::{MlDsaScheme, PrivateKey, PublicKey};
use crate::types::{min_fee, Tx};
use bytes::{Buf, BufMut};
use commonware_broadcast::buffered;
use commonware_codec::{EncodeSize, Error as CodecError, Read, Write, Encode};
use commonware_consensus::{
    Application, Block, Heightable, Reporter, Reporters,
    marshal::{self, Update, ancestry::Ancestry, core::Actor as MarshalActor, resolver::p2p as marshal_resolver, standard::Inline},
    simplex::{
        self,
        config::{ForwardPolicy, SkipBudget, SkipPolicy},
        elector::RoundRobin,
        types::{Activity, Context as SimplexContext},
    },
    types::{Epoch, FixedEpocher, Height, ViewDelta},
};
use commonware_cryptography::{Blake3, Digestible, Hasher, Signer as _, blake3::Digest, certificate::ConstantProvider};
use commonware_p2p::{Blocker, Provider, Receiver, Recipients, Sender};
use commonware_parallel::Sequential;
use commonware_runtime::{BufferPooler, Clock, Handle, Metrics, Spawner, Storage, buffer::paged::CacheRef};
use commonware_storage::archive::immutable;
use commonware_utils::{NZU16, NZU64, NZUsize, ordered::Set};
use futures::StreamExt;
use parking_lot::Mutex;
use rand_core::CryptoRng;
use std::{collections::HashMap, num::NonZeroU32, sync::Arc, time::{Duration, SystemTime, UNIX_EPOCH}};
use tracing::{debug, error, warn};

pub type Shared = Arc<Mutex<Chain>>;
pub type MarshalMailbox = marshal::core::Mailbox<MlDsaScheme, marshal::standard::Standard<CBlock>>;

pub const MAX_BLOCK_TXS: usize = 512;
pub const MAX_TX_BYTES: usize = 64 * 1024;
pub const MAX_BLOCK_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_MESSAGE_SIZE: u32 = 8 * 1024 * 1024;
/// How far in the future (by our clock) a block timestamp may be.
pub const MAX_CLOCK_DRIFT_MS: u64 = 5_000;

/// p2p channels.
pub const CH_VOTES: u64 = 0;
pub const CH_CERTS: u64 = 1;
pub const CH_RESOLVER: u64 = 2;
pub const CH_BACKFILL: u64 = 3;
pub const CH_BLOCKS: u64 = 4;
pub const CH_MEMPOOL: u64 = 5;
pub const CHANNELS: [u64; 6] = [CH_VOTES, CH_CERTS, CH_RESOLVER, CH_BACKFILL, CH_BLOCKS, CH_MEMPOOL];

pub fn namespace(chain_id: &str) -> Vec<u8> {
    format!("blockula/{chain_id}").into_bytes()
}

// ------------------------------------------------------------------------------------ block

/// The block consensus agrees on. Txs are carried as the exact JSON bytes the proposer chose,
/// so the digest is over canonical bytes and every node parses the same thing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CBlock {
    pub parent: Digest,
    pub height: u64,
    pub timestamp_ms: u64,
    /// `boo1…` address of the proposer (must be the round's leader). Tips go here.
    pub proposer: String,
    /// Latest height the proposer had executed, and its state root (checked by verifiers that
    /// have executed it too).
    pub exec_height: u64,
    pub exec_root: [u8; 32],
    pub txs: Vec<Vec<u8>>,
    digest: Digest,
}

impl CBlock {
    pub fn new(parent: Digest, height: u64, timestamp_ms: u64, proposer: String, exec_height: u64, exec_root: [u8; 32], txs: Vec<Vec<u8>>) -> Self {
        let mut b = Self { parent, height, timestamp_ms, proposer, exec_height, exec_root, txs, digest: Digest([0; 32]) };
        b.digest = Blake3::hash(&[b"BLOCKULA_CBLOCK_V1", &b.encode_body()]);
        b
    }

    fn encode_body(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.body_size());
        self.write_body(&mut out);
        out
    }

    fn body_size(&self) -> usize {
        32 + 8 + 8 + 4 + self.proposer.len() + 8 + 32 + 4 + self.txs.iter().map(|t| 4 + t.len()).sum::<usize>()
    }

    fn write_body(&self, buf: &mut impl BufMut) {
        buf.put_slice(&self.parent.0);
        buf.put_u64(self.height);
        buf.put_u64(self.timestamp_ms);
        buf.put_u32(self.proposer.len() as u32);
        buf.put_slice(self.proposer.as_bytes());
        buf.put_u64(self.exec_height);
        buf.put_slice(&self.exec_root);
        buf.put_u32(self.txs.len() as u32);
        for t in &self.txs {
            buf.put_u32(t.len() as u32);
            buf.put_slice(t);
        }
    }

    pub fn digest_bytes(&self) -> [u8; 32] {
        self.digest.0
    }

    pub fn parent_bytes(&self) -> [u8; 32] {
        self.parent.0
    }

    pub fn parse_txs(&self) -> Vec<Option<Tx>> {
        self.txs.iter().map(|t| serde_json::from_slice(t).ok()).collect()
    }
}

impl Write for CBlock {
    fn write(&self, buf: &mut impl BufMut) {
        self.write_body(buf);
    }
}
impl EncodeSize for CBlock {
    fn encode_size(&self) -> usize {
        self.body_size()
    }
}
impl Read for CBlock {
    type Cfg = ();
    fn read_cfg(buf: &mut impl Buf, _: &()) -> Result<Self, CodecError> {
        fn need(buf: &impl Buf, n: usize) -> Result<(), CodecError> {
            if buf.remaining() < n { Err(CodecError::EndOfBuffer) } else { Ok(()) }
        }
        need(buf, 32 + 8 + 8 + 4)?;
        let mut parent = [0u8; 32];
        buf.copy_to_slice(&mut parent);
        let height = buf.get_u64();
        let timestamp_ms = buf.get_u64();
        let plen = buf.get_u32() as usize;
        if plen > 128 {
            return Err(CodecError::Invalid("CBlock", "proposer too long"));
        }
        need(buf, plen + 8 + 32 + 4)?;
        let mut p = vec![0u8; plen];
        buf.copy_to_slice(&mut p);
        let proposer = String::from_utf8(p).map_err(|_| CodecError::Invalid("CBlock", "proposer utf8"))?;
        let exec_height = buf.get_u64();
        let mut exec_root = [0u8; 32];
        buf.copy_to_slice(&mut exec_root);
        let n = buf.get_u32() as usize;
        if n > MAX_BLOCK_TXS {
            return Err(CodecError::Invalid("CBlock", "too many txs"));
        }
        let mut txs = Vec::with_capacity(n);
        let mut total = 0;
        for _ in 0..n {
            need(buf, 4)?;
            let len = buf.get_u32() as usize;
            total += len;
            if len > MAX_TX_BYTES || total > MAX_BLOCK_BYTES {
                return Err(CodecError::Invalid("CBlock", "tx too large"));
            }
            need(buf, len)?;
            let mut t = vec![0u8; len];
            buf.copy_to_slice(&mut t);
            txs.push(t);
        }
        Ok(CBlock::new(Digest(parent), height, timestamp_ms, proposer, exec_height, exec_root, txs))
    }
}
impl Digestible for CBlock {
    type Digest = Digest;
    fn digest(&self) -> Digest {
        self.digest
    }
}
impl Heightable for CBlock {
    fn height(&self) -> Height {
        Height::new(self.height)
    }
}
impl Block for CBlock {
    fn parent(&self) -> Digest {
        self.parent
    }
}

/// Height-0 block every node derives from genesis.json.
pub fn genesis_block(g: &Genesis) -> CBlock {
    let seed = Blake3::hash(&[b"BLOCKULA_GENESIS", g.chain_id.as_bytes(), &g.created_at_ms.to_be_bytes(), serde_json::to_string(&g.validators).unwrap().as_bytes()]);
    let root = g.initial_state().root();
    CBlock::new(seed, 0, g.created_at_ms, String::new(), 0, hex_32(&root), vec![])
}

pub fn hex_32(s: &str) -> [u8; 32] {
    hex::decode(s).ok().and_then(|v| v.try_into().ok()).unwrap_or([0; 32])
}

pub fn participants(g: &Genesis) -> anyhow::Result<Set<PublicKey>> {
    let keys = g
        .validators
        .iter()
        .map(|v| hex::decode(&v.public_key).ok().and_then(|b| PublicKey::from_bytes(&b)).ok_or_else(|| anyhow::anyhow!("bad validator public key for {}", v.address)))
        .collect::<anyhow::Result<Vec<_>>>()?;
    Set::try_from(keys).map_err(|_| anyhow::anyhow!("duplicate validator keys in genesis"))
}

fn ms(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

// ------------------------------------------------------------------------------ application

#[derive(Clone)]
pub struct App {
    pub chain: Shared,
    pub block_time_ms: u64,
}

/// Stateless checks every tx in a proposed block must pass.
pub fn check_tx_stateless(tx: &Tx, chain_id: &str) -> anyhow::Result<()> {
    if tx.chain_id != chain_id {
        anyhow::bail!("wrong chain id");
    }
    tx.sender()?;
    if tx.fee < min_fee(tx.size()) {
        anyhow::bail!("fee below minimum");
    }
    tx.verify_signature()
}

impl<E> Application<E> for App
where
    E: rand_core::Rng + Spawner + Metrics + Clock,
{
    type SigningScheme = MlDsaScheme;
    type Context = SimplexContext<Digest, PublicKey>;
    type Block = CBlock;
    type Input = ();

    async fn propose(&mut self, (ctx, cctx): (E, Self::Context), mut ancestry: impl Ancestry<CBlock>, _: ()) -> Option<CBlock> {
        let parent = ancestry.next().await?;
        let target = parent.timestamp_ms + self.block_time_ms;
        let now = ms(ctx.current());
        if target > now {
            ctx.sleep(Duration::from_millis(target - now)).await;
        }
        let timestamp_ms = ms(ctx.current()).max(target);
        let height = parent.height + 1;
        let proposer = cctx.leader.address();

        let (chain_id, exec_height, exec_root, mut scratch, mempool) = {
            let c = self.chain.lock();
            (c.genesis.chain_id.clone(), c.height(), hex_32(&c.state.root()), c.state.clone(), c.mempool.clone())
        };
        // Blocks between what we've executed and the parent are ordered but not yet executed:
        // replay them on a scratch state so we don't re-propose (or overdraw) their txs.
        let mut pending = vec![parent.clone()];
        if parent.height > exec_height {
            while pending.last().unwrap().height > exec_height + 1 {
                if pending.len() > 64 {
                    // Far behind (still catching up): propose an empty block rather than guess.
                    return Some(CBlock::new(parent.digest(), height, timestamp_ms, proposer, exec_height, exec_root, vec![]));
                }
                pending.push(ancestry.next().await?);
            }
        } else {
            pending.clear();
        }
        let mut included = std::collections::HashSet::new();
        for b in pending.iter().rev() {
            for tx in b.parse_txs().into_iter().flatten() {
                included.insert(tx.id());
                let _ = scratch.apply(&tx, &chain_id, &b.proposer, b.height);
            }
        }
        let mut txs = vec![];
        let mut bytes = 0;
        for tx in mempool {
            if txs.len() >= MAX_BLOCK_TXS {
                break;
            }
            if included.contains(&tx.id()) || scratch.apply(&tx, &chain_id, &proposer, height).is_err() {
                continue;
            }
            let raw = serde_json::to_vec(&tx).expect("tx json");
            if raw.len() > MAX_TX_BYTES || bytes + raw.len() > MAX_BLOCK_BYTES {
                continue;
            }
            bytes += raw.len();
            txs.push(raw);
        }
        debug!(height, txs = txs.len(), "proposing");
        Some(CBlock::new(parent.digest(), height, timestamp_ms, proposer, exec_height, exec_root, txs))
    }

    async fn verify(&mut self, (ctx, cctx): (E, Self::Context), mut ancestry: impl Ancestry<CBlock>) -> bool {
        let (Some(block), Some(parent)) = (ancestry.next().await, ancestry.next().await) else { return false };
        let reject = |why: &str| {
            warn!(height = block.height, why, "rejecting block");
            false
        };
        if block.parent != parent.digest() || block.height != parent.height + 1 {
            return reject("bad parent link");
        }
        if block.timestamp_ms < parent.timestamp_ms + self.block_time_ms {
            return reject("block too early");
        }
        if block.timestamp_ms > ms(ctx.current()) + MAX_CLOCK_DRIFT_MS {
            return reject("timestamp too far in the future");
        }
        if block.proposer != cctx.leader.address() {
            return reject("proposer is not the leader");
        }
        if block.exec_height >= block.height {
            return reject("exec_height not below block height");
        }
        let chain_id = {
            let c = self.chain.lock();
            if let Some(root) = c.root_at(block.exec_height) {
                if hex_32(root) != block.exec_root {
                    return reject("state root mismatch");
                }
            }
            c.genesis.chain_id.clone()
        };
        let mut seen = std::collections::HashSet::new();
        for raw in &block.txs {
            let Ok(tx) = serde_json::from_slice::<Tx>(raw) else { return reject("unparseable tx") };
            if !seen.insert(tx.id()) {
                return reject("duplicate tx");
            }
            if let Err(e) = check_tx_stateless(&tx, &chain_id) {
                warn!(error = %e, "bad tx in block");
                return reject("invalid tx");
            }
        }
        true
    }
}

// ------------------------------------------------------------------------------- reporters

/// Remembers finalization certificates by block digest so executed blocks can carry their proof.
#[derive(Clone, Default)]
pub struct CertCapture {
    certs: Arc<Mutex<HashMap<Digest, String>>>,
}

impl Reporter for CertCapture {
    type Activity = Activity<MlDsaScheme, Digest>;
    fn report(&mut self, activity: Self::Activity) -> commonware_actor::Feedback {
        if let Activity::Finalization(f) = activity {
            let mut certs = self.certs.lock();
            if certs.len() > 4096 {
                certs.clear();
            }
            certs.insert(f.proposal.payload, hex::encode(f.encode()));
        }
        commonware_actor::Feedback::Ok
    }
}

/// Receives finalized blocks from marshal, in order, and executes them.
#[derive(Clone)]
pub struct Executor {
    pub chain: Shared,
    pub certs: CertCapture,
}

pub fn finalized_from(b: &CBlock, certificate: String) -> Finalized {
    Finalized {
        height: b.height,
        hash: hex::encode(b.digest.0),
        parent: hex::encode(b.parent.0),
        timestamp_ms: b.timestamp_ms,
        proposer: b.proposer.clone(),
        txs: b.parse_txs().into_iter().flatten().collect(),
        certificate,
    }
}

impl Reporter for Executor {
    type Activity = Update<CBlock>;
    fn report(&mut self, activity: Self::Activity) -> commonware_actor::Feedback {
        use commonware_utils::Acknowledgement as _;
        match activity {
            Update::Tip(_, height, _) => {
                let mut c = self.chain.lock();
                c.finalized_tip = c.finalized_tip.max(height.get());
            }
            Update::Block(block, ack) => {
                let mut c = self.chain.lock();
                if block.height <= c.height() {
                    ack.acknowledge(); // redelivery after restart: already executed
                } else {
                    let cert = self.certs.certs.lock().remove(&block.digest).unwrap_or_default();
                    match c.execute(finalized_from(&block, cert)) {
                        Ok(()) => {
                            debug!(height = block.height, txs = c.blocks.last().map(|b| b.txs.len()).unwrap_or(0), root = %c.state.root(), "executed");
                            ack.acknowledge();
                        }
                        // Not acknowledging makes marshal stop: better than diverging silently.
                        Err(e) => error!(error = %e, height = block.height, "failed to execute finalized block"),
                    }
                }
            }
        }
        commonware_actor::Feedback::Ok
    }
}

// ----------------------------------------------------------------------------------- wiring

pub struct ValidatorConfig {
    pub chain: Shared,
    pub key: PrivateKey,
    pub participants: Set<PublicKey>,
    pub genesis: CBlock,
    pub chain_id: String,
    pub block_time_ms: u64,
    /// Prefix for all on-disk partitions (lets several validators share one runtime in tests).
    pub partition: String,
}

pub type Channel<S, R> = (S, R);

/// Starts marshal, simplex and mempool gossip for one validator over already-registered p2p
/// channels (`CHANNELS` order). Generic over runtime and network so the deterministic simulator
/// and the real tokio + authenticated-discovery stack share this code.
pub async fn start_validator<E, Pv, Bl, S, R>(context: E, cfg: ValidatorConfig, provider: Pv, blocker: Bl, mut channels: Vec<Channel<S, R>>) -> (Handle<()>, MarshalMailbox)
where
    E: BufferPooler + Clock + CryptoRng + Spawner + Storage + Metrics,
    Pv: Provider<PublicKey = PublicKey> + Clone,
    Bl: Blocker<PublicKey = PublicKey> + Clone,
    S: Sender<PublicKey = PublicKey>,
    R: Receiver<PublicKey = PublicKey>,
{
    assert_eq!(channels.len(), CHANNELS.len());
    let mempool_net = channels.pop().unwrap();
    let blocks_net = channels.pop().unwrap();
    let backfill_net = channels.pop().unwrap();
    let resolver_net = channels.pop().unwrap();
    let certs_net = channels.pop().unwrap();
    let votes_net = channels.pop().unwrap();

    let ns = namespace(&cfg.chain_id);
    let me = cfg.key.public_key();
    let scheme = MlDsaScheme::signer(&ns, cfg.participants.clone(), cfg.key.clone()).expect("validator key must be in the validator set");
    let page_cache = CacheRef::from_pooler(&context, NZU16!(4096), NZUsize!(1024));
    let prefix = cfg.partition.clone();
    let chain = cfg.chain.clone();
    let block_time_ms = cfg.block_time_ms;
    let genesis = cfg.genesis.clone();

    let (marshal_tx, marshal_rx) = futures::channel::oneshot::channel();
    let handle = context.child("validator").spawn(move |context| async move {
        let resolver = marshal_resolver::init(
            context.child("marshal_resolver"),
            marshal_resolver::Config {
                public_key: me.clone(),
                peer_provider: provider.clone(),
                blocker: blocker.clone(),
                mailbox_size: NZUsize!(1024),
                timeout: Duration::from_secs(2),
                fetch_retry_timeout: Duration::from_millis(200),
                priority_requests: false,
                priority_responses: false,
            },
            backfill_net,
        );
        let (broadcast_engine, buffer) = buffered::Engine::new(
            context.child("broadcast"),
            buffered::Config { public_key: me.clone(), mailbox_size: NZUsize!(1024), deque_size: 16, priority: false, codec_config: (), peer_provider: provider.clone() },
        );
        broadcast_engine.start(blocks_net);

        let finalizations = immutable::Archive::init(context.child("finalizations"), archive_config(&prefix, "finalizations", page_cache.clone(), u32::MAX as usize)).await.expect("finalizations archive");
        let blocks = immutable::Archive::init(context.child("blocks"), archive_config(&prefix, "blocks", page_cache.clone(), ())).await.expect("blocks archive");
        let (marshal_actor, marshal, floor) = MarshalActor::init(
            context.child("marshal"),
            finalizations,
            blocks,
            marshal::Config {
                provider: ConstantProvider::new(scheme.clone()),
                epocher: FixedEpocher::new(NZU64!(u64::MAX / 2)),
                start: marshal::Start::Genesis(genesis.clone()),
                partition_prefix: format!("{prefix}-marshal"),
                mailbox_size: NZUsize!(1024),
                view_retention: ViewDelta::new(10),
                prunable_items_per_section: NZU64!(64),
                page_cache: page_cache.clone(),
                replay_buffer: NZUsize!(1 << 20),
                key_write_buffer: NZUsize!(1 << 20),
                value_write_buffer: NZUsize!(1 << 20),
                block_codec_config: (),
                max_repair: NZUsize!(64),
                max_pending_acks: NZUsize!(1),
                strategy: Sequential,
            },
        )
        .await;
        if let Some(processed) = floor.height() {
            let ours = chain.lock().height();
            if processed.get() > ours {
                error!(marshal = processed.get(), ours, "consensus storage is ahead of blocks.jsonl; delete the `consensus` dir to resync");
            }
        }
        let _ = marshal_tx.send(marshal.clone());

        let certs = CertCapture::default();
        let executor = Executor { chain: chain.clone(), certs: certs.clone() };
        marshal_actor.start(executor, buffer, resolver);

        let app = Inline::new(context.child("inline"), App { chain: chain.clone(), block_time_ms }, marshal.clone(), FixedEpocher::new(NZU64!(u64::MAX / 2)));
        let bt = Duration::from_millis(block_time_ms);
        let engine = simplex::Engine::new(
            context.child("simplex"),
            simplex::config::Config {
                scheme,
                elector: RoundRobin::<Blake3>::default(),
                blocker: blocker.clone(),
                automaton: app.clone(),
                relay: app,
                reporter: Reporters::from((marshal.clone(), certs)),
                strategy: Sequential,
                partition: format!("{prefix}-simplex"),
                mailbox_size: NZUsize!(1024),
                epoch: Epoch::zero(),
                floor: simplex::config::Floor::Genesis(genesis.digest()),
                replay_buffer: NZUsize!(1 << 20),
                write_buffer: NZUsize!(1 << 20),
                page_cache,
                leader_timeout: bt + Duration::from_secs(2),
                certification_timeout: bt + Duration::from_secs(3),
                timeout_retry: Duration::from_secs(5),
                fetch_timeout: Duration::from_secs(2),
                view_retention: ViewDelta::new(10),
                skip: SkipPolicy::Enabled { timeout: bt * 10 + Duration::from_secs(10), budget: SkipBudget::Participants },
                forward: ForwardPolicy::Disabled,
                track_historical_votes: false,
            },
        );
        engine.start(votes_net, certs_net, resolver_net);

        // Mempool gossip: push every locally accepted tx to all validators; accept theirs.
        let (gossip_tx, mut gossip_rx) = futures::channel::mpsc::unbounded::<Tx>();
        chain.lock().gossip = Some(gossip_tx);
        let (mut sender, mut receiver) = mempool_net;
        context.child("mempool_out").spawn(move |_| async move {
            while let Some(tx) = gossip_rx.next().await {
                let raw = serde_json::to_vec(&tx).expect("tx json");
                let _ = sender.send(Recipients::All, raw, false);
            }
        });
        loop {
            let Ok((from, msg)) = receiver.recv().await else { break };
            let raw: Vec<u8> = msg.as_ref().to_vec();
            if raw.len() > MAX_TX_BYTES {
                continue;
            }
            let Ok(tx) = serde_json::from_slice::<Tx>(&raw) else {
                debug!(?from, "unparseable gossiped tx");
                continue;
            };
            let mut c = chain.lock();
            // Don't echo gossip back out; the originator already sent it to everyone.
            let g = c.gossip.take();
            if let Err(e) = c.submit_tx(tx) {
                debug!(error = %e, "dropping gossiped tx");
            }
            c.gossip = g;
        }
    });
    let mailbox = marshal_rx.await.expect("marshal started");
    (handle, mailbox)
}

fn archive_config<C>(prefix: &str, name: &str, page_cache: CacheRef, codec_config: C) -> immutable::Config<C> {
    immutable::Config {
        metadata_partition: format!("{prefix}-{name}-metadata"),
        freezer_table_partition: format!("{prefix}-{name}-freezer-table"),
        freezer_table_initial_size: 64,
        freezer_table_resize_frequency: 10,
        freezer_table_resize_chunk_size: 10,
        freezer_key_partition: format!("{prefix}-{name}-freezer-key"),
        freezer_key_page_cache: page_cache,
        freezer_value_partition: format!("{prefix}-{name}-freezer-value"),
        freezer_value_target_size: 1 << 20,
        freezer_value_compression: None,
        ordinal_partition: format!("{prefix}-{name}-ordinal"),
        items_per_section: NZU64!(1024),
        codec_config,
        replay_buffer: NZUsize!(1 << 20),
        freezer_key_write_buffer: NZUsize!(1 << 20),
        freezer_value_write_buffer: NZUsize!(1 << 20),
        ordinal_write_buffer: NZUsize!(1 << 20),
    }
}

pub fn quota() -> commonware_runtime::Quota {
    commonware_runtime::Quota::per_second(NonZeroU32::new(10_000).unwrap())
}
