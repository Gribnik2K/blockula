// Browser-only: passkey-backed Blockula wallets via the WebAuthn PRF extension.
//
// The passkey (Face ID / Touch ID / security key) produces a symmetric PRF secret,
// which deterministically derives the ML-DSA-65 wallet key. Same passkey → same wallet
// on every synced device, with no seed phrase. The passkey's own P-256 signature is never
// used on-chain, so the account stays quantum-safe.

import { Client, Wallet } from "./index.js";

const PRF_SALT = new TextEncoder().encode("blockula/wallet/v1");

/**
 * The WebAuthn relying-party id. Passkeys (and so wallets) are bound to it: a wallet created
 * under one rpId can't be unlocked under another. Pin it to your production domain; it must be
 * the page's hostname or a registrable parent of it.
 */
function rpId(explicit?: string) {
  return explicit ?? location.hostname;
}

type PrfResults = { prf?: { enabled?: boolean; results?: { first?: ArrayBuffer } } };

function walletFrom(cred: PublicKeyCredential | null, prf: Uint8Array, client?: Client) {
  const wallet = Wallet.fromPasskeyPrf(prf, client);
  if (cred) wallet.credentialId = new Uint8Array(cred.rawId);
  return wallet;
}

async function prfFromCredential(cred: PublicKeyCredential | null): Promise<Uint8Array> {
  const ext = cred?.getClientExtensionResults() as PrfResults | undefined;
  const first = ext?.prf?.results?.first;
  if (!first) {
    throw new Error("This passkey provider doesn't support the PRF extension. Try iCloud Keychain, Google Password Manager or 1Password.");
  }
  return new Uint8Array(first);
}

/** Create a new passkey and the wallet it unlocks. */
export async function createPasskeyWallet(opts: { username: string; client?: Client; rpName?: string; rpId?: string }): Promise<Wallet> {
  const cred = (await navigator.credentials.create({
    publicKey: {
      rp: { id: rpId(opts.rpId), name: opts.rpName ?? "Blockula" },
      user: { id: crypto.getRandomValues(new Uint8Array(16)), name: opts.username, displayName: opts.username },
      challenge: crypto.getRandomValues(new Uint8Array(32)),
      pubKeyCredParams: [{ type: "public-key", alg: -7 }, { type: "public-key", alg: -8 }, { type: "public-key", alg: -257 }],
      authenticatorSelection: { residentKey: "required", userVerification: "required" },
      extensions: { prf: { eval: { first: PRF_SALT } } } as AuthenticationExtensionsClientInputs,
    },
  })) as PublicKeyCredential | null;

  // Some authenticators only return PRF output on get(), not create(). Only retry with get()
  // when the authenticator said PRF is enabled, so PRF-less providers fail fast with one prompt.
  const ext = cred?.getClientExtensionResults() as PrfResults | undefined;
  if (ext?.prf?.results?.first) return walletFrom(cred, new Uint8Array(ext.prf.results.first), opts.client);
  if (ext?.prf?.enabled) return unlockPasskeyWallet({ client: opts.client, rpId: opts.rpId, credentialId: cred ? new Uint8Array(cred.rawId) : undefined });
  return walletFrom(cred, await prfFromCredential(cred), opts.client);
}

/** Unlock an existing passkey wallet (shows the OS passkey picker). */
export async function unlockPasskeyWallet(opts: { client?: Client; credentialId?: Uint8Array; rpId?: string } = {}): Promise<Wallet> {
  const cred = (await navigator.credentials.get({
    publicKey: {
      rpId: rpId(opts.rpId),
      challenge: crypto.getRandomValues(new Uint8Array(32)),
      userVerification: "required",
      allowCredentials: opts.credentialId ? [{ type: "public-key", id: opts.credentialId as BufferSource }] : [],
      extensions: { prf: { eval: { first: PRF_SALT } } } as AuthenticationExtensionsClientInputs,
    },
  })) as PublicKeyCredential | null;
  return walletFrom(cred, await prfFromCredential(cred), opts.client);
}
