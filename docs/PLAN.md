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
- **Issuance:** starts at ~6% a year, decays to a permanent **1.5% tail emission** so security never depends on fees alone. Split 70% stakers / 20% contribution pool / 10% treasury (see Proof of Contribution).
- **Names:** length-based price (3 chars: 100, 4 chars: 25, 5+ chars: 5 FANG) to the treasury (live). Phase 1 adds annual renewal at 20% of price so squatters bleed.
- **State rent:** stored data and names hold a refundable deposit, which caps state growth.
- **Treasury:** 10% of issuance plus name fees. Pays for sponsored onboarding txs (new users need zero FANG), faucets, audits and grants.
- **Ghost Mode:** a small verification premium, partly offset by aggregation.
- **Health targets:** ≥50% staked, Nakamoto coefficient ≥10, slashing for double-signing (heavy) and downtime (light).
- Get legal review before any sale or distribution.

## 🪦 Proof of Contribution (rewarding useful work)

**Principle:** anything counted automatically gets farmed (commit counts, lines changed, even merged PRs). So rewards come from **people with something at stake**, are paid from a **fixed per-epoch pool** (spam dilutes other contributors instead of printing tokens), and **vest slowly** so they can be clawed back. This is separate from consensus: block production stays pure proof-of-stake, because mixing in reputation would give attackers a cheaper path to control.

### Pieces
- **Souls:** non-transferable reputation. Earned slowly, decays over time, can't be bought. It boosts reward share, jury eligibility and voting weight.
- **Identity:** link GitHub by posting your key id in a public gist; validators check it independently. New identities need a **vouch** from someone with Soul. Vouchers earn a small cut of the newcomer's early rewards, and lose stake and Soul if the newcomer turns out to be a sybil.
- **Repos:** registered with a stake plus a governance vote. Blockula's own repo is #1, so building Blockula earns FANG.
- **Claims:** "PR #123, merge SHA, my GitHub id" plus a small refundable bond. Validators each check the GitHub API, and a claim needs most of them (e.g. 2/3) to agree. A PR pays only after **30 days without a revert**.
- **Séances (review juries):** each epoch, a random jury of staked reviewers is drawn. The randomness comes from block hashes (quantum-safe). Jurors **commit, then reveal** pairwise judgments ("which PR mattered more?"). Jurors who keep diverging badly from consensus lose Soul. Reviewing is itself rewarded work.
- **Payout:** governance splits the pool across repos; maintainer weights and jury scores split each repo's share across PRs. Each person's payout is **square-rooted**, so 50 tiny PRs earn less than one solid PR. Rewards **vest over 90 days with clawback**.
- **Hunts:** bounties on specific issues, funded by anyone. Explicit demand, and the least gameable reward type, so they ship first.
- **Covens:** teams claiming Hunts together, with the payout split recorded on-chain and shared streaks. **Seasons** (Season 1: October) with leaderboards ranked by Soul, not tokens.
- **Profiles** in the explorer: merged PRs, Soul, who vouched for whom.

### 🐈‍⬛ Familiars: agent contributions, encouraged
Agents are first-class contributors, but every agent is the **Familiar of a human sponsor** who has Soul.
- **Binding:** a Familiar is a session key on the sponsor's account plus an on-chain `familiar_of` link. Its rewards go to the sponsor (who can set a split), and its mistakes cost the sponsor's Soul.
- **Volume doesn't pay:** all of a sponsor's Familiars are pooled for the square-root payout. There's a per-Soul-tier cap on Familiars and open claims, and the claim bond grows with the number of open claims.
- **Agent-shaped work pays best, because it can be checked objectively:**
  - Hunts with **acceptance tests**: the bounty pays when validators re-run the declared tests and they pass.
  - **Fuzzing and bug repros**: a crash that reproduces deterministically from the submitted input.
  - **Benchmarks**: validators re-run them to measure the gain.
  - **Test coverage**, docs, translations and spec conformance.
- **Network work:** Familiars can run nodes and validators through `start_dev_node` and `blockula join`. Validator rewards come from stake and uptime, so they can't be farmed.
- **Triage:** Familiars can pre-screen claims and flag spam. They earn only when a human jury upholds the flag; jurors themselves must be human.
- **MCP tools** (Phase 1.5): `list_hunts`, `claim_hunt`, `submit_contribution`, `my_contributions`, `become_familiar`. AGENTS.md gets a "contribute to Blockula" prompt: *find an open Hunt, implement it, make its acceptance tests pass, submit.*

### Why farming fails
| Attack | Defense |
|---|---|
| Bot or AI spam PRs | Must be merged and survive 30 days, bond slashed if the jury marks spam, square-root payout pooled per sponsor |
| Fake GitHub accounts (sybils) | Vouch required, voucher slashed if the account is fake, rewards start small until Soul builds up |
| Maintainer merging their own fake accounts' PRs | Juries randomly audit each repo, repo budget slashed if caught, vesting with clawback |
| Jurors colluding | Random selection, commit-reveal, no reviewing people you vouched for, flagging of suspicious reviewer–author pairs |
| Typo and whitespace PRs | Minimum score threshold |
| Fleets of agents | Pooled per sponsor, capped by Soul tier, sponsor slashed for spam |

