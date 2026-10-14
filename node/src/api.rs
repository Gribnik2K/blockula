use crate::chain::{short, treasury, Chain, Role};
use crate::consensus::MarshalMailbox;
use crate::crypto;
use crate::types::*;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

pub type Shared = Arc<Mutex<Chain>>;

/// Everything the HTTP handlers need.
#[derive(Clone)]
pub struct Node {
    pub chain: Shared,
    /// Validators: marshal, to serve finalized blocks + certificates to joining nodes.
    pub marshal: Option<MarshalMailbox>,
    /// Full nodes: validator HTTP endpoints that txs and faucet requests are forwarded to.
    pub peers: Vec<String>,
}

impl Node {
    async fn forward(&self, path: &str, body: &Value) -> Result<Value, ApiError> {
        let client = reqwest::Client::new();
        let mut last = String::from("no peers configured");
        for peer in &self.peers {
            match client.post(format!("{}/api{path}", peer.trim_end_matches('/'))).json(body).send().await {
                Ok(res) => {
                    let ok = res.status().is_success();
                    let v: Value = res.json().await.map_err(|e| ApiError(StatusCode::BAD_GATEWAY, e.to_string()))?;
                    if ok {
                        return Ok(v);
                    }
                    return Err(ApiError(StatusCode::BAD_REQUEST, v["error"].as_str().unwrap_or("peer rejected the request").to_string()));
                }
                Err(e) => last = e.to_string(),
            }
        }
        Err(ApiError(StatusCode::BAD_GATEWAY, format!("couldn't reach a validator: {last}")))
    }
}

const EXPLORER: &str = include_str!("explorer.html");

struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError(StatusCode::BAD_REQUEST, e.to_string())
    }
}

fn not_found(what: &str) -> ApiError {
    ApiError(StatusCode::NOT_FOUND, format!("{what} not found"))
}

type ApiResult = Result<Json<Value>, ApiError>;

pub fn router(node: Node) -> Router {
    Router::new()
        .route("/", get(|| async { Html(EXPLORER) }))
        .route("/api/status", get(status))
        .route("/api/blocks", get(blocks))
        .route("/api/block/{id}", get(block))
        .route("/api/tx/{id}", get(tx))
        .route("/api/tx", post(submit))
        .route("/api/account/{id}", get(account))
        .route("/api/name/{name}", get(name))
        .route("/api/key/{key_id}", get(key_accounts))
        .route("/api/search/{q}", get(search))
        .route("/api/faucet", post(faucet))
        .route("/api/genesis", get(genesis))
        .route("/api/sync", get(sync))
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(node)
}

fn label(c: &Chain, addr: &str) -> Value {
    let name = if addr == treasury() { Some("🏛️ treasury".to_string()) } else if c.genesis.is_validator(addr) { c.state.primary_name(addr).or(Some("🧛 validator".into())) } else { c.state.primary_name(addr) };
    json!({ "address": addr, "name": name, "short": short(addr) })
}

fn tx_json(c: &Chain, tx: &Tx, height: Option<u64>) -> Value {
    let sender = tx.sender().unwrap_or_default();
    let (kind, to, amount) = match &tx.action {
        Action::Transfer { to, amount } => ("transfer", c.state.resolve(to).ok(), Some(*amount)),
        Action::RegisterName { .. } => ("register_name", None, None),
        Action::AddKey { .. } => ("add_key", None, None),
        Action::RemoveKey { .. } => ("remove_key", None, None),
    };
    json!({
        "id": tx.id(),
        "summary": c.state.describe(tx),
        "kind": kind,
        "status": if height.is_some() { "finalized" } else { "pending" },
        "height": height,
        "from": label(c, &sender),
        "to": to.map(|t| label(c, &t)),
        "amount": amount.map(|a| a.to_string()),
        "amount_display": amount.map(fmt_fang),
        "fee": tx.fee.to_string(),
        "fee_display": fmt_fang(tx.fee),
        "size_bytes": tx.size(),
        "nonce": tx.nonce,
        "signer_key": tx.signer_key_id().ok(),
        "memo": tx.memo,
        "action": tx.action,
        "signature_scheme": "ML-DSA-65 (FIPS 204)",
    })
}

