//! A single-use invitation for a pinned TLS 1.3 session. All framing is inside TLS.
use droid_camera_source::{DEFAULT_MODE, LEGACY_MODE, VideoMode};
use ring::{digest, rand};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{
    ClientConfig, ClientConnection, DigitallySignedStruct, ServerConfig, ServerConnection,
};
use std::io::{self, Read, Write};
use std::net::{SocketAddrV4, TcpStream};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

pub const MAX_JPEG: usize = 3 * 1024 * 1024;
const LEGACY_MAX_JPEG: usize = 256 * 1024;
pub const PAIR_TIMEOUT: Duration = Duration::from_secs(8);
pub const INVITE_LIFETIME: Duration = Duration::from_secs(180);
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(10);
const FRAME_TIMEOUT: Duration = Duration::from_secs(3);
const POLL: Duration = Duration::from_millis(125);
const MAGIC: [u8; 4] = *b"DW02";
const LEGACY_MAGIC: [u8; 4] = *b"DW01";
const ACCEPT: [u8; 4] = *b"DWOK";
const CREDIT: u8 = 1;

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureSettings {
    mode: VideoMode,
    quality: u8,
    front: bool,
}

impl Default for CaptureSettings {
    fn default() -> Self {
        Self {
            mode: DEFAULT_MODE,
            quality: 80,
            front: false,
        }
    }
}

impl CaptureSettings {
    pub const LEGACY: Self = Self {
        mode: LEGACY_MODE,
        quality: 75,
        front: false,
    };
    pub fn new(mode: VideoMode, quality: u8, front: bool) -> Result<Self> {
        if !(40..=95).contains(&quality) {
            return Err("JPEG quality must be between 40 and 95".into());
        }
        Ok(Self {
            mode,
            quality,
            front,
        })
    }
    pub fn mode(self) -> VideoMode {
        self.mode
    }
    pub fn quality(self) -> u8 {
        self.quality
    }
    pub fn front(self) -> bool {
        self.front
    }
    pub fn wire(self) -> [u8; 8] {
        let mut bytes = [0; 8];
        bytes[..6].copy_from_slice(&self.mode.wire());
        bytes[6] = self.quality;
        bytes[7] = u8::from(self.front);
        bytes
    }
    pub fn from_wire(bytes: [u8; 8]) -> Result<Self> {
        if bytes[7] > 1 {
            return Err("invalid camera selection".into());
        }
        Self::new(
            VideoMode::from_wire(bytes[..6].try_into().unwrap())?,
            bytes[6],
            bytes[7] == 1,
        )
    }
}

pub struct Invite {
    version: u8,
    pub address: SocketAddrV4,
    pin: [u8; 32],
    secret: Zeroizing<[u8; 32]>,
}

impl Invite {
    pub fn parse(code: &str) -> Result<Self> {
        let parts: Vec<_> = code.trim().splitn(6, ':').collect();
        let ["droidcam", version @ ("1" | "2"), ip, port, pin, secret] = parts.as_slice() else {
            return Err("invalid Droid Webcam invitation".into());
        };
        let address: SocketAddrV4 = format!("{ip}:{port}").parse()?;
        if address.port() == 0
            || address.ip().is_unspecified()
            || address.ip().is_multicast()
            || address.ip().is_broadcast()
        {
            return Err("invitation needs a unicast IPv4 address and port".into());
        }
        let mut invite = Self {
            version: version.parse()?,
            address,
            pin: [0; 32],
            secret: Zeroizing::new([0; 32]),
        };
        hex::decode_to_slice(pin, &mut invite.pin)?;
        hex::decode_to_slice(secret, &mut *invite.secret)?;
        Ok(invite)
    }

    pub fn code(&self) -> Zeroizing<String> {
        Zeroizing::new(format!(
            "droidcam:{}:{}:{}:{}",
            self.version,
            self.address,
            hex::encode(self.pin),
            *Zeroizing::new(hex::encode(&self.secret[..]))
        ))
    }
}

pub struct Pairing {
    config: Arc<ServerConfig>,
    invite: Invite,
    expires: Instant,
    used: bool,
}

