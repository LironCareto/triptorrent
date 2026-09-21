#![doc = "Transport framing and end-to-end encrypted M1 sessions."]

use snow::{Builder, HandshakeState, TransportState, params::NoiseParams};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;
use thiserror::Error;
use triptorrent_core::{NETWORK_ID, PROTOCOL_VERSION};
use triptorrent_protocol::{Message, ProtocolError};

const REGISTRATION_MAGIC: &[u8; 4] = b"TTR1";
const MAX_FRAME_LENGTH: usize = 1024 * 1024;
const MAX_NOISE_MESSAGE: usize = 65_535;
const NOISE_PSK_PATTERN: &str = "Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s";
const NOISE_PROVIDER_PATTERN: &str = "Noise_KN_25519_ChaChaPoly_BLAKE2s";

/// Ephemeral key material for an M2 content-provider process.
pub struct PeerKeypair {
    /// Private Noise key. It must remain local to the provider process.
    pub private: [u8; 32],
    /// Public Noise key advertised through the temporary bootstrap.
    pub public: [u8; 32],
}

/// Generates an ephemeral X25519 keypair through the reviewed Noise implementation.
///
/// # Errors
///
/// Returns [`NetError`] if Noise setup or key generation fails.
pub fn generate_peer_keypair() -> Result<PeerKeypair, NetError> {
    let params = parse_noise_pattern(NOISE_PROVIDER_PATTERN)?;
    let keypair = Builder::new(params).generate_keypair()?;
    Ok(PeerKeypair {
        private: keypair
            .private
            .try_into()
            .map_err(|_| NetError::InvalidKeyLength)?,
        public: keypair
            .public
            .try_into()
            .map_err(|_| NetError::InvalidKeyLength)?,
    })
}

/// The endpoint role used only to pair connections at a relay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RelayRole {
    /// The peer exposing a file.
    Sender,
    /// The peer requesting a file.
    Receiver,
}

impl RelayRole {
    const fn wire_byte(self) -> u8 {
        match self {
            Self::Sender => 0,
            Self::Receiver => 1,
        }
    }
}

/// A framed byte transport. Encryption and file messages remain above this layer.
pub trait FrameTransport {
    /// Sends one opaque frame.
    ///
    /// # Errors
    ///
    /// Returns [`NetError`] when the transport fails.
    fn send_frame(&mut self, frame: &[u8]) -> Result<(), NetError>;

    /// Receives one opaque frame.
    ///
    /// # Errors
    ///
    /// Returns [`NetError`] when the transport closes or sends an invalid frame.
    fn receive_frame(&mut self) -> Result<Vec<u8>, NetError>;
}

/// A TCP connection that reaches a peer exclusively through a relay route.
pub struct TcpRelayTransport {
    stream: TcpStream,
}

impl TcpRelayTransport {
    /// Connects to a relay and registers one route endpoint.
    ///
    /// # Errors
    ///
    /// Returns [`NetError`] for invalid routes or connection failures.
    pub fn connect(address: SocketAddr, route: &str, role: RelayRole) -> Result<Self, NetError> {
        let stream = TcpStream::connect(address)?;
        Self::register(stream, route, role, None)
    }

    /// Connects and applies an I/O timeout, used by M2 failover attempts.
    ///
    /// # Errors
    ///
    /// Returns [`NetError`] for invalid routes, timeouts, or connection failures.
    pub fn connect_with_timeout(
        address: SocketAddr,
        route: &str,
        role: RelayRole,
        timeout: Duration,
    ) -> Result<Self, NetError> {
        let stream = TcpStream::connect_timeout(&address, timeout)?;
        Self::register(stream, route, role, Some(timeout))
    }

    fn register(
        mut stream: TcpStream,
        route: &str,
        role: RelayRole,
        timeout: Option<Duration>,
    ) -> Result<Self, NetError> {
        validate_route(route)?;
        stream.set_nodelay(true)?;
        stream.set_read_timeout(timeout)?;
        stream.set_write_timeout(timeout)?;
        let mut registration = Vec::with_capacity(10 + NETWORK_ID.len() + route.len());
        registration.extend_from_slice(REGISTRATION_MAGIC);
        registration.extend_from_slice(&PROTOCOL_VERSION.to_be_bytes());
        registration.push(u8::try_from(NETWORK_ID.len()).map_err(|_| NetError::InvalidNetwork)?);
        registration.extend_from_slice(NETWORK_ID.as_bytes());
        registration.push(role.wire_byte());
        registration.extend_from_slice(
            &u16::try_from(route.len())
                .map_err(|_| NetError::InvalidRoute)?
                .to_be_bytes(),
        );
        registration.extend_from_slice(route.as_bytes());
        write_frame(&mut stream, &registration)?;
        Ok(Self { stream })
    }
}

impl FrameTransport for TcpRelayTransport {
    fn send_frame(&mut self, frame: &[u8]) -> Result<(), NetError> {
        write_frame(&mut self.stream, frame).map_err(NetError::Io)
    }

    fn receive_frame(&mut self) -> Result<Vec<u8>, NetError> {
        read_frame(&mut self.stream)?.ok_or_else(|| {
            NetError::Io(io::Error::new(io::ErrorKind::UnexpectedEof, "relay closed"))
        })
    }
}

/// A `TripTorrent` application session encrypted independently of its transport.
pub struct EncryptedSession<T> {
    transport: T,
    noise: TransportState,
}

