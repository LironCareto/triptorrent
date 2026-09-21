use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::thread;
use std::time::Duration;
use std::{env, fs};
use triptorrent_cli::{
    FetchMonitor, FetchOptions, ShareOptions, VerifiedTransferProgress, fetch_file,
    fetch_file_via_overlay_with_monitor, fetch_file_via_overlay_with_options, identify, parse_psk,
    share_file, share_file_via_overlay_with_control, share_file_via_overlay_with_options,
};
use triptorrent_node::{
    ConfigOverrides, FetchControl, NodeConfig, PersistentFetchStats, TransferEngine,
    VerifiedProgress, run_daemon,
};
use triptorrent_node_api::{LocalApiClient, ProviderContribution};
use triptorrent_overlay::BootstrapClient;
use triptorrent_protocol::RelayAdvertisement;

#[derive(Debug, Parser)]
#[command(
    name = "triptorrent",
    about = "TripTorrent Testnet v1 reference client and services"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the centralized Testnet v1 bootstrap and rendezvous service.
    Bootstrap {
        /// Address on which the bootstrap accepts control connections.
        #[arg(long, default_value = "127.0.0.1:7100")]
        listen: SocketAddr,
        /// Lease duration for peer and relay registrations.
        #[arg(long, default_value_t = 5_000)]
        lease_ms: u64,
    },
    /// Run the opaque local TCP relay.
    Relay {
        /// Address on which the relay accepts peer connections.
        #[arg(long, default_value = "127.0.0.1:7000")]
        listen: SocketAddr,
        /// Testnet v1 bootstrap address. Omit it to retain manual M1 behavior.
        #[arg(long)]
        bootstrap: Option<SocketAddr>,
        /// Ephemeral Testnet v1 relay identifier.
        #[arg(long)]
        id: Option<String>,
        /// Reachable relay address to advertise; defaults to the listen address.
        #[arg(long)]
        advertise: Option<SocketAddr>,
    },
    /// Print the experimental content ID for a file.
    Id {
        /// File to identify.
        file: PathBuf,
    },
    /// Share one file through an existing relay route.
    Share {
        /// Testnet v1 bootstrap address for automatic discovery and routing.
        #[arg(long)]
        bootstrap: Option<SocketAddr>,
        /// Manual M1 relay address.
        #[arg(long)]
        relay: Option<SocketAddr>,
        #[arg(long)]
        route: Option<String>,
        #[arg(long, value_parser = parse_psk)]
        key: Option<[u8; 32]>,
        #[arg(long)]
        file: PathBuf,
        /// Inclusive M4 chunk indices/ranges to offer, for example `0-7,12`; defaults to all.
        #[arg(long)]
        available: Option<String>,
        /// Maximum provider upload rate in bytes per second; omitted or zero is unlimited.
        #[arg(long)]
        upload_limit: Option<u64>,
    },
    /// Fetch one content ID through an existing relay route.
    Fetch {
        /// Testnet v1 bootstrap address for automatic discovery and routing.
        #[arg(long)]
        bootstrap: Option<SocketAddr>,
        /// Manual M1 relay address.
        #[arg(long)]
        relay: Option<SocketAddr>,
        #[arg(long, value_parser = parse_psk)]
        key: Option<[u8; 32]>,
        #[arg(long)]
        route: Option<String>,
        #[arg(long)]
        content: triptorrent_core::ContentId,
        #[arg(long)]
        output: PathBuf,
        /// Maximum aggregate download rate in bytes per second; omitted or zero is unlimited.
        #[arg(long)]
        download_limit: Option<u64>,
    },
    /// Manage the persistent M5 node process.
    Node {
        #[command(subcommand)]
        command: NodeCommand,
    },
    /// Manage content in the persistent node.
    Content {
        #[command(subcommand)]
        command: ContentCommand,
    },
    /// Inspect persistent transfers.
    Transfer {
        #[command(subcommand)]
        command: TransferCommand,
    },
    /// Inspect or probe the explicitly experimental public-testnet profile.
    Testnet {
        #[command(subcommand)]
        command: TestnetCommand,
    },
}