impl Pairing {
    pub fn new(address: SocketAddrV4) -> Result<Self> {
        let identity = rcgen::generate_simple_self_signed(vec!["droid-webcam.local".into()])?;
        let cert = identity.cert.der().clone();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut config = ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])?
            .with_no_client_auth()
            .with_single_cert(
                vec![cert.clone()],
                PrivatePkcs8KeyDer::from(identity.signing_key.serialize_der()).into(),
            )?;
        config.send_tls13_tickets = 0;
        let mut secret = Zeroizing::new([0; 32]);
        rand::SecureRandom::fill(&rand::SystemRandom::new(), &mut *secret)
            .map_err(|_| "operating system random generator failed")?;
        let pin = fingerprint(&cert);
        Ok(Self {
            config: Arc::new(config),
            invite: Invite {
                version: 2,
                address,
                pin,
                secret,
            },
            expires: Instant::now() + INVITE_LIFETIME,
            used: false,
        })
    }

    pub fn invite(&self) -> &Invite {
        &self.invite
    }
    pub fn expired(&self) -> bool {
        Instant::now() >= self.expires
    }
    pub fn used(&self) -> bool {
        self.used
    }

    pub fn accept(&mut self, socket: TcpStream, stop: Arc<AtomicBool>) -> Result<Receiver> {
        if self.used || self.expired() {
            return Err("invitation has been used or expired".into());
        }
        let mut tls = rustls::StreamOwned::new(
            ServerConnection::new(self.config.clone())?,
            TimedSocket::new(
                socket,
                stop,
                self.expires.min(Instant::now() + PAIR_TIMEOUT),
            )?,
        );
        let mut proof = Zeroizing::new([0; 36]);
        tls.read_exact(&mut *proof)?;
        if (proof[..4] != MAGIC && proof[..4] != LEGACY_MAGIC)
            || !bool::from(proof[4..].ct_eq(&*self.invite.secret))
        {
            return Err("pairing proof rejected".into());
        }
        if self.expired() {
            return Err("invitation expired".into());
        }
        // Consume before sending the acknowledgement, including if the peer closes early.
        self.used = true;
        self.invite.secret.fill(0);
        let legacy = proof[..4] == LEGACY_MAGIC;
        let settings = if legacy {
            CaptureSettings::LEGACY
        } else {
            let mut bytes = [0; 8];
            tls.read_exact(&mut bytes)?;
            CaptureSettings::from_wire(bytes)?
        };
        tls.write_all(&ACCEPT)?;
        if !legacy {
            tls.write_all(&settings.wire())?;
        }
        tls.flush()?;
        Ok(Receiver {
            tls,
            settings,
            max_jpeg: if legacy { LEGACY_MAX_JPEG } else { MAX_JPEG },
            first: true,
        })
    }
}

pub struct Receiver {
    tls: rustls::StreamOwned<ServerConnection, TimedSocket>,
    settings: CaptureSettings,
    max_jpeg: usize,
    first: bool,
}

impl Receiver {
    pub fn settings(&self) -> CaptureSettings {
        self.settings
    }

    /// Returns false on an authenticated, explicit stop. EOF is a disconnect.
    pub fn next_frame(&mut self, frame: &mut Vec<u8>) -> Result<bool> {
        let timeout = if self.first {
            FIRST_FRAME_TIMEOUT
        } else {
            FRAME_TIMEOUT
        };
        self.tls.sock.deadline = Instant::now() + timeout;
        let received = read_frame(&mut self.tls, frame, self.max_jpeg)?;
        self.first = false;
        Ok(received)
    }

    pub fn acknowledge_frame(&mut self) -> Result<()> {
        self.tls.sock.deadline = Instant::now() + FRAME_TIMEOUT;
        self.tls.write_all(&[CREDIT])?;
        self.tls.flush()?;
        Ok(())
    }
}

fn read_frame(reader: &mut impl Read, frame: &mut Vec<u8>, max_jpeg: usize) -> Result<bool> {
    let mut header = [0; 4];
    reader.read_exact(&mut header)?;
    let size = u32::from_be_bytes(header) as usize;
    if size > max_jpeg {
        return Err("frame exceeds the negotiated size limit".into());
    }
    frame.resize(size, 0);
    reader.read_exact(frame)?;
    Ok(size != 0)
}

pub struct Sender {
    tls: rustls::StreamOwned<ClientConnection, TimedSocket>,
    max_jpeg: usize,
}

