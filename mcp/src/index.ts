#!/usr/bin/env node
// 🧛 Blockula MCP server. Add to Claude Code with:
//   claude mcp add blockula -- npx -y @blockula/mcp
//
// Env:
//   BLOCKULA_RPC       node URL (default http://localhost:7070)
//   BLOCKULA_HOME      where the agent wallet lives (default ~/.blockula)
//   BLOCKULA_MAX_SEND  max FANG per send() call (default 100) — a guardrail until on-chain session keys land
//   BLOCKULA_BIN       path to the `blockula` binary (default: found on PATH)

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { Client, Wallet, parseFang } from "@blockula/sdk";
import { spawn, execFileSync } from "node:child_process";
import { existsSync, mkdirSync, openSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { z } from "zod";

const RPC = process.env.BLOCKULA_RPC ?? "http://localhost:7070";
const HOME = process.env.BLOCKULA_HOME ?? join(homedir(), ".blockula");
const WALLET_FILE = join(HOME, "agent-wallet.json");
const MAX_SEND = parseFang(process.env.BLOCKULA_MAX_SEND ?? "100");
const BIN = process.env.BLOCKULA_BIN ?? "blockula";

const client = new Client(RPC);
const server = new McpServer({ name: "blockula", version: "0.1.0" });

const text = (v: unknown) => ({ content: [{ type: "text" as const, text: typeof v === "string" ? v : JSON.stringify(v, null, 2) }] });

interface WalletFile { address: string; key_id: string; seed: string; account?: string }

function readWalletFile(): WalletFile | null {
  return existsSync(WALLET_FILE) ? JSON.parse(readFileSync(WALLET_FILE, "utf8")) : null;
}

function loadWallet(): Wallet | null {
  const f = readWalletFile();
  if (!f) return null;
  const w = Wallet.fromSeed(f.seed, client);
  if (f.account) w.account = f.account;
  return w;
}

function requireWallet(): Wallet {
  const w = loadWallet();
  if (!w) throw new Error("No agent wallet yet — call create_wallet first.");
  return w;
}

async function nodeUp() {
  try { await client.status(); return true; } catch { return false; }
}

function tool<S extends z.ZodRawShape>(name: string, description: string, shape: S, run: (args: z.infer<z.ZodObject<S>>) => Promise<unknown>) {
  server.registerTool(name, { description, inputSchema: shape }, (async (args: z.infer<z.ZodObject<S>>) => {
    try {
      return text(await run(args));
    } catch (e) {
      const msg = (e as Error).message;
      const hint = /fetch failed|ECONNREFUSED/.test(msg) ? ` (no node at ${RPC} — call start_dev_node)` : "";
      return { ...text(`Error: ${msg}${hint}`), isError: true };
    }
  }) as never);
}

tool("status", "Blockula network status: chain id, height, supply, fees burned, and the crypto in use.", {}, () => client.status());

tool("start_dev_node", "Start a local Blockula dev chain (with faucet + built-in explorer) if one isn't already running. Requires the `blockula` binary.", { reset: z.boolean().optional().describe("wipe the dev chain first") }, async ({ reset }) => {
  if (!reset && (await nodeUp())) return `Already running at ${RPC} (explorer: ${RPC})`;
  const port = new URL(RPC).port || "7070";
  mkdirSync(HOME, { recursive: true });
  const log = openSync(join(HOME, "dev-node.log"), "a");
  const child = spawn(BIN, ["dev", "--port", port, ...(reset ? ["--reset"] : [])], { detached: true, stdio: ["ignore", log, log] });
  child.on("error", () => {});
  child.unref();
  for (let i = 0; i < 40; i++) {
    await new Promise((r) => setTimeout(r, 250));
    if (await nodeUp()) return `🧛 Dev chain is up. Explorer: ${RPC} · logs: ${join(HOME, "dev-node.log")}`;
  }
  throw new Error(`node didn't start — is \`${BIN}\` installed? See ${join(HOME, "dev-node.log")}`);
});

tool("summon_chain", "Create a brand-new Blockula chain (fresh genesis + validator key) that you can run as your own network.", { chain_id: z.string().regex(/^[a-z0-9-]{3,40}$/), faucet: z.boolean().default(true) }, async ({ chain_id, faucet }) => {
  const out = execFileSync(BIN, ["summon", chain_id, ...(faucet ? [] : ["--no-faucet"])], { encoding: "utf8" });
  return out.trim();
});

tool("create_wallet", "Create this agent's post-quantum (ML-DSA-65) wallet, stored locally. Returns the existing one if it already exists.", {}, async () => {
  let w = loadWallet();
  const existed = !!w;
  if (!w) {
    w = Wallet.generate(client);
    mkdirSync(HOME, { recursive: true });
    writeFileSync(WALLET_FILE, JSON.stringify({ address: w.address, key_id: w.keyId, seed: w.seedHex }, null, 2), { mode: 0o600 });
  }
  return {
    key_id: w.keyId,
    own_address: w.address,
    acting_for: w.account,
    existed,
    stored_at: WALLET_FILE,
    next_steps: "To spend from the user's account: ask them to open the Blockula web wallet → Devices & agents → 'Give an agent a budget' and paste this key_id. Then call use_account with their .boo name. (Or use faucet to fund this key's own address for testing.)",
  };
});

tool("use_account", "Act for a user's account (name or address) that has authorized this agent's key — usually as a session key with a spending limit. Pass 'self' to go back to the key's own address.", { account: z.string() }, async ({ account }) => {
  const f = readWalletFile();
  if (!f) throw new Error("No agent wallet yet — call create_wallet first.");
  const w = loadWallet()!;
  if (account === "self") {
    delete f.account;
  } else {
    const addr = await client.resolve(account);
    const { accounts } = await client.keyAccounts(w.keyId);
    const grant = accounts.find((a: { account: { address: string } }) => a.account.address === addr);
    if (!grant) throw new Error(`${account} hasn't authorized this key yet. Ask the user to add key_id ${w.keyId} in their wallet (Devices & agents).`);
    f.account = addr;
    writeFileSync(WALLET_FILE, JSON.stringify(f, null, 2), { mode: 0o600 });
    return { acting_for: grant.account.name ?? addr, role: grant.role, label: grant.label };
  }
  writeFileSync(WALLET_FILE, JSON.stringify(f, null, 2), { mode: 0o600 });
  return { acting_for: w.address };
});

tool("wallet_info", "This agent's address, balance, .boo names and recent activity.", {}, async () => {
  const w = requireWallet();
  const a = await client.account(w.account).catch(() => null);
  const mine = a?.keys?.find((k: { key_id: string }) => k.key_id === w.keyId);
  return { key_id: w.keyId, acting_for: a?.account?.name ?? w.account, my_permissions: mine ?? "not authorized on this account", address: w.account, balance: a?.balance_display ?? "0 FANG", names: a?.names ?? [], recent: (a?.recent_txs ?? []).slice(0, 10).map((t: { summary: string; id: string }) => `${t.summary} (${t.id.slice(0, 12)}…)`) };
});

tool("faucet", "Get 100 test FANG on a dev/test chain (to this agent's wallet, or any name/address).", { to: z.string().optional() }, async ({ to }) => {
  const target = to ?? requireWallet().account;
  const tx = await client.faucet(target);
  return tx.summary;
});

tool("send", "Send FANG to a .boo name or boo1… address. Amounts are in FANG, e.g. \"1.5\".", { to: z.string(), amount: z.string(), memo: z.string().max(256).optional() }, async ({ to, amount, memo }) => {
  if (parseFang(amount) > MAX_SEND) throw new Error(`amount exceeds this agent's per-send limit (BLOCKULA_MAX_SEND=${process.env.BLOCKULA_MAX_SEND ?? "100"} FANG)`);
  const tx = await requireWallet().send(to, amount, { memo });
  return { summary: tx.summary, fee: tx.fee_display, tx_id: tx.id, block: tx.height, explorer: `${RPC}/#/tx/${tx.id}` };
});

tool("check_name", "Check whether a .boo name is available and what it costs.", { name: z.string() }, ({ name }) => client.name(name));

tool("register_name", "Claim a .boo name for this agent's wallet (3–32 chars, a-z 0-9 -). Shorter names cost more.", { name: z.string() }, async ({ name }) => {
  const tx = await requireWallet().registerName(name);
  return { summary: tx.summary, tx_id: tx.id, explorer: `${RPC}/#/tx/${tx.id}` };
});

tool("lookup", "Look up anything — a .boo name, boo1… address, block number/hash or tx id — and get a plain-English explanation.", { query: z.string() }, async ({ query }) => {
  const hit = await client.search(query);
  const data = hit.type === "block" ? await client.block(hit.id) : hit.type === "tx" ? await client.tx(hit.id) : hit.type === "account" ? await client.account(hit.id) : await client.name(hit.id);
  return { type: hit.type, explorer: `${RPC}/#/${hit.type}/${hit.id}`, ...data };
});

tool("latest_blocks", "The most recent blocks, summarized.", { limit: z.number().int().min(1).max(50).default(10) }, async ({ limit }) => {
  const blocks = await client.blocks(limit);
  return blocks.map((b: { summary: string; hash: string }) => `${b.summary} · ${b.hash.slice(0, 12)}…`);
});

await server.connect(new StdioServerTransport());
