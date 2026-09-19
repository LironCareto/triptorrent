#![doc = "Temporary M2 bootstrap, discovery, lease, and simulation support."]

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use thiserror::Error;
use triptorrent_core::{ContentId, PeerId};
use triptorrent_protocol::{
    OverlayRequest, OverlayResponse, PeerAdvertisement, PeerCapability, RelayAdvertisement,
    RouteAssignment,
};

const MAX_CONTROL_FRAME: usize = 256 * 1024;
const CONTROL_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
struct PeerLease {
    advertisement: PeerAdvertisement,
    expires_at: u64,
}

#[derive(Clone)]
struct RelayLease {
    advertisement: RelayAdvertisement,
    expires_at: u64,
}

#[derive(Clone)]
struct QueuedAssignment {
    assignment: RouteAssignment,
    expires_at: u64,
}

/// Counts of currently live overlay state, used by tests and simulations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OverlaySnapshot {
    /// Number of live peer leases.
    pub peers: usize,
    /// Number of live relay leases.
    pub relays: usize,
    /// Total content advertisements across live peers.
    pub content_advertisements: usize,
}

/// Deterministic in-memory state machine for the temporary M2 bootstrap.
pub struct Registry {
    lease_ms: u64,
    peers: BTreeMap<PeerId, PeerLease>,
    relays: BTreeMap<String, RelayLease>,
    assignments: HashMap<PeerId, VecDeque<QueuedAssignment>>,
    next_route: u64,
}

impl Registry {
    /// Creates an empty registry whose entries expire after `lease_ms`.
    #[must_use]
    pub fn new(lease_ms: u64) -> Self {
        Self {
            lease_ms: lease_ms.max(1),
            peers: BTreeMap::new(),
            relays: BTreeMap::new(),
            assignments: HashMap::new(),
            next_route: 0,
        }
    }

    /// Returns the server-controlled lease duration.
    #[must_use]
    pub const fn lease_ms(&self) -> u64 {
        self.lease_ms
    }

    /// Registers or replaces one peer advertisement.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError`] if the identity does not match its public key or no content is
    /// advertised.
    pub fn register_peer(
        &mut self,
        now_ms: u64,
        advertisement: PeerAdvertisement,
    ) -> Result<(), RegistryError> {
        if advertisement.peer_id != PeerId::from_public_key(&advertisement.public_key) {
            return Err(RegistryError::PeerIdentityMismatch);
        }
        if advertisement.content_ids.is_empty() {
            return Err(RegistryError::NoContent);
        }
        if !advertisement
            .capabilities
            .contains(&PeerCapability::RelayedTransferV0)
        {
            return Err(RegistryError::UnsupportedPeer);
        }
        self.purge(now_ms);
        self.peers.insert(
            advertisement.peer_id,
            PeerLease {
                advertisement,
                expires_at: now_ms.saturating_add(self.lease_ms),
            },
        );
        Ok(())
    }

    /// Renews one existing peer lease.
    #[must_use]
    pub fn heartbeat_peer(&mut self, now_ms: u64, peer_id: PeerId) -> bool {
        self.purge(now_ms);
        let Some(peer) = self.peers.get_mut(&peer_id) else {
            return false;
        };
        peer.expires_at = now_ms.saturating_add(self.lease_ms);
        true
    }

    /// Registers or replaces one relay advertisement.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError`] if its identifier or TCP address is invalid.
    pub fn register_relay(
        &mut self,
        now_ms: u64,
        advertisement: RelayAdvertisement,
    ) -> Result<(), RegistryError> {
        validate_relay_id(&advertisement.relay_id)?;
        advertisement
            .address
            .parse::<SocketAddr>()
            .map_err(|_| RegistryError::InvalidRelayAddress)?;
        self.purge(now_ms);
        self.relays.insert(
            advertisement.relay_id.clone(),
            RelayLease {
                advertisement,
                expires_at: now_ms.saturating_add(self.lease_ms),
            },
        );
        Ok(())
    }

