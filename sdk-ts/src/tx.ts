// Canonical transaction encoding. Must stay byte-identical to node/src/types.rs `Tx::signing_bytes`.

export type KeyRole =
  | { type: "owner" }
  /** Transfers only, at most `limit` drops (amount + fee) per `period_blocks`, until block `expires_at`. */
  | { type: "session"; limit: string; period_blocks: number; expires_at: number };

export type Action =
  | { type: "transfer"; to: string; amount: string }
  | { type: "register_name"; name: string }
  | { type: "add_key"; key_id: string; label: string; role: KeyRole }
  | { type: "remove_key"; key_id: string }
  /** Bond `bond` drops and offer `consensus_key` (hex ML-DSA-65 public key) as a validator. */
  | {
      type: "register_validator"; consensus_key: string; p2p_addr: string; http_addr: string; bond: string;
      /** Proof of possession: the consensus key's ML-DSA-65 signature over `popMessage(...)` with context `POP_CONTEXT` (see `blockula validator pop`). */
      pop: string;
    }
  /** Season 0 allowlist (admin accounts only). */
  | { type: "approve_validator"; validator: string }
  | { type: "remove_validator"; validator: string }
  /** Reclaim the bond of a removed validator you own, one epoch after it left. */
  | { type: "unbond"; validator: string };

export interface UnsignedTx {
  chain_id: string;
  /** The account acting; the signing key must be authorized on it. */
  from: string;
  nonce: number;
  fee: string;
  action: Action;
  memo: string;
}

export interface Tx extends UnsignedTx {
  public_key: string;
  signature: string;
}

export const PUBLIC_KEY_LEN = 1952;
export const SIGNATURE_LEN = 3309;
export const DROPS_PER_FANG = 1_000_000_000n;

const utf8 = new TextEncoder();

class Writer {
  private parts: number[] = [];
  bytes(b: Uint8Array) { for (const x of b) this.parts.push(x); }
  u8(n: number) { this.parts.push(n & 0xff); }
  u16(n: number) { this.u8(n); this.u8(n >> 8); }
  u64(n: bigint) { for (let i = 0n; i < 8n; i++) this.u8(Number((n >> (8n * i)) & 0xffn)); }
  str(s: string) { const b = utf8.encode(s); this.u16(b.length); this.bytes(b); }
  done() { return Uint8Array.from(this.parts); }
}

export function signingBytes(tx: UnsignedTx): Uint8Array {
  const w = new Writer();
  w.bytes(utf8.encode("BLOCKULA_TX_V2"));
  w.str(tx.chain_id);
  w.str(tx.from);
  w.u64(BigInt(tx.nonce));
  w.u64(BigInt(tx.fee));
  const a = tx.action;
  switch (a.type) {
    case "transfer":
      w.u8(0); w.str(a.to); w.u64(BigInt(a.amount));
      break;
    case "register_name":
      w.u8(1); w.str(a.name);
      break;
    case "add_key":
      w.u8(2); w.str(a.key_id); w.str(a.label);
      if (a.role.type === "owner") w.u8(0);
      else { w.u8(1); w.u64(BigInt(a.role.limit)); w.u64(BigInt(a.role.period_blocks)); w.u64(BigInt(a.role.expires_at)); }
      break;
    case "remove_key":
      w.u8(3); w.str(a.key_id);
      break;
    case "register_validator":
      w.u8(4); w.str(a.consensus_key); w.str(a.p2p_addr); w.str(a.http_addr); w.u64(BigInt(a.bond)); w.str(a.pop);
      break;
    case "approve_validator":
      w.u8(5); w.str(a.validator);
      break;
    case "remove_validator":
      w.u8(6); w.str(a.validator);
      break;
    case "unbond":
      w.u8(7); w.str(a.validator);
      break;
  }
  w.str(tx.memo);
  return w.done();
}

/** FIPS 204 context string for validator proofs of possession (node/src/types.rs `POP_CONTEXT`). */
export const POP_CONTEXT = utf8.encode("blockula/validator-pop/v1");

/** What a validator's consensus key signs to register (node/src/types.rs `pop_message`). */
export function popMessage(chainId: string, owner: string, p2pAddr: string, httpAddr: string): Uint8Array {
  const w = new Writer();
  w.bytes(utf8.encode("BLOCKULA_VALIDATOR_POP_V1"));
  for (const s of [chainId, owner, p2pAddr, httpAddr]) w.str(s);
  return w.done();
}

/** Mirrors `min_fee` in node/src/types.rs: base fee is priced per byte and burned. */
export function minFee(tx: UnsignedTx): bigint {
  const size = signingBytes(tx).length + PUBLIC_KEY_LEN + SIGNATURE_LEN;
  return 10_000n + BigInt(size) * 10n;
}

/** "1.5" → 1500000000n drops. Accepts numbers, decimal strings or bigint drops. */
export function parseFang(amount: string | number | bigint): bigint {
  if (typeof amount === "bigint") return amount;
  const s = String(amount).trim();
  if (!/^\d+(\.\d{1,9})?$/.test(s)) throw new Error(`invalid FANG amount: ${s}`);
  const [whole, frac = ""] = s.split(".");
  return BigInt(whole) * DROPS_PER_FANG + BigInt(frac.padEnd(9, "0"));
}

export function formatFang(drops: bigint | string): string {
  const d = BigInt(drops);
  const whole = d / DROPS_PER_FANG;
  const frac = (d % DROPS_PER_FANG).toString().padStart(9, "0").replace(/0+$/, "");
  return `${whole}${frac ? "." + frac : ""} FANG`;
}