impl Sender {
    pub fn connect(invite: &Invite) -> Result<Self> {
        let settings = if invite.version == 1 {
            CaptureSettings::LEGACY
        } else {
            CaptureSettings::default()
        };
        Self::connect_with_settings(invite, settings)
    }

    pub fn connect_with_settings(invite: &Invite, settings: CaptureSettings) -> Result<Self> {
        if invite.version == 1 && settings != CaptureSettings::LEGACY {
            return Err("a legacy host only supports the original capture mode".into());
        }
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = ClientConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(&[&rustls::version::TLS13])?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PinVerifier(invite.pin, provider)))
            .with_no_client_auth();
        let deadline = Instant::now() + PAIR_TIMEOUT;
        let socket = TcpStream::connect_timeout(&invite.address.into(), PAIR_TIMEOUT)?;
        let mut tls = rustls::StreamOwned::new(
            ClientConnection::new(
                Arc::new(config),
                ServerName::try_from("droid-webcam.local")?,
            )?,
            TimedSocket::new(socket, Arc::new(AtomicBool::new(false)), deadline)?,
        );
        let mut proof = Zeroizing::new([0; 36]);
        proof[..4].copy_from_slice(if invite.version == 1 {
            &LEGACY_MAGIC
        } else {
            &MAGIC
        });
        proof[4..].copy_from_slice(&*invite.secret);
        tls.write_all(&*proof)?;
        if invite.version == 2 {
            tls.write_all(&settings.wire())?;
        }
        tls.flush()?;
        let mut acknowledgement = [0; 4];
        tls.read_exact(&mut acknowledgement)?;
        if acknowledgement != ACCEPT {
            return Err("unexpected pairing acknowledgement".into());
        }
        if invite.version == 2 {
            let mut accepted = [0; 8];
            tls.read_exact(&mut accepted)?;
            if CaptureSettings::from_wire(accepted)? != settings {
                return Err("host changed the capture settings".into());
            }
        }
        Ok(Self {
            tls,
            max_jpeg: if invite.version == 1 {
                LEGACY_MAX_JPEG
            } else {
                MAX_JPEG
            },
        })
    }

    pub fn send(&mut self, jpeg: &[u8]) -> Result<()> {
        if jpeg.is_empty() || jpeg.len() > self.max_jpeg {
            return Err("invalid JPEG record length".into());
        }
        self.write(jpeg)?;
        let mut credit = [0; 1];
        self.tls.read_exact(&mut credit)?;
        if credit != [CREDIT] {
            return Err("unexpected frame acknowledgement".into());
        }
        Ok(())
    }

    pub fn stop(mut self) -> Result<()> {
        self.write(&[])
    }

    fn write(&mut self, payload: &[u8]) -> Result<()> {
        self.tls.sock.deadline = Instant::now() + FRAME_TIMEOUT;
        self.tls.write_all(&(payload.len() as u32).to_be_bytes())?;
        self.tls.write_all(payload)?;
        self.tls.flush()?;
        Ok(())
    }
}

#[derive(Debug)]
struct PinVerifier([u8; 32], Arc<rustls::crypto::CryptoProvider>);

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        cert: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        if bool::from(fingerprint(cert).ct_eq(&self.0)) {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General("TLS 1.2 is disabled".into()))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        sig: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            sig,
            &self.1.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.1.signature_verification_algorithms.supported_schemes()
    }
}

fn fingerprint(cert: &CertificateDer<'_>) -> [u8; 32] {
    digest::digest(&digest::SHA256, cert.as_ref())
        .as_ref()
        .try_into()
        .unwrap()
}

struct TimedSocket {
    socket: TcpStream,
    deadline: Instant,
    stop: Arc<AtomicBool>,
}

impl TimedSocket {
    fn new(socket: TcpStream, stop: Arc<AtomicBool>, deadline: Instant) -> io::Result<Self> {
        socket.set_nodelay(true)?;
        Ok(Self {
            socket,
            deadline,
            stop,
        })
    }

    fn timeout(&self) -> io::Result<Duration> {
        if self.stop.load(Ordering::Relaxed) {
            return Err(io::Error::other("host stopped"));
        }
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "peer timed out"));
        }
        Ok(left.min(POLL))
    }
}

impl Read for TimedSocket {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            self.socket.set_read_timeout(Some(self.timeout()?))?;
            match self.socket.read(buf) {
                Err(e)
                    if e.kind() == io::ErrorKind::TimedOut
                        || e.kind() == io::ErrorKind::WouldBlock => {}
                result => return result,
            }
        }
    }
}