impl<T: FrameTransport> EncryptedSession<T> {
    /// Performs the initiator side of the reviewed Noise handshake.
    ///
    /// # Errors
    ///
    /// Returns [`NetError`] when setup, key agreement, or transport I/O fails.
    pub fn initiator(transport: T, psk: &[u8; 32]) -> Result<Self, NetError> {
        let handshake = noise_builder(psk)?.build_initiator()?;
        Self::handshake_initiator(transport, handshake)
    }

    /// Performs the responder side of the reviewed Noise handshake.
    ///
    /// # Errors
    ///
    /// Returns [`NetError`] when setup, key agreement, or transport I/O fails.
    pub fn responder(transport: T, psk: &[u8; 32]) -> Result<Self, NetError> {
        let handshake = noise_builder(psk)?.build_responder()?;
        Self::handshake_responder(transport, handshake)
    }

    /// Establishes the M2 provider side using an ephemeral static private key.
    ///
    /// # Errors
    ///
    /// Returns [`NetError`] when setup, key agreement, or transport I/O fails.
    pub fn provider(transport: T, private_key: &[u8; 32]) -> Result<Self, NetError> {
        let params = parse_noise_pattern(NOISE_PROVIDER_PATTERN)?;
        let handshake = Builder::new(params)
            .local_private_key(private_key)?
            .build_initiator()?;
        Self::handshake_initiator(transport, handshake)
    }

    /// Establishes the M2 receiver side using the discovered provider public key.
    ///
    /// # Errors
    ///
    /// Returns [`NetError`] when setup, peer authentication, or transport I/O fails.
    pub fn receiver(transport: T, provider_public_key: &[u8; 32]) -> Result<Self, NetError> {
        let params = parse_noise_pattern(NOISE_PROVIDER_PATTERN)?;
        let handshake = Builder::new(params)
            .remote_public_key(provider_public_key)?
            .build_responder()?;
        Self::handshake_responder(transport, handshake)
    }

    fn handshake_initiator(
        mut transport: T,
        mut handshake: HandshakeState,
    ) -> Result<Self, NetError> {
        let mut output = vec![0_u8; MAX_NOISE_MESSAGE];
        let length = handshake.write_message(&[], &mut output)?;
        transport.send_frame(&output[..length])?;
        let response = transport.receive_frame()?;
        handshake.read_message(&response, &mut output)?;
        Ok(Self {
            transport,
            noise: handshake.into_transport_mode()?,
        })
    }

    fn handshake_responder(
        mut transport: T,
        mut handshake: HandshakeState,
    ) -> Result<Self, NetError> {
        let request = transport.receive_frame()?;
        let mut output = vec![0_u8; MAX_NOISE_MESSAGE];
        handshake.read_message(&request, &mut output)?;
        let length = handshake.write_message(&[], &mut output)?;
        transport.send_frame(&output[..length])?;
        Ok(Self {
            transport,
            noise: handshake.into_transport_mode()?,
        })
    }

    /// Encrypts and sends one application message.
    ///
    /// # Errors
    ///
    /// Returns [`NetError`] for serialization, encryption, or transport failures.
    pub fn send(&mut self, message: Message) -> Result<(), NetError> {
        let plaintext = triptorrent_protocol::encode(message)?;
        let mut ciphertext = vec![0_u8; plaintext.len() + 16];
        let length = self.noise.write_message(&plaintext, &mut ciphertext)?;
        self.transport.send_frame(&ciphertext[..length])
    }

    /// Receives, authenticates, and decodes one application message.
    ///
    /// # Errors
    ///
    /// Returns [`NetError`] for invalid ciphertext, messages, or transport failures.
    pub fn receive(&mut self) -> Result<Message, NetError> {
        let ciphertext = self.transport.receive_frame()?;
        let mut plaintext = vec![0_u8; ciphertext.len()];
        let length = self.noise.read_message(&ciphertext, &mut plaintext)?;
        Ok(triptorrent_protocol::decode(&plaintext[..length])?)
    }
}

fn noise_builder(psk: &[u8; 32]) -> Result<Builder<'_>, NetError> {
    let params = parse_noise_pattern(NOISE_PSK_PATTERN)?;
    Ok(Builder::new(params).psk(0, psk)?)
}

fn parse_noise_pattern(pattern: &str) -> Result<NoiseParams, NetError> {
    pattern.parse().map_err(|_| NetError::InvalidNoisePattern)
}

fn validate_route(route: &str) -> Result<(), NetError> {
    if route.is_empty()
        || route.len() > 128
        || !route
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(NetError::InvalidRoute);
    }
    Ok(())
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

/// Network, session, or wire-protocol failure.
#[derive(Debug, Error)]
pub enum NetError {
    /// TCP or framing failure.
    #[error("transport error: {0}")]
    Io(#[from] io::Error),
    /// Noise handshake or authenticated-encryption failure.
    #[error("secure session error: {0}")]
    Noise(#[from] snow::Error),
    /// File-transfer wire message failure.
    #[error("protocol error: {0}")]
    Protocol(#[from] ProtocolError),
    /// Route names are deliberately restricted in M1.
    #[error("route must be 1-128 ASCII letters, digits, '-' or '_'")]
    InvalidRoute,
    /// The static Noise name embedded in the prototype was invalid.
    #[error("invalid built-in Noise protocol name")]
    InvalidNoisePattern,
    /// A Noise backend returned an unexpected key length.
    #[error("Noise backend returned an invalid key length")]
    InvalidKeyLength,
    /// The built-in network identifier cannot be represented by the registration format.
    #[error("invalid built-in network identifier")]
    InvalidNetwork,
}
