#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = triptorrent_interop::parse_bencode(data);
    let _ = triptorrent_interop::torrent_identity(data);
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = triptorrent_interop::parse_magnet(text);
        let _ = triptorrent_interop::sanitize_path_component(text);
    }
});
