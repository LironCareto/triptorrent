use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use triptorrent_cli::{fetch_file, identify, parse_psk, share_file};

#[derive(Debug, Parser)]
#[command(name = "triptorrent", about = "Experimental local TripTorrent M1 demo")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the opaque local TCP relay.
    Relay {
        /// Address on which the relay accepts peer connections.
        #[arg(long, default_value = "127.0.0.1:7000")]
        listen: SocketAddr,
    },
    /// Print the experimental content ID for a file.
    Id {
        /// File to identify.
        file: PathBuf,
    },
    /// Share one file through an existing relay route.
    Share {
        #[arg(long, default_value = "127.0.0.1:7000")]
        relay: SocketAddr,
        #[arg(long)]
        route: String,
        #[arg(long, value_parser = parse_psk)]
        key: [u8; 32],
        #[arg(long)]
        file: PathBuf,
    },
    /// Fetch one content ID through an existing relay route.
    Fetch {
        #[arg(long, default_value = "127.0.0.1:7000")]
        relay: SocketAddr,
        #[arg(long)]
        route: String,
        #[arg(long, value_parser = parse_psk)]
        key: [u8; 32],
        #[arg(long)]
        content: triptorrent_core::ContentId,
        #[arg(long)]
        output: PathBuf,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Relay { listen } => {
            let listener = TcpListener::bind(listen)
                .with_context(|| format!("failed to bind relay at {listen}"))?;
            println!("relay listening on {listen}");
            triptorrent_relay::serve(&listener).context("relay stopped")?;
        }
        Command::Id { file } => println!("{}", identify(&file)?),
        Command::Share {
            relay,
            route,
            key,
            file,
        } => {
            let content_id = identify(&file)?;
            println!("sharing {content_id}; waiting for receiver on route {route}");
            share_file(relay, &route, &key, &file)?;
            println!("transfer complete");
        }
        Command::Fetch {
            relay,
            route,
            key,
            content,
            output,
        } => {
            fetch_file(relay, &route, &key, content, &output)?;
            println!("verified {content} and wrote {}", output.display());
        }
    }
    Ok(())
}