#[derive(Debug, Subcommand)]
enum TestnetCommand {
    /// Print software, wire, network, discovery, and transfer identifiers.
    Status,
    /// Check bootstrap negotiation and optionally relay TCP reachability.
    Health {
        #[arg(long)]
        bootstrap: SocketAddr,
        #[arg(long)]
        relay: Option<SocketAddr>,
    },
    /// Perform a verified canonical transfer through bootstrap and relay infrastructure.
    Probe {
        #[arg(long)]
        bootstrap: SocketAddr,
        #[arg(
            long,
            default_value = "fdd7d693c7ca9bb83ff74f5b900f03eb7f3fbda3ab46043f7938c4473d72e199"
        )]
        content: triptorrent_core::ContentId,
        /// Preserve the verified artifact at this path; otherwise use and remove a temporary file.
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
enum NodeCommand {
    /// Create a local node configuration with a fresh API token.
    Init {
        #[arg(long, default_value = "triptorrent.toml")]
        config: PathBuf,
        #[arg(long, default_value = "triptorrent-data")]
        data_dir: PathBuf,
        #[arg(long, default_value = "127.0.0.1:7331")]
        api_listen: SocketAddr,
        #[arg(long)]
        bootstrap: Option<SocketAddr>,
    },
    /// Run the persistent node in the foreground.
    Start {
        #[arg(long, default_value = "triptorrent.toml")]
        config: PathBuf,
        #[arg(long)]
        data_dir: Option<PathBuf>,
        #[arg(long)]
        api_listen: Option<SocketAddr>,
        #[arg(long)]
        bootstrap: Option<SocketAddr>,
        #[arg(long)]
        download_limit: Option<u64>,
        #[arg(long)]
        upload_limit: Option<u64>,
        #[arg(long)]
        max_concurrent_transfers: Option<usize>,
        #[arg(long)]
        log_level: Option<String>,
    },
    /// Query persistent node health and counters.
    Status {
        #[arg(long, default_value = "triptorrent.toml")]
        config: PathBuf,
    },
    /// Request a clean node shutdown.
    Stop {
        #[arg(long, default_value = "triptorrent.toml")]
        config: PathBuf,
    },
    /// Start a persistent download managed by the node.
    Fetch {
        #[arg(long, default_value = "triptorrent.toml")]
        config: PathBuf,
        #[arg(long)]
        content: triptorrent_core::ContentId,
        /// Keep the completed content private instead of advertising it.
        #[arg(long)]
        no_share: bool,
    },
    /// Show bootstrap and provider diagnostics.
    Diagnostics {
        #[arg(long, default_value = "triptorrent.toml")]
        config: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
enum ContentCommand {
    /// Import a file into managed storage.
    Add {
        #[arg(long, default_value = "triptorrent.toml")]
        config: PathBuf,
        file: PathBuf,
        /// Import without advertising the content.
        #[arg(long)]
        no_share: bool,
    },
    /// List locally indexed content and integrity state.
    List {
        #[arg(long, default_value = "triptorrent.toml")]
        config: PathBuf,
    },
    /// Remove content metadata and optionally its managed bytes.
    Remove {
        #[arg(long, default_value = "triptorrent.toml")]
        config: PathBuf,
        #[arg(long)]
        content: triptorrent_core::ContentId,
        #[arg(long)]
        delete_bytes: bool,
    },
}

#[derive(Debug, Subcommand)]
enum TransferCommand {
    /// List persistent transfer records.
    List {
        #[arg(long, default_value = "triptorrent.toml")]
        config: PathBuf,
    },
    /// Inspect one persistent transfer.
    Show {
        #[arg(long, default_value = "triptorrent.toml")]
        config: PathBuf,
        id: i64,
    },
    /// Pause a running persistent download at a verified-chunk boundary.
    Pause {
        #[arg(long, default_value = "triptorrent.toml")]
        config: PathBuf,
        id: i64,
    },
    /// Resume an explicitly paused persistent download.
    Resume {
        #[arg(long, default_value = "triptorrent.toml")]
        config: PathBuf,
        id: i64,
    },
}

fn main() -> Result<()> {
    run(Cli::parse().command)
}

fn run(command: Command) -> Result<()> {
    match command {
        Command::Bootstrap { listen, lease_ms } => {
            let listener = TcpListener::bind(listen)
                .with_context(|| format!("failed to bind bootstrap at {listen}"))?;
            println!("TripTorrent Testnet v1 bootstrap listening on {listen}; lease={lease_ms}ms");
            triptorrent_overlay::serve(&listener, lease_ms).context("bootstrap stopped")?;
        }
        Command::Relay {
            listen,
            bootstrap,
            id,
            advertise,
        } => run_relay(listen, bootstrap, id, advertise)?,
        Command::Id { file } => println!("{}", identify(&file)?),
        Command::Share {
            bootstrap,
            relay,
            route,
            key,
            file,
            available,
            upload_limit,
        } => run_share(bootstrap, relay, route, key, &file, available, upload_limit)?,
        Command::Fetch {
            bootstrap,
            relay,
            route,
            key,
            content,
            output,
            download_limit,
        } => run_fetch(
            bootstrap,
            relay,
            route,
            key,
            content,
            &output,
            download_limit,
        )?,
        Command::Node { command } => run_node(command)?,
        Command::Content { command } => run_content(command)?,
        Command::Transfer { command } => run_transfer(command)?,
        Command::Testnet { command } => run_testnet(command)?,
    }
    Ok(())
}

fn run_testnet(command: TestnetCommand) -> Result<()> {
    match command {
        TestnetCommand::Status => print_json(&json!({
            "software_version": env!("CARGO_PKG_VERSION"),
            "wire_protocol_version": triptorrent_core::PROTOCOL_VERSION,
            "network": triptorrent_core::NETWORK_ID,
            "discovery_profile": triptorrent_core::DISCOVERY_PROFILE,
            "transfer_protocol": "relayed-swarm-v1",
            "public_endpoints_configured": false,
        }))?,
        TestnetCommand::Health { bootstrap, relay } => {
            BootstrapClient::new(bootstrap).health()?;
            if let Some(relay) = relay {
                TcpStream::connect_timeout(&relay, Duration::from_secs(3))
                    .with_context(|| format!("relay {relay} is unreachable"))?;
            }
            print_json(&json!({
                "status": "pass",
                "network": triptorrent_core::NETWORK_ID,
                "wire_protocol_version": triptorrent_core::PROTOCOL_VERSION,
                "bootstrap": bootstrap,
                "relay": relay,
            }))?;
        }
        TestnetCommand::Probe {
            bootstrap,
            content,
            output,
        } => {
            let temporary = output.is_none();
            let output = output.unwrap_or_else(|| {
                env::temp_dir().join(format!("triptorrent-probe-{}.bin", std::process::id()))
            });
            fetch_file_via_overlay_with_options(
                bootstrap,
                content,
                &output,
                FetchOptions::default(),
            )?;
            let bytes = fs::metadata(&output)?.len();
            print_json(&json!({
                "status": "pass",
                "software_version": env!("CARGO_PKG_VERSION"),
                "wire_protocol_version": triptorrent_core::PROTOCOL_VERSION,
                "network": triptorrent_core::NETWORK_ID,
                "discovery_profile": triptorrent_core::DISCOVERY_PROFILE,
                "transfer_protocol": "relayed-swarm-v1",
                "bootstrap": bootstrap,
                "content_id": content,
                "verified_bytes": bytes,
                "direct_provider_connection": false,
            }))?;
            if temporary {
                fs::remove_file(&output).with_context(|| {
                    format!("failed to remove probe output {}", output.display())
                })?;
            }
        }
    }
    Ok(())
}

struct CliTransferEngine;

struct NodeFetchMonitor<'a>(&'a FetchControl);

impl FetchMonitor for NodeFetchMonitor<'_> {
    fn should_continue(&self) -> bool {
        self.0.should_continue()
    }

