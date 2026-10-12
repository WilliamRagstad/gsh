use crate::{auth::ClientAuthProvider, config};
use anyhow::{anyhow, bail, Context};
use dialoguer::Confirm;
use futures_util::{
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use libgsh::{
    client::ClientStream,
    rsa::{
        pkcs1::EncodeRsaPublicKey, pkcs1v15::Signature, signature::SignatureEncoding, RsaPublicKey,
    },
    sha2::{Digest, Sha256},
    shared::protocol::{
        client_auth,
        client_hello::{self, MonitorInfo},
        server_auth_ack::AuthStatus,
        server_hello_ack::{AuthMethod, SignatureMethod},
        server_message::ServerEvent,
        status_update::StatusType,
        ClientAuth, ClientHello, ClientMessage, ServerHelloAck, ServerMessage, StatusUpdate,
    },
    shared::{auth::AuthProvider, PROTOCOL_VERSION},
    webrtc::{CandidateMessage, GshSession, SessionRole, VideoCodec},
};
use prost::Message as ProstMessage;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::{collections::VecDeque, path::Path, time::Duration};
use tokio::time::Instant;
use tokio::{io::AsyncWriteExt, net::TcpStream};
use tokio_rustls::rustls::{
    self,
    client::danger::{ServerCertVerified, ServerCertVerifier},
    crypto::{ring as provider, CryptoProvider},
    time_provider,
};
use tokio_rustls::TlsConnector;
use tokio_tungstenite::{
    connect_async_tls_with_config,
    tungstenite::{
        client::IntoClientRequest,
        http::{header, HeaderValue},
        protocol::{Message, WebSocketConfig},
    },
    Connector, MaybeTlsStream,
};

pub async fn shutdown_tls(stream: &mut ClientStream) -> anyhow::Result<()> {
    log::trace!("Exiting gracefully...");
    stream.get_inner().get_mut().1.send_close_notify();
    stream
        .send(StatusUpdate {
            kind: StatusType::Exit as i32,
            details: None,
        })
        .await?;
    stream.get_inner().get_mut().0.shutdown().await?;
    log::trace!("Connection closed.");
    Ok(())
}

fn tls_config(insecure: bool, ca_file: Option<&Path>) -> anyhow::Result<rustls::ClientConfig> {
    let root_store = if insecure {
        rustls::RootCertStore::empty()
    } else {
        let mut roots =
            rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let native_roots = rustls_native_certs::load_native_certs();
        for error in native_roots.errors {
            log::warn!("Unable to load one native TLS root certificate: {error}");
        }
        for certificate in native_roots.certs {
            if let Err(error) = roots.add(certificate) {
                log::debug!("Skipping duplicate/invalid native TLS root: {error}");
            }
        }
        if let Some(ca_file) = ca_file {
            let file = std::fs::File::open(ca_file)
                .with_context(|| format!("opening custom CA file {}", ca_file.display()))?;
            let mut added_certificates = 0;
            for certificate in rustls_pemfile::certs(&mut std::io::BufReader::new(file)) {
                roots.add(certificate.context("parsing certificate from custom CA file")?)?;
                added_certificates += 1;
            }
            if added_certificates == 0 {
                bail!(
                    "custom CA file {} contains no certificates",
                    ca_file.display()
                );
            }
        }
        roots
    };
    let mut config = rustls::ClientConfig::builder_with_details(
        CryptoProvider {
            cipher_suites: vec![provider::cipher_suite::TLS13_CHACHA20_POLY1305_SHA256],
            kx_groups: vec![provider::kx_group::X25519],
            ..provider::default_provider()
        }
        .into(),
        Arc::new(time_provider::DefaultTimeProvider),
    )
    .with_protocol_versions(&[&rustls::version::TLS13])?
    .with_root_certificates(root_store)
    .with_no_client_auth();
    if insecure {
        config
            .dangerous()
            .set_certificate_verifier(Arc::new(NoCertificateVerification {}));
    }
    Ok(config)
}

async fn verify_host(
    known_hosts: &mut config::KnownHosts,
    host: &str,
    certs: &[rustls::pki_types::CertificateDer<'_>],
) -> anyhow::Result<bool> {
    let mut fingerprints: Vec<Vec<u8>> = Vec::new();
    for cert in certs {
        let fingerprint = Sha256::digest(cert.as_ref());
        fingerprints.push(fingerprint.to_vec());
    }
    if let Some(known) = known_hosts.find_host(host) {
        if known.compare(&fingerprints) {
            log::info!("Host {} verified successfully.", host);
            Ok(true)
        } else {
            log::warn!(
                "Host {} fingerprint mismatch. Expected: {:X?}, Found: {:X?}",
                host,
                known.fingerprints,
                fingerprints
            );
            Ok(false)
        }
    } else {
        if fingerprints.is_empty() {
            log::error!(
                "Host {} has no fingerprints. Use --insecure to skip verification.",
                host
            );
            return Ok(false);
        }
        log::warn!(
            "Host {} not found in known hosts. Please verify the host's fingerprint.",
            host
        );
        println!("Host {} fingerprints: {:X?}", host, fingerprints);
        let confirmation = Confirm::new()
            .with_prompt("Do you want to add this host to known hosts?")
            .default(false)
            .interact()?;
        if confirmation {
            known_hosts.add_host(host.to_string(), fingerprints.clone(), None, None);
            log::info!("Host {} added to known hosts.", host);
            Ok(true)
        } else {
            log::warn!("Host {} not added to known hosts.", host);
            Ok(false)
        }
    }
}

pub async fn connect_tls(
    host: &str,
    port: u16,
    insecure: bool,
    monitors: Vec<MonitorInfo>,
    mut known_hosts: config::KnownHosts,
    id_files: config::IdFiles,
    id_override: Option<String>,
) -> anyhow::Result<(ServerHelloAck, ClientStream)> {
    let server_name = host.to_string().try_into()?;
    let tls_config = Arc::new(tls_config(insecure, None)?);
    let tls_connector = TlsConnector::from(tls_config);
    let addr = format!("{}:{}", host, port);
    let sock = TcpStream::connect(&addr).await?;
    let mut tls_stream = tls_connector.connect(server_name, sock).await?;
    if !insecure {
        let certs = tls_stream.get_ref().1.peer_certificates().unwrap();
        if !verify_host(&mut known_hosts, host, certs).await? {
            tls_stream.get_mut().1.send_close_notify();
            tls_stream.get_mut().0.shutdown().await?;
            log::warn!("Host verification failed. Connection closed.");
            return Err(anyhow::anyhow!("Host verification failed."));
        }
    }
    let mut messages = ClientStream::new(tls_stream);
    let hello = libgsh::client::handshake(
        &mut messages,
        monitors,
        ClientAuthProvider::new(known_hosts, id_files, id_override),
        host,
    )
    .await?;

    Ok((hello, messages))
}

pub async fn connect_webrtc(
    host: &str,
    port: u16,
    origin: &str,
    ca_file: Option<&Path>,
    monitors: Vec<MonitorInfo>,
    mut known_hosts: config::KnownHosts,
    id_files: config::IdFiles,
    id_override: Option<String>,
) -> anyhow::Result<(ServerHelloAck, Arc<GshSession>, WebRtcSignaling)> {
    let host_for_url = if host.starts_with('[') || !host.contains(':') {
        host.to_owned()
    } else {
        format!("[{host}]")
    };
    let url = format!("wss://{host_for_url}:{port}/signal");
    let mut request = url.into_client_request()?;
    request
        .headers_mut()
        .insert(header::ORIGIN, HeaderValue::from_str(origin)?);

    let tls_config = Arc::new(tls_config(false, ca_file)?);
    let mut websocket_config = WebSocketConfig::default();
    websocket_config.max_message_size = Some(1024 * 1024);
    websocket_config.max_frame_size = Some(1024 * 1024);
    let (mut websocket, _) = connect_async_tls_with_config(
        request,
        Some(websocket_config),
        true,
        Some(Connector::Rustls(tls_config)),
    )
    .await
    .context("connecting to WSS signaling endpoint")?;
    let certificates = match websocket.get_ref() {
        MaybeTlsStream::Rustls(stream) => stream
            .get_ref()
            .1
            .peer_certificates()
            .context("WSS peer did not provide a certificate")?,
        _ => bail!("WSS connection did not use rustls"),
    };
    if !verify_host(&mut known_hosts, host, certificates).await? {
        bail!("Host verification failed.");
    }

    let local_dimensions = monitors
        .first()
        .map(|monitor| (monitor.width, monitor.height))
        .filter(|(width, height)| *width > 0 && *height > 0);
    let os = match std::env::consts::OS {
        "linux" => client_hello::Os::Linux,
        "windows" => client_hello::Os::Windows,
        "macos" => client_hello::Os::Macos,
        _ => client_hello::Os::Unknown,
    } as i32;
    send_client_message(
        &mut websocket,
        ClientHello {
            protocol_version: PROTOCOL_VERSION,
            os,
            os_version: os_info::get().version().to_string(),
            monitors,
        }
        .into(),
    )
    .await?;
    let ServerEvent::ServerHelloAck(hello) = receive_server_message(&mut websocket)
        .await?
        .server_event
        .ok_or_else(|| anyhow!("server sent an empty GSH message"))?
    else {
        bail!("expected ServerHelloAck during WSS handshake");
    };

    let mut auth_provider = ClientAuthProvider::new(known_hosts, id_files, id_override);
    match hello.auth_method.as_ref() {
        Some(AuthMethod::Password(_)) => {
            let auth = ClientAuth {
                auth_data: Some(client_auth::AuthData::Password(client_auth::Password {
                    password: auth_provider.password(host),
                })),
            };
            send_client_message(&mut websocket, auth.into()).await?;
            let ack = receive_auth_ack(&mut websocket).await?;
            if ack.status != AuthStatus::Success as i32 {
                bail!("password authentication failed: {}", ack.message);
            }
            auth_provider.password_success_cb();
        }
        Some(AuthMethod::Signature(SignatureMethod { sign_message })) => {
            let (signature, public_key): (Signature, RsaPublicKey) =
                auth_provider.signature(host, sign_message).ok_or_else(|| {
                    anyhow!("signature authentication requires a configured identity")
                })?;
            let public_key = public_key.to_pkcs1_pem(libgsh::rsa::pkcs1::LineEnding::LF)?;
            let auth = ClientAuth {
                auth_data: Some(client_auth::AuthData::Signature(client_auth::Signature {
                    signature: signature.to_bytes().to_vec(),
                    public_key: public_key.into_bytes(),
                })),
            };
            send_client_message(&mut websocket, auth.into()).await?;
            let ack = receive_auth_ack(&mut websocket).await?;
            if ack.status != AuthStatus::Success as i32 {
                bail!("signature authentication failed: {}", ack.message);
            }
            auth_provider.signature_success_cb();
        }
        None => bail!("native WSS client requires server authentication"),
    }

    let (width, height) = hello
        .windows
        .first()
        .map(|window| (window.width, window.height))
        .filter(|(width, height)| *width > 0 && *height > 0)
        .or(local_dimensions)
        .unwrap_or((1920, 1080));
    let session = Arc::new(GshSession::new_with_video_codecs(
        reactor_webrtc::PeerConnectionFactory::builder().build()?,
        reactor_webrtc::RtcConfiguration::default(),
        SessionRole::OffererSubscriber,
        width,
        height,
        &[VideoCodec::H264, VideoCodec::Vp8],
    )?);
    let offer = session.create_offer()?;
    websocket
        .send(Message::Text(
            json!({"type": "offer", "sdp": offer.sdp})
                .to_string()
                .into(),
        ))
        .await?;

    let mut answer_set = false;
    let mut pending_candidates = VecDeque::new();
    let mut candidate_count = 0_usize;
    let mut candidate_bytes = 0_usize;
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut connection_poll = tokio::time::interval(Duration::from_millis(100));
    connection_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            incoming = websocket.next() => {
                let Some(incoming) = incoming else { bail!("WSS signaling connection closed during WebRTC negotiation"); };
                match incoming? {
                    Message::Text(text) => {
                        let value: Value = serde_json::from_str(text.as_str())?;
                        match value.get("type").and_then(Value::as_str) {
                            Some("answer") if !answer_set => {
                                let sdp = value.get("sdp").and_then(Value::as_str)
                                    .ok_or_else(|| anyhow!("answer has no SDP string"))?;
                                session.accept_answer(libgsh::webrtc::SdpMessage {
                                    kind: libgsh::webrtc::DescriptionKind::Answer,
                                    sdp: sdp.to_owned(),
                                })?;
                                answer_set = true;
                                while let Some(candidate) = pending_candidates.pop_front() {
                                    session.add_ice_candidate(candidate)?;
                                }
                            }
                            Some("candidate") => {
                                let candidate = parse_candidate(&value)?;
                                candidate_count += 1;
                                candidate_bytes = candidate_bytes
                                    .checked_add(candidate.candidate.len())
                                    .and_then(|size| size.checked_add(candidate.sdp_mid.as_ref().map_or(0, String::len)))
                                    .ok_or_else(|| anyhow!("ICE candidate size overflow"))?;
                                if candidate_count > 256 || candidate_bytes > 64 * 1024 {
                                    bail!("ICE candidate limits exceeded");
                                }
                                if answer_set {
                                    session.add_ice_candidate(candidate)?;
                                } else {
                                    pending_candidates.push_back(candidate);
                                }
                            }
                            Some("status") => {
                                if let Some(status) = value.get("message").and_then(Value::as_str) {
                                    log::info!("Remote desktop: {status}");
                                }
                            }
                            other => bail!("unexpected WSS signaling message: {other:?}"),
                        }
                    }
                    Message::Binary(_) => bail!("unexpected binary GSH message after authentication"),
                    Message::Ping(payload) => websocket.send(Message::Pong(payload)).await?,
                    Message::Close(_) => bail!("WSS signaling connection closed during WebRTC negotiation"),
                    Message::Pong(_) => {}
                    _ => {}
                }
            }
            candidate = session.next_ice_candidate() => {
                if let Some(candidate) = candidate? {
                    websocket.send(Message::Text(json!({
                        "type": "candidate",
                        "candidate": {
                            "candidate": candidate.candidate,
                            "sdpMid": candidate.sdp_mid,
                            "sdpMLineIndex": candidate.sdp_mline_index,
                        }
                    }).to_string().into())).await?;
                }
            }
            _ = connection_poll.tick() => {
                if answer_set && session.is_connected() && session.input_channel_open() {
                    log::info!("Native WebRTC session connected; codec={:?}", session.negotiated_video_codec());
                    break;
                }
            }
            _ = tokio::time::sleep_until(deadline) => bail!("timed out waiting for WebRTC connection"),
        }
    }

    Ok((
        hello,
        session,
        WebRtcSignaling::new(websocket, candidate_count, candidate_bytes),
    ))
}

