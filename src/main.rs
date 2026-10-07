use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use swing::shutdown::{self, Exit};
use swing::{
    config, health, key, login, mirror, pair, publish, replicas, service, settings, stats, stop,
    up, webring,
};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Args)]
struct ConfigArg {
    #[arg(
        long,
        help = "Config file (default: $SWING_CONFIG, ./swing.toml if it exists, or swing.toml in the per-user data directory)"
    )]
    config: Option<PathBuf>,
}

impl ConfigArg {
    fn as_deref(&self) -> Option<&Path> {
        self.config.as_deref()
    }
}

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
        #[command(flatten)]
        config: ConfigArg,
        #[arg(
            long,
            value_name = "PATH",
            help = "Append logs to this file instead of stderr"
        )]
        log_file: Option<PathBuf>,
        #[arg(
            long,
            env = "SWING_NO_PORT_SHIFT",
            help = "Never move the dashboard or Kubo gateway port during setup; fail if it is in use"
        )]
        no_port_shift: bool,
        #[cfg(windows)]
        #[arg(long, hide = true)]
        exit_with_parent: bool,
    },
    #[command(
        about = "Stop a running `swing up` instance gracefully, via its dashboard API (POST /api/shutdown or /api/restart)"
    )]
    Stop {
        #[command(flatten)]
        config: ConfigArg,
        #[arg(
            long,
            help = "Ask it to restart instead of staying stopped (needs [dashboard].listen)"
        )]
        restart: bool,
        #[arg(
            long,
            default_value_t = service::GRACEFUL_STOP_TIMEOUT.as_secs(),
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
        #[command(flatten)]
        config: ConfigArg,
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
            value_name = "MODE",
            help = "Dotfile check: off, warn, require (default: config or require)"
        )]
        check_dotfiles: Option<String>,
        #[arg(
            long,
            value_name = "MODE",
            help = "Size check (over 512 MiB): off, warn, require (default: config or warn)"
        )]
        check_size: Option<String>,
        #[arg(
            long,
            value_name = "MODE",
            help = "Same-CID check against your latest version on the relays: off, warn, require (default: config or require; require stops without publishing and exits 0)"
        )]
        check_unchanged: Option<String>,
        #[arg(
            long,
            help = "Display title of the site (self-claimed, shown to readers)"
        )]
        title: Option<String>,
        #[arg(short, long, help = "Update note shown to readers (event content)")]
        message: Option<String>,
        #[arg(
            short,
            long,
            help = "Publish files that are new since your latest version without asking (required when stdin is not a terminal)"
        )]
        yes: bool,
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
        #[command(flatten)]
        config: ConfigArg,
    },
    #[command(
        about = "Show who reports holding each site (default: your own sites) and how many hold the latest version"
    )]
    Replicas {
        #[command(flatten)]
        config: ConfigArg,
        #[arg(value_name = "KEY", help = "Site authors (npub, hex or nprofile)")]
        keys: Vec<String>,
    },
    #[command(
        about = "Check stored versions against Kubo MFS and list leftover paths, via a running `swing up`'s dashboard API (GET /api/status; exits non-zero on problems)"
    )]
    Status {
        #[command(flatten)]
        config: ConfigArg,
    },
    #[command(
        about = "Show CPU, memory and IPFS traffic sampled by a running `swing up`, via its dashboard API (GET /api/stats)"
    )]
    Stats {
        #[command(flatten)]
        config: ConfigArg,
        #[arg(
            long,
            value_name = "DURATION",
            default_value = "1h",
            value_parser = config::parse_duration_secs,
            help = "How far back to summarize, e.g. 30m, 6h, 1d (samples are kept for 1d)"
        )]
        last: u64,
        #[arg(long, help = "Print the raw samples as JSON")]
        json: bool,
    },
    #[command(
        about = "Show mutual mirror relations as a webring graph, crawling follow sets from the given accounts (default: yourself)"
    )]
    Webring {
        #[command(flatten)]
        config: ConfigArg,
        #[arg(
            long,
            default_value_t = 2,
            help = "Hops to crawl outbound from the starting accounts"
        )]
        depth: usize,
        #[arg(long, value_enum, default_value_t = webring::Format::Text)]
        format: webring::Format,
        #[arg(value_name = "KEY", help = "Starting accounts (npub, hex or nprofile)")]
        keys: Vec<String>,
    },
    #[command(about = "Sign with a signer app on your phone (NIP-46) instead of a secret key")]
    Signer {
        #[command(subcommand)]
        action: SignerCommand,
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
        #[command(flatten)]
        config: ConfigArg,
        #[arg(long, help = "Only print the login link and code")]
        no_browser: bool,
    },
    #[command(
        about = "Replace the dashboard token, logging out every browser session (POST /api/token/rotate, or the token file directly when swing up is not running)"
    )]
    RotateToken {
        #[command(flatten)]
        config: ConfigArg,
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
enum SignerCommand {
    #[command(
        about = "Pair a signer app by showing a nostrconnect:// QR code in the terminal, and save it to remote-signer.json"
    )]
    Pair {
        #[command(flatten)]
        config: ConfigArg,
        #[arg(
            long = "relay",
            value_name = "URL",
            default_value = pair::DEFAULT_RELAY,
            help = "Relay both swing and the signer app can reach (repeatable, up to 5)"
        )]
        relays: Vec<String>,
    },
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
        #[command(flatten)]
        config: ConfigArg,
        #[arg(
            long,
            help = "Register a systemd system unit instead of a user unit (Linux only)"
        )]
        system: bool,
        #[arg(
            long,
            value_name = "USER",
            requires = "system",
            help = "User (name or uid) the system unit runs as; defaults to the sudo caller (Linux only)"
        )]
        run_as: Option<String>,
        #[arg(
            long,
            requires = "system",
            help = "Allow the system unit to run as root, together with `--run-as root` (Linux only)"
        )]
        allow_root: bool,
        #[arg(long, help = "Register without starting it now")]
        no_start: bool,
        #[arg(
            long,
            help = "Do not register swing-tray to start at login (Windows and macOS)"
        )]
        no_tray: bool,
    },
    #[command(about = "Remove the service registration")]
    Uninstall {
        #[arg(
            long,
            help = "Target the systemd system unit instead of the user unit (Linux only)"
        )]
        system: bool,
        #[arg(
            long,
            value_name = "DIR",
            hide = true,
            help = "Only remove registrations whose swing executable is under DIR; leave the others as they are"
        )]
        only_from: Option<PathBuf>,
    },
    #[command(about = "Start the registered service")]
    Start {
        #[arg(
            long,
            help = "Target the systemd system unit instead of the user unit (Linux only)"
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
            help = "Target the systemd system unit instead of the user unit (Linux only)"
        )]
        system: bool,
        #[arg(
            long,
            value_name = "DIR",
            hide = true,
            help = "Instead of the status, tell whether the registrations run swing from under DIR: exit 0 if all do, 3 if nothing is registered, 4 otherwise"
        )]
        points_into: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum MirrorCommand {
    #[command(about = "List pubkeys in the mirror set")]
    List {
        #[command(flatten)]
        config: ConfigArg,
    },
    #[command(
        about = "Add pubkeys (npub, hex or nprofile) to the mirror set, via a running `swing up`'s dashboard API (POST /api/mirror/add)"
    )]
    Add {
        #[command(flatten)]
        config: ConfigArg,
        #[arg(required = true, num_args = 1.., value_name = "KEY")]
        keys: Vec<String>,
    },
    #[command(
        about = "Remove pubkeys from the mirror set, via a running `swing up`'s dashboard API (POST /api/mirror/remove)"
    )]
    Remove {
        #[command(flatten)]
        config: ConfigArg,
        #[arg(required = true, num_args = 1.., value_name = "KEY")]
        keys: Vec<String>,
    },
}

