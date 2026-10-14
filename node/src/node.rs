//! Process wiring: validators (tokio runtime + authenticated discovery p2p + consensus) and
//! full nodes (`join`: verified sync over HTTP from validators).

use crate::api::{self, Node};
use crate::chain::{Chain, Genesis};
use crate::consensus::{self, CBlock, Shared, ValidatorConfig, CHANNELS, MAX_MESSAGE_SIZE};
use crate::pq::{MlDsaScheme, PrivateKey};
use anyhow::{anyhow, bail, Context, Result};
use commonware_codec::{Decode, DecodeExt};
use commonware_consensus::simplex::types::Finalization;
use commonware_cryptography::{Digestible, blake3::Digest, certificate::Verifier as _};
use commonware_p2p::{Manager as _, authenticated::discovery};
use commonware_runtime::{Runner as _, Supervisor as _, tokio as cw_tokio};
use commonware_utils::NZUsize;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

async fn serve_http(node: Node, port: u16) {
    let home = node.chain.lock().home.clone();
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("❌ couldn't bind http port {port}: {e}");
            std::process::exit(1);
        }
    };
    println!("🦇 Explorer:  http://localhost:{port}\n🔌 API:       http://localhost:{port}/api/status\n📁 Data:      {}", home.display());
    if let Err(e) = axum::serve(listener, api::router(node)).await {
        eprintln!("http server failed: {e}");
    }
}

fn port_of(addr: &str) -> Option<u16> {
    addr.rsplit(':').next()?.parse().ok()
}

