#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = triptorrent_protocol::decode(data);
    let _ = triptorrent_protocol::decode_overlay_request(data);
    let _ = triptorrent_protocol::decode_overlay_response(data);
});