    /// Renews one existing relay lease.
    #[must_use]
    pub fn heartbeat_relay(&mut self, now_ms: u64, relay_id: &str) -> bool {
        self.purge(now_ms);
        let Some(relay) = self.relays.get_mut(relay_id) else {
            return false;
        };
        relay.expires_at = now_ms.saturating_add(self.lease_ms);
        true
    }

    /// Selects a live provider and relay, creates a route, and queues it for the provider.
    #[must_use]
    pub fn discover(&mut self, now_ms: u64, content_id: ContentId) -> Option<RouteAssignment> {
        self.purge(now_ms);
        let provider = self
            .peers
            .values()
            .find(|peer| peer.advertisement.content_ids.contains(&content_id))?;
        let relay = self.relays.values().next()?;
        self.next_route = self.next_route.wrapping_add(1);
        let assignment = RouteAssignment {
            provider_id: provider.advertisement.peer_id,
            provider_public_key: provider.advertisement.public_key,
            relay_id: relay.advertisement.relay_id.clone(),
            relay_address: relay.advertisement.address.clone(),
            route: format!("m2-{:016x}", self.next_route),
        };
        self.assignments
            .entry(assignment.provider_id)
            .or_default()
            .push_back(QueuedAssignment {
                assignment: assignment.clone(),
                expires_at: now_ms.saturating_add(self.lease_ms),
            });
        Some(assignment)
    }

    /// Renews a provider and returns its next still-valid route assignment.
    #[must_use]
    pub fn poll_peer(&mut self, now_ms: u64, peer_id: PeerId) -> Option<RouteAssignment> {
        if !self.heartbeat_peer(now_ms, peer_id) {
            return None;
        }
        let active_relays = &self.relays;
        let queue = self.assignments.get_mut(&peer_id)?;
        while let Some(queued) = queue.pop_front() {
            if queued.expires_at > now_ms && active_relays.contains_key(&queued.assignment.relay_id)
            {
                return Some(queued.assignment);
            }
        }
        None
    }

    /// Removes a failed relay and any route assignments that reference it.
    pub fn report_relay_failure(&mut self, relay_id: &str) {
        self.relays.remove(relay_id);
        for queue in self.assignments.values_mut() {
            queue.retain(|queued| queued.assignment.relay_id != relay_id);
        }
    }

    /// Purges expired state and returns live registry counts.
    #[must_use]
    pub fn snapshot(&mut self, now_ms: u64) -> OverlaySnapshot {
        self.purge(now_ms);
        OverlaySnapshot {
            peers: self.peers.len(),
            relays: self.relays.len(),
            content_advertisements: self
                .peers
                .values()
                .map(|peer| peer.advertisement.content_ids.len())
                .sum(),
        }
    }

    fn purge(&mut self, now_ms: u64) {
        self.peers.retain(|_, peer| peer.expires_at > now_ms);
        self.relays.retain(|_, relay| relay.expires_at > now_ms);
        self.assignments.retain(|peer_id, queue| {
            if !self.peers.contains_key(peer_id) {
                return false;
            }
            queue.retain(|queued| {
                queued.expires_at > now_ms && self.relays.contains_key(&queued.assignment.relay_id)
            });
            !queue.is_empty()
        });
    }
}

/// Validation failure in the temporary overlay registry.
#[derive(Debug, Error)]
pub enum RegistryError {
    /// The advertised peer ID was not derived from its public key.
    #[error("peer ID does not match the advertised public key")]
    PeerIdentityMismatch,
    /// Peers must advertise at least one content ID.
    #[error("peer advertisement contains no content IDs")]
    NoContent,
    /// M2 peers must explicitly advertise the relayed transfer capability.
    #[error("peer does not advertise the experimental relayed-transfer capability")]
    UnsupportedPeer,
    /// Relay identifiers use the same restricted alphabet as relay routes.
    #[error("relay ID must be 1-64 ASCII letters, digits, '-' or '_'")]
    InvalidRelayId,
    /// Relay endpoints must be concrete socket addresses in M2.
    #[error("relay address must be a valid IP socket address")]
    InvalidRelayAddress,
}

fn validate_relay_id(relay_id: &str) -> Result<(), RegistryError> {
    if relay_id.is_empty()
        || relay_id.len() > 64
        || !relay_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(RegistryError::InvalidRelayId);
    }
    Ok(())
}

