use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use swing::shutdown::{self, Exit};
use swing::{
    config, health, key, login, mirror, publish, replicas, service, settings, stop, up, webring,
};
use tracing::info;
use tracing_subscriber::EnvFilter;

// Not #[tokio::main]: shutdown_timeout keeps a stuck blocking thread from holding the process open.
const RUNTIME_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

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
    #[command(
        about = "Run Kubo (if managed) and the mirror agent under one supervisor; restarts either when it fails"
    )]
    Up {
        #[arg(long, help = "Config file (default: $SWING_CONFIG or ./swing.toml)")]
        config: Option<PathBuf>,
        #[arg(
            long,
            value_name = "PATH",
            help = "Append logs to this file instead of stderr"
        )]
        log_file: Option<PathBuf>,
        #[cfg(windows)]
        #[arg(long, hide = true)]
        exit_with_parent: bool,
    },
    #[command(
        about = "Stop a running `swing up` instance gracefully, via its dashboard API (POST /api/shutdown or /api/restart)"
    )]
    Stop {
        #[arg(long, help = "Config file (default: $SWING_CONFIG or ./swing.toml)")]
        config: Option<PathBuf>,
        #[arg(
            long,
            help = "Ask it to restart instead of staying stopped (needs [dashboard].listen)"
        )]
        restart: bool,
        #[arg(
            long,
            default_value_t = 60,
            help = "Seconds to wait for it to stop before giving up"
        )]
        timeout: u64,
    },
    #[command(
        about = "Register swing up as a login/system service (systemd user unit, launchd agent, or Task Scheduler)"
    )]
    Service {
        #[command(subcommand)]
        action: ServiceCommand,
    },
    #[command(
        about = "Log in to the dashboard of a running `swing up` and manage its access token"
    )]
    Dashboard {
        #[command(subcommand)]
        action: DashboardCommand,
    },
    #[command(about = "Add a static site to IPFS and announce its CID on Nostr")]
    Publish {
        #[arg(long, help = "Config file (default: $SWING_CONFIG or ./swing.toml)")]
        config: Option<PathBuf>,
        #[arg(long, help = "Site identifier (d tag), e.g. your domain name")]
        site: String,
        #[arg(
            long,
            help = "Canonical HTTP(S) URL of the site; omit for an IPFS-only site"
        )]
        url: Option<String>,
        #[arg(
            long,
            value_name = "MODE",
            help = "NIP-05 check: off, warn, require (default: config or warn)"
        )]
        nip05: Option<String>,
        #[arg(
            long,
            help = "Display title of the site (self-claimed, shown to readers)"
        )]
        title: Option<String>,
        #[arg(short, long, help = "Update note shown to readers (event content)")]
        message: Option<String>,
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
        about = "Show who reports holding each site (default: your own sites) and how many hold the latest version"
    )]
    Replicas {
        #[arg(long, help = "Config file (default: $SWING_CONFIG or ./swing.toml)")]
        config: Option<PathBuf>,
        #[arg(value_name = "KEY", help = "Site authors (npub, hex or nprofile)")]
        keys: Vec<String>,
    },
    #[command(
        about = "Check stored versions against Kubo MFS and list leftover paths, via a running `swing up`'s dashboard API (GET /api/status; exits non-zero on problems)"
    )]
    Status {
        #[arg(long, help = "Config file (default: $SWING_CONFIG or ./swing.toml)")]
        config: Option<PathBuf>,
    },
    #[command(
        about = "Show mutual mirror relations as a webring graph, crawling follow sets from the given accounts (default: yourself)"
    )]
    Webring {
        #[arg(long, help = "Config file (default: $SWING_CONFIG or ./swing.toml)")]
        config: Option<PathBuf>,
        #[arg(
            long,
            default_value_t = 2,
            help = "Hops to crawl from the starting accounts, following both directions"
        )]
        depth: usize,
        #[arg(long, value_enum, default_value_t = webring::Format::Text)]
        format: webring::Format,
        #[arg(value_name = "KEY", help = "Starting accounts (npub, hex or nprofile)")]
        keys: Vec<String>,
    },
    #[command(about = "Generate or inspect Nostr keys (no config or secret key required)")]
    Key {
        #[command(subcommand)]
        action: KeyCommand,
    },
    #[command(
        about = "Print generated reference files from the settings catalog (no config file read)"
    )]
    Config {
        #[command(subcommand)]
        action: ConfigCommand,
    },
}

#[derive(Subcommand)]
enum DashboardCommand {
    #[command(
        about = "Open the dashboard in a browser with a single-use login link, via a running `swing up`'s dashboard API (POST /api/login-code)"
    )]
    Open {
        #[arg(long, help = "Config file (default: $SWING_CONFIG or ./swing.toml)")]
        config: Option<PathBuf>,
        #[arg(long, help = "Only print the login link and code")]
        no_browser: bool,
    },
    #[command(
        about = "Replace the dashboard token, logging out every browser session (POST /api/token/rotate, or the token file directly when swing up is not running)"
    )]
    RotateToken {
        #[arg(long, help = "Config file (default: $SWING_CONFIG or ./swing.toml)")]
        config: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum ConfigCommand {
    #[command(about = "Print swing.example.toml, generated from the settings catalog")]
    Example,
    #[command(about = "Print .env.example, generated from the settings catalog")]
    EnvExample,
}

#[derive(Subcommand)]
enum KeyCommand {
    #[command(about = "Generate a fresh Nostr keypair and print nsec/npub/hex")]
    Generate,
}

#[derive(Subcommand)]
enum ServiceCommand {
    #[command(about = "Register swing up as a login/system service")]
    Install {
        #[arg(long, help = "Config file (default: $SWING_CONFIG or ./swing.toml)")]
        config: Option<PathBuf>,
        #[arg(
            long,
            help = "Register a systemd system unit instead of a user unit (Linux only)"
        )]
        system: bool,
        #[arg(long, help = "Register without starting it now")]
        no_start: bool,
    },
    #[command(about = "Remove the service registration")]
    Uninstall {
        #[arg(
            long,
            help = "Register a systemd system unit instead of a user unit (Linux only)"
        )]
        system: bool,
    },
    #[command(about = "Stop the registered service (does not remove the registration)")]
    Stop {
        #[arg(
            long,
            help = "Target the systemd system unit instead of the user unit (Linux only)"
        )]
        system: bool,
    },
    #[command(about = "Show the service status")]
    Status {
        #[arg(
            long,
            help = "Register a systemd system unit instead of a user unit (Linux only)"
        )]
        system: bool,
    },
}