    fn verified_progress(&self, progress: VerifiedTransferProgress) {
        self.0.report(VerifiedProgress {
            bytes_total: progress.bytes_total,
            verified_bytes: progress.verified_bytes,
            total_chunks: progress.total_chunks,
            verified_chunks: progress.verified_chunks,
            provider_count: progress.provider_count,
            retry_count: progress.retry_count,
            rejected_chunks: progress.rejected_chunks,
        });
    }
}

impl TransferEngine for CliTransferEngine {
    fn serve_once(
        &self,
        bootstrap: SocketAddr,
        path: &Path,
        upload_limit: u64,
        keep_running: &AtomicBool,
    ) -> Result<()> {
        share_file_via_overlay_with_control(
            bootstrap,
            path,
            &ShareOptions {
                availability: None,
                upload_limit: Some(upload_limit),
            },
            Some(keep_running),
        )?;
        Ok(())
    }

    fn fetch(
        &self,
        bootstrap: SocketAddr,
        content_id: triptorrent_core::ContentId,
        output: &Path,
        download_limit: u64,
        control: &FetchControl,
    ) -> Result<PersistentFetchStats> {
        let stats = fetch_file_via_overlay_with_monitor(
            bootstrap,
            content_id,
            output,
            FetchOptions {
                download_limit: Some(download_limit),
            },
            &NodeFetchMonitor(control),
        )?;
        let provider_contributions = stats
            .accepted_by_provider
            .iter()
            .map(|(provider, verified_chunks)| ProviderContribution {
                provider_id: provider.to_string(),
                verified_chunks: *verified_chunks,
                rejected_requests: stats
                    .rejected_by_provider
                    .iter()
                    .find_map(|(candidate, rejected)| (candidate == provider).then_some(*rejected))
                    .unwrap_or(0),
            })
            .collect();
        Ok(PersistentFetchStats {
            resumed_chunks: stats.resumed_chunks,
            provider_contributions,
            retry_count: stats.retries,
            rejected_chunks: stats
                .rejected_by_provider
                .iter()
                .map(|(_, rejected)| rejected)
                .sum(),
        })
    }
}