fn block_json(c: &Chain, b: &Block, full: bool) -> Value {
    let mut v = json!({
        "height": b.height,
        "hash": b.hash,
        "prev_hash": b.prev_hash,
        "timestamp_ms": b.timestamp_ms,
        "proposer": label(c, &b.proposer),
        "state_root": b.state_root,
        "tx_count": b.txs.len(),
        "summary": format!("Block #{} sealed with {} transaction{}", b.height, b.txs.len(), if b.txs.len() == 1 { "" } else { "s" }),
    });
    if full {
        v["txs"] = b.txs.iter().map(|t| tx_json(c, t, Some(b.height))).collect();
        v["signature_bytes"] = json!(b.signature.len() / 2);
    }
    v
}

async fn status(State(n): State<Node>) -> ApiResult {
    let c = n.chain.lock();
    Ok(Json(json!({
        "chain_id": c.genesis.chain_id,
        "token": c.genesis.token,
        "decimals": c.genesis.decimals,
        "height": c.height(),
        "block_time_ms": c.genesis.block_time_ms,
        "validator": label(&c, &c.genesis.validators[0].address),
        "validators": c.genesis.validators.iter().map(|v| label(&c, &v.address)).collect::<Vec<_>>(),
        "role": c.role,
        "finalized_height": c.finalized_tip.max(c.height()),
        "syncing": c.height() < c.finalized_tip,
        "sync_progress": if c.finalized_tip == 0 { 1.0 } else { (c.height() as f64 / c.finalized_tip as f64).min(1.0) },
        "state_root": c.state.root(),
        "consensus": if c.genesis.validators.len() == 1 { "simplex BFT (single validator)" } else { "simplex BFT" },
        "faucet": c.genesis.faucet,
        "supply": c.state.supply.to_string(),
        "supply_display": fmt_fang(c.state.supply),
        "burned": c.state.burned.to_string(),
        "burned_display": fmt_fang(c.state.burned),
        "accounts": c.state.accounts.len(),
        "names": c.state.names.len(),
        "mempool": c.mempool.len(),
        "data_dir": c.home.display().to_string(),
        "crypto": { "signatures": "ML-DSA-65 (FIPS 204)", "hash": "BLAKE3-256", "addresses": "bech32m(boo, BLAKE3(pubkey))", "p2p_transport": "X25519 + ChaCha20-Poly1305 (not post-quantum), ML-DSA-65 peer auth" },
    })))
}

#[derive(Deserialize)]
struct Page {
    limit: Option<usize>,
    before: Option<u64>,
}

async fn blocks(State(n): State<Node>, Query(p): Query<Page>) -> ApiResult {
    let c = n.chain.lock();
    let limit = p.limit.unwrap_or(20).min(100);
    let before = p.before.unwrap_or(u64::MAX);
    let out: Vec<Value> = c.blocks.iter().rev().filter(|b| b.height < before).take(limit).map(|b| block_json(&c, b, false)).collect();
    Ok(Json(json!(out)))
}

fn find_block<'a>(c: &'a Chain, id: &str) -> Option<&'a Block> {
    match id.parse::<u64>() {
        Ok(h) if h >= 1 => c.blocks.get(h as usize - 1),
        _ => c.blocks.iter().find(|b| b.hash == id),
    }
}

async fn block(State(n): State<Node>, Path(id): Path<String>) -> ApiResult {
    let c = n.chain.lock();
    let b = find_block(&c, &id).ok_or_else(|| not_found("block"))?;
    Ok(Json(block_json(&c, b, true)))
}

