mod api;
mod chain;
mod crypto;
mod types;

use anyhow::Result;
use chain::Chain;
use clap::{Parser, Subcommand};
use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "blockula", version, about = "🧛 Blockula — a post-quantum blockchain that never dies")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Start a local dev chain with a faucet and the built-in explorer
    Dev {
        #[arg(long, default_value = "7070")]
        port: u16,
        /// Wipe the dev chain and start fresh
        #[arg(long)]
        reset: bool,
    },
    /// Create a brand-new chain of your own (new genesis + validator key)
    Summon {
        chain_id: String,
        #[arg(long)]
        home: Option<PathBuf>,
        #[arg(long)]
        no_faucet: bool,
        #[arg(long, default_value = "1000")]
        block_time_ms: u64,
    },
    /// Run a chain created with `summon`
    Run {
        #[arg(long)]
        home: PathBuf,
        #[arg(long, default_value = "7070")]
        port: u16,
    },
    /// Generate an ML-DSA-65 keypair
    Keygen,
}

fn blockula_dir() -> PathBuf {
    std::env::var_os("BLOCKULA_HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap()).join(".blockula"))
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Dev { port, reset } => {
            let home = blockula_dir().join("dev");
            if reset {
                std::fs::remove_dir_all(&home).ok();
            }
            if !home.join("genesis.json").exists() {
                Chain::summon(&home, "blockula-dev", true, 1000)?;
            }
            run(home, port).await
        }
        Cmd::Summon { chain_id, home, no_faucet, block_time_ms } => {
            let home = home.unwrap_or_else(|| blockula_dir().join(&chain_id));
            if home.join("genesis.json").exists() {
                anyhow::bail!("{} already has a chain", home.display());
            }
            Chain::summon(&home, &chain_id, !no_faucet, block_time_ms)?;
            println!("🪦 Summoned chain `{chain_id}` at {}\n   Start it with: blockula run --home {}", home.display(), home.display());
            Ok(())
        }
        Cmd::Run { home, port } => run(home, port).await,
        Cmd::Keygen => {
            let k = crypto::Keys::generate();
            println!("{}", serde_json::json!({ "address": k.address(), "seed": k.seed_hex() }));
            Ok(())
        }
    }
}

async fn run(home: PathBuf, port: u16) -> Result<()> {
    let chain = Chain::open(&home)?;
    let block_time = Duration::from_millis(chain.genesis.block_time_ms);
    println!("🧛 Blockula `{}` rising from height {}", chain.genesis.chain_id, chain.height());
    let shared = Arc::new(Mutex::new(chain));

    let producer = shared.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(block_time);
        loop {
            tick.tick().await;
            if let Err(e) = producer.lock().produce_block() {
                eprintln!("block production failed: {e:#}");
            }
        }
    });

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    println!("🦇 Explorer:  http://localhost:{port}\n🔌 API:       http://localhost:{port}/api/status\n📁 Data:      {}", home.display());
    axum::serve(listener, api::router(shared)).await?;
    Ok(())
}
