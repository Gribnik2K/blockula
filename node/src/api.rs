use crate::chain::{short, treasury, Chain};
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

pub fn router(chain: Shared) -> Router {
    Router::new()
        .route("/", get(|| async { Html(EXPLORER) }))
        .route("/api/status", get(status))
        .route("/api/blocks", get(blocks))
        .route("/api/block/{id}", get(block))
        .route("/api/tx/{id}", get(tx))
        .route("/api/tx", post(submit))
        .route("/api/account/{id}", get(account))
        .route("/api/name/{name}", get(name))
        .route("/api/search/{q}", get(search))
        .route("/api/faucet", post(faucet))
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(chain)
}

fn label(c: &Chain, addr: &str) -> Value {
    let name = if addr == treasury() { Some("🏛️ treasury".to_string()) } else if addr == c.genesis.validator { c.state.primary_name(addr).or(Some("🧛 validator".into())) } else { c.state.primary_name(addr) };
    json!({ "address": addr, "name": name, "short": short(addr) })
}

fn tx_json(c: &Chain, tx: &Tx, height: Option<u64>) -> Value {
    let sender = tx.sender().unwrap_or_default();
    let (kind, to, amount) = match &tx.action {
        Action::Transfer { to, amount } => ("transfer", c.state.resolve(to).ok(), Some(*amount)),
        Action::RegisterName { .. } => ("register_name", None, None),
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

async fn status(State(s): State<Shared>) -> ApiResult {
    let c = s.lock();
    Ok(Json(json!({
        "chain_id": c.genesis.chain_id,
        "token": c.genesis.token,
        "decimals": c.genesis.decimals,
        "height": c.height(),
        "block_time_ms": c.genesis.block_time_ms,
        "validator": label(&c, &c.genesis.validator),
        "faucet": c.genesis.faucet,
        "supply": c.state.supply.to_string(),
        "supply_display": fmt_fang(c.state.supply),
        "burned": c.state.burned.to_string(),
        "burned_display": fmt_fang(c.state.burned),
        "accounts": c.state.accounts.len(),
        "names": c.state.names.len(),
        "mempool": c.mempool.len(),
        "data_dir": c.home.display().to_string(),
        "crypto": { "signatures": "ML-DSA-65 (FIPS 204)", "hash": "BLAKE3-256", "addresses": "bech32m(boo, BLAKE3(pubkey))" },
    })))
}

#[derive(Deserialize)]
struct Page {
    limit: Option<usize>,
    before: Option<u64>,
}

async fn blocks(State(s): State<Shared>, Query(p): Query<Page>) -> ApiResult {
    let c = s.lock();
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

async fn block(State(s): State<Shared>, Path(id): Path<String>) -> ApiResult {
    let c = s.lock();
    let b = find_block(&c, &id).ok_or_else(|| not_found("block"))?;
    Ok(Json(block_json(&c, b, true)))
}

async fn tx(State(s): State<Shared>, Path(id): Path<String>) -> ApiResult {
    let c = s.lock();
    if let Some(&(h, i)) = c.tx_index.get(&id) {
        return Ok(Json(tx_json(&c, &c.blocks[h as usize - 1].txs[i], Some(h))));
    }
    let pending = c.mempool.iter().find(|t| t.id() == id).ok_or_else(|| not_found("transaction"))?;
    Ok(Json(tx_json(&c, pending, None)))
}

async fn submit(State(s): State<Shared>, Json(tx): Json<Tx>) -> ApiResult {
    let mut c = s.lock();
    let id = c.submit(tx)?;
    Ok(Json(json!({ "id": id, "status": "pending" })))
}

async fn account(State(s): State<Shared>, Path(id): Path<String>) -> ApiResult {
    let c = s.lock();
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
        "recent_txs": recent,
    })))
}

async fn name(State(s): State<Shared>, Path(n): Path<String>) -> ApiResult {
    let c = s.lock();
    let n = normalize_name(&n)?;
    match c.state.names.get(&n) {
        Some(r) => Ok(Json(json!({ "name": format!("{n}{NAME_SUFFIX}"), "available": false, "owner": label(&c, &r.owner), "registered_at": r.registered_at }))),
        None => Ok(Json(json!({ "name": format!("{n}{NAME_SUFFIX}"), "available": true, "price": name_price(&n).to_string(), "price_display": fmt_fang(name_price(&n)) }))),
    }
}

async fn search(State(s): State<Shared>, Path(q): Path<String>) -> ApiResult {
    let c = s.lock();
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

async fn faucet(State(s): State<Shared>, Json(body): Json<HashMap<String, String>>) -> ApiResult {
    let to = body.get("to").ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "missing `to`".into()))?;
    let id = s.lock().faucet(to, 100 * DROPS_PER_FANG)?;
    Ok(Json(json!({ "id": id, "amount_display": "100 FANG" })))
}
