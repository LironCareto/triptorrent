use std::fs;
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::thread;
use tempfile::tempdir;
use triptorrent_cli::{fetch_file_via_overlay, identify, share_file_via_overlay};
use triptorrent_overlay::BootstrapClient;
use triptorrent_protocol::RelayAdvertisement;

fn start_bootstrap(lease_ms: u64) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    thread::spawn(move || triptorrent_overlay::serve(&listener, lease_ms));
    address
}

fn start_relay(bootstrap: SocketAddr, relay_id: &str) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    BootstrapClient::new(bootstrap)
        .register_relay(RelayAdvertisement {
            relay_id: relay_id.into(),
            address: address.to_string(),
        })
        .unwrap();
    thread::spawn(move || triptorrent_relay::serve(&listener));
    address
}

fn write_source(path: &PathBuf, seed: u8) -> Vec<u8> {
    let mut bytes = Vec::new();
    for value in 0_u8..=255 {
        bytes.extend([value ^ seed; 257]);
    }
    fs::write(path, &bytes).unwrap();
    bytes
}

#[test]
fn transfer_uses_discovery_and_an_automatic_route() {
    let directory = tempdir().unwrap();
    let source = directory.path().join("source.bin");
    let destination = directory.path().join("destination.bin");
    let expected = write_source(&source, 0x21);
    let content_id = identify(&source).unwrap();
    let bootstrap = start_bootstrap(30_000);
    start_relay(bootstrap, "relay-a");

    let sender_path = source.clone();
    let sender = thread::spawn(move || share_file_via_overlay(bootstrap, &sender_path));
    fetch_file_via_overlay(bootstrap, content_id, &destination).unwrap();

    assert_eq!(sender.join().unwrap().unwrap(), content_id);
    assert_eq!(fs::read(destination).unwrap(), expected);
}

#[test]
fn dead_relay_is_replaced_before_transfer() {
    let directory = tempdir().unwrap();
    let source = directory.path().join("failover-source.bin");
    let destination = directory.path().join("failover-destination.bin");
    let expected = write_source(&source, 0x42);
    let content_id = identify(&source).unwrap();
    let bootstrap = start_bootstrap(30_000);

    let dead_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let dead_address = dead_listener.local_addr().unwrap();
    drop(dead_listener);
    BootstrapClient::new(bootstrap)
        .register_relay(RelayAdvertisement {
            relay_id: "relay-a-dead".into(),
            address: dead_address.to_string(),
        })
        .unwrap();
    start_relay(bootstrap, "relay-b-live");

    let sender_path = source.clone();
    let sender = thread::spawn(move || share_file_via_overlay(bootstrap, &sender_path));
    fetch_file_via_overlay(bootstrap, content_id, &destination).unwrap();

    assert_eq!(sender.join().unwrap().unwrap(), content_id);
    assert_eq!(fs::read(destination).unwrap(), expected);
}

#[test]
fn simultaneous_peers_use_independent_routes() {
    let directory = tempdir().unwrap();
    let source_a = directory.path().join("source-a.bin");
    let source_b = directory.path().join("source-b.bin");
    let destination_a = directory.path().join("destination-a.bin");
    let destination_b = directory.path().join("destination-b.bin");
    let expected_a = write_source(&source_a, 0x11);
    let expected_b = write_source(&source_b, 0x77);
    let content_a = identify(&source_a).unwrap();
    let content_b = identify(&source_b).unwrap();
    let bootstrap = start_bootstrap(30_000);
    start_relay(bootstrap, "relay-shared");

    let sender_a = thread::spawn(move || share_file_via_overlay(bootstrap, &source_a));
    let sender_b = thread::spawn(move || share_file_via_overlay(bootstrap, &source_b));
    let receiver_a =
        thread::spawn(move || fetch_file_via_overlay(bootstrap, content_a, &destination_a));
    let receiver_b =
        thread::spawn(move || fetch_file_via_overlay(bootstrap, content_b, &destination_b));

    receiver_a.join().unwrap().unwrap();
    receiver_b.join().unwrap().unwrap();
    assert_eq!(sender_a.join().unwrap().unwrap(), content_a);
    assert_eq!(sender_b.join().unwrap().unwrap(), content_b);
    assert_eq!(
        fs::read(directory.path().join("destination-a.bin")).unwrap(),
        expected_a
    );
    assert_eq!(
        fs::read(directory.path().join("destination-b.bin")).unwrap(),
        expected_b
    );
}
