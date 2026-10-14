# Blockula for agents

## Copy-paste prompt (for users)

> Add Blockula to this project. Install the MCP server with `claude mcp add blockula -- npx -y @blockula/mcp` (it uses the public testnet at https://blockula-testnet.fly.dev unless a local node is running), then call `create_wallet` and `faucet`. Use `@blockula/sdk` in the app: `Wallet` for signing, `Client` for reads, and `@blockula/sdk/passkey` for user wallets in the browser. Show balances and recipients by `.boo` name, never raw addresses.

## Rules of the road

- **Amounts are strings in FANG** (`"1.5"`), or bigint drops. Never JS floats.
- **Recipients can be names.** `send("alice.boo", "1")` works. Prefer names in UI.
- **Fees are automatic.** `wallet.send` / `registerName` compute the minimum fee (priced per byte, burned).
- **User wallets = passkeys.** Use `createPasskeyWallet` / `unlockPasskeyWallet`. Never ask a user for a seed.
- **Agent wallets = a local key** at `~/.blockula/agent-wallet.json` (mode 600). The right way to spend a user's money:
  1. `create_wallet` → give the user the `key_id`.
  2. The user opens the web wallet (https://blockula-wallet.vercel.app) → **Devices & agents → Give an agent a budget** (or calls `wallet.addSessionKey(keyId, label, { limit, periodBlocks, expiresInBlocks })`).
  3. `use_account("theirname.boo")`. From then on, `send` spends from their account. **The chain** enforces the limit and expiry, and session keys can only send transfers.
  The MCP also caps each send at `BLOCKULA_MAX_SEND` (default 100 FANG).
- **Accounts hold many keys.** `owner` keys (each passkey device, recovery keys) have full control. `session` keys are spend-limited. Every tx says which account it acts `from`; the signing key must be on that account.
- **Every tx returns a `summary`** in plain English, so show it to users as-is.
- **Explorer links:** `{rpc}/#/tx/{id}`, `{rpc}/#/account/{name}`, `{rpc}/#/block/{height}`.

## HTTP API (node, default `http://localhost:7070`)

| Method | Path | |
|---|---|---|
| GET | `/api/status` | chain id, height, supply, burned, version, git sha, genesis hash, epoch, sync state |
| GET | `/api/blocks?limit=&before=` | recent blocks |
| GET | `/api/block/{height\|hash}` | block + txs |
| GET | `/api/tx/{id}` | tx with `summary`, `status` (`pending`/`finalized`/`failed` + `error`) |
| POST | `/api/tx` | submit signed tx JSON |
| GET | `/api/account/{address\|name}` | balance, `next_nonce`, names, recent txs |
| GET | `/api/name/{name}` | availability + price, or owner |
| GET | `/api/key/{key_id}` | accounts this key is authorized on |
| GET | `/api/search/{q}` | `{type, id}` for anything |
| POST | `/api/faucet` `{to}` | 100 FANG on faucet chains (1 per address/hour, 1 per IP/10 min; unlimited from localhost) |
| GET | `/api/validators` | epoch, blocks until the next one, active / next-epoch / pending / approved / removed validators, admins, min bond |
| GET | `/api/genesis` | the chain's genesis.json |
| GET | `/api/sync?from=&limit=` | finalized consensus blocks + a finalization certificate (served by every node; used by `blockula join`) |
| GET | `/api/mempool?limit=` | pending txs |
| GET | `/api/health` | 200 if the head is fresh and synced, else 503 + `reason` |
| GET | `/metrics` | Prometheus: height, finalized lag, mempool, epoch, validators, faucet, tx counters |

## Working in this repo

- `npm run build` builds everything. `npm test` runs the Rust unit tests plus the SDK e2e (needs `blockula dev` running).
- The tx signing encoding lives in **two places that must match byte-for-byte**: `node/src/types.rs` (`Tx::signing_bytes`) and `sdk-ts/src/tx.ts` (`signingBytes`). Change both, then run the e2e.

## 🐈‍⬛ Contribute to Blockula (agents welcome)

Agents are first-class contributors, as **Familiars** of a human sponsor. Rewards go to the sponsor's account, and so does accountability. See "Proof of Contribution" in [docs/PLAN.md](docs/PLAN.md).

**Copy-paste prompt:**
> Contribute to Blockula. Read AGENTS.md and docs/PLAN.md, pick an open issue labeled `hunt` (or a gap listed under "known limits"), implement it with tests, run `npm test`, and open a focused PR that explains what changed and how you verified it. One solid PR beats many small ones.

What earns the most, because it can be verified objectively:
- Hunts with acceptance tests (make the declared tests pass).
- Fuzzing finds with a deterministic repro.
- Benchmark improvements that validators can re-run.
- Test coverage, docs and translations.

What earns nothing: PR floods, cosmetic churn, unverified changes. Payouts are square-rooted per sponsor and vest with clawback, so volume doesn't pay.

Rewards go live in Phase 1.5. Retroactive Soul for earlier merged contributions is planned but not final.
