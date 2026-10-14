mod api;
mod chain;
mod consensus;
mod crypto;
mod node;
mod pq;
mod types;

#[cfg(test)]
mod sim;

use anyhow::Result;
use chain::Chain;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "blockula", version, about = "🧛 Blockula — a post-quantum blockchain that never dies")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Start a local single-validator dev chain with a faucet and the built-in explorer
    Dev {
        #[arg(long, default_value = "7070")]
        port: u16,
        /// Wipe the dev chain and start fresh
        #[arg(long)]
        reset: bool,
    },
    /// Create a brand-new chain of your own (genesis + validator keys)
    Summon {
        chain_id: String,
        #[arg(long)]
        home: Option<PathBuf>,
        #[arg(long)]
        no_faucet: bool,
        #[arg(long, default_value = "1000")]
        block_time_ms: u64,
        /// Number of validators. With N > 1, `home/v0 … home/vN-1` are created for a local testnet.
        #[arg(long, default_value = "1")]
        validators: usize,
    },
    /// Run a validator (a home with validator.key) or a joined full node
    Run {
        #[arg(long)]
        home: PathBuf,
        /// HTTP port (defaults to the one in genesis for validators, 7171 for full nodes)
        #[arg(long)]
        port: Option<u16>,
    },
    /// Join an existing chain as a full node: sync finalized blocks from a validator and serve the API
    Join {
        /// genesis.json path, or a validator's HTTP URL (e.g. http://127.0.0.1:7070)
        #[arg(long)]
        genesis: String,
        #[arg(long)]
        home: PathBuf,
        #[arg(long, default_value = "7171")]
        port: u16,
    },
    /// Generate an ML-DSA-65 keypair
    Keygen,
}

fn blockula_dir() -> PathBuf {
    std::env::var_os("BLOCKULA_HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap()).join(".blockula"))
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("BLOCKULA_LOG").unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn,blockula=info")))
        .with_writer(std::io::stderr)
        .init();
    match Cli::parse().cmd {
        Cmd::Dev { port, reset } => {
            let home = blockula_dir().join("dev");
            let stale = home.join("genesis.json").exists() && chain::Genesis::load(&home.join("genesis.json")).is_err();
            if reset || stale {
                if stale {
                    eprintln!("♻️  The dev chain was made by an older Blockula; starting a fresh one.");
                }
                std::fs::remove_dir_all(&home).ok();
            }
            if !home.join("genesis.json").exists() {
                Chain::summon(&home, "blockula-dev", true, 1000, 1, Some(port))?;
            }
            node::run_validator(home, Some(port))
        }
        Cmd::Summon { chain_id, home, no_faucet, block_time_ms, validators } => {
            let home = home.unwrap_or_else(|| blockula_dir().join(&chain_id));
            if home.join("genesis.json").exists() {
                anyhow::bail!("{} already has a chain", home.display());
            }
            // A single-validator chain keeps the classic port; testnets get free ports.
            let homes = Chain::summon(&home, &chain_id, !no_faucet, block_time_ms, validators, (validators == 1).then_some(7070))?;
            let g = chain::Genesis::load(&homes[0].join("genesis.json"))?;
            println!("🪦 Summoned chain `{chain_id}` with {validators} validator{} at {}", if validators == 1 { "" } else { "s" }, home.display());
            for (h, v) in homes.iter().zip(&g.validators) {
                println!("   blockula run --home {}    # http://{}  p2p {}", h.display(), v.http_addr, v.p2p_addr);
            }
            Ok(())
        }
        Cmd::Run { home, port } => {
            if home.join("validator.key").exists() {
                node::run_validator(home, port)
            } else {
                node::run_full_node(home, port.unwrap_or(7171))
            }
        }
        Cmd::Join { genesis, home, port } => {
            node::join(&genesis, &home)?;
            node::run_full_node(home, port)
        }
        Cmd::Keygen => {
            let k = crypto::Keys::generate();
            println!("{}", serde_json::json!({ "address": k.address(), "seed": k.seed_hex() }));
            Ok(())
        }
    }
}
