#![doc = "A bounded relay that pairs routes and forwards opaque frames."]

use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const REGISTRATION_MAGIC: &[u8; 4] = b"TTR0";
const MAX_FRAME_LENGTH: usize = 1024 * 1024;
const MAX_ROUTE_LENGTH: usize = 128;
const DEFAULT_MAX_PENDING_ROUTES: usize = 1_024;
const DEFAULT_MAX_ACTIVE_PAIRS: usize = 256;
const DEFAULT_MAX_REGISTRATION_CONNECTIONS: usize = 256;
const DEFAULT_PENDING_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Role {
    Sender,
    Receiver,
}

struct PendingRoute {
    sender: Option<TcpStream>,
    receiver: Option<TcpStream>,
    created: Instant,
}

impl PendingRoute {
    fn new() -> Self {
        Self {
            sender: None,
            receiver: None,
            created: Instant::now(),
        }
    }
}

/// Explicit resource limits for the temporary relay service.
#[derive(Clone, Copy, Debug)]
pub struct RelayLimits {
    /// Maximum incomplete route registrations retained at once.
    pub max_pending_routes: usize,
    /// Maximum concurrently forwarding route pairs.
    pub max_active_pairs: usize,
    /// Maximum connections concurrently reading or processing registration.
    pub max_registration_connections: usize,
    /// Maximum lifetime of an unpaired registration.
    pub pending_timeout: Duration,
}

impl Default for RelayLimits {
    fn default() -> Self {
        Self {
            max_pending_routes: DEFAULT_MAX_PENDING_ROUTES,
            max_active_pairs: DEFAULT_MAX_ACTIVE_PAIRS,
            max_registration_connections: DEFAULT_MAX_REGISTRATION_CONNECTIONS,
            pending_timeout: DEFAULT_PENDING_TIMEOUT,
        }
    }
}

struct RelayState {
    pending: HashMap<String, PendingRoute>,
    completed: VecDeque<String>,
    limits: RelayLimits,
}

impl RelayState {
    fn new(limits: RelayLimits) -> Self {
        Self {
            pending: HashMap::new(),
            completed: VecDeque::new(),
            limits,
        }
    }

    fn prune(&mut self) {
        self.pending
            .retain(|_, route| route.created.elapsed() < self.limits.pending_timeout);
    }

    fn register(
        &mut self,
        route: String,
        role: Role,
        stream: TcpStream,
    ) -> io::Result<Option<(TcpStream, TcpStream)>> {
        self.prune();
        if self.completed.contains(&route) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "completed route cannot be reused",
            ));
        }
        if !self.pending.contains_key(&route)
            && self.pending.len() >= self.limits.max_pending_routes
        {
            return Err(io::Error::other("relay pending-route limit reached"));
        }
        let entry = self
            .pending
            .entry(route.clone())
            .or_insert_with(PendingRoute::new);
        match role {
            Role::Sender if entry.sender.is_none() => entry.sender = Some(stream),
            Role::Receiver if entry.receiver.is_none() => entry.receiver = Some(stream),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "duplicate relay role",
                ));
            }
        }
        if entry.sender.is_none() || entry.receiver.is_none() {
            return Ok(None);
        }
        let mut paired = self
            .pending
            .remove(&route)
            .ok_or_else(|| io::Error::other("paired route disappeared"))?;
        self.completed.push_back(route);
        while self.completed.len() > self.limits.max_pending_routes {
            self.completed.pop_front();
        }
        Ok(Some((
            paired
                .sender
                .take()
                .ok_or_else(|| io::Error::other("missing paired sender"))?,
            paired
                .receiver
                .take()
                .ok_or_else(|| io::Error::other("missing paired receiver"))?,
        )))
    }
}

/// Runs a relay until the listener is closed or an accept operation fails.
///
/// # Errors
///
/// Returns an I/O error if accepting a connection fails.
pub fn serve(listener: &TcpListener) -> io::Result<()> {
    serve_with_limits(listener, RelayLimits::default())
}

/// Runs the relay with explicit resource limits, primarily for bounded tests.
///
/// Malformed client registrations are isolated to that connection. Listener
/// acceptance failures remain service-level errors.
///
/// # Errors
///
/// Returns an I/O error if accepting a connection fails.
pub fn serve_with_limits(listener: &TcpListener, limits: RelayLimits) -> io::Result<()> {
    let state = Arc::new(Mutex::new(RelayState::new(limits)));
    let active = Arc::new(AtomicUsize::new(0));
    let registrations = Arc::new(AtomicUsize::new(0));
    for incoming in listener.incoming() {
        let mut stream = incoming?;
        if !try_acquire(&registrations, limits.max_registration_connections) {
            continue;
        }
        let state = Arc::clone(&state);
        let active = Arc::clone(&active);
        let registrations = Arc::clone(&registrations);
        thread::spawn(move || {
            let pair = (|| {
                stream.set_read_timeout(Some(Duration::from_secs(10)))?;
                let (route, role) = read_registration(&mut stream)?;
                stream.set_read_timeout(None)?;
                state
                    .lock()
                    .map_err(|_| io::Error::other("relay state lock poisoned"))?
                    .register(route, role, stream)
            })();
            registrations.fetch_sub(1, Ordering::Relaxed);
            let Ok(Some((sender, receiver))) = pair else {
                return;
            };
            if !try_acquire(&active, limits.max_active_pairs) {
                return;
            }
            thread::spawn(move || {
                let _ = relay_pair(sender, receiver);
                active.fetch_sub(1, Ordering::Relaxed);
            });
        });
    }
    Ok(())
}