fn init_tracing(log_file: Option<&PathBuf>, default_filter: &str) -> Result<()> {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));
    match log_file {
        Some(path) => {
            let mut options = std::fs::OpenOptions::new();
            options.create(true).append(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let file = options
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
    let (log_file, default_filter) = match &cli.command {
        Command::Up { log_file, .. } => (log_file.as_ref(), "info"),
        _ => (None, "info,nostr_sdk=warn,nostr_connect=warn"),
    };
    init_tracing(log_file, default_filter)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let logs_to_file = log_file.is_some();
    let result = runtime.block_on(run(cli));
    // Not #[tokio::main]: shutdown_timeout keeps a stuck blocking thread from holding the process open.
    runtime.shutdown_timeout(shutdown::RUNTIME_SHUTDOWN_TIMEOUT);
    if logs_to_file && let Err(e) = &result {
        tracing::error!("{e:#}");
    }
    result
}

async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Up {
            config,
            no_port_shift,
            #[cfg(windows)]
            exit_with_parent,
            ..
        } => {
            let watch = shutdown::cancel_on_signal(up::FORCE_EXIT_GRACE)?;
            #[cfg(windows)]
            if exit_with_parent {
                watch.cancel_when_parent_exits()?;
            }
            let signal = watch.token();
            loop {
                let cfg = config::Config::load(config.as_deref())?;
                match up::run(cfg, signal.child_token(), !no_port_shift).await? {
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
                run_as,
                allow_root,
                no_start,
                no_tray,
            } => service::install(
                config.as_deref(),
                &service::InstallOptions {
                    system,
                    run_as: run_as.as_deref(),
                    allow_root,
                    no_start,
                    no_tray,
                },
            ),
            ServiceCommand::Uninstall { system, only_from } => {
                service::uninstall(system, only_from.as_deref()).await
            }
            ServiceCommand::Start { system } => service::start(system),
            ServiceCommand::Stop { system } => service::stop(system).await,
            ServiceCommand::Status {
                system,
                points_into: None,
            } => service::status(system),
            ServiceCommand::Status {
                system,
                points_into: Some(dir),
            } => {
                let code = service::placement(system, &dir)?.exit_code();
                if code != 0 {
                    std::process::exit(code);
                }
                Ok(())
            }
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
            check_dotfiles,
            check_size,
            check_unchanged,
            title,
            message,
            yes,
            dir,
        } => {
            let cfg = config::Config::load(config.as_deref())?;
            let request = publish::Request {
                d: site,
                url,
                title,
                message,
                modes: publish::ModeOverrides {
                    nip05,
                    check_dotfiles,
                    check_size,
                    check_unchanged,
                },
                yes,
            };
            publish::run(cfg, &dir, request).await
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
        Command::Stats { config, last, json } => {
            let cfg = config::Config::load(config.as_deref())?;
            stats::show(&cfg, last, json).await
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
        Command::Signer { action } => match action {
            SignerCommand::Pair { config, relays } => {
                let cfg = config::Config::load(config.as_deref())?;
                pair::run(&cfg, &relays).await
            }
        },
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