/// Stateless client for the temporary bootstrap request/response service.
#[derive(Clone, Copy)]
pub struct BootstrapClient {
    address: SocketAddr,
}

impl BootstrapClient {
    /// Creates a client for `address`.
    #[must_use]
    pub const fn new(address: SocketAddr) -> Self {
        Self { address }
    }

    /// Registers a peer and returns its server-controlled lease duration.
    ///
    /// # Errors
    ///
    /// Returns [`OverlayError`] for transport, protocol, or registry failures.
    pub fn register_peer(&self, peer: PeerAdvertisement) -> Result<u64, OverlayError> {
        expect_registered(self.request(OverlayRequest::RegisterPeer(peer))?)
    }

    /// Registers a relay and returns its server-controlled lease duration.
    ///
    /// # Errors
    ///
    /// Returns [`OverlayError`] for transport, protocol, or registry failures.
    pub fn register_relay(&self, relay: RelayAdvertisement) -> Result<u64, OverlayError> {
        expect_registered(self.request(OverlayRequest::RegisterRelay(relay))?)
    }

    /// Renews a peer lease without polling for an assignment.
    ///
    /// # Errors
    ///
    /// Returns [`OverlayError`] if the peer is unknown or communication fails.
    pub fn heartbeat_peer(&self, peer_id: PeerId) -> Result<u64, OverlayError> {
        expect_registered(self.request(OverlayRequest::HeartbeatPeer { peer_id })?)
    }

    /// Renews a relay lease.
    ///
    /// # Errors
    ///
    /// Returns [`OverlayError`] if the relay is unknown or communication fails.
    pub fn heartbeat_relay(&self, relay_id: &str) -> Result<u64, OverlayError> {
        expect_registered(self.request(OverlayRequest::HeartbeatRelay {
            relay_id: relay_id.to_owned(),
        })?)
    }

    /// Renews a peer lease and polls its next route assignment.
    ///
    /// # Errors
    ///
    /// Returns [`OverlayError`] if the peer is unknown or communication fails.
    pub fn poll_peer(&self, peer_id: PeerId) -> Result<Option<RouteAssignment>, OverlayError> {
        match self.request(OverlayRequest::PollPeer { peer_id })? {
            OverlayResponse::Assignment(assignment) => Ok(assignment),
            OverlayResponse::Error(message) => Err(OverlayError::Remote(message)),
            _ => Err(OverlayError::UnexpectedResponse),
        }
    }

    /// Discovers a provider and automatically coordinated relay route.
    ///
    /// # Errors
    ///
    /// Returns [`OverlayError`] if communication or decoding fails.
    pub fn discover(&self, content_id: ContentId) -> Result<Option<RouteAssignment>, OverlayError> {
        match self.request(OverlayRequest::Discover { content_id })? {
            OverlayResponse::Route(route) => Ok(route),
            OverlayResponse::Error(message) => Err(OverlayError::Remote(message)),
            _ => Err(OverlayError::UnexpectedResponse),
        }
    }

    /// Reports a relay that could not be used before transfer.
    ///
    /// # Errors
    ///
    /// Returns [`OverlayError`] if communication fails.
    pub fn report_relay_failure(&self, relay_id: &str) -> Result<(), OverlayError> {
        match self.request(OverlayRequest::ReportRelayFailure {
            relay_id: relay_id.to_owned(),
        })? {
            OverlayResponse::Acknowledged => Ok(()),
            OverlayResponse::Error(message) => Err(OverlayError::Remote(message)),
            _ => Err(OverlayError::UnexpectedResponse),
        }
    }

    fn request(&self, request: OverlayRequest) -> Result<OverlayResponse, OverlayError> {
        let mut stream = TcpStream::connect_timeout(&self.address, CONTROL_TIMEOUT)?;
        stream.set_read_timeout(Some(CONTROL_TIMEOUT))?;
        stream.set_write_timeout(Some(CONTROL_TIMEOUT))?;
        let encoded = triptorrent_protocol::encode_overlay_request(request)?;
        write_frame(&mut stream, &encoded)?;
        let response = read_frame(&mut stream)?.ok_or(OverlayError::ConnectionClosed)?;
        Ok(triptorrent_protocol::decode_overlay_response(&response)?)
    }
}

