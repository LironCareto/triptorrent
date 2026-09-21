# Independent Python Testnet v1 implementation

This directory contains a small receiver written from `spec/testnet-v1.md`. It does not import Rust code, generated bindings, or invoke TripTorrent to encode messages.

Install the pinned libraries and run conformance:

```bash
python -m pip install -r interop/python/requirements.txt
python interop/python/conformance.py
```

Run a real receiver against an advertised Rust provider:

```bash
python interop/python/triptorrent_v1.py fetch --bootstrap 127.0.0.1:7100 --content <ID> --output received.bin
```

The implementation uses `noiseprotocol` for Noise KN and `blake3` for normative content and chunk digests. It is intentionally limited to the receiver side of the M9 vertical slice.
