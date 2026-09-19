use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use triptorrent_cli::{
    fetch_file, fetch_file_via_overlay, identify, parse_psk, share_file, share_file_via_overlay,
};
use triptorrent_overlay::BootstrapClient;
use triptorrent_protocol::RelayAdvertisement;

#[derive(Debug, Parser)]
#[command(
    name = "triptorrent",
    about = "Experimental local TripTorrent M1/M2 demo"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the temporary M2 bootstrap and rendezvous service.
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
        /// M2 bootstrap address. Omit it to retain manual M1 behavior.
        #[arg(long)]
        bootstrap: Option<SocketAddr>,
        /// Ephemeral M2 relay identifier.
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
        /// M2 bootstrap address for automatic discovery and routing.
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
    },
    /// Fetch one content ID through an existing relay route.
    Fetch {
        /// M2 bootstrap address for automatic discovery and routing.
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
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Bootstrap { listen, lease_ms } => {
            let listener = TcpListener::bind(listen)
                .with_context(|| format!("failed to bind bootstrap at {listen}"))?;
            println!("temporary M2 bootstrap listening on {listen}; lease={lease_ms}ms");
            triptorrent_overlay::serve(&listener, lease_ms).context("bootstrap stopped")?;
        }
        Command::Relay {
            listen,
            bootstrap,
            id,
            advertise,
        } => {
            let listener = TcpListener::bind(listen)
                .with_context(|| format!("failed to bind relay at {listen}"))?;
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
        }
        Command::Id { file } => println!("{}", identify(&file)?),
        Command::Share {
            bootstrap,
            relay,
            route,
            key,
            file,
        } => {
            let content_id = identify(&file)?;
            if let Some(bootstrap) = bootstrap {
                if relay.is_some() || route.is_some() || key.is_some() {
                    bail!("M2 share does not accept --relay, --route, or --key");
                }
                println!("advertising {content_id} through bootstrap {bootstrap}");
                share_file_via_overlay(bootstrap, &file)?;
            } else {
                let relay = relay.context("manual M1 share requires --relay")?;
                let route = route.context("manual M1 share requires --route")?;
                let key = key.context("manual M1 share requires --key")?;
                println!("sharing {content_id}; waiting for receiver on route {route}");
                share_file(relay, &route, &key, &file)?;
            }
            println!("transfer complete");
        }
        Command::Fetch {
            bootstrap,
            relay,
            route,
            key,
            content,
            output,
        } => {
            if let Some(bootstrap) = bootstrap {
                if relay.is_some() || route.is_some() || key.is_some() {
                    bail!("M2 fetch does not accept --relay, --route, or --key");
                }
                fetch_file_via_overlay(bootstrap, content, &output)?;
            } else {
                let relay = relay.context("manual M1 fetch requires --relay")?;
                let route = route.context("manual M1 fetch requires --route")?;
                let key = key.context("manual M1 fetch requires --key")?;
                fetch_file(relay, &route, &key, content, &output)?;
            }
            println!("verified {content} and wrote {}", output.display());
        }
    }
    Ok(())
}