async fn tx(State(n): State<Node>, Path(id): Path<String>) -> ApiResult {
    let c = n.chain.lock();
    if let Some(&(h, i)) = c.tx_index.get(&id) {
        return Ok(Json(tx_json(&c, &c.blocks[h as usize - 1].txs[i], Some(h))));
    }
    let pending = c.mempool.iter().find(|t| t.id() == id).ok_or_else(|| not_found("transaction"))?;
    Ok(Json(tx_json(&c, pending, None)))
}

async fn submit(State(n): State<Node>, Json(tx): Json<Tx>) -> ApiResult {
    if n.chain.lock().role == Role::FullNode {
        // Check locally first for a good error, then hand it to a validator.
        {
            let c = n.chain.lock();
            let mut scratch = c.state.clone();
            for q in &c.mempool {
                let _ = scratch.apply(q, &c.genesis.chain_id, &c.genesis.validators[0].address, c.height() + 1);
            }
            scratch.apply(&tx, &c.genesis.chain_id, &c.genesis.validators[0].address, c.height() + 1)?;
        }
        n.forward("/tx", &serde_json::to_value(&tx).unwrap()).await?;
        let id = tx.id();
        n.chain.lock().track_pending(tx);
        return Ok(Json(json!({ "id": id, "status": "pending" })));
    }
    let id = n.chain.lock().submit(tx)?;
    Ok(Json(json!({ "id": id, "status": "pending" })))
}

async fn account(State(n): State<Node>, Path(id): Path<String>) -> ApiResult {
    let c = n.chain.lock();
    let addr = c.state.resolve(&id)?;
    let acct = c.state.account(&addr);
    let names: Vec<String> = c.state.names.values().filter(|r| r.owner == addr).map(|r| format!("{}{NAME_SUFFIX}", r.name)).collect();
    let recent: Vec<Value> = c.history.get(&addr).map(|ids| {
        ids.iter().rev().take(25).filter_map(|id| c.tx_index.get(id)).map(|&(h, i)| tx_json(&c, &c.blocks[h as usize - 1].txs[i], Some(h))).collect()
    }).unwrap_or_default();
    Ok(Json(json!({
        "account": label(&c, &addr),
        "balance": acct.balance.to_string(),
        "balance_display": fmt_fang(acct.balance),
        "nonce": acct.nonce,
        "next_nonce": c.next_nonce(&addr),
        "names": names,
        "keys": c.state.keys_of(&addr).into_iter().map(|(id, k)| json!({ "key_id": id, "label": k.label, "role": k.role, "added_at": k.added_at, "spent_this_period": k.spent.to_string(), "window_start": k.window_start })).collect::<Vec<_>>(),
        "recent_txs": recent,
    })))
}

/// Which accounts a key may act for — lets a newly linked device find its account.
async fn key_accounts(State(n): State<Node>, Path(key_id): Path<String>) -> ApiResult {
    let c = n.chain.lock();
    let mut accounts: Vec<Value> = c.state.accounts.iter().filter(|(_, a)| a.keys.contains_key(&key_id)).map(|(addr, a)| json!({ "account": label(&c, addr), "role": a.keys[&key_id].role, "label": a.keys[&key_id].label })).collect();
    if let Ok(bytes) = hex::decode(&key_id) {
        if let Ok(bytes) = <[u8; 32]>::try_from(bytes) {
            let own = crypto::address_from_bytes(&bytes);
            let listed = accounts.iter().any(|a| a["account"]["address"] == own.as_str());
            if !listed && c.state.keys_of(&own).contains_key(&key_id) {
                accounts.insert(0, json!({ "account": label(&c, &own), "role": { "type": "owner" }, "label": "original key" }));
            }
        }
    }
    Ok(Json(json!({ "key_id": key_id, "accounts": accounts })))
}

async fn name(State(node): State<Node>, Path(n): Path<String>) -> ApiResult {
    let c = node.chain.lock();
    let n = normalize_name(&n)?;
    match c.state.names.get(&n) {
        Some(r) => Ok(Json(json!({ "name": format!("{n}{NAME_SUFFIX}"), "available": false, "owner": label(&c, &r.owner), "registered_at": r.registered_at }))),
        None => Ok(Json(json!({ "name": format!("{n}{NAME_SUFFIX}"), "available": true, "price": name_price(&n).to_string(), "price_display": fmt_fang(name_price(&n)) }))),
    }
}