fn load_node_config(path: &Path, overrides: &ConfigOverrides) -> Result<NodeConfig> {
    NodeConfig::load(path, overrides)
}

fn print_json(value: &Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn api_client(config: &NodeConfig) -> LocalApiClient {
    LocalApiClient::new(triptorrent_node_api::LocalApiConnection {
        api_listen: config.api_listen,
        api_token: config.api_token.clone(),
    })
}

fn run_node(command: NodeCommand) -> Result<()> {
    match command {
        NodeCommand::Init {
            config,
            data_dir,
            api_listen,
            bootstrap,
        } => {
            let mut node = NodeConfig::initialized(data_dir)?;
            node.api_listen = api_listen;
            node.bootstrap = bootstrap;
            node.create(&config)?;
            println!("created node configuration {}", config.display());
        }
        NodeCommand::Start {
            config,
            data_dir,
            api_listen,
            bootstrap,
            download_limit,
            upload_limit,
            max_concurrent_transfers,
            log_level,
        } => {
            let config = load_node_config(
                &config,
                &ConfigOverrides {
                    data_dir,
                    api_listen,
                    bootstrap,
                    download_limit,
                    upload_limit,
                    max_concurrent_transfers,
                    log_level,
                },
            )?;
            run_daemon(&config, Arc::new(CliTransferEngine))?;
        }
        NodeCommand::Status { config } => {
            let config = load_node_config(&config, &ConfigOverrides::default())?;
            print_json(&api_client(&config).request_value("GET", "/v1/status", None)?)?;
        }
        NodeCommand::Stop { config } => {
            let config = load_node_config(&config, &ConfigOverrides::default())?;
            print_json(&api_client(&config).request_value(
                "POST",
                "/v1/shutdown",
                Some(json!({})),
            )?)?;
        }
        NodeCommand::Fetch {
            config,
            content,
            no_share,
        } => {
            let config = load_node_config(&config, &ConfigOverrides::default())?;
            print_json(&api_client(&config).request_value(
                "POST",
                "/v1/fetches",
                Some(json!({"content_id": content.to_string(), "shared": !no_share})),
            )?)?;
        }
        NodeCommand::Diagnostics { config } => {
            let config = load_node_config(&config, &ConfigOverrides::default())?;
            print_json(&api_client(&config).request_value("GET", "/v1/diagnostics", None)?)?;
        }
    }
    Ok(())
}

fn run_content(command: ContentCommand) -> Result<()> {
    match command {
        ContentCommand::Add {
            config,
            file,
            no_share,
        } => {
            let config = load_node_config(&config, &ConfigOverrides::default())?;
            print_json(&api_client(&config).request_value(
                "POST",
                "/v1/content",
                Some(json!({"path": file, "shared": !no_share})),
            )?)?;
        }
        ContentCommand::List { config } => {
            let config = load_node_config(&config, &ConfigOverrides::default())?;
            print_json(&api_client(&config).request_value("GET", "/v1/content", None)?)?;
        }
        ContentCommand::Remove {
            config,
            content,
            delete_bytes,
        } => {
            let config = load_node_config(&config, &ConfigOverrides::default())?;
            let target = format!("/v1/content/{content}?delete_bytes={delete_bytes}");
            print_json(&api_client(&config).request_value("DELETE", &target, None)?)?;
        }
    }
    Ok(())
}

fn run_transfer(command: TransferCommand) -> Result<()> {
    match command {
        TransferCommand::List { config } => {
            let config = load_node_config(&config, &ConfigOverrides::default())?;
            print_json(&api_client(&config).request_value("GET", "/v1/transfers", None)?)?;
        }
        TransferCommand::Show { config, id } => {
            let config = load_node_config(&config, &ConfigOverrides::default())?;
            print_json(&api_client(&config).request_value(
                "GET",
                &format!("/v1/transfers/{id}"),
                None,
            )?)?;
        }
        TransferCommand::Pause { config, id } => {
            let config = load_node_config(&config, &ConfigOverrides::default())?;
            print_json(&api_client(&config).pause_transfer(id)?)?;
        }
        TransferCommand::Resume { config, id } => {
            let config = load_node_config(&config, &ConfigOverrides::default())?;
            print_json(&api_client(&config).resume_transfer(id)?)?;
        }
    }
    Ok(())
}

fn run_relay(
    listen: SocketAddr,
    bootstrap: Option<SocketAddr>,
    id: Option<String>,
    advertise: Option<SocketAddr>,
) -> Result<()> {
    let listener =
        TcpListener::bind(listen).with_context(|| format!("failed to bind relay at {listen}"))?;
    let bound_address = listener.local_addr()?;
    if let Some(bootstrap) = bootstrap {
        let relay_id = id.unwrap_or_else(|| format!("relay-{}", bound_address.port()));
        let relay = RelayAdvertisement {
            relay_id: relay_id.clone(),
            address: advertise.unwrap_or(bound_address).to_string(),
        };
        let client = BootstrapClient::new(bootstrap);
        let lease_ms = client.register_relay(relay.clone())?;
        let heartbeat_relay_id = relay_id.clone();
        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_millis((lease_ms / 3).max(100)));
                if client.heartbeat_relay(&heartbeat_relay_id).is_err() {
                    let _ = client.register_relay(relay.clone());
                }
            }
        });
        println!("relay {relay_id} registered with bootstrap {bootstrap}");
    }
    println!("relay listening on {bound_address}");
    triptorrent_relay::serve(&listener).context("relay stopped")?;
    Ok(())
}