pub enum WebRtcSignalEvent {
    Candidate(CandidateMessage),
    Status(String),
}

pub struct WebRtcSignaling {
    reader: tokio::sync::Mutex<SplitStream<WebSocketClientStream>>,
    writer: tokio::sync::Mutex<SplitSink<WebSocketClientStream, Message>>,
    candidate_limits: Mutex<(usize, usize)>,
}

type WebSocketClientStream = tokio_tungstenite::WebSocketStream<MaybeTlsStream<TcpStream>>;

impl WebRtcSignaling {
    fn new(
        websocket: WebSocketClientStream,
        received_candidates: usize,
        received_candidate_bytes: usize,
    ) -> Self {
        let (writer, reader) = websocket.split();
        Self {
            reader: tokio::sync::Mutex::new(reader),
            writer: tokio::sync::Mutex::new(writer),
            candidate_limits: Mutex::new((received_candidates, received_candidate_bytes)),
        }
    }

    pub async fn receive(&self) -> anyhow::Result<Option<WebRtcSignalEvent>> {
        loop {
            let incoming = self.reader.lock().await.next().await;
            let Some(incoming) = incoming else {
                return Ok(None);
            };
            match incoming? {
                Message::Text(text) => {
                    let value: Value = serde_json::from_str(text.as_str())?;
                    match value.get("type").and_then(Value::as_str) {
                        Some("candidate") => {
                            let candidate = parse_candidate(&value)?;
                            let mut limits = self
                                .candidate_limits
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner());
                            limits.0 += 1;
                            limits.1 = limits
                                .1
                                .checked_add(candidate.candidate.len())
                                .and_then(|size| {
                                    size.checked_add(
                                        candidate.sdp_mid.as_ref().map_or(0, String::len),
                                    )
                                })
                                .ok_or_else(|| anyhow!("ICE candidate size overflow"))?;
                            if limits.0 > 256 || limits.1 > 64 * 1024 {
                                bail!("ICE candidate limits exceeded");
                            }
                            return Ok(Some(WebRtcSignalEvent::Candidate(candidate)));
                        }
                        Some("status") => {
                            let status = value
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("unspecified server status")
                                .to_owned();
                            return Ok(Some(WebRtcSignalEvent::Status(status)));
                        }
                        other => bail!("unexpected WSS signaling message: {other:?}"),
                    }
                }
                Message::Binary(_) => bail!("unexpected binary GSH message after authentication"),
                Message::Ping(payload) => {
                    self.writer
                        .lock()
                        .await
                        .send(Message::Pong(payload))
                        .await?
                }
                Message::Close(_) => return Ok(None),
                Message::Pong(_) => {}
                _ => {}
            }
        }
    }

    pub async fn send_candidate(&self, candidate: CandidateMessage) -> anyhow::Result<()> {
        self.writer
            .lock()
            .await
            .send(Message::Text(
                json!({
                    "type": "candidate",
                    "candidate": {
                        "candidate": candidate.candidate,
                        "sdpMid": candidate.sdp_mid,
                        "sdpMLineIndex": candidate.sdp_mline_index,
                    }
                })
                .to_string()
                .into(),
            ))
            .await?;
        Ok(())
    }

    pub async fn close(&self) -> anyhow::Result<()> {
        self.writer.lock().await.close().await?;
        Ok(())
    }
}