fn expect_registered(response: OverlayResponse) -> Result<u64, OverlayError> {
    match response {
        OverlayResponse::Registered { lease_ms } => Ok(lease_ms),
        OverlayResponse::Error(message) => Err(OverlayError::Remote(message)),
        _ => Err(OverlayError::UnexpectedResponse),
    }
}

/// Runs the temporary M2 bootstrap service until its listener fails.
///
/// # Errors
///
/// Returns an I/O error if accepting a connection fails.
pub fn serve(listener: &TcpListener, lease_ms: u64) -> io::Result<()> {
    let registry = Arc::new(Mutex::new(Registry::new(lease_ms)));
    let epoch = Instant::now();
    for incoming in listener.incoming() {
        let stream = incoming?;
        let registry = Arc::clone(&registry);
        thread::spawn(move || {
            let _ = handle_connection(stream, &registry, elapsed_ms(epoch));
        });
    }
    Ok(())
}

fn handle_connection(
    mut stream: TcpStream,
    registry: &Mutex<Registry>,
    now_ms: u64,
) -> Result<(), OverlayError> {
    stream.set_read_timeout(Some(CONTROL_TIMEOUT))?;
    stream.set_write_timeout(Some(CONTROL_TIMEOUT))?;
    let frame = read_frame(&mut stream)?.ok_or(OverlayError::ConnectionClosed)?;
    let request = triptorrent_protocol::decode_overlay_request(&frame)?;
    let response = {
        let mut registry = registry.lock().map_err(|_| OverlayError::StatePoisoned)?;
        handle_request(&mut registry, now_ms, request)
    };
    let encoded = triptorrent_protocol::encode_overlay_response(response)?;
    write_frame(&mut stream, &encoded)?;
    Ok(())
}

fn handle_request(
    registry: &mut Registry,
    now_ms: u64,
    request: OverlayRequest,
) -> OverlayResponse {
    match request {
        OverlayRequest::RegisterPeer(peer) => match registry.register_peer(now_ms, peer) {
            Ok(()) => OverlayResponse::Registered {
                lease_ms: registry.lease_ms(),
            },
            Err(error) => OverlayResponse::Error(error.to_string()),
        },
        OverlayRequest::HeartbeatPeer { peer_id } => {
            if registry.heartbeat_peer(now_ms, peer_id) {
                OverlayResponse::Registered {
                    lease_ms: registry.lease_ms(),
                }
            } else {
                OverlayResponse::Error("peer lease is not registered".into())
            }
        }
        OverlayRequest::PollPeer { peer_id } => {
            if registry.heartbeat_peer(now_ms, peer_id) {
                OverlayResponse::Assignment(registry.poll_peer(now_ms, peer_id))
            } else {
                OverlayResponse::Error("peer lease is not registered".into())
            }
        }
        OverlayRequest::RegisterRelay(relay) => match registry.register_relay(now_ms, relay) {
            Ok(()) => OverlayResponse::Registered {
                lease_ms: registry.lease_ms(),
            },
            Err(error) => OverlayResponse::Error(error.to_string()),
        },
        OverlayRequest::HeartbeatRelay { relay_id } => {
            if registry.heartbeat_relay(now_ms, &relay_id) {
                OverlayResponse::Registered {
                    lease_ms: registry.lease_ms(),
                }
            } else {
                OverlayResponse::Error("relay lease is not registered".into())
            }
        }
        OverlayRequest::Discover { content_id } => {
            OverlayResponse::Route(registry.discover(now_ms, content_id))
        }
        OverlayRequest::ReportRelayFailure { relay_id } => {
            registry.report_relay_failure(&relay_id);
            OverlayResponse::Acknowledged
        }
    }
}