pub fn run_validator(home: PathBuf, port: Option<u16>) -> Result<()> {
    let chain = Chain::open(&home)?;
    let genesis = chain.genesis.clone();
    let seed: [u8; 32] = hex::decode(std::fs::read_to_string(home.join("validator.key"))?.trim())?.try_into().map_err(|_| anyhow!("bad validator.key"))?;
    let key = PrivateKey::from_seed_bytes(seed);
    let me = chain.me().unwrap();
    let info = genesis.validators.iter().find(|v| v.address == me).cloned().ok_or_else(|| anyhow!("not a validator"))?;
    let http_port = port.or_else(|| port_of(&info.http_addr)).unwrap_or(7070);
    let participants = consensus::participants(&genesis)?;
    let solo = genesis.validators.len() == 1;
    let listen: SocketAddr = if solo { "127.0.0.1:0".parse()? } else { info.p2p_addr.parse().with_context(|| format!("bad p2p_addr {}", info.p2p_addr))? };
    let mut bootstrappers = vec![];
    for v in &genesis.validators {
        if v.address == me {
            continue;
        }
        let pk = crate::pq::PublicKey::from_bytes(&hex::decode(&v.public_key)?).ok_or_else(|| anyhow!("bad key"))?;
        let addr: SocketAddr = v.p2p_addr.parse()?;
        bootstrappers.push((pk, addr.into()));
    }
    println!(
        "🧛 Blockula `{}` rising from height {} — validator {} of {} (simplex BFT, ML-DSA-65)",
        genesis.chain_id,
        chain.height(),
        participants.iter().position(|p| p.address() == me).unwrap_or(0) + 1,
        participants.len()
    );
    let shared: Shared = Arc::new(Mutex::new(chain));
    let block_time_ms = genesis.block_time_ms;
    let genesis_block = consensus::genesis_block(&genesis);
    let chain_id = genesis.chain_id.clone();

    let cfg = cw_tokio::Config::new().with_storage_directory(home.join("consensus"));
    cw_tokio::Runner::new(cfg).start(|context| async move {
        let mut p2p_cfg = discovery::Config::local(
            key.clone(),
            &[consensus::namespace(&chain_id).as_slice(), b"/p2p"].concat(),
            listen,
            listen,
            bootstrappers,
            NZUsize!(participants.len().max(2)),
            MAX_MESSAGE_SIZE,
        );
        p2p_cfg.mailbox_size = NZUsize!(4096);
        let (mut network, mut oracle) = discovery::Network::new(context.child("network"), p2p_cfg);
        oracle.track(0, participants.clone());
        let channels: Vec<_> = CHANNELS.iter().map(|c| network.register(*c, consensus::quota())).collect();
        network.start();

        let vcfg = ValidatorConfig { chain: shared.clone(), key, participants, genesis: genesis_block, chain_id, block_time_ms, partition: "blockula".into() };
        let (handle, marshal) = consensus::start_validator(context.child("node"), vcfg, oracle.clone(), oracle.clone(), channels).await;
        tokio::spawn(serve_http(Node { chain: shared, marshal: Some(marshal), peers: vec![] }, http_port));
        if let Err(e) = handle.await {
            eprintln!("consensus stopped: {e:?}");
        }
    });
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct JoinConfig {
    peers: Vec<String>,
}

/// Prepares a full-node home: genesis.json (copied or fetched from a validator) + join.json.
pub fn join(genesis: &str, home: &Path) -> Result<()> {
    std::fs::create_dir_all(home)?;
    if home.join("validator.key").exists() {
        bail!("{} is a validator home", home.display());
    }
    let (g, mut peers): (Genesis, Vec<String>) = if genesis.starts_with("http://") || genesis.starts_with("https://") {
        let url = genesis.trim_end_matches('/').to_string();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
        let g: Genesis = rt.block_on(async { reqwest::get(format!("{url}/api/genesis")).await?.error_for_status()?.json().await })?;
        (g, vec![url])
    } else {
        (Genesis::load(Path::new(genesis))?, vec![])
    };
    for v in &g.validators {
        let url = format!("http://{}", v.http_addr);
        if !peers.contains(&url) {
            peers.push(url);
        }
    }
    let path = home.join("genesis.json");
    if path.exists() {
        let existing = Genesis::load(&path)?;
        if serde_json::to_string(&existing)? != serde_json::to_string(&g)? {
            bail!("{} already holds a different chain", home.display());
        }
    } else {
        std::fs::write(&path, serde_json::to_string_pretty(&g)?)?;
    }
    std::fs::write(home.join("join.json"), serde_json::to_string_pretty(&JoinConfig { peers })?)?;
    Ok(())
}

pub fn run_full_node(home: PathBuf, port: u16) -> Result<()> {
    let jc: JoinConfig = serde_json::from_slice(&std::fs::read(home.join("join.json")).context("not a joined home — use `blockula join`")?)?;
    let chain = Chain::open(&home)?;
    println!("🦇 Full node for `{}` at height {} — syncing from {}", chain.genesis.chain_id, chain.height(), jc.peers.join(", "));
    let shared: Shared = Arc::new(Mutex::new(chain));
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    rt.block_on(async move {
        tokio::spawn(serve_http(Node { chain: shared.clone(), marshal: None, peers: jc.peers.clone() }, port));
        sync_loop(shared, jc.peers).await;
    });
    Ok(())
}

#[derive(Deserialize)]
struct SyncBatch {
    tip: u64,
    blocks: Vec<String>,
    to: Option<u64>,
    finalization: Option<String>,
}

async fn sync_loop(chain: Shared, peers: Vec<String>) {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(20)).build().unwrap();
    let (verifier, block_time) = {
        let c = chain.lock();
        let parts = consensus::participants(&c.genesis).expect("genesis validators");
        (MlDsaScheme::verifier(&consensus::namespace(&c.genesis.chain_id), parts), Duration::from_millis(c.genesis.block_time_ms))
    };
    let mut peer = 0;
    loop {
        let from = chain.lock().height() + 1;
        let url = format!("{}/api/sync?from={from}&limit=200", peers[peer % peers.len()]);
        let result: Result<SyncBatch> = async { Ok(client.get(&url).send().await?.error_for_status()?.json().await?) }.await;
        match result {
            Ok(batch) => {
                {
                    let mut c = chain.lock();
                    c.finalized_tip = c.finalized_tip.max(batch.tip);
                }
                if batch.blocks.is_empty() {
                    tokio::time::sleep(block_time / 2).await;
                    continue;
                }
                if let Err(e) = apply_batch(&chain, &verifier, &batch) {
                    eprintln!("⚠️  rejected sync batch from {}: {e:#}", peers[peer % peers.len()]);
                    peer += 1;
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
            Err(e) => {
                tracing::debug!(error = %e, "sync request failed");
                peer += 1;
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

/// Verifies a batch (contiguous heights, hash-linked to our tip, quorum finalization over the
/// last block) and executes it.
fn apply_batch(chain: &Shared, verifier: &MlDsaScheme, batch: &SyncBatch) -> Result<()> {
    let blocks: Vec<CBlock> = batch.blocks.iter().map(|h| Ok(CBlock::decode(hex::decode(h)?.as_slice())?)).collect::<Result<_>>()?;
    let fin_hex = batch.finalization.as_ref().ok_or_else(|| anyhow!("missing finalization"))?;
    let fin = Finalization::<MlDsaScheme, Digest>::decode_cfg(hex::decode(fin_hex)?.as_slice(), &verifier.certificate_codec_config())?;
    let last = blocks.last().ok_or_else(|| anyhow!("empty batch"))?;
    if Some(last.height) != batch.to || fin.proposal.payload != last.digest() {
        bail!("finalization does not cover the batch");
    }
    if !fin.verify(&mut commonware_utils::sys_rng(), verifier, &commonware_parallel::Sequential) {
        bail!("invalid finalization certificate");
    }
    let mut c = chain.lock();
    let mut prev = c.last_hash().to_string();
    let mut height = c.height();
    for b in &blocks {
        if b.height != height + 1 || hex::encode(b.parent_bytes()) != prev {
            bail!("batch does not extend our chain at height {}", b.height);
        }
        prev = hex::encode(b.digest_bytes());
        height = b.height;
    }
    for b in &blocks {
        let cert = if b.height == last.height { fin_hex.clone() } else { String::new() };
        c.execute(consensus::finalized_from(b, cert))?;
    }
    Ok(())
}
