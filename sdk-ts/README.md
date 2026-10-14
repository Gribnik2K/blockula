# @blockula/sdk

🧛 TypeScript SDK for **Blockula**, a post-quantum blockchain. It covers ML-DSA-65 (FIPS 204) wallets, `.boo` names, transfers, multi-key accounts, agent session keys, and passkey-unlocked wallets.

```bash
npm install @blockula/sdk
```

```ts
import { Client, Wallet } from "@blockula/sdk";

const wallet = Wallet.generate(new Client("http://localhost:7070"));
await wallet.faucet();                        // test chains: 100 FANG
await wallet.registerName("vlad");            // vlad.boo
await wallet.send("alice.boo", "1.5", { memo: "trick or treat" });

// Let an agent spend up to 10 FANG/day from this account (enforced on-chain)
await wallet.addSessionKey(agentKeyId, "claude", { limit: "10", periodBlocks: 86_400, expiresInBlocks: 30 * 86_400 });
```

In the browser, a passkey unlocks the wallet. There's no seed phrase, and the passkey's (non-quantum-safe) signature is never used on-chain. Its PRF output derives the ML-DSA key:

```ts
import { createPasskeyWallet, unlockPasskeyWallet } from "@blockula/sdk/passkey";
const wallet = await createPasskeyWallet({ username: "vlad" });
```

Amounts are FANG strings (`"1.5"`) or bigint drops (1 FANG = 10⁹ drops). Every transaction result includes a plain-English `summary`.

- `send("alice.boo", …)` resolves the name first and signs the **address**, so whoever you checked is exactly who gets paid.
- Node errors throw `ApiError` (`.status`: 429 means rate limited). A transaction that fails at execution throws `TxFailedError` from `waitForTx`.
- Pass `rpId` to `createPasskeyWallet`/`unlockPasskeyWallet` to pin wallets to your production domain. Passkeys are bound to that hostname for good.

The public testnet is `https://blockula-testnet.fly.dev`, and the wallet is https://blockula-wallet.vercel.app.

⚠️ Early and unaudited: test chains only.
