use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use swing::{agent, config, health, key, mirror, publish};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "swing",
    version,
    about = "Nostr + IPFS mutual site-mirror tool"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "Run the mirror agent: follow the mirror set and store sites in Kubo MFS")]
    Agent {
        #[arg(long, help = "Config file (default: $SWING_CONFIG or ./swing.toml)")]
        config: Option<PathBuf>,
    },
    #[command(about = "Add a static site to IPFS and announce its CID on Nostr")]
    Publish {
        #[arg(long, help = "Config file (default: $SWING_CONFIG or ./swing.toml)")]
        config: Option<PathBuf>,
        #[arg(long, help = "Site identifier (d tag); defaults to the URL host")]
        site: Option<String>,
        #[arg(long, help = "Canonical HTTPS URL of the site")]
        url: String,
        #[arg(
            long,
            value_name = "MODE",
            help = "NIP-05 check: off, warn, require (default: config or warn)"
        )]
        nip05: Option<String>,
        #[arg(help = "Directory containing the built static site")]
        dir: PathBuf,
    },
    #[command(about = "Manage the mirror set (NIP-51 follow set of sites to keep)")]
    Mirror {
        #[command(subcommand)]
        action: MirrorCommand,
    },
    #[command(about = "Show followed sites, their latest CIDs and storage status")]
    Sites {
        #[arg(long, help = "Config file (default: $SWING_CONFIG or ./swing.toml)")]
        config: Option<PathBuf>,
    },
    #[command(
        about = "Check stored versions against Kubo MFS and list leftover paths (exits non-zero on problems)"
    )]
    Status {
        #[arg(long, help = "Config file (default: $SWING_CONFIG or ./swing.toml)")]
        config: Option<PathBuf>,
    },
    #[command(about = "Generate or inspect Nostr keys (no config or secret key required)")]
    Key {
        #[command(subcommand)]
        action: KeyCommand,
    },
}

#[derive(Subcommand)]
enum KeyCommand {
    #[command(about = "Generate a fresh Nostr keypair and print nsec/npub/hex")]
    Generate,
}

#[derive(Subcommand)]
enum MirrorCommand {
    #[command(about = "List pubkeys in the mirror set")]
    List {
        #[arg(long, help = "Config file (default: $SWING_CONFIG or ./swing.toml)")]
        config: Option<PathBuf>,
    },
    #[command(about = "Add pubkeys (npub, hex or nprofile) to the mirror set")]
    Add {
        #[arg(long, help = "Config file (default: $SWING_CONFIG or ./swing.toml)")]
        config: Option<PathBuf>,
        #[arg(required = true, num_args = 1.., value_name = "KEY")]
        keys: Vec<String>,
    },
    #[command(about = "Remove pubkeys from the mirror set")]
    Remove {
        #[arg(long, help = "Config file (default: $SWING_CONFIG or ./swing.toml)")]
        config: Option<PathBuf>,
        #[arg(required = true, num_args = 1.., value_name = "KEY")]
        keys: Vec<String>,
    },
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let cli = Cli::parse();
    match cli.command {
        Command::Agent { config } => {
            let cfg = config::Config::load(config.as_deref())?;
            agent::run(cfg).await
        }
        Command::Publish {
            config,
            site,
            url,
            nip05,
            dir,
        } => {
            let cfg = config::Config::load(config.as_deref())?;
            publish::run(cfg, site, url, &dir, nip05).await
        }
        Command::Mirror { action } => match action {
            MirrorCommand::List { config } => {
                let cfg = config::Config::load(config.as_deref())?;
                mirror::list(&cfg).await
            }
            MirrorCommand::Add { config, keys } => {
                let cfg = config::Config::load(config.as_deref())?;
                mirror::add(&cfg, &keys).await
            }
            MirrorCommand::Remove { config, keys } => {
                let cfg = config::Config::load(config.as_deref())?;
                mirror::remove(&cfg, &keys).await
            }
        },
        Command::Sites { config } => {
            let cfg = config::Config::load(config.as_deref())?;
            mirror::sites(&cfg).await
        }
        Command::Status { config } => {
            let cfg = config::Config::load(config.as_deref())?;
            health::status(&cfg).await
        }
        Command::Key { action } => match action {
            KeyCommand::Generate => key::generate(),
        },
    }
}
