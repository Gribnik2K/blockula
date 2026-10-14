# Contributing to Blockula 🧛

Humans and agents are both welcome. Agents contribute as **Familiars** of a human sponsor, who is accountable for their work and receives their rewards (see "Proof of Contribution" in [docs/PLAN.md](docs/PLAN.md)).

## Hunts: bounties with acceptance criteria
- Open Hunts are GitHub issues labeled [`hunt`](https://github.com/jackdishman/blockula/issues?q=is%3Aopen+label%3Ahunt). Each has objective acceptance criteria and a FANG bounty escrowed on-chain.
- `familiar-friendly` marks Hunts specified tightly enough for an agent to finish end to end.
- **To claim one:** comment on the issue, link your GitHub handle to your Blockula account (`link_github`, with a public gist containing your `boo1…` address), and open a PR that references the issue and meets every acceptance criterion.
- **Payout:** when the PR merges, the Hunt's creator awards it on-chain (`award_hunt`), and the explorer shows it on your profile.

## What a good PR looks like
- One focused change, with tests. `cargo test --manifest-path node/Cargo.toml` and `bash scripts/local-testnet.sh` pass (CI runs both).
- The Rust and TypeScript transaction encodings stay byte-identical (`node/src/types.rs` ⇄ `sdk-ts/src/tx.ts`).
- Explain what changed and how you verified it.
- Volume doesn't pay: rewards are square-rooted per sponsor and vest with clawback. One solid PR beats ten small ones.

## Security
Please report vulnerabilities privately via GitHub Security Advisories on this repo rather than in a public issue.
