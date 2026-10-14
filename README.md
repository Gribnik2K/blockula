# 🧛 Blockula

**The post-quantum blockchain that never dies.** Wallets unlocked by passkeys, human-readable `.boo` names, an explorer that explains itself, and an SDK + MCP server so any coding agent can use it, or run its own copy, from one prompt.

Quantum entanglement is literally "spooky action at a distance". We took that personally.

| | |
|---|---|
| Token | **FANG** (1 FANG = 10⁹ drops 🩸) |
| Signatures | **ML-DSA-65** (NIST FIPS 204), no elliptic curves anywhere |
| Hashing | BLAKE3-256 |
| Addresses | `boo1…` = bech32m(BLAKE3(public key)), or just `yourname.boo` |
| Privacy (Phase 2) | 👻 **Ghost Mode** shielded transfers + 🪦 **The Crypt** encrypted storage, built on hash-based STARKs + ML-KEM |

## Quickstart

```bash
npm install && npm run build
npm run dev
```

That builds the Rust node, SDK and MCP server, then starts a local chain with a faucet. Open http://localhost:7070 for the explorer.

```ts
import { Client, Wallet } from "@blockula/sdk";

const wallet = Wallet.generate(new Client("http://localhost:7070"));
await wallet.faucet();                       // 100 test FANG
await wallet.registerName("vlad");           // vlad.boo
await wallet.send("alice.boo", "1.5", { memo: "trick or treat" });
```

In the browser, a passkey unlocks the wallet (no seed phrase):

```ts
import { createPasskeyWallet, unlockPasskeyWallet } from "@blockula/sdk/passkey";
const wallet = await createPasskeyWallet({ username: "vlad" });
```

## For agents

```bash
claude mcp add blockula -- npx -y @blockula/mcp
```

Then paste the prompt from [AGENTS.md](AGENTS.md). MCP tools: `start_dev_node`, `summon_chain`, `create_wallet`, `wallet_info`, `faucet`, `send`, `check_name`, `register_name`, `lookup`, `latest_blocks`, `status`.

## Run your own chain

`summon` creates a new genesis and validator key in `~/.blockula/mychain`; `run` starts it.

```bash
blockula summon mychain
blockula run --home ~/.blockula/mychain --port 7070
```

## Layout

```
node/     Rust node: chain, ML-DSA crypto, HTTP API, built-in explorer (src/explorer.html)
sdk-ts/   @blockula/sdk: wallets, tx signing, client, passkey (WebAuthn PRF) helpers
mcp/      @blockula/mcp: MCP server for coding agents
docs/     PLAN.md: architecture, tokenomics, roadmap
```

## Status

Phase 0: a single-validator dev chain. **Not audited. Don't put real value on it.** See [docs/PLAN.md](docs/PLAN.md).
