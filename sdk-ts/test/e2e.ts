// End-to-end against a running node: `blockula dev --reset` then `npm test`.
// Proves TS-signed ML-DSA transactions verify in the Rust node.
import assert from "node:assert/strict";
import { Client, Wallet, formatFang, minFee, parseFang } from "../src/index.js";

const client = new Client(process.env.BLOCKULA_RPC ?? "http://localhost:7070");

assert.equal(parseFang("1.5"), 1_500_000_000n);
assert.equal(formatFang(1_500_000_000n), "1.5 FANG");

const alice = Wallet.generate(client);
const bob = Wallet.generate(client);
const name = `alice${Date.now() % 100000}`;
console.log("alice:", alice.address);

await alice.faucet();
console.log("after faucet:", await alice.balance());

const reg = await alice.registerName(name);
console.log("✓", reg.summary);

const sent = await alice.send(bob.address, "12.5", { memo: "trick or treat" });
console.log("✓", sent.summary);

const viaName = await bob.client.account(`${name}.boo`);
assert.equal(viaName.account.address, alice.address);

const bobTx = await bob.send(`${name}.boo`, "2");
assert.equal(await bob.balance(), formatFang(parseFang("10.5") - BigInt(bobTx.fee)));
assert.equal(BigInt(bobTx.fee), minFee({ chain_id: (await client.status()).chain_id, nonce: 0, fee: "0", action: bobTx.action, memo: "" }));
console.log("✓", bobTx.summary);

const status = await client.status();
console.log(`✓ all good at height ${status.height}, burned so far: ${status.burned_display}`);