#[derive(Subcommand)]
enum MirrorCommand {
    #[command(about = "List pubkeys in the mirror set")]
    List {
        #[arg(long, help = "Config file (default: $SWING_CONFIG or ./swing.toml)")]
        config: Option<PathBuf>,
    },
    #[command(
        about = "Add pubkeys (npub, hex or nprofile) to the mirror set, via a running `swing up`'s dashboard API (POST /api/mirror/add)"
    )]
    Add {
        #[arg(long, help = "Config file (default: $SWING_CONFIG or ./swing.toml)")]
        config: Option<PathBuf>,
        #[arg(required = true, num_args = 1.., value_name = "KEY")]
        keys: Vec<String>,
    },
    #[command(
        about = "Remove pubkeys from the mirror set, via a running `swing up`'s dashboard API (POST /api/mirror/remove)"
    )]
    Remove {
        #[arg(long, help = "Config file (default: $SWING_CONFIG or ./swing.toml)")]
        config: Option<PathBuf>,
        #[arg(required = true, num_args = 1.., value_name = "KEY")]
        keys: Vec<String>,
    },
}

fn init_tracing(log_file: Option<&PathBuf>) -> Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    match log_file {
        Some(path) => {
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .with_context(|| format!("opening log file {}", path.display()))?;
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_ansi(false)
                .with_writer(file)
                .init();
        }
        None => {
            tracing_subscriber::fmt().with_env_filter(filter).init();
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let log_file = match &cli.command {
        Command::Up { log_file, .. } => log_file.as_ref(),
        _ => None,
    };
    init_tracing(log_file)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let logs_to_file = log_file.is_some();
    let result = runtime.block_on(run(cli));
    runtime.shutdown_timeout(RUNTIME_SHUTDOWN_TIMEOUT);
    if logs_to_file && let Err(e) = &result {
        tracing::error!("{e:#}");
    }
    result
}

async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Up {
            config,
            #[cfg(windows)]
            exit_with_parent,
            ..
        } => {
            let signal = shutdown::cancel_on_signal()?;
            #[cfg(windows)]
            if exit_with_parent {
                shutdown::cancel_when_parent_exits(signal.clone())?;
            }
            loop {
                let cfg = config::Config::load(config.as_deref())?;
                match up::run(cfg, signal.child_token()).await? {
                    Exit::Stop => return Ok(()),
                    Exit::Restart => {
                        info!("restarting: reloading configuration");
                        continue;
                    }
                }
            }
        }
        other => run_other(other).await,
    }
}

async fn run_other(command: Command) -> Result<()> {
    match command {
        Command::Up { .. } => unreachable!("handled in run"),
        Command::Stop {
            config,
            restart,
            timeout,
        } => {
            let cfg = config::Config::load(config.as_deref())?;
            stop::run(&cfg, restart, Duration::from_secs(timeout)).await
        }
        Command::Service { action } => match action {
            ServiceCommand::Install {
                config,
                system,
                no_start,
            } => service::install(config.as_deref(), system, no_start),
            ServiceCommand::Uninstall { system } => service::uninstall(system).await,
            ServiceCommand::Stop { system } => service::stop(system).await,
            ServiceCommand::Status { system } => service::status(system),
        },
        Command::Dashboard { action } => match action {
            DashboardCommand::Open { config, no_browser } => {
                let cfg = config::Config::load(config.as_deref())?;
                login::open(&cfg, no_browser).await
            }
            DashboardCommand::RotateToken { config } => {
                let cfg = config::Config::load(config.as_deref())?;
                login::rotate_token(&cfg).await
            }
        },
        Command::Publish {
            config,
            site,
            url,
            nip05,
            title,
            message,
            dir,
        } => {
            let cfg = config::Config::load(config.as_deref())?;
            publish::run(cfg, site, url, &dir, nip05, title, message).await
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
        Command::Replicas { config, keys } => {
            let cfg = config::Config::load(config.as_deref())?;
            replicas::show(&cfg, &keys).await
        }
        Command::Status { config } => {
            let cfg = config::Config::load(config.as_deref())?;
            health::status(&cfg).await
        }
        Command::Webring {
            config,
            depth,
            format,
            keys,
        } => {
            let cfg = config::Config::load(config.as_deref())?;
            webring::show(&cfg, &keys, depth, format).await
        }
        Command::Key { action } => match action {
            KeyCommand::Generate => key::generate(),
        },
        Command::Config { action } => match action {
            ConfigCommand::Example => {
                print!("{}", settings::render_toml_example());
                Ok(())
            }
            ConfigCommand::EnvExample => {
                print!("{}", settings::render_env_example());
                Ok(())
            }
        },
    }
}