### Forks and governance
- **Code forks:** a share (e.g. 10%) of a fork's rewards flows back to the original repo's contributors.
- **Chain forks:** `blockula summon` can start from a Soul-ledger snapshot, so a forked network keeps its community. Forking stays a credible exit.
- **Voting weight:** about √stake + Soul. **Conviction voting** for treasury and repo budgets. Protocol upgrades go through a vote, then a timelock, then validators signal adoption. **Private ballots** via Ghost Mode later (tallies public, votes hidden).
- **Issuance split:** 70% stakers / 20% contribution pool / 10% treasury. The pool is fixed per epoch, so contribution rewards never add inflation.

## Network and sync

What runs today (Commonware `=2026.9.0`: consensus, p2p, broadcast, storage, runtime):
- **Consensus:** simplex BFT. Votes and finality certificates are ML-DSA-65 (deterministic FIPS 204 signing, the Commonware namespace is the FIPS 204 context string; `node/src/pq.rs`). A certificate is a signer bitmap plus one 3,309-byte signature per signer, with no aggregation. Leaders rotate round-robin; `block_time_ms` paces proposals.
- **Deferred execution:** consensus orders blocks (parent, height, proposer = leader, timestamp, txs as signed JSON, plus the proposer's latest executed `(height, state root)`). Before voting, validators check structure, size limits (512 txs / 4 MiB), timestamps, proposer, tx signatures, chain id and minimum fee, and the attached state root if they've executed that height. Finalized blocks are executed in order: invalid txs are skipped, and `blocks.jsonl` is synced before marshal is acknowledged.
- **Dissemination and backfill:** marshal (standard/inline) broadcasts full blocks, delivers finalized blocks in order, and backfills anything a restarted validator missed.
- **p2p:** `authenticated::discovery` over tokio with ML-DSA-65 identities (the validator's account seed). The genesis lists every validator's `public_key`, `p2p_addr` and `http_addr`.
- **Mempool:** a tx submitted to any validator is checked against state + mempool, then gossiped to all validators on its own channel and de-duplicated by id. After each finalized block, included or now-invalid txs are evicted.
- **Faucet (testnets):** each validator's genesis-funded account pays its own faucet drips, so there's no shared secret and no nonce races. Full nodes forward faucet requests.
- `blockula summon <id> [--validators N]`, `blockula run --home`, `blockula dev` (one-validator simplex).
- **`blockula join --genesis <file|validator URL> --home <dir>`:** a non-validator full node. It serves the API + explorer immediately (`/api/status` shows `syncing`, `sync_progress` and `finalized_height`). It pulls finalized blocks from validators over HTTP (`/api/sync`), checks the hash links and an ML-DSA quorum finalization certificate per batch, and executes them. Txs and faucet requests are forwarded to a validator. This is a full backfill from genesis; snapshot sync is next.

Next:
1. Snapshot sync: download state at a finalized height and verify it against a certified state root (later, a STARK proof of the chain's history), then backfill history in the background.
2. Full nodes on p2p. Discovery only admits peers in the tracked set, so permissionless joining needs a secondary-peer admission policy.
3. A light-client mode that keeps no full state at all.
- Validators: about 100, ML-DSA signatures. There is no BLS-style aggregation (a 67-of-100 certificate is ~222 KB), so keep the set small and add STARK signature aggregation later.

## Roadmap

| Phase | Scope | Status |
|---|---|---|
| 0 | Dev node, ML-DSA accounts, transfers, `.boo` names, fee burn, built-in explorer + tutorial, TS SDK, passkey helpers, MCP server, `summon`/`run` | ✅ built |
| 1 | Multi-validator BFT (Commonware) ✅, p2p ✅, `join` ✅ (snapshot sync next), account abstraction + session keys ✅, standalone web wallet (Next.js) with passkey onboarding, indexer + public explorer, public testnet, npm publish | in progress |
| 1.5 | GitHub linking, validator checks of GitHub facts, Hunts with acceptance tests, Familiars + MCP contribution tools | |
| 2 | Séances, Souls, vouching, vesting/clawback, contribution pool, Ghost Mode + viewing keys, The Crypt, proof aggregation | |
| 3 | Staking/slashing, tokenomics live, governance (conviction voting, repo budgets, fork royalties), audits, mainnet | |
| 4 | WASM/RISC-V contracts, light clients, STARK signature aggregation | |

## Known limits

- **The p2p transport is not post-quantum.** Commonware's handshake authenticates peers with ML-DSA-65, but the session key comes from X25519 and traffic is encrypted with ChaCha20-Poly1305. Recorded traffic could be decrypted by a future quantum computer. Consensus messages are public anyway; making the key exchange PQ (e.g. hybrid X25519 + ML-KEM-768) needs a patched `commonware-cryptography` handshake.
- The validator set is fixed in genesis: no staking, no epochs or reconfiguration, and no slashing yet (attributable fault evidence is reported by simplex but not acted on).
- `join` fully backfills from genesis over HTTP and trusts no single peer (certificates are verified), but there's no snapshot sync yet. Full nodes don't serve `/api/sync` themselves.
- The p2p preset is `local` (allows private IPs), and HTTP binds to 127.0.0.1. Fine for testnets on one machine or a LAN; real deployments need the `recommended` preset and a public bind.
- The state root covers state *before* the block it's embedded in (deferred execution), so a block's own post-state root is in `blocks.jsonl`, not in the certified consensus block.
- If `consensus/` is ahead of `blocks.jsonl` (e.g. `blocks.jsonl` deleted), the node logs an error; delete `consensus/` to resync.
- State is cloned per tx and the state root is a flat hash (Phase 1 moves to a Merkle tree for light-client proofs).
- The faucet is unthrottled (testnets only).
- `sdk-ts/src/passkey.ts` typechecks but hasn't been exercised in a real browser yet.
