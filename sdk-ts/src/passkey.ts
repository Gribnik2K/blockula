// Browser-only: passkey-backed Blockula wallets via the WebAuthn PRF extension.
//
// The passkey (Face ID / Touch ID / security key) produces a symmetric PRF secret,
// which deterministically derives the ML-DSA-65 wallet key. Same passkey → same wallet
// on every synced device, with no seed phrase. The passkey's own P-256 signature is never
// used on-chain, so the account stays quantum-safe.

import { Client, Wallet } from "./index.js";

const PRF_SALT = new TextEncoder().encode("blockula/wallet/v1");

function rpId() {
  return location.hostname;
}

async function prfFromCredential(cred: PublicKeyCredential | null): Promise<Uint8Array> {
  const ext = cred?.getClientExtensionResults() as { prf?: { results?: { first?: ArrayBuffer } } } | undefined;
  const first = ext?.prf?.results?.first;
  if (!first) {
    throw new Error("This passkey provider doesn't support the PRF extension. Try iCloud Keychain, Google Password Manager or 1Password.");
  }
  return new Uint8Array(first);
}

/** Create a new passkey and the wallet it unlocks. */
export async function createPasskeyWallet(opts: { username: string; client?: Client; rpName?: string }): Promise<Wallet> {
  const cred = (await navigator.credentials.create({
    publicKey: {
      rp: { id: rpId(), name: opts.rpName ?? "Blockula" },
      user: { id: crypto.getRandomValues(new Uint8Array(16)), name: opts.username, displayName: opts.username },
      challenge: crypto.getRandomValues(new Uint8Array(32)),
      pubKeyCredParams: [{ type: "public-key", alg: -7 }, { type: "public-key", alg: -8 }, { type: "public-key", alg: -257 }],
      authenticatorSelection: { residentKey: "required", userVerification: "required" },
      extensions: { prf: { eval: { first: PRF_SALT } } } as AuthenticationExtensionsClientInputs,
    },
  })) as PublicKeyCredential | null;

  // Some authenticators only return PRF output on get(), not create().
  try {
    return Wallet.fromPasskeyPrf(await prfFromCredential(cred), opts.client);
  } catch {
    return unlockPasskeyWallet({ client: opts.client, credentialId: cred ? new Uint8Array(cred.rawId) : undefined });
  }
}

/** Unlock an existing passkey wallet (shows the OS passkey picker). */
export async function unlockPasskeyWallet(opts: { client?: Client; credentialId?: Uint8Array } = {}): Promise<Wallet> {
  const cred = (await navigator.credentials.get({
    publicKey: {
      rpId: rpId(),
      challenge: crypto.getRandomValues(new Uint8Array(32)),
      userVerification: "required",
      allowCredentials: opts.credentialId ? [{ type: "public-key", id: opts.credentialId as BufferSource }] : [],
      extensions: { prf: { eval: { first: PRF_SALT } } } as AuthenticationExtensionsClientInputs,
    },
  })) as PublicKeyCredential | null;
  return Wallet.fromPasskeyPrf(await prfFromCredential(cred), opts.client);
}
