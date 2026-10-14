# Blockula plan

## Decisions (locked)

| Decision | Choice |
|---|---|
| Name / token | Blockula / FANG (1 FANG = 10⁹ drops) |
| Names | `.boo`, native to the protocol |
| Node | Rust; multi-validator consensus on **Commonware** primitives (Phase 1) |
| Execution | **Native modules only** in v1 (accounts, transfers, names, shielded pool, storage, staking). WASM/RISC-V contracts in v2. No EVM (it's bound to ECDSA). |
| L1 vs L2 | Standalone L1, because settling on Ethereum would bring back its ECDSA/BLS weakness |

## Why this is quantum-safe

- **Signatures:** ML-DSA-65 (FIPS 204) for users and validators. SLH-DSA (FIPS 205, hash-only) for cold recovery keys.
- **Hashes:** BLAKE3-256. Grover's algorithm only halves security, and 256-bit addresses keep ~128-bit collision resistance even against quantum attacks.
- **Encryption:** ML-KEM-768 (FIPS 203) for note encryption and The Crypt.
- **ZK:** hash-based STARKs (FRI + Poseidon2, e.g. Plonky3) only. No Groth16/KZG, because pairings fall to Shor's algorithm.
- **Passkeys:** WebAuthn signs with P-256, which is not quantum-safe, so the passkey never signs on-chain. Its **PRF extension** output (symmetric) derives the ML-DSA key: `seed = BLAKE3-KDF("blockula passkey wallet v1", prf)`. Implemented in `sdk-ts/src/passkey.ts`.

## Accounts

Native account abstraction (Phase 1). Each account holds a list of authorized keys with roles:
- **device:** passkey-derived ML-DSA keys (one per passkey provider)
- **recovery:** SLH-DSA cold key (downloadable recovery kit) and/or guardians (social recovery)
- **agent session:** ML-DSA key with spend limit per day, expiry and allowed modules. Replaces today's client-side `BLOCKULA_MAX_SEND`.
- **Fallback:** for authenticators without PRF, a password-encrypted key, clearly labeled as weaker.

## 👻 Ghost Mode (private transfers) and 🪦 The Crypt (private storage), Phase 2

- An Orchard-style shielded pool: notes are Poseidon2 commitments, spending publishes a nullifier, and a STARK proves validity. Note contents are encrypted to the recipient with ML-KEM.
- **Viewing keys** for selective disclosure (accountants, auditors, compliance).
- Proofs are about 50–200 KB, so validators **recursively aggregate** a block's proofs into one.
- The explorer shows "👻 a ghost transaction happened" (nullifier, fee, proof size), never parties or amounts. The tutorial walks users through exactly this.
- **The Crypt:** blobs are ML-KEM-encrypted and stored by nodes. On-chain: commitment + access list + storage deposit. Sharing re-encrypts the data key to another user.
- Later: private names, private voting, selective-disclosure credentials ("over 18" without revealing a birthday).

## Tokenomics (FANG)

- **Fees:** the per-byte base fee is **burned** 🔥 (live in Phase 0); tips go to the proposer. Priced by bytes because post-quantum signatures are about 3.3 KB.
- **Issuance:** starts at ~6% a year to stakers, decays to a permanent **1.5% tail emission** so security never depends on fees alone.
- **Names:** length-based price (3 chars: 100, 4 chars: 25, 5+ chars: 5 FANG) to the treasury (live). Phase 1 adds annual renewal at 20% of price so squatters bleed.
- **State rent:** stored data and names hold a refundable deposit, which caps state growth.
- **Treasury:** ~10% of issuance plus name fees. Pays for sponsored onboarding txs (new users need zero FANG), faucets, audits and grants.
- **Ghost Mode:** a small verification premium, partly offset by aggregation.
- **Health targets:** ≥50% staked, Nakamoto coefficient ≥10, slashing for double-signing (heavy) and downtime (light).
- Get legal review before any sale or distribution.

## Network and sync

- `blockula summon <id>`: new chain. `blockula run`: run it. `blockula dev`: local chain with faucet. (All live.)
- Phase 1 adds `blockula join <network>`:
  1. Download a state snapshot and verify it against a validator-signed checkpoint (later, a STARK proof of the chain's history).
  2. Serve the built-in explorer **immediately**.
  3. Backfill history in the background.
- Light-client mode keeps no full state at all.
- Validators: about 100, ML-DSA signatures. There is no BLS-style aggregation, so keep the set small and add STARK signature aggregation later.

## Roadmap

| Phase | Scope | Status |
|---|---|---|
| 0 | Dev node, ML-DSA accounts, transfers, `.boo` names, fee burn, built-in explorer + tutorial, TS SDK, passkey helpers, MCP server, `summon`/`run` | ✅ built |
| 1 | Multi-validator BFT (Commonware), p2p + snapshot sync + `join`, account abstraction + session keys, standalone web wallet (Next.js) with passkey onboarding, indexer + public explorer, public testnet, npm publish | next |
| 2 | Ghost Mode + viewing keys, The Crypt, proof aggregation | |
| 3 | Staking/slashing, tokenomics live, audits, mainnet | |
| 4 | WASM/RISC-V contracts, light clients, STARK signature aggregation | |

## Phase 0 known limits

- One validator, no p2p. Blocks are signed and replay-verified on restart, but there's no fork choice.
- State is cloned per tx and the state root is a flat hash (fine for dev; Phase 1 moves to a Merkle tree for light-client proofs).
- The faucet is unthrottled (dev only).
- `sdk-ts/src/passkey.ts` typechecks but hasn't been exercised in a real browser yet.