fn try_acquire(counter: &AtomicUsize, limit: usize) -> bool {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            (current < limit).then_some(current + 1)
        })
        .is_ok()
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
    relay_pair(
        sender.ok_or_else(|| io::Error::other("missing sender"))?,
        receiver.ok_or_else(|| io::Error::other("missing receiver"))?,
    )
}

fn read_registration(stream: &mut TcpStream) -> io::Result<(String, Role)> {
    let frame = read_frame(stream)?.ok_or_else(|| {
        io::Error::new(io::ErrorKind::UnexpectedEof, "missing relay registration")
    })?;
    parse_registration(&frame)
}

fn parse_registration(frame: &[u8]) -> io::Result<(String, Role)> {
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
    if route.len() > MAX_ROUTE_LENGTH
        || !route
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid relay route",
        ));
    }
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

        let opaque = b"not a protocol message or plaintext file";
        write_frame(&mut sender, opaque).unwrap();
        assert_eq!(read_frame(&mut receiver).unwrap().unwrap(), opaque);
        sender.shutdown(Shutdown::Both).unwrap();
        receiver.shutdown(Shutdown::Both).unwrap();
        relay.join().unwrap().unwrap();
    }

    #[test]
    fn malformed_registration_and_oversized_frame_fail_closed() {
        for malformed in [
            b"bad".as_slice(),
            b"TTR0\x02route".as_slice(),
            b"TTR0\0../route".as_slice(),
        ] {
            assert!(parse_registration(malformed).is_err());
        }
        let mut encoded = Vec::new();
        encoded.extend_from_slice(&u32::try_from(MAX_FRAME_LENGTH + 1).unwrap().to_be_bytes());
        assert!(read_frame(&mut encoded.as_slice()).is_err());
    }

    #[test]
    fn slow_registration_does_not_block_other_routes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || serve_with_limits(&listener, RelayLimits::default()));
        let slow = TcpStream::connect(address).unwrap();
        thread::sleep(Duration::from_millis(20));

        let mut sender = TcpStream::connect(address).unwrap();
        write_frame(&mut sender, b"TTR0\0concurrent").unwrap();
        let mut receiver = TcpStream::connect(address).unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        write_frame(&mut receiver, b"TTR0\x01concurrent").unwrap();
        write_frame(&mut sender, b"opaque").unwrap();
        assert_eq!(read_frame(&mut receiver).unwrap().unwrap(), b"opaque");

        drop(slow);
        sender.shutdown(Shutdown::Both).unwrap();
        receiver.shutdown(Shutdown::Both).unwrap();
    }

    fn loopback_stream() -> TcpStream {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let connector = thread::spawn(move || TcpStream::connect(address).unwrap());
        let (accepted, _) = listener.accept().unwrap();
        drop(connector.join().unwrap());
        accepted
    }

    #[test]
    fn hostile_routes_are_bounded_cleaned_and_not_reused() {
        let limits = RelayLimits {
            max_pending_routes: 1,
            max_active_pairs: 1,
            max_registration_connections: 1,
            pending_timeout: Duration::from_secs(1),
        };
        let mut state = RelayState::new(limits);

        assert!(
            state
                .register("squat".into(), Role::Sender, loopback_stream())
                .unwrap()
                .is_none()
        );
        assert!(
            state
                .register("other".into(), Role::Sender, loopback_stream())
                .is_err()
        );
        assert!(
            state
                .register("squat".into(), Role::Sender, loopback_stream())
                .is_err()
        );

        let pair = state
            .register("squat".into(), Role::Receiver, loopback_stream())
            .unwrap();
        assert!(pair.is_some());
        drop(pair);
        assert!(
            state
                .register("squat".into(), Role::Sender, loopback_stream())
                .is_err()
        );

        state.limits.pending_timeout = Duration::ZERO;
        assert!(
            state
                .register("fresh".into(), Role::Sender, loopback_stream())
                .unwrap()
                .is_none()
        );
        assert_eq!(state.pending.len(), 1);

        let counter = AtomicUsize::new(0);
        assert!(try_acquire(&counter, 1));
        assert!(!try_acquire(&counter, 1));
        counter.fetch_sub(1, Ordering::Relaxed);
        assert!(try_acquire(&counter, 1));
    }
}