fn run_share(
    bootstrap: Option<SocketAddr>,
    relay: Option<SocketAddr>,
    route: Option<String>,
    key: Option<[u8; 32]>,
    file: &Path,
    available: Option<String>,
    upload_limit: Option<u64>,
) -> Result<()> {
    let content_id = identify(file)?;
    if let Some(bootstrap) = bootstrap {
        if relay.is_some() || route.is_some() || key.is_some() {
            bail!("M4 share does not accept --relay, --route, or --key");
        }
        println!("advertising {content_id} through bootstrap {bootstrap}");
        share_file_via_overlay_with_options(
            bootstrap,
            file,
            &ShareOptions {
                availability: available,
                upload_limit,
            },
        )?;
    } else {
        if available.is_some() || upload_limit.is_some() {
            bail!("manual M1 share does not accept --available or --upload-limit");
        }
        let relay = relay.context("manual M1 share requires --relay")?;
        let route = route.context("manual M1 share requires --route")?;
        let key = key.context("manual M1 share requires --key")?;
        println!("sharing {content_id}; waiting for receiver on route {route}");
        share_file(relay, &route, &key, file)?;
    }
    println!("transfer complete");
    Ok(())
}

fn run_fetch(
    bootstrap: Option<SocketAddr>,
    relay: Option<SocketAddr>,
    route: Option<String>,
    key: Option<[u8; 32]>,
    content: triptorrent_core::ContentId,
    output: &Path,
    download_limit: Option<u64>,
) -> Result<()> {
    if let Some(bootstrap) = bootstrap {
        if relay.is_some() || route.is_some() || key.is_some() {
            bail!("M4 fetch does not accept --relay, --route, or --key");
        }
        fetch_file_via_overlay_with_options(
            bootstrap,
            content,
            output,
            FetchOptions { download_limit },
        )?;
    } else {
        if download_limit.is_some() {
            bail!("manual M1 fetch does not accept --download-limit");
        }
        let relay = relay.context("manual M1 fetch requires --relay")?;
        let route = route.context("manual M1 fetch requires --route")?;
        let key = key.context("manual M1 fetch requires --key")?;
        fetch_file(relay, &route, &key, content, output)?;
    }
    println!("verified {content} and wrote {}", output.display());
    Ok(())
}
