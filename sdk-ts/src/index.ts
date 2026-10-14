import { ml_dsa65 } from "@noble/post-quantum/ml-dsa.js";
import { blake3 } from "@noble/hashes/blake3.js";
import { bytesToHex, hexToBytes, randomBytes, utf8ToBytes } from "@noble/hashes/utils.js";
import { bech32m } from "@scure/base";
import { minFee, parseFang, signingBytes, type Action, type Tx, type UnsignedTx } from "./tx.js";

export * from "./tx.js";

export const DEFAULT_RPC = "http://localhost:7070";

export function addressFromPublicKey(publicKey: Uint8Array): string {
  return bech32m.encode("boo", bech32m.toWords(blake3(publicKey)));
}

export function isAddress(s: string): boolean {
  try {
    const { prefix, words } = bech32m.decode(s as `${string}1${string}`);
    return prefix === "boo" && bech32m.fromWords(words).length === 32;
  } catch {
    return false;
  }
}

/** Wallet = one ML-DSA-65 keypair derived from a 32-byte seed (FIPS 204 ξ). */
export class Wallet {
  readonly publicKey: Uint8Array;
  readonly address: string;
  private readonly secretKey: Uint8Array;

  private constructor(readonly seed: Uint8Array, public client: Client = new Client()) {
    const keys = ml_dsa65.keygen(seed);
    this.publicKey = keys.publicKey;
    this.secretKey = keys.secretKey;
    this.address = addressFromPublicKey(keys.publicKey);
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
    const [status, account] = await Promise.all([this.client.status(), this.client.account(this.address).catch(() => null)]);
    const unsigned: UnsignedTx = { chain_id: status.chain_id, nonce: account?.next_nonce ?? 0, fee: "0", action, memo };
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

  balance() {
    return this.client.account(this.address).then((a) => a.balance_display as string, () => "0 FANG");
  }

  /** Dev/test chains only: drip 100 FANG to this wallet. */
  faucet(wait = true) {
    return this.client.faucet(this.address, wait);
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