async fn send_client_message(
    websocket: &mut tokio_tungstenite::WebSocketStream<MaybeTlsStream<TcpStream>>,
    message: ClientMessage,
) -> anyhow::Result<()> {
    websocket
        .send(Message::Binary(message.encode_to_vec().into()))
        .await?;
    Ok(())
}

async fn receive_server_message(
    websocket: &mut tokio_tungstenite::WebSocketStream<MaybeTlsStream<TcpStream>>,
) -> anyhow::Result<ServerMessage> {
    loop {
        match websocket
            .next()
            .await
            .ok_or_else(|| anyhow!("WSS signaling connection closed during GSH handshake"))??
        {
            Message::Binary(bytes) => return Ok(ServerMessage::decode(bytes)?),
            Message::Ping(payload) => websocket.send(Message::Pong(payload)).await?,
            Message::Close(_) => bail!("WSS signaling connection closed during GSH handshake"),
            Message::Text(_) | Message::Pong(_) => {}
            _ => {}
        }
    }
}

async fn receive_auth_ack(
    websocket: &mut tokio_tungstenite::WebSocketStream<MaybeTlsStream<TcpStream>>,
) -> anyhow::Result<libgsh::shared::protocol::ServerAuthAck> {
    let ServerEvent::ServerAuthAck(ack) = receive_server_message(websocket)
        .await?
        .server_event
        .ok_or_else(|| anyhow!("server sent an empty GSH message"))?
    else {
        bail!("expected ServerAuthAck after ClientAuth");
    };
    Ok(ack)
}