impl Write for TimedSocket {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        loop {
            self.socket.set_write_timeout(Some(self.timeout()?))?;
            match self.socket.write(buf) {
                Err(e)
                    if e.kind() == io::ErrorKind::TimedOut
                        || e.kind() == io::ErrorKind::WouldBlock => {}
                result => return result,
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.socket.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn pinned_pairing_rejects_wrong_pin_and_secret_then_consumes_invite() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let std::net::SocketAddr::V4(address) = listener.local_addr().unwrap() else {
            unreachable!()
        };
        let mut pairing = Pairing::new(address).unwrap();
        let invite = Invite::parse(&pairing.invite().code()).unwrap();
        let mut bad_pin = Invite::parse(&invite.code()).unwrap();
        bad_pin.pin[0] ^= 1;
        let mut bad_secret = Invite::parse(&invite.code()).unwrap();
        bad_secret.secret[0] ^= 1;
        let task = thread::spawn(move || {
            let stop = Arc::new(AtomicBool::new(false));
            for _ in 0..2 {
                assert!(
                    pairing
                        .accept(listener.accept().unwrap().0, stop.clone())
                        .is_err()
                );
                assert!(!pairing.used());
            }
            let mut receiver = pairing
                .accept(listener.accept().unwrap().0, stop.clone())
                .unwrap();
            let mut frame = Vec::new();
            assert!(receiver.next_frame(&mut frame).unwrap());
            assert_eq!(frame, [1, 2, 3]);
            receiver.acknowledge_frame().unwrap();
            assert!(!receiver.next_frame(&mut frame).unwrap());
            assert!(pairing.used());
            assert!(pairing.accept(listener.accept().unwrap().0, stop).is_err());
        });
        assert!(Sender::connect(&bad_pin).is_err());
        assert!(Sender::connect(&bad_secret).is_err());
        let mut sender = Sender::connect(&invite).unwrap();
        sender.send(&[1, 2, 3]).unwrap();
        sender.stop().unwrap();
        assert!(Sender::connect(&invite).is_err());
        task.join().unwrap();
    }

    #[test]
    fn settings_reject_unsupported_dimensions_rates_quality_and_camera() {
        for mode in droid_camera_source::SUPPORTED_MODES {
            for quality in [40, 55, 80, 95] {
                for front in [false, true] {
                    let settings = CaptureSettings::new(mode, quality, front).unwrap();
                    assert_eq!(
                        CaptureSettings::from_wire(settings.wire()).unwrap(),
                        settings
                    );
                }
            }
        }
        let valid = CaptureSettings::default().wire();
        for (index, value) in [(0, 255), (4, 255), (5, 0), (6, 0), (6, 96), (7, 2)] {
            let mut invalid = valid;
            invalid[index] = value;
            assert!(CaptureSettings::from_wire(invalid).is_err());
        }
    }

    #[test]
    fn malformed_authenticated_settings_consume_the_invitation() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let std::net::SocketAddr::V4(address) = listener.local_addr().unwrap() else {
            unreachable!()
        };
        let mut pairing = Pairing::new(address).unwrap();
        let invite = Invite::parse(&pairing.invite().code()).unwrap();
        let task = thread::spawn(move || {
            assert!(
                pairing
                    .accept(
                        listener.accept().unwrap().0,
                        Arc::new(AtomicBool::new(false))
                    )
                    .is_err()
            );
            assert!(pairing.used());
        });
        let invalid = CaptureSettings {
            mode: DEFAULT_MODE,
            quality: 0,
            front: false,
        };
        assert!(Sender::connect_with_settings(&invite, invalid).is_err());
        task.join().unwrap();
    }

    #[test]
    fn oversized_and_truncated_records_are_rejected() {
        let mut frame = Vec::new();
        let header = (MAX_JPEG as u32 + 1).to_be_bytes();
        assert!(read_frame(&mut &header[..], &mut frame, MAX_JPEG).is_err());
        assert_eq!(frame.capacity(), 0);
        assert!(read_frame(&mut &[0, 0, 0, 2, 1][..], &mut frame, MAX_JPEG).is_err());
        assert!(read_frame(&mut &[0, 0][..], &mut frame, MAX_JPEG).is_err());
    }
}
