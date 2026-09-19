use std::fs;
use std::net::TcpListener;
use std::thread;
use tempfile::tempdir;
use triptorrent_cli::{fetch_file, identify, share_file};

#[test]
fn peer_b_reconstructs_exact_file_through_relay() {
    let directory = tempdir().unwrap();
    let source = directory.path().join("source.bin");
    let destination = directory.path().join("destination.bin");
    let mut expected = Vec::new();
    for value in 0_u8..=255 {
        expected.extend([value; 257]);
    }
    fs::write(&source, &expected).unwrap();
    let content_id = identify(&source).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let relay_address = listener.local_addr().unwrap();
    let _relay = thread::spawn(move || triptorrent_relay::serve(&listener));

    let sender_path = source.clone();
    let key = [0x42; 32];
    let sender =
        thread::spawn(move || share_file(relay_address, "integration", &key, &sender_path));
    fetch_file(relay_address, "integration", &key, content_id, &destination).unwrap();

    assert_eq!(sender.join().unwrap().unwrap(), content_id);
    assert_eq!(fs::read(destination).unwrap(), expected);
}
