#![doc = "A local relay that pairs routes and forwards opaque frames."]

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

const REGISTRATION_MAGIC: &[u8; 4] = b"TTR0";
const MAX_FRAME_LENGTH: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Role {
    Sender,
    Receiver,
}

#[derive(Default)]
struct PendingRoute {
    sender: Option<TcpStream>,
    receiver: Option<TcpStream>,
}

/// Runs a relay until the listener is closed or an accept operation fails.
///
/// # Errors
///
/// Returns an I/O error if accepting a connection fails.
pub fn serve(listener: &TcpListener) -> io::Result<()> {
    let mut pending = HashMap::<String, PendingRoute>::new();
    for incoming in listener.incoming() {
        let mut stream = incoming?;
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        let (route, role) = read_registration(&mut stream)?;
        stream.set_read_timeout(None)?;
        let entry = pending.entry(route.clone()).or_default();
        match role {
            Role::Sender => entry.sender = Some(stream),
            Role::Receiver => entry.receiver = Some(stream),
        }
        if entry.sender.is_some() && entry.receiver.is_some() {
            let sender = entry
                .sender
                .take()
                .ok_or_else(|| io::Error::other("missing sender after route was paired"))?;
            let receiver = entry
                .receiver
                .take()
                .ok_or_else(|| io::Error::other("missing receiver after route was paired"))?;
            pending.remove(&route);
            thread::spawn(move || {
                let _ = relay_pair(sender, receiver);
            });
        }
    }
    Ok(())
}

/// Accepts and serves exactly one sender/receiver pair, primarily for tests.
///
/// # Errors
///
/// Returns an I/O error for invalid registration, connection, or forwarding failures.
pub fn serve_once(listener: &TcpListener) -> io::Result<()> {
    let mut sender = None;
    let mut receiver = None;
    let mut expected_route = None;
    while sender.is_none() || receiver.is_none() {
        let (mut stream, _) = listener.accept()?;
        let (route, role) = read_registration(&mut stream)?;
        if let Some(expected) = &expected_route {
            if expected != &route {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "route IDs did not match",
                ));
            }
        } else {
            expected_route = Some(route);
        }
        match role {
            Role::Sender if sender.is_none() => sender = Some(stream),
            Role::Receiver if receiver.is_none() => receiver = Some(stream),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "duplicate relay role",
                ));
            }
        }
    }
    let sender = sender.ok_or_else(|| io::Error::other("missing sender"))?;
    let receiver = receiver.ok_or_else(|| io::Error::other("missing receiver"))?;
    relay_pair(sender, receiver)
}

fn read_registration(stream: &mut TcpStream) -> io::Result<(String, Role)> {
    let frame = read_frame(stream)?.ok_or_else(|| {
        io::Error::new(io::ErrorKind::UnexpectedEof, "missing relay registration")
    })?;
    if frame.len() < 6 || &frame[..4] != REGISTRATION_MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid relay registration",
        ));
    }
    let role = match frame[4] {
        0 => Role::Sender,
        1 => Role::Receiver,
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid role")),
    };
    let route = String::from_utf8(frame[5..].to_vec())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "route is not UTF-8"))?;
    Ok((route, role))
}

fn relay_pair(sender: TcpStream, receiver: TcpStream) -> io::Result<()> {
    let sender_read = sender.try_clone()?;
    let receiver_write = receiver.try_clone()?;
    let sender_to_receiver = thread::spawn(move || forward(sender_read, receiver_write));
    let receiver_to_sender = forward(receiver, sender);
    let sender_to_receiver = sender_to_receiver
        .join()
        .map_err(|_| io::Error::other("relay forwarding thread panicked"))?;
    receiver_to_sender.and(sender_to_receiver)
}

fn forward(mut source: TcpStream, mut destination: TcpStream) -> io::Result<()> {
    while let Some(frame) = read_frame(&mut source)? {
        write_frame(&mut destination, &frame)?;
    }
    destination.shutdown(Shutdown::Write)
}

fn write_frame(writer: &mut impl Write, frame: &[u8]) -> io::Result<()> {
    if frame.len() > MAX_FRAME_LENGTH {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    let length = u32::try_from(frame.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "frame length overflow"))?;
    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(frame)?;
    writer.flush()
}

fn read_frame(reader: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut length = [0_u8; 4];
    match reader.read_exact(&mut length) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let length = usize::try_from(u32::from_be_bytes(length))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid frame length"))?;
    if length > MAX_FRAME_LENGTH {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    let mut frame = vec![0; length];
    reader.read_exact(&mut frame)?;
    Ok(Some(frame))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_forwards_opaque_frames_without_interpreting_them() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let relay = thread::spawn(move || serve_once(&listener));

        let mut sender = TcpStream::connect(address).unwrap();
        write_frame(&mut sender, b"TTR0\0opaque-test").unwrap();
        let mut receiver = TcpStream::connect(address).unwrap();
        write_frame(&mut receiver, b"TTR0\x01opaque-test").unwrap();

        let arbitrary_ciphertext = b"not a protocol message or plaintext file";
        write_frame(&mut sender, arbitrary_ciphertext).unwrap();
        assert_eq!(
            read_frame(&mut receiver).unwrap().unwrap(),
            arbitrary_ciphertext
        );
        sender.shutdown(Shutdown::Both).unwrap();
        receiver.shutdown(Shutdown::Both).unwrap();
        relay.join().unwrap().unwrap();
    }
}
