import { formatFang } from "@blockula/sdk";

/** "123.456789 FANG" → "123.4568" with a separate unit, for big displays. */
export function fangParts(drops: string | bigint) {
  const s = formatFang(BigInt(drops)).replace(/ FANG$/, "");
  const [w, f] = s.split(".");
  const whole = BigInt(w).toLocaleString("en-US");
  return { whole, frac: f ? f.slice(0, 4).replace(/0+$/, "") : "" };
}

export const shortAddr = (a: string) => (a.length > 18 ? `${a.slice(0, 10)}…${a.slice(-6)}` : a);

/** A short, human-checkable code for a 64-hex key id, e.g. "A1B2-C3D4-…-9F8E". */
export const keyCode = (keyId: string) => {
  const k = keyId.toUpperCase();
  return `${k.slice(0, 4)}-${k.slice(4, 8)}-${k.slice(-4)}`;
};

export const isKeyId = (s: string) => /^[0-9a-f]{64}$/.test(s.trim().toLowerCase());

/** Rough human duration for a block count at the chain's block time. */
export function blocksToHuman(blocks: number, blockTimeMs = 1000) {
  const s = (blocks * blockTimeMs) / 1000;
  if (s < 90) return `${Math.round(s)}s`;
  if (s < 5400) return `${Math.round(s / 60)} min`;
  if (s < 86400) return `${Math.round(s / 3600)} h`;
  const d = Math.round(s / 86400);
  return d === 1 ? "day" : `${d} days`;
}
