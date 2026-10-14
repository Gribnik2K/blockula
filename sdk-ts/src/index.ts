import { ml_dsa65 } from "@noble/post-quantum/ml-dsa.js";
import { blake3 } from "@noble/hashes/blake3.js";
import { bytesToHex, hexToBytes, randomBytes, utf8ToBytes } from "@noble/hashes/utils.js";
import { bech32m } from "@scure/base";
import { minFee, parseFang, signingBytes, type Action, type KeyRole, type Tx, type UnsignedTx } from "./tx.js";

export * from "./tx.js";

export const DEFAULT_RPC = "http://localhost:7070";

export function addressFromPublicKey(publicKey: Uint8Array): string {
  return bech32m.encode("boo", bech32m.toWords(blake3(publicKey)));
}

/** A key's id: BLAKE3(public key) as hex. Same 32 bytes as inside the `boo1…` address it derives. */
export function keyId(publicKey: Uint8Array): string {
  return bytesToHex(blake3(publicKey));
}

export function isAddress(s: string): boolean {
  try {
    const { prefix, words } = bech32m.decode(s as `${string}1${string}`);
    return prefix === "boo" && bech32m.fromWords(words).length === 32;
  } catch {
    return false;
  }
}

/**
 * Wallet = one ML-DSA-65 key (from a 32-byte seed, FIPS 204 ξ) acting for one account.
 * By default the account is the key's own address. Call `actAs(account)` to use a key that
 * was added to another account (a second device, or an agent session key).
 */
export class Wallet {
  readonly publicKey: Uint8Array;
  /** This key's own address. */
  readonly address: string;
  /** The account this wallet acts for (defaults to `address`). */
  account: string;
  /** WebAuthn credential id, when the wallet was unlocked by a passkey. Not secret. */
  credentialId?: Uint8Array;
  private readonly secretKey: Uint8Array;

  private constructor(readonly seed: Uint8Array, public client: Client = new Client()) {
    const keys = ml_dsa65.keygen(seed);
    this.publicKey = keys.publicKey;
    this.secretKey = keys.secretKey;
    this.address = addressFromPublicKey(keys.publicKey);
    this.account = this.address;
  }

  get keyId() {
    return keyId(this.publicKey);
  }

  /** Act for another account this key has been authorized on. Accepts a name or address. */
  async actAs(account: string) {
    this.account = await this.client.resolve(account);
    return this;
  }

  static generate(client?: Client) {
    return new Wallet(randomBytes(32), client);
  }

  static fromSeed(seed: string | Uint8Array, client?: Client) {
    const bytes = typeof seed === "string" ? hexToBytes(seed) : seed;
    if (bytes.length !== 32) throw new Error("seed must be 32 bytes");
    return new Wallet(bytes, client);
  }

  /**
   * Derive a wallet from a WebAuthn PRF output. The passkey never signs transactions
   * (its P-256 signatures aren't quantum-safe); it only unlocks this ML-DSA key.
   */
  static fromPasskeyPrf(prfOutput: Uint8Array, client?: Client) {
    return new Wallet(blake3(prfOutput, { context: utf8ToBytes("blockula passkey wallet v1") }), client);
  }

  get seedHex() {
    return bytesToHex(this.seed);
  }

  sign(tx: UnsignedTx): Tx {
    const signature = ml_dsa65.sign(signingBytes(tx), this.secretKey);
    return { ...tx, public_key: bytesToHex(this.publicKey), signature: bytesToHex(signature) };
  }

  async build(action: Action, memo = ""): Promise<Tx> {
    const [status, account] = await Promise.all([this.client.status(), this.client.account(this.account).catch(() => null)]);
    const unsigned: UnsignedTx = { chain_id: status.chain_id, from: this.account, nonce: account?.next_nonce ?? 0, fee: "0", action, memo };
    unsigned.fee = minFee(unsigned).toString();
    return this.sign(unsigned);
  }