async fn search(State(n): State<Node>, Path(q): Path<String>) -> ApiResult {
    let c = n.chain.lock();
    let q = q.trim().to_lowercase();
    let hit = if find_block(&c, &q).is_some() {
        json!({ "type": "block", "id": q })
    } else if c.tx_index.contains_key(&q) || c.mempool.iter().any(|t| t.id() == q) {
        json!({ "type": "tx", "id": q })
    } else if crypto::is_address(&q) {
        json!({ "type": "account", "id": q })
    } else if let Ok(n) = normalize_name(&q) {
        if c.state.names.contains_key(&n) { json!({ "type": "account", "id": format!("{n}{NAME_SUFFIX}") }) } else { json!({ "type": "name", "id": n }) }
    } else {
        return Err(not_found("nothing matching that"));
    };
    Ok(Json(hit))
}

async fn faucet(State(n): State<Node>, Json(body): Json<HashMap<String, String>>) -> ApiResult {
    let to = body.get("to").ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "missing `to`".into()))?.clone();
    let to = &to;
    let role = n.chain.lock().role;
    if role == Role::FullNode {
        let v = n.forward("/faucet", &json!({ "to": to })).await?;
        if let Ok(tx) = serde_json::from_value::<Tx>(v["tx"].clone()) {
            n.chain.lock().track_pending(tx);
        }
        return Ok(Json(v));
    }
    let tx = n.chain.lock().faucet(to, 100 * DROPS_PER_FANG)?;
    Ok(Json(json!({ "id": tx.id(), "amount_display": "100 FANG", "tx": tx })))
}

async fn genesis(State(n): State<Node>) -> ApiResult {
    Ok(Json(serde_json::to_value(&n.chain.lock().genesis).unwrap()))
}

#[derive(Deserialize)]
struct SyncQuery {
    from: u64,
    limit: Option<u64>,
}

/// Finalized consensus blocks `[from, to]` plus the finalization certificate of block `to`, so a
/// joining node can verify the whole batch (hash links + one ML-DSA quorum certificate).
async fn sync(State(n): State<Node>, Query(q): Query<SyncQuery>) -> ApiResult {
    use commonware_codec::Encode;
    use commonware_consensus::types::Height;
    let marshal = n.marshal.clone().ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "only validators serve sync".into()))?;
    let tip = n.chain.lock().height();
    let from = q.from.max(1);
    if from > tip {
        return Ok(Json(json!({ "tip": tip, "blocks": [], "to": null, "finalization": null })));
    }
    let to = (from + q.limit.unwrap_or(100).clamp(1, 500) - 1).min(tip);
    // Find the highest height in range that has a stored finalization (normally `to` itself).
    let mut cert_height = None;
    let mut h = to;
    while h >= from {
        if let Some(f) = marshal.get_finalization(Height::new(h)).await {
            cert_height = Some((h, hex::encode(f.encode())));
            break;
        }
        h -= 1;
    }
    if cert_height.is_none() {
        for h in to + 1..=tip.min(to + 64) {
            if let Some(f) = marshal.get_finalization(Height::new(h)).await {
                cert_height = Some((h, hex::encode(f.encode())));
                break;
            }
        }
    }
    let Some((to, finalization)) = cert_height else {
        return Ok(Json(json!({ "tip": tip, "blocks": [], "to": null, "finalization": null })));
    };
    let mut blocks = vec![];
    for h in from..=to {
        let b = marshal.get_block(Height::new(h)).await.ok_or_else(|| not_found("block"))?;
        blocks.push(hex::encode(b.encode()));
    }
    Ok(Json(json!({ "tip": tip, "blocks": blocks, "to": to, "finalization": finalization })))
}
