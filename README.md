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

Then paste the prompt from [AGENTS.md](AGENTS.md). MCP tools: `start_dev_node`, `summon_chain`, `create_wallet`, `use_account`, `wallet_info`, `faucet`, `send`, `check_name`, `register_name`, `lookup`, `latest_blocks`, `status`.

## Run your own chain

`summon` creates a genesis plus validator keys; `run` starts a validator. Consensus is
[Commonware](https://commonware.xyz) **simplex BFT** with ML-DSA-65 votes and certificates, so the
same binary runs a 1-validator chain or a multi-validator network.

```bash
# one validator (HTTP on 7070)
blockula summon mychain
blockula run --home ~/.blockula/mychain

# a local 4-validator testnet: ~/.blockula/mynet/v0 … v3, ports picked automatically
blockula summon mynet --validators 4
blockula run --home ~/.blockula/mynet/v0     # repeat for v1, v2, v3 (each prints its URL)

# a full node: syncs finalized blocks from the validators, serves the API + explorer right away
blockula join --genesis http://127.0.0.1:<v0 http port> --home ~/.blockula/mynet-full --port 7171
```

Any node accepts txs (they're gossiped to the leader) and, on chains with a faucet, faucet
requests (each validator's own genesis-funded account pays; full nodes forward). Nodes restart
where they left off; a validator that was down catches up via block backfill.
`scripts/local-testnet.sh` runs 4 validators + a full node and the SDK e2e against them.

**Testnet only.** The p2p transport authenticates peers with ML-DSA-65 but encrypts with
X25519 + ChaCha20-Poly1305, which is not post-quantum (see [docs/PLAN.md](docs/PLAN.md#known-limits)).

## Layout

```
node/     Rust node: consensus (Commonware simplex + marshal), chain execution, ML-DSA crypto, HTTP API, explorer
scripts/  local-testnet.sh: 4 validators + full node smoke test
sdk-ts/   @blockula/sdk: wallets, tx signing, client, passkey (WebAuthn PRF) helpers
mcp/      @blockula/mcp: MCP server for coding agents
docs/     PLAN.md: architecture, tokenomics, roadmap
```

## Status

Phase 1 in progress: multi-validator BFT, p2p and `join` work on local testnets. **Not audited. Don't put real value on it.** See [docs/PLAN.md](docs/PLAN.md).
