// End-to-end against a running node: `blockula dev --reset` then `npm test`.
// Proves TS-signed ML-DSA transactions verify in the Rust node.
import assert from "node:assert/strict";
import { ml_dsa65 } from "@noble/post-quantum/ml-dsa.js";
import { randomBytes } from "@noble/hashes/utils.js";
import { Client, POP_CONTEXT, Wallet, addressFromPublicKey, formatFang, minFee, parseFang, popMessage } from "../src/index.js";

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
assert.equal(BigInt(bobTx.fee), minFee({ chain_id: (await client.status()).chain_id, from: bob.address, nonce: 0, fee: "0", action: bobTx.action, memo: "" }));
console.log("✓", bobTx.summary);

// Account abstraction: alice lets an agent key spend up to 5 FANG per 100 blocks from her account.
const agent = Wallet.generate(client);
const grant = await alice.addSessionKey(agent.keyId, "claude", { limit: "5", periodBlocks: 100, expiresInBlocks: 1000 });
console.log("✓", grant.summary);
await agent.actAs(`${name}.boo`);
const paid = await agent.send(bob.address, "3", { memo: "agent errand" });
console.log("✓", paid.summary);
await assert.rejects(agent.send(bob.address, "3"), /limit reached/);
await assert.rejects(agent.registerName(`x${name}`), /only send transfers/);
console.log("✓ session key limits enforced");

// A second device (e.g. another passkey) gets full control, then revokes the agent.
const phone = Wallet.generate(client);
await alice.addOwnerKey(phone.keyId, "phone");
await phone.actAs(alice.address);
await phone.removeKey(agent.keyId);
await assert.rejects(agent.send(bob.address, "0.1"), /isn't authorized/);
console.log("✓ second device revoked the agent");

// Validator onboarding (Season 0): anyone can bond and register; only admins approve.
const vinfo = await client.validators();
// Stands in for a validator node's consensus key; it proves possession by signing the registration.
const nodeKeys = ml_dsa65.keygen(randomBytes(32));
const node = { address: addressFromPublicKey(nodeKeys.publicKey) };
const consensusKey = Buffer.from(nodeKeys.publicKey).toString("hex");
const p2pAddr = "203.0.113.7:30300", httpAddr = "203.0.113.7:7070";
const chainId = (await client.status()).chain_id;
const pop = Buffer.from(ml_dsa65.sign(popMessage(chainId, alice.address, p2pAddr, httpAddr), nodeKeys.secretKey, { context: POP_CONTEXT })).toString("hex");
const minBond = BigInt(vinfo.min_bond);
if (minBond + parseFang("1") <= BigInt((await client.account(alice.address)).balance)) {
    // A PoP for someone else's key (or another owner) is rejected.
  const badPop = Buffer.from(ml_dsa65.sign(popMessage(chainId, bob.address, p2pAddr, httpAddr), nodeKeys.secretKey, { context: POP_CONTEXT })).toString("hex");
  await assert.rejects(alice.registerValidator({ consensusKey, p2pAddr, httpAddr, bond: minBond, pop: badPop }), /pop/);
  const reg = await alice.registerValidator({ consensusKey, p2pAddr, httpAddr, bond: minBond, pop });
  console.log("✓", reg.summary);
  const after = await client.validators();
  assert.ok(after.pending.some((v: any) => v.validator.address === node.address), "registered validator is pending");
  await assert.rejects(alice.approveValidator(node.address), /only validator admins/);
  await assert.rejects(alice.unbond(node.address), /only removed validators/);
  console.log(`✓ validator pending admin approval (epoch ${after.epoch}, ${after.blocks_until_next_epoch} blocks to the next one)`);
} else {
  console.log(`(skipping validator registration: min bond ${vinfo.min_bond_display} exceeds the faucet drip)`);
}

// Hunts: bob escrows a bounty on a GitHub issue, alice chips in, bob awards it to alice's
// Familiar (an agent session key she registered).
const agent2 = Wallet.generate(client);
await alice.addSessionKey(agent2.keyId, "claude", { limit: "1", periodBlocks: 100, expiresInBlocks: 1000 });
console.log("✓", (await alice.registerFamiliar(agent2.keyId, "claude")).summary);
const created = await bob.createHunt({ repo: "jackdishman/blockula", issue: 1, amount: "10", title: "Shared tx test vectors", expiresInBlocks: 500 });
console.log("✓", created.summary);
const huntId = (await client.account(bob.address)).hunts_created.at(-1);
console.log("✓", (await alice.fundHunt(huntId, "1")).summary);
await assert.rejects(alice.awardHunt(huntId, alice.address, 7), /creator or a validator admin/);
const awarded = await bob.awardHunt(huntId, `${name}.boo`, 42, agent2.keyId);
console.log("✓", awarded.summary);
const h = await client.hunt(huntId);
assert.equal(h.status, "awarded");
assert.equal(h.amount_display, "11 FANG");
assert.equal(h.familiar, "claude");
assert.equal(h.pr_url, "https://github.com/jackdishman/blockula/pull/42");
await assert.rejects(bob.cancelHunt(huntId), /already/);
const prof = await client.account(alice.address);
assert.equal(prof.hunts_won.length, 1);
assert.equal(prof.familiars[0].label, "claude");
// GitHub link: a claim on-chain, verified off-chain by the node (this gist id doesn't exist).
console.log("✓", (await alice.linkGithub(`alice${Date.now() % 100000}`, "0123456789abcdef")).summary);
assert.equal((await client.github((await client.account(alice.address)).github.handle)).verified, false);
console.log("✓ hunts, Familiars and GitHub links work");

const status = await client.status();
console.log(`✓ all good at height ${status.height}, burned so far: ${status.burned_display}`);
