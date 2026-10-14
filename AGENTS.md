# Blockula for agents

## Copy-paste prompt (for users)

> Add Blockula to this project. Install the MCP server with `claude mcp add blockula -- npx -y @blockula/mcp`, call `start_dev_node`, then `create_wallet` and `faucet`. Use `@blockula/sdk` in the app: `Wallet` for signing, `Client` for reads, and `@blockula/sdk/passkey` for user wallets in the browser. Show balances and recipients by `.boo` name, never raw addresses.

## Rules of the road

- **Amounts are strings in FANG** (`"1.5"`), or bigint drops. Never JS floats.
- **Recipients can be names.** `send("alice.boo", "1")` works. Prefer names in UI.
- **Fees are automatic.** `wallet.send` / `registerName` compute the minimum fee (priced per byte, burned).
- **User wallets = passkeys.** Use `createPasskeyWallet` / `unlockPasskeyWallet`. Never ask a user for a seed.
- **Agent wallets = local seed** at `~/.blockula/agent-wallet.json` (mode 600). The MCP caps each send at `BLOCKULA_MAX_SEND` (default 100 FANG).
- **Every tx returns a `summary`** in plain English, so show it to users as-is.
- **Explorer links:** `{rpc}/#/tx/{id}`, `{rpc}/#/account/{name}`, `{rpc}/#/block/{height}`.

## HTTP API (node, default `http://localhost:7070`)

| Method | Path | |
|---|---|---|
| GET | `/api/status` | chain id, height, supply, burned |
| GET | `/api/blocks?limit=&before=` | recent blocks |
| GET | `/api/block/{height\|hash}` | block + txs |
| GET | `/api/tx/{id}` | tx with `summary`, `status` (`pending`/`finalized`) |
| POST | `/api/tx` | submit signed tx JSON |
| GET | `/api/account/{address\|name}` | balance, `next_nonce`, names, recent txs |
| GET | `/api/name/{name}` | availability + price, or owner |
| GET | `/api/search/{q}` | `{type, id}` for anything |
| POST | `/api/faucet` `{to}` | 100 FANG (dev chains) |

## Working in this repo

- `npm run build` builds everything. `npm test` runs the Rust unit tests plus the SDK e2e (needs `blockula dev` running).
- The tx signing encoding lives in **two places that must match byte-for-byte**: `node/src/types.rs` (`Tx::signing_bytes`) and `sdk-ts/src/tx.ts` (`signingBytes`). Change both, then run the e2e.
