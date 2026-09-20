use anyhow::Result;
use clap::Parser;
use std::net::SocketAddr;
use std::path::PathBuf;
use triptorrent_desktop::{DesktopApp, DesktopController, DesktopOptions};

#[derive(Debug, Parser)]
#[command(
    name = "triptorrent-desktop",
    about = "TripTorrent graphical reference client"
)]
struct Args {
    /// Existing or first-run node configuration path.
    #[arg(long)]
    config: Option<PathBuf>,
    /// Compiled `triptorrent` sidecar executable.
    #[arg(long)]
    node_binary: Option<PathBuf>,
    /// Managed data directory used only when creating a first-run config.
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// Loopback API endpoint used only when creating a first-run config.
    #[arg(long)]
    api_listen: Option<SocketAddr>,
    /// Temporary M2 bootstrap used only when creating a first-run config.
    #[arg(long)]
    bootstrap: Option<SocketAddr>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let mut options = DesktopOptions::system_default()?;
    if let Some(config) = args
        .config
        .or_else(|| std::env::var_os("TRIPTORRENT_CONFIG").map(PathBuf::from))
    {
        options.config_path = config;
    }
    if let Some(binary) = args
        .node_binary
        .or_else(|| std::env::var_os("TRIPTORRENT_NODE_BINARY").map(PathBuf::from))
    {
        options.node_binary = binary;
    }
    if let Some(data_dir) = args.data_dir {
        options.data_dir = data_dir;
    }
    if let Some(api_listen) = args.api_listen {
        options.api_listen = api_listen;
    }
    options.bootstrap = args.bootstrap;

    let controller = DesktopController::start(options);
    let native = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("TripTorrent")
            .with_inner_size([1050.0, 700.0])
            .with_min_inner_size([820.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "TripTorrent",
        native,
        Box::new(move |_context| Ok(Box::new(DesktopApp::new(controller)))),
    )
    .map_err(|error| anyhow::anyhow!(error.to_string()))
}