fn elapsed_ms(epoch: Instant) -> u64 {
    u64::try_from(epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn write_frame(writer: &mut impl Write, frame: &[u8]) -> io::Result<()> {
    if frame.len() > MAX_CONTROL_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "control frame too large",
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
    if length > MAX_CONTROL_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "control frame too large",
        ));
    }
    let mut frame = vec![0; length];
    reader.read_exact(&mut frame)?;
    Ok(Some(frame))
}

/// Bootstrap transport, protocol, or state failure.
#[derive(Debug, Error)]
pub enum OverlayError {
    /// TCP or control framing failure.
    #[error("bootstrap transport error: {0}")]
    Io(#[from] io::Error),
    /// Experimental overlay message failure.
    #[error("bootstrap protocol error: {0}")]
    Protocol(#[from] triptorrent_protocol::ProtocolError),
    /// The bootstrap closed before returning a response.
    #[error("bootstrap closed the connection")]
    ConnectionClosed,
    /// The bootstrap returned a response for a different operation.
    #[error("bootstrap returned an unexpected response")]
    UnexpectedResponse,
    /// The bootstrap rejected the request.
    #[error("bootstrap rejected request: {0}")]
    Remote(String),
    /// In-memory state was poisoned by a panicking handler.
    #[error("bootstrap state lock is poisoned")]
    StatePoisoned,
}

/// Deterministic, no-I/O harness for multi-node and churn simulations.
pub struct Simulation {
    now_ms: u64,
    registry: Registry,
}

impl Simulation {
    /// Creates an empty simulation.
    #[must_use]
    pub fn new(lease_ms: u64) -> Self {
        Self {
            now_ms: 0,
            registry: Registry::new(lease_ms),
        }
    }

    /// Advances simulated monotonic time.
    pub fn advance(&mut self, milliseconds: u64) {
        self.now_ms = self.now_ms.saturating_add(milliseconds);
    }

    /// Registers a logical peer at the current simulated time.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError`] for an invalid advertisement.
    pub fn register_peer(&mut self, advertisement: PeerAdvertisement) -> Result<(), RegistryError> {
        self.registry.register_peer(self.now_ms, advertisement)
    }

    /// Registers a logical relay at the current simulated time.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError`] for an invalid advertisement.
    pub fn register_relay(
        &mut self,
        advertisement: RelayAdvertisement,
    ) -> Result<(), RegistryError> {
        self.registry.register_relay(self.now_ms, advertisement)
    }

    /// Discovers a route at the current simulated time.
    #[must_use]
    pub fn discover(&mut self, content_id: ContentId) -> Option<RouteAssignment> {
        self.registry.discover(self.now_ms, content_id)
    }

    /// Removes a failed relay immediately.
    pub fn fail_relay(&mut self, relay_id: &str) {
        self.registry.report_relay_failure(relay_id);
    }

    /// Returns current live-node and advertisement counts.
    #[must_use]
    pub fn snapshot(&mut self) -> OverlaySnapshot {
        self.registry.snapshot(self.now_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(seed: u8, content_ids: Vec<ContentId>) -> PeerAdvertisement {
        let public_key = [seed; 32];
        PeerAdvertisement {
            peer_id: PeerId::from_public_key(&public_key),
            public_key,
            capabilities: vec![PeerCapability::RelayedTransferV0],
            content_ids,
        }
    }

    fn relay(relay_id: &str, port: u16) -> RelayAdvertisement {
        RelayAdvertisement {
            relay_id: relay_id.into(),
            address: format!("127.0.0.1:{port}"),
        }
    }

    #[test]
    fn peers_relays_and_content_are_registered_and_discovered() {
        let content_id = ContentId::digest(b"discoverable");
        let provider = peer(1, vec![content_id]);
        let mut registry = Registry::new(1_000);
        registry.register_peer(0, provider.clone()).unwrap();
        registry.register_relay(0, relay("relay-a", 7000)).unwrap();

        assert_eq!(
            registry.snapshot(0),
            OverlaySnapshot {
                peers: 1,
                relays: 1,
                content_advertisements: 1,
            }
        );
        let receiver_route = registry.discover(0, content_id).unwrap();
        assert_eq!(receiver_route.provider_id, provider.peer_id);
        assert_eq!(receiver_route.relay_id, "relay-a");
        assert_eq!(
            registry.poll_peer(0, provider.peer_id).unwrap(),
            receiver_route
        );
    }

    #[test]
    fn stale_peer_and_relay_registrations_expire() {
        let content_id = ContentId::digest(b"expires");
        let mut registry = Registry::new(100);
        registry
            .register_peer(0, peer(2, vec![content_id]))
            .unwrap();
        registry.register_relay(0, relay("relay-a", 7000)).unwrap();
        assert!(registry.discover(99, content_id).is_some());
        assert!(registry.discover(100, content_id).is_none());
        assert_eq!(registry.snapshot(100).peers, 0);
        assert_eq!(registry.snapshot(100).relays, 0);
    }

    #[test]
    fn stale_route_assignments_expire_independently() {
        let content_id = ContentId::digest(b"route expires");
        let provider = peer(9, vec![content_id]);
        let mut registry = Registry::new(100);
        registry.register_peer(0, provider.clone()).unwrap();
        registry.register_relay(0, relay("relay-a", 7000)).unwrap();
        assert!(registry.discover(0, content_id).is_some());
        assert!(registry.heartbeat_peer(50, provider.peer_id));
        assert!(registry.heartbeat_relay(50, "relay-a"));

        assert!(registry.poll_peer(100, provider.peer_id).is_none());
        assert_eq!(registry.snapshot(100).peers, 1);
        assert_eq!(registry.snapshot(100).relays, 1);
    }

    #[test]
    fn peer_reconnect_restores_content_discovery() {
        let content_id = ContentId::digest(b"reconnect");
        let provider = peer(3, vec![content_id]);
        let mut registry = Registry::new(100);
        registry.register_peer(0, provider.clone()).unwrap();
        registry.register_relay(0, relay("relay-a", 7000)).unwrap();
        assert!(registry.discover(100, content_id).is_none());

        registry.register_peer(101, provider).unwrap();
        registry
            .register_relay(101, relay("relay-a", 7000))
            .unwrap();
        assert!(registry.discover(101, content_id).is_some());
    }

    #[test]
    fn failed_relay_is_replaced_by_an_alternative() {
        let content_id = ContentId::digest(b"failover");
        let mut registry = Registry::new(1_000);
        registry
            .register_peer(0, peer(4, vec![content_id]))
            .unwrap();
        registry.register_relay(0, relay("relay-a", 7000)).unwrap();
        registry.register_relay(0, relay("relay-b", 7001)).unwrap();
        assert_eq!(
            registry.discover(0, content_id).unwrap().relay_id,
            "relay-a"
        );
        registry.report_relay_failure("relay-a");
        assert_eq!(
            registry.discover(1, content_id).unwrap().relay_id,
            "relay-b"
        );
    }

    #[test]
    fn simulation_handles_ten_peers_three_relays_and_churn() {
        let mut simulation = Simulation::new(100);
        let contents: Vec<_> = (0_u8..10).map(|seed| ContentId::digest(&[seed])).collect();
        for (seed, content_id) in (1_u8..=10).zip(contents.iter().copied()) {
            simulation
                .register_peer(peer(seed, vec![content_id]))
                .unwrap();
        }
        for (index, relay_id) in ["relay-a", "relay-b", "relay-c"].iter().enumerate() {
            simulation
                .register_relay(relay(relay_id, 7_000 + u16::try_from(index).unwrap()))
                .unwrap();
        }
        assert_eq!(simulation.snapshot().peers, 10);
        assert_eq!(simulation.snapshot().relays, 3);
        assert!(simulation.discover(contents[7]).is_some());

        simulation.fail_relay("relay-a");
        assert_eq!(
            simulation.discover(contents[7]).unwrap().relay_id,
            "relay-b"
        );
        simulation.advance(100);
        assert_eq!(simulation.snapshot().peers, 0);
        assert_eq!(simulation.snapshot().relays, 0);

        simulation
            .register_peer(peer(8, vec![contents[7]]))
            .unwrap();
        simulation.register_relay(relay("relay-c", 7002)).unwrap();
        assert!(simulation.discover(contents[7]).is_some());
    }
}