fn parse_candidate(value: &Value) -> anyhow::Result<CandidateMessage> {
    let candidate = value
        .get("candidate")
        .ok_or_else(|| anyhow!("ICE message has no candidate"))?;
    let candidate_text = candidate
        .get("candidate")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("ICE candidate has no candidate string"))?;
    let sdp_mid = candidate
        .get("sdpMid")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if candidate_text.len() > 4096 || sdp_mid.as_ref().is_some_and(|mid| mid.len() > 64) {
        bail!("ICE candidate exceeds protocol size limit");
    }
    let sdp_mline_index = candidate
        .get("sdpMLineIndex")
        .and_then(Value::as_u64)
        .map(u16::try_from)
        .transpose()?;
    Ok(CandidateMessage {
        candidate: candidate_text.to_owned(),
        sdp_mid,
        sdp_mline_index,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use libgsh::sha2::Digest;
    use libgsh::{
        rsa::{
            pkcs1::DecodeRsaPublicKey, pkcs1v15::VerifyingKey, rand_core::OsRng,
            signature::Verifier, RsaPrivateKey,
        },
        shared::protocol::client_message::ClientEvent,
        shared::protocol::{
            client_auth::AuthData,
            server_hello_ack::{self, WindowSettings},
            ClientMessage,
        },
        tokio_rustls::rustls,
    };
    use tokio::net::TcpListener;
    use tokio_rustls::TlsAcceptor;
    use tokio_tungstenite::accept_hdr_async;

    #[tokio::test]
    async fn native_webrtc_client_authenticates_and_receives_media() {
        let temp = tempfile::tempdir().unwrap();
        let (certificate, private_key) = libgsh::shared::cert::self_signed(&["127.0.0.1"]).unwrap();
        let certificate_pem = certificate.cert.pem();
        let certificate_der = certificate.cert.der().clone();
        let ca_path = temp.path().join("ca.pem");
        std::fs::write(&ca_path, certificate_pem).unwrap();
        let tls_config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![certificate_der.clone()], private_key)
        .unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(tls_config));

        let identity_private = RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
        let identity_public = RsaPublicKey::from(&identity_private);
        let identity_pem = libgsh::shared::cert::keys_to_pem(&identity_private, &identity_public);
        let identity_path = temp.path().join("identity.pem");
        std::fs::write(&identity_path, identity_pem).unwrap();

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let origin = format!("https://127.0.0.1:{port}");
        let expected_origin = origin.clone();
        let authorized_key = identity_public.clone();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let tls = acceptor.accept(socket).await.unwrap();
            let mut websocket = accept_hdr_async(
                tls,
                move |request: &tokio_tungstenite::tungstenite::handshake::server::Request,
                      response| {
                    assert_eq!(
                        request
                            .headers()
                            .get(header::ORIGIN)
                            .unwrap()
                            .to_str()
                            .unwrap(),
                        expected_origin
                    );
                    Ok(response)
                },
            )
            .await
            .unwrap();

            let hello = match websocket.next().await.unwrap().unwrap() {
                Message::Binary(bytes) => {
                    let message = ClientMessage::decode(bytes).unwrap();
                    let Some(ClientEvent::ClientHello(hello)) = message.client_event else {
                        panic!("expected ClientHello");
                    };
                    hello
                }
                message => panic!("unexpected message before ClientHello: {message:?}"),
            };
            assert_eq!(hello.protocol_version, PROTOCOL_VERSION);

            let challenge = b"native-client-webrtc-test-challenge".to_vec();
            let hello = ServerHelloAck {
                windows: vec![WindowSettings {
                    window_id: 0,
                    width: 320,
                    height: 240,
                    monitor_id: None,
                    initial_mode: server_hello_ack::window_settings::WindowMode::Windowed as i32,
                    title: "Test desktop".to_string(),
                    always_on_top: false,
                    allow_resize: true,
                    resize_frame: false,
                    frame_anchor: server_hello_ack::window_settings::WindowAnchor::Center as i32,
                }],
                format: server_hello_ack::FrameFormat::Rgba as i32,
                compression: None,
                auth_method: Some(AuthMethod::Signature(SignatureMethod {
                    sign_message: challenge.clone(),
                })),
            };
            let response = ServerMessage {
                server_event: Some(ServerEvent::ServerHelloAck(hello)),
            };
            websocket
                .send(Message::Binary(response.encode_to_vec().into()))
                .await
                .unwrap();

            let auth = match websocket.next().await.unwrap().unwrap() {
                Message::Binary(bytes) => {
                    let message = ClientMessage::decode(bytes).unwrap();
                    let Some(ClientEvent::ClientAuth(auth)) = message.client_event else {
                        panic!("expected ClientAuth");
                    };
                    auth
                }
                message => panic!("unexpected message before ClientAuth: {message:?}"),
            };
            let Some(AuthData::Signature(auth)) = auth.auth_data else {
                panic!("expected RSA signature authentication");
            };
            let public_key_pem = String::from_utf8(auth.public_key).unwrap();
            let public_key = RsaPublicKey::from_pkcs1_pem(&public_key_pem).unwrap();
            assert_eq!(public_key, authorized_key);
            let signature = Signature::try_from(auth.signature.as_slice()).unwrap();
            VerifyingKey::<Sha256>::new(public_key)
                .verify(&challenge, &signature)
                .unwrap();
            let response = ServerMessage {
                server_event: Some(ServerEvent::ServerAuthAck(
                    libgsh::shared::protocol::ServerAuthAck {
                        status: AuthStatus::Success as i32,
                        message: "ok".to_string(),
                    },
                )),
            };
            websocket
                .send(Message::Binary(response.encode_to_vec().into()))
                .await
                .unwrap();

            let publisher = Arc::new(
                GshSession::new_raw_video(
                    reactor_webrtc::PeerConnectionFactory::builder()
                        .build()
                        .unwrap(),
                    reactor_webrtc::RtcConfiguration::default(),
                    SessionRole::AnswererPublisher,
                    320,
                    240,
                )
                .unwrap(),
            );
            let offer_sdp = match websocket.next().await.unwrap().unwrap() {
                Message::Text(text) => {
                    let message: Value = serde_json::from_str(text.as_str()).unwrap();
                    assert_eq!(message["type"], "offer");
                    message["sdp"].as_str().unwrap().to_string()
                }
                message => panic!("unexpected message before SDP offer: {message:?}"),
            };
            let answer = publisher
                .accept_offer(libgsh::webrtc::SdpMessage {
                    kind: libgsh::webrtc::DescriptionKind::Offer,
                    sdp: offer_sdp,
                })
                .unwrap();
            websocket
                .send(Message::Text(
                    json!({"type": "answer", "sdp": answer.sdp})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();

            let mut poll = tokio::time::interval(Duration::from_millis(10));
            loop {
                tokio::select! {
                    incoming = websocket.next() => {
                        let Some(incoming) = incoming else { panic!("WSS client closed before ICE connected"); };
                        if let Message::Text(text) = incoming.unwrap() {
                            let value: Value = serde_json::from_str(text.as_str()).unwrap();
                            if value["type"] == "candidate" {
                                publisher.add_ice_candidate(parse_candidate(&value).unwrap()).unwrap();
                            }
                        }
                    }
                    candidate = publisher.next_ice_candidate() => {
                        if let Some(candidate) = candidate.unwrap() {
                            websocket.send(Message::Text(json!({
                                "type": "candidate",
                                "candidate": {
                                    "candidate": candidate.candidate,
                                    "sdpMid": candidate.sdp_mid,
                                    "sdpMLineIndex": candidate.sdp_mline_index,
                                }
                            }).to_string().into())).await.unwrap();
                        }
                    }
                    _ = poll.tick() => {
                        if publisher.is_connected() && publisher.input_channel_open() {
                            break;
                        }
                    }
                }
            }

            let frame = vec![0x40; 320 * 240 * 4];
            for _ in 0..10 {
                match publisher.send_raw_video(&frame) {
                    Ok(()) | Err(libgsh::webrtc::SessionError::VideoBackpressure) => {}
                    Err(error) => panic!("native test publisher failed: {error}"),
                }
                tokio::time::sleep(Duration::from_millis(34)).await;
            }
            let input = tokio::time::timeout(Duration::from_secs(5), publisher.recv_input())
                .await
                .expect("native input was not received")
                .unwrap();
            tokio::time::sleep(Duration::from_millis(250)).await;
            input
        });

        let mut id_files = config::IdFiles::default();
        id_files.add_ephemeral_id_file("test-identity", identity_path);
        let known_hosts = config::KnownHosts {
            hosts: vec![config::KnownHost {
                host: "127.0.0.1".to_string(),
                fingerprints: vec![Sha256::digest(certificate_der.as_ref()).to_vec()],
                id_file_ref: Some("test-identity".to_string()),
                password: None,
            }],
        };
        let (hello, session, _signaling) = tokio::time::timeout(
            Duration::from_secs(20),
            connect_webrtc(
                "127.0.0.1",
                port,
                &origin,
                Some(&ca_path),
                Vec::new(),
                known_hosts,
                id_files,
                Some("test-identity".to_string()),
            ),
        )
        .await
        .expect("native WSS/WebRTC connection timed out")
        .unwrap();
        assert_eq!(hello.windows[0].width, 320);
        let input = libgsh::shared::protocol::UserInput::default();
        session.send_input(&input).unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), session.recv_video())
                .await
                .expect("native video receive timed out")
                .unwrap()
                .width,
            320
        );
        assert_eq!(server.await.unwrap(), input);
    }

    #[test]
    fn parses_remote_ice_candidate_shape() {
        let value = json!({
            "type": "candidate",
            "candidate": {
                "candidate": "candidate:1 1 UDP 2122260223 192.0.2.1 5000 typ host",
                "sdpMid": "video",
                "sdpMLineIndex": 1,
            }
        });
        let candidate = parse_candidate(&value).unwrap();
        assert_eq!(candidate.sdp_mid.as_deref(), Some("video"));
        assert_eq!(candidate.sdp_mline_index, Some(1));
        assert!(candidate.candidate.starts_with("candidate:1"));
    }

    #[test]
    fn rejects_oversized_remote_ice_fields() {
        let value = json!({"candidate": {"candidate": "x".repeat(4097)}});
        assert!(parse_candidate(&value).is_err());

        let value = json!({"candidate": {"candidate": "candidate:1", "sdpMid": "x".repeat(65)}});
        assert!(parse_candidate(&value).is_err());
    }
}

#[derive(Debug, Clone)]
struct NoCertificateVerification {}

impl ServerCertVerifier for NoCertificateVerification {
    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
        ]
    }

    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
}