  /** Send FANG to a `.boo` name or `boo1…` address. `amount` is in FANG ("1.5") or bigint drops. */
  async send(to: string, amount: string | number | bigint, opts: { memo?: string; wait?: boolean } = {}) {
    const tx = await this.build({ type: "transfer", to, amount: parseFang(amount).toString() }, opts.memo);
    return this.client.submit(tx, opts.wait ?? true);
  }

  async registerName(name: string, opts: { wait?: boolean } = {}) {
    const tx = await this.build({ type: "register_name", name });
    return this.client.submit(tx, opts.wait ?? true);
  }

  /** Authorize another device's key (e.g. a second passkey) with full control. */
  async addOwnerKey(keyIdHex: string, label: string) {
    return this.client.submit(await this.build({ type: "add_key", key_id: keyIdHex, label, role: { type: "owner" } }));
  }

  /**
   * Let an agent/app key spend from this account, transfers only, within a limit.
   * Example: `addSessionKey(agentKeyId, "claude", { limit: "10", periodBlocks: 86_400, expiresInBlocks: 30 * 86_400 })`.
   */
  async addSessionKey(keyIdHex: string, label: string, opts: { limit: string | number | bigint; periodBlocks: number; expiresInBlocks: number }) {
    const { height } = await this.client.status();
    const role: KeyRole = { type: "session", limit: parseFang(opts.limit).toString(), period_blocks: opts.periodBlocks, expires_at: height + opts.expiresInBlocks };
    return this.client.submit(await this.build({ type: "add_key", key_id: keyIdHex, label, role }));
  }

  async removeKey(keyIdHex: string) {
    return this.client.submit(await this.build({ type: "remove_key", key_id: keyIdHex }));
  }

  balance() {
    return this.client.account(this.account).then((a) => a.balance_display as string, () => "0 FANG");
  }

  /** Dev/test chains only: drip 100 FANG to this wallet. */
  faucet(wait = true) {
    return this.client.faucet(this.account, wait);
  }
}

export class Client {
  constructor(readonly rpc: string = DEFAULT_RPC) {}

  private async call(path: string, body?: unknown) {
    const res = await fetch(this.rpc.replace(/\/$/, "") + "/api" + path, body === undefined ? undefined : { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
    const json = await res.json();
    if (!res.ok) throw new Error(json.error ?? res.statusText);
    return json;
  }

  status() { return this.call("/status"); }
  blocks(limit = 20) { return this.call(`/blocks?limit=${limit}`); }
  block(heightOrHash: number | string) { return this.call(`/block/${heightOrHash}`); }
  tx(id: string) { return this.call(`/tx/${id}`); }
  account(addressOrName: string) { return this.call(`/account/${encodeURIComponent(addressOrName)}`); }
  name(name: string) { return this.call(`/name/${encodeURIComponent(name)}`); }
  /** Accounts a key is authorized on (its own address, plus any it was added to). */
  keyAccounts(keyIdHex: string) { return this.call(`/key/${keyIdHex}`); }
  search(q: string) { return this.call(`/search/${encodeURIComponent(q)}`); }

  async resolve(nameOrAddress: string): Promise<string> {
    if (isAddress(nameOrAddress)) return nameOrAddress;
    const r = await this.name(nameOrAddress);
    if (r.available) throw new Error(`${r.name} isn't registered`);
    return r.owner.address;
  }

  async submit(tx: Tx, wait = true) {
    const { id } = await this.call("/tx", tx);
    return wait ? this.waitForTx(id) : this.tx(id);
  }

  async faucet(to: string, wait = true) {
    const { id } = await this.call("/faucet", { to });
    return wait ? this.waitForTx(id) : this.tx(id);
  }

  async waitForTx(id: string, timeoutMs = 15_000) {
    const start = Date.now();
    for (;;) {
      const tx = await this.tx(id);
      if (tx.status === "finalized") return tx;
      if (Date.now() - start > timeoutMs) throw new Error(`tx ${id} still pending after ${timeoutMs}ms`);
      await new Promise((r) => setTimeout(r, 250));
    }
  }
}
