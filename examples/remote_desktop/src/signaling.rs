use crate::nvenc::{NvencEncoder, GOP_FRAMES, OUTPUT_FPS, OUTPUT_HEIGHT, OUTPUT_WIDTH};
use crate::{application::RemoteDesktopApplication, frame_source::CapturedFrame};
use anyhow::{anyhow, bail, Context, Result};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        ConnectInfo, State,
    },
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use futures_util::{SinkExt, StreamExt};
use libgsh::{
    pacing::FramePacer,
    shared::{
        protocol::{
            client_message::ClientEvent,
            server_auth_ack::AuthStatus,
            user_input::{
                self, key_event::KeyAction, mouse_event::MouseAction, mouse_event::MouseButton,
                InputType,
            },
            ClientMessage, ServerMessage, StatusUpdate, UserInput,
        },
        PROTOCOL_VERSION,
    },
    webrtc::{
        CandidateMessage, EncodedVideoFrame, GshSession, SessionRole, VideoCodec, VideoInputMode,
    },
    webrtc_input::{GshInputEvent, GshInputRouter},
};
use prost::Message as ProstMessage;
use reactor_webrtc::{PeerConnectionFactory, RtcConfiguration};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const MAX_SIGNAL_MESSAGE_BYTES: usize = 1024 * 1024;
const CSS_LINE_HEIGHT_PX: f64 = 16.0;
const MAX_PRE_AUTH_SESSIONS: usize = 4;
const MAX_ACTIVE_SESSIONS: usize = 16;
const MAX_AUTH_FAILURES_PER_MINUTE: usize = 5;
const MAX_AUTH_RATE_LIMITER_IPS: usize = 1024;
const AUTH_FAILURE_WINDOW: Duration = Duration::from_secs(60);
const MAX_ICE_CANDIDATES_PER_SESSION: usize = 256;
const MAX_ICE_CANDIDATE_BYTES: usize = 4096;
const MAX_SDP_MID_BYTES: usize = 64;
const MAX_PENDING_CANDIDATE_BYTES: usize = 64 * 1024;
const MAX_SDP_CANDIDATES: usize = 256;
const MAX_SDP_CANDIDATE_BYTES: usize = 4096;
const MAX_TOTAL_SDP_CANDIDATE_BYTES: usize = 64 * 1024;
const MAX_SDP_BYTES: usize = 1024 * 1024;
const GSH_AUTH_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(60);
const OFFER_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
struct WebState {
    application: RemoteDesktopApplication,
    allowed_origins: Arc<HashSet<String>>,
    pre_auth_sessions: Arc<tokio::sync::Semaphore>,
    active_sessions: Arc<tokio::sync::Semaphore>,
    auth_rate_limiter: Arc<AuthRateLimiter>,
}

#[derive(Default)]
struct AuthAttemptWindow {
    failures: VecDeque<Instant>,
    in_flight: usize,
}

#[derive(Default)]
struct AuthRateLimiter {
    windows: Mutex<HashMap<IpAddr, AuthAttemptWindow>>,
}

impl AuthRateLimiter {
    fn reserve(self: &Arc<Self>, ip: IpAddr) -> Option<AuthAttemptPermit> {
        let now = Instant::now();
        let mut windows = self
            .windows
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for window in windows.values_mut() {
            while window
                .failures
                .front()
                .is_some_and(|attempt| now.duration_since(*attempt) >= AUTH_FAILURE_WINDOW)
            {
                window.failures.pop_front();
            }
        }
        windows.retain(|_, window| window.in_flight > 0 || !window.failures.is_empty());
        if !windows.contains_key(&ip) && windows.len() >= MAX_AUTH_RATE_LIMITER_IPS {
            let oldest_idle_ip = windows
                .iter()
                .filter(|(_, window)| window.in_flight == 0)
                .min_by_key(|(_, window)| window.failures.front().copied())
                .map(|(ip, _)| *ip);
            if let Some(oldest_idle_ip) = oldest_idle_ip {
                windows.remove(&oldest_idle_ip);
            } else {
                return None;
            }
        }
        let window = windows.entry(ip).or_default();
        if window.failures.len() + window.in_flight >= MAX_AUTH_FAILURES_PER_MINUTE {
            return None;
        }
        window.in_flight += 1;
        Some(AuthAttemptPermit {
            limiter: self.clone(),
            ip,
            completed: false,
        })
    }
}

struct AuthAttemptPermit {
    limiter: Arc<AuthRateLimiter>,
    ip: IpAddr,
    completed: bool,
}

impl AuthAttemptPermit {
    fn succeed(mut self) {
        self.finish(false);
    }

    fn fail(mut self) {
        self.finish(true);
    }

    fn finish(&mut self, failed: bool) {
        let mut windows = self
            .limiter
            .windows
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(window) = windows.get_mut(&self.ip) {
            window.in_flight = window.in_flight.saturating_sub(1);
            if failed {
                window.failures.push_back(Instant::now());
            }
        }
        self.completed = true;
    }
}

impl Drop for AuthAttemptPermit {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        // Malformed, abandoned, and timed-out handshakes count against the
        // same per-IP budget as explicit credential failures.
        self.finish(true);
    }
}

pub fn router(application: RemoteDesktopApplication, allowed_origins: Vec<String>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/client.js", get(client_js))
        .route("/signal", get(signal_route))
        .with_state(WebState {
            application,
            allowed_origins: Arc::new(allowed_origins.into_iter().collect()),
            pre_auth_sessions: Arc::new(tokio::sync::Semaphore::new(MAX_PRE_AUTH_SESSIONS)),
            active_sessions: Arc::new(tokio::sync::Semaphore::new(MAX_ACTIVE_SESSIONS)),
            auth_rate_limiter: Arc::new(AuthRateLimiter::default()),
        })
}

fn is_allowed_origin(allowed_origins: &HashSet<String>, origin: Option<&str>) -> bool {
    origin.is_some_and(|origin| allowed_origins.contains(origin))
}

async fn index(State(state): State<WebState>) -> impl IntoResponse {
    let websocket_origins = state
        .allowed_origins
        .iter()
        .map(|origin| origin.replacen("https://", "wss://", 1))
        .collect::<Vec<_>>()
        .join(" ");
    let content_security_policy = format!(
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self' {websocket_origins}; media-src 'self' blob:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'"
    );
    let headers = [
        (
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_str(&content_security_policy)
                .expect("configured HTTPS origins must form a valid CSP"),
        ),
        (
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ),
        (
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ),
    ];
    (headers, Html(include_str!("../static/index.html")))
}

async fn client_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../static/client.js"),
    )
}

async fn signal_route(
    State(state): State<WebState>,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    upgrade: WebSocketUpgrade,
) -> Result<impl IntoResponse, StatusCode> {
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok());
    if !is_allowed_origin(&state.allowed_origins, origin) {
        return Err(StatusCode::FORBIDDEN);
    }
    let permit = state
        .active_sessions
        .clone()
        .try_acquire_owned()
        .map_err(|_| StatusCode::TOO_MANY_REQUESTS)?;
    let auth_attempt = state
        .auth_rate_limiter
        .reserve(peer.ip())
        .ok_or(StatusCode::TOO_MANY_REQUESTS)?;
    let pre_auth_permit = state
        .pre_auth_sessions
        .clone()
        .try_acquire_owned()
        .map_err(|_| StatusCode::TOO_MANY_REQUESTS)?;
    Ok(upgrade
        .max_message_size(MAX_SIGNAL_MESSAGE_BYTES)
        .max_frame_size(MAX_SIGNAL_MESSAGE_BYTES)
        .on_upgrade(move |socket| async move {
            if let Err(error) =
                run_session(socket, state, permit, pre_auth_permit, auth_attempt).await
            {
                log::warn!("WebRTC session ended: {error:#}");
            }
        }))
}

async fn run_session(
    socket: WebSocket,
    state: WebState,
    _permit: tokio::sync::OwnedSemaphorePermit,
    pre_auth_permit: tokio::sync::OwnedSemaphorePermit,
    auth_attempt: AuthAttemptPermit,
) -> Result<()> {
    let (mut socket_tx, mut socket_rx) = socket.split();
    let auth_deadline = Instant::now() + GSH_AUTH_HANDSHAKE_TIMEOUT;
    let client_hello = receive_gsh_message(&mut socket_rx, "ClientHello", auth_deadline).await?;
    let ClientEvent::ClientHello(client_hello) = client_hello
        .client_event
        .ok_or_else(|| anyhow!("first GSH WebSocket message must contain ClientHello"))?
    else {
        bail!("first GSH WebSocket message must contain ClientHello");
    };
    if client_hello.protocol_version != PROTOCOL_VERSION {
        send_gsh_message_before(
            &mut socket_tx,
            ServerMessage {
                server_event: Some(
                    libgsh::shared::protocol::server_message::ServerEvent::StatusUpdate(
                        StatusUpdate {
                            kind: libgsh::shared::protocol::status_update::StatusType::Exit as i32,
                            details: None,
                        },
                    ),
                ),
            },
            auth_deadline,
        )
        .await?;
        bail!(
            "unsupported GSH protocol version {}",
            client_hello.protocol_version
        );
    }

    let server_hello = state.application.server_hello();
    let auth_method = server_hello
        .auth_method
        .clone()
        .ok_or_else(|| anyhow!("remote desktop WSS requires authentication"))?;
    send_gsh_message_before(
        &mut socket_tx,
        ServerMessage {
            server_event: Some(
                libgsh::shared::protocol::server_message::ServerEvent::ServerHelloAck(server_hello),
            ),
        },
        auth_deadline,
    )
    .await?;

    let client_auth = receive_gsh_message(&mut socket_rx, "ClientAuth", auth_deadline).await?;
    let ClientEvent::ClientAuth(client_auth) = client_auth
        .client_event
        .ok_or_else(|| anyhow!("expected ClientAuth after ServerHelloAck"))?
    else {
        bail!("expected ClientAuth after ServerHelloAck");
    };
    let auth_result = state
        .application
        .auth_verifier()
        .verify(&auth_method, client_auth.auth_data.as_ref());
    if Instant::now() >= auth_deadline {
        auth_attempt.fail();
        bail!("GSH authentication deadline expired during verification");
    }
    if let Err(error) = auth_result {
        auth_attempt.fail();
        send_gsh_message_before(
            &mut socket_tx,
            ServerMessage {
                server_event: Some(
                    libgsh::shared::protocol::server_message::ServerEvent::ServerAuthAck(
                        libgsh::shared::protocol::ServerAuthAck {
                            status: AuthStatus::Failure as i32,
                            message: error.to_string(),
                        },
                    ),
                ),
            },
            auth_deadline,
        )
        .await?;
        return Err(anyhow!("GSH authentication failed: {error}"));
    }
    auth_attempt.succeed();
    drop(pre_auth_permit);
    send_gsh_message_before(
        &mut socket_tx,
        ServerMessage {
            server_event: Some(
                libgsh::shared::protocol::server_message::ServerEvent::ServerAuthAck(
                    libgsh::shared::protocol::ServerAuthAck {
                        status: AuthStatus::Success as i32,
                        message: "Authentication successful".to_string(),
                    },
                ),
            ),
        },
        auth_deadline,
    )
    .await?;

    let application = state.application;
    let mut captured_frames = application.subscribe_frames();
    let initial_frame = tokio::time::timeout(Duration::from_secs(10), captured_frames.next())
        .await
        .context("waiting for first screen-capture frame")?
        .ok_or_else(|| anyhow!("screen-capture source closed"))?;

    let (width, height) = application.dimensions();
    if (initial_frame.width, initial_frame.height) != (width, height) {
        bail!(
            "capture dimensions changed before session start: configured {width}x{height}, captured {}x{}",
            initial_frame.width,
            initial_frame.height
        );
    }
    run_webrtc_session(
        &mut socket_tx,
        &mut socket_rx,
        application,
        captured_frames,
        initial_frame,
    )
    .await
}

struct SignaledOffer {
    sdp: String,
    candidates: VecDeque<CandidateMessage>,
    candidate_count: usize,
    current_frame: Arc<CapturedFrame>,
}

async fn receive_offer(
    socket_tx: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    socket_rx: &mut futures_util::stream::SplitStream<WebSocket>,
    captured_frames: &mut crate::frame_source::FrameSource,
    mut current_frame: Arc<CapturedFrame>,
    width: u32,
    height: u32,
) -> Result<Option<SignaledOffer>> {
    let mut candidates = VecDeque::new();
    let mut candidate_count = 0_usize;
    let mut candidate_bytes = 0_usize;
    let deadline = tokio::time::sleep(OFFER_TIMEOUT);
    tokio::pin!(deadline);

    loop {
        tokio::select! {
            incoming = socket_rx.next() => {
                let Some(incoming) = incoming else { return Ok(None); };
                match incoming? {
                    Message::Text(text) => {
                        let value: Value = serde_json::from_str(text.as_str())?;
                        match value.get("type").and_then(Value::as_str) {
                            Some("offer") => {
                                let sdp = value.get("sdp").and_then(Value::as_str)
                                    .ok_or_else(|| anyhow!("offer has no SDP string"))?;
                                validate_offer_sdp(sdp)?;
                                return Ok(Some(SignaledOffer {
                                    sdp: sdp.to_owned(),
                                    candidates,
                                    candidate_count,
                                    current_frame,
                                }));
                            }
                            Some("candidate") => {
                                let candidate = parse_candidate(&value)?;
                                candidate_count += 1;
                                if candidate_count > MAX_ICE_CANDIDATES_PER_SESSION {
                                    bail!("too many ICE candidates before SDP offer");
                                }
                                let candidate_size = candidate.candidate.len()
                                    + candidate.sdp_mid.as_ref().map_or(0, String::len)
                                    + std::mem::size_of_val(&candidate.sdp_mline_index);
                                candidate_bytes = candidate_bytes
                                    .checked_add(candidate_size)
                                    .ok_or_else(|| anyhow!("pending ICE candidate size overflow"))?;
                                if candidate_bytes > MAX_PENDING_CANDIDATE_BYTES {
                                    bail!("pending ICE candidates exceed {MAX_PENDING_CANDIDATE_BYTES} bytes");
                                }
                                candidates.push_back(candidate);
                            }
                            _ => {}
                        }
                    }
                    Message::Close(_) => return Ok(None),
                    Message::Ping(payload) => socket_tx.send(Message::Pong(payload)).await?,
                    Message::Binary(_) | Message::Pong(_) => {}
                }
            }
            captured = captured_frames.next() => {
                let Some(captured) = captured else {
                    bail!("screen-capture source closed before SDP offer");
                };
                if (captured.width, captured.height) != (width, height) {
                    bail!("capture dimensions changed before WebRTC negotiation");
                }
                current_frame = captured;
            }
            _ = &mut deadline => bail!("timed out waiting for authenticated WebRTC offer"),
        }
    }
}

async fn run_webrtc_session(
    socket_tx: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    socket_rx: &mut futures_util::stream::SplitStream<WebSocket>,
    application: RemoteDesktopApplication,
    mut captured_frames: crate::frame_source::FrameSource,
    initial_frame: Arc<CapturedFrame>,
) -> Result<()> {
    let (width, height) = application.dimensions();
    let Some(offer) = receive_offer(
        socket_tx,
        socket_rx,
        &mut captured_frames,
        initial_frame,
        width,
        height,
    )
    .await?
    else {
        return Ok(());
    };
    let rtp_timestamp_origin = offer.current_frame.captured_at;
    let mut last_nvenc_sequence = offer.current_frame.sequence;

    let offers_h264 = sdp_offers_h264(&offer.sdp);
    let offers_vp8 = sdp_offers_vp8(&offer.sdp);
    let mut nvenc_encoder = if application.nvenc_supported() && offers_h264 {
        match NvencEncoder::spawn(width, height) {
            Ok(encoder) => {
                log::info!("Using FFmpeg h264_nvenc at 1920x1080/60 for this peer");
                Some(encoder)
            }
            Err(error) => {
                log::warn!("NVENC session startup failed; using VP8 software fallback: {error:#}");
                None
            }
        }
    } else {
        None
    };
    if nvenc_encoder.is_none() && !offers_vp8 {
        bail!("NVENC unavailable and remote offer has no VP8 software-fallback codec");
    }
    if nvenc_encoder.is_none() {
        log::info!("Using libwebrtc VP8 software fallback for this peer");
    }

    let session = Arc::new(if nvenc_encoder.is_some() {
        GshSession::new_with_video_input(
            PeerConnectionFactory::builder().build()?,
            RtcConfiguration::default(),
            SessionRole::AnswererPublisher,
            OUTPUT_WIDTH,
            OUTPUT_HEIGHT,
            VideoInputMode::PreEncoded,
            &[VideoCodec::H264],
        )?
    } else {
        GshSession::new_raw_video(
            PeerConnectionFactory::builder().build()?,
            RtcConfiguration::default(),
            SessionRole::AnswererPublisher,
            width,
            height,
        )?
    });
    let answer = session.accept_offer(libgsh::webrtc::SdpMessage {
        kind: libgsh::webrtc::DescriptionKind::Offer,
        sdp: offer.sdp,
    })?;
    for candidate in offer.candidates {
        session.add_ice_candidate(candidate)?;
    }
    send_signal(
        socket_tx,
        json!({
            "type": "answer",
            "sdp": answer.sdp
        }),
    )
    .await?;

    let (mut nvenc_input, mut encoded_frames, mut nvenc_process) = match nvenc_encoder.take() {
        Some(encoder) => {
            let (input, output, process) = encoder.into_parts();
            (Some(input), Some(output), Some(process))
        }
        None => (None, None, None),
    };
    let mut pending_encoded_frame: Option<EncodedVideoFrame> = None;
    let mut input_router = GshInputRouter::default();
    let mut current_frame = offer.current_frame;
    let mut pacer = FramePacer::new(60)?;
    let mut connection_poll = tokio::time::interval(Duration::from_millis(100));
    connection_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let negotiation_deadline = tokio::time::sleep(OFFER_TIMEOUT);
    tokio::pin!(negotiation_deadline);
    let mut candidate_count = offer.candidate_count;
    let mut was_connected = false;
    let mut frame_number = 0_u32;
    let mut last_nvenc_writer_report = 0_u64;

    let result: Result<()> = async {
        loop {
            let encoded_access_unit = async {
                match encoded_frames.as_mut() {
                    Some(receiver) => Some(receiver.recv().await),
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                incoming = socket_rx.next() => {
                    let Some(incoming) = incoming else { break; };
                    match incoming? {
                        Message::Text(text) => {
                            let value: Value = serde_json::from_str(text.as_str())?;
                            match value.get("type").and_then(Value::as_str) {
                                Some("offer") => {
                                    bail!("renegotiation offers are not supported by this session");
                                }
                                Some("candidate") => {
                                    let candidate = parse_candidate(&value)?;
                                    candidate_count += 1;
                                    if candidate_count > MAX_ICE_CANDIDATES_PER_SESSION {
                                        bail!("too many ICE candidates in one session");
                                    }
                                    session.add_ice_candidate(candidate)?;
                                }
                                _ => {}
                            }
                        }
                        Message::Close(_) => break,
                        Message::Ping(payload) => socket_tx.send(Message::Pong(payload)).await?,
                        Message::Binary(_) | Message::Pong(_) => {}
                    }
                }
                candidate = session.next_ice_candidate() => {
                    if let Some(candidate) = candidate? {
                        send_signal(socket_tx, json!({
                            "type": "candidate",
                            "candidate": {
                                "candidate": candidate.candidate,
                                "sdpMid": candidate.sdp_mid,
                                "sdpMLineIndex": candidate.sdp_mline_index
                            }
                        })).await?;
                    }
                }
                control = session.recv_control() => {
                    match control {
                        Ok(bytes) => match input_router.handle_control(&bytes) {
                            Ok(events) => dispatch_input(&application, events, width, height),
                            Err(error) => log::warn!("Rejected GSH control input: {error}"),
                        },
                        Err(libgsh::webrtc::SessionError::ControlQueueOverflow(dropped)) => {
                            log::warn!("Dropped {dropped} GSH control message(s)");
                        }
                        Err(libgsh::webrtc::SessionError::QueueClosed) => break,
                        Err(error) => log::warn!("GSH control channel error: {error}"),
                    }
                }
                input = session.recv_input() => {
                    match input {
                        Ok(input) => application.handle_input(&input),
                        Err(libgsh::webrtc::SessionError::InputQueueOverflow(dropped)) => {
                            log::warn!("Dropped {dropped} native GSH input message(s)");
                        }
                        Err(libgsh::webrtc::SessionError::QueueClosed) => break,
                        Err(error) => log::warn!("Native GSH input channel error: {error}"),
                    }
                }
                realtime = session.recv_realtime() => {
                    match realtime {
                        Ok(bytes) => match input_router.handle_realtime(&bytes) {
                            Ok(events) => dispatch_input(&application, events, width, height),
                            Err(error) => log::warn!("Rejected GSH realtime input: {error}"),
                        },
                        Err(libgsh::webrtc::SessionError::QueueClosed) => break,
                        Err(error) => log::warn!("GSH realtime channel error: {error}"),
                    }
                }
                captured = captured_frames.next() => {
                    let Some(captured) = captured else { break; };
                    if (captured.width, captured.height) != (width, height) {
                        log::warn!("Capture dimensions changed; ending fixed-size WebRTC session");
                        break;
                    }
                    current_frame = captured;
                }
                access_unit = encoded_access_unit, if nvenc_input.is_some() && pending_encoded_frame.is_none() => {
                    let Some(access_unit) = access_unit else {
                        bail!("NVENC output channel closed");
                    };
                    let access_unit = access_unit.ok_or_else(|| anyhow!("NVENC encoder stopped"))?;
                    let frame = EncodedVideoFrame {
                        data: access_unit.data,
                        codec: VideoCodec::H264,
                        is_key_frame: access_unit.is_key_frame,
                        width: OUTPUT_WIDTH,
                        height: OUTPUT_HEIGHT,
                        rtp_timestamp: access_unit
                            .captured_at
                            .ok_or_else(|| anyhow!("NVENC output missing capture timestamp"))?
                            .saturating_duration_since(rtp_timestamp_origin)
                            .as_nanos()
                            .saturating_mul(90_000)
                            .checked_div(1_000_000_000)
                            .unwrap_or_default() as u32,
                    };
                    match session.send_video(frame) {
                        Ok(()) => frame_number = frame_number.wrapping_add(1),
                        Err(error) if matches!(error.error, libgsh::webrtc::SessionError::VideoBackpressure) => {
                            pending_encoded_frame = error.frame;
                        }
                        Err(error) => bail!("H.264 video submission failed: {}", error.error),
                    }
                }
                tick = pacer.wait_next() => {
                    if session.is_connected() {
                        let mut capture_slot_open = true;
                        if let Some(frame) = pending_encoded_frame.take() {
                            match session.send_video(frame) {
                                Ok(()) => frame_number = frame_number.wrapping_add(1),
                                Err(error) if matches!(error.error, libgsh::webrtc::SessionError::VideoBackpressure) => {
                                    pending_encoded_frame = error.frame;
                                    capture_slot_open = false;
                                }
                                Err(error) => bail!("H.264 video submission failed: {}", error.error),
                            }
                        }
                        if capture_slot_open {
                            if let Some(input) = nvenc_input.as_ref() {
                                if current_frame.sequence != last_nvenc_sequence {
                                    input.submit_latest(current_frame.clone());
                                    last_nvenc_sequence = current_frame.sequence;
                                    let written = input.frames_written();
                                    if written.saturating_sub(last_nvenc_writer_report)
                                        >= u64::from(OUTPUT_FPS)
                                    {
                                        log::debug!("NVENC submitted {written} raw capture frames");
                                        last_nvenc_writer_report = written;
                                    }
                                }
                            } else if session.negotiated_video_codec() == Some(VideoCodec::Vp8) {
                                let bgra = rgba_to_bgra(&current_frame.rgba)?;
                                match session.send_raw_video(&bgra) {
                                    Ok(()) => frame_number = frame_number.wrapping_add(1),
                                    Err(libgsh::webrtc::SessionError::VideoBackpressure) => {
                                        log::trace!("Raw VP8 frame skipped under encoder backpressure");
                                    }
                                    Err(error) => bail!("Raw video submission failed: {error}"),
                                }
                            }
                        }
                    }
                    if tick.skipped > 0 {
                        log::debug!("Frame pacer skipped {} deadline(s)", tick.skipped);
                    }
                }
                _ = connection_poll.tick() => {
                    if session.is_connected() {
                        was_connected = true;
                    } else if was_connected {
                        break;
                    }
                }
                _ = &mut negotiation_deadline, if !session.is_connected() => {
                    bail!("timed out waiting for direct WebRTC ICE connection");
                }
                feedback = session.recv_encoder_feedback(), if session.encoder_feedback_available() => {
                    match feedback? {
                        libgsh::webrtc::VideoEncoderFeedback::RequestKeyFrame => {
                            if nvenc_input.is_some() {
                                log::warn!("Receiver requested an H.264 keyframe; CLI NVENC cannot force IDR immediately; next periodic IDR is due within about {:.3}s", GOP_FRAMES as f32 / OUTPUT_FPS as f32);
                            }
                        }
                        libgsh::webrtc::VideoEncoderFeedback::RateUpdate { bitrate_bps, framerate_fps } => {
                            log::info!("Encoder feedback {bitrate_bps} bps @ {framerate_fps:.1} fps; NVENC target remains fixed");
                        }
                    }
                }
            }
        }
        Ok(())
    }
    .await;

    for event in input_router.release_all() {
        dispatch_input(&application, vec![event], width, height);
    }
    result?;
    encoded_frames.as_mut().map(|receiver| receiver.close());
    nvenc_input.take();
    if let Some(process) = nvenc_process.take() {
        process.shutdown().await?;
    }
    log::debug!("WebRTC remote-desktop session ended after {frame_number} frame submissions");
    Ok(())
}

async fn receive_gsh_message(
    socket: &mut futures_util::stream::SplitStream<WebSocket>,
    name: &str,
    deadline: Instant,
) -> Result<ClientMessage> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            bail!("timed out waiting for {name}");
        }
        let incoming = tokio::time::timeout(remaining, socket.next())
            .await
            .with_context(|| format!("timed out waiting for {name}"))?;
        match incoming {
            Some(Ok(Message::Binary(bytes))) => {
                return Ok(<ClientMessage as ProstMessage>::decode(bytes)?);
            }
            Some(Ok(Message::Close(_))) | None => {
                bail!("WebSocket closed during GSH authentication")
            }
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            Some(Ok(Message::Text(_))) => {
                bail!("GSH authentication messages must be protobuf binary frames")
            }
            Some(Err(error)) => return Err(error.into()),
        }
    }
}

async fn send_gsh_message_before(
    socket: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    message: ServerMessage,
    deadline: Instant,
) -> Result<()> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        bail!("GSH authentication deadline expired before response");
    }
    tokio::time::timeout(remaining, send_gsh_message(socket, message))
        .await
        .context("GSH authentication response timed out")??;
    Ok(())
}

async fn send_gsh_message(
    socket: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    message: ServerMessage,
) -> Result<()> {
    socket
        .send(Message::Binary(
            ProstMessage::encode_to_vec(&message).into(),
        ))
        .await?;
    Ok(())
}

async fn send_signal(
    socket: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    value: Value,
) -> Result<()> {
    socket.send(Message::Text(value.to_string().into())).await?;
    Ok(())
}

fn parse_candidate(value: &Value) -> Result<CandidateMessage> {
    let candidate = value
        .get("candidate")
        .ok_or_else(|| anyhow!("candidate message has no candidate object"))?;
    let candidate_text = candidate
        .get("candidate")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("candidate has no SDP string"))?
        .to_owned();
    if candidate_text.len() > MAX_ICE_CANDIDATE_BYTES {
        bail!("ICE candidate exceeds {} bytes", MAX_ICE_CANDIDATE_BYTES);
    }
    let sdp_mid = candidate
        .get("sdpMid")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    if sdp_mid
        .as_ref()
        .is_some_and(|mid| mid.len() > MAX_SDP_MID_BYTES)
    {
        bail!("ICE sdpMid exceeds {MAX_SDP_MID_BYTES} bytes");
    }
    Ok(CandidateMessage {
        candidate: candidate_text,
        sdp_mid,
        sdp_mline_index: candidate
            .get("sdpMLineIndex")
            .and_then(Value::as_u64)
            .and_then(|index| u16::try_from(index).ok()),
    })
}

fn sdp_offers_h264(sdp: &str) -> bool {
    sdp_offers_codec(sdp, "h264/")
}

fn sdp_offers_vp8(sdp: &str) -> bool {
    sdp_offers_codec(sdp, "vp8/")
}

fn sdp_offers_codec(sdp: &str, codec_name: &str) -> bool {
    let mut video_payloads = HashSet::new();
    let mut matching_payloads = HashSet::new();
    for line in sdp.lines().map(str::trim) {
        if let Some(media) = line.strip_prefix("m=video ") {
            video_payloads.extend(media.split_whitespace().skip(2));
        } else if let Some(mapping) = line.strip_prefix("a=rtpmap:") {
            let Some((payload, codec)) = mapping.split_once(char::is_whitespace) else {
                continue;
            };
            if codec.to_ascii_lowercase().starts_with(codec_name) {
                matching_payloads.insert(payload);
            }
        }
    }
    video_payloads
        .iter()
        .any(|payload| matching_payloads.contains(payload))
}

fn validate_offer_sdp(sdp: &str) -> Result<()> {
    if sdp.len() > MAX_SDP_BYTES {
        bail!("SDP offer exceeds {MAX_SDP_BYTES} bytes");
    }
    let mut count = 0_usize;
    let mut total_candidate_bytes = 0_usize;
    for line in sdp.lines().filter(|line| line.starts_with("a=candidate:")) {
        count += 1;
        if count > MAX_SDP_CANDIDATES {
            bail!("SDP offer contains too many ICE candidates");
        }
        if line.len() > MAX_SDP_CANDIDATE_BYTES {
            bail!("SDP ICE candidate exceeds {MAX_SDP_CANDIDATE_BYTES} bytes");
        }
        total_candidate_bytes = total_candidate_bytes
            .checked_add(line.len())
            .ok_or_else(|| anyhow!("SDP candidate size overflow"))?;
        if total_candidate_bytes > MAX_TOTAL_SDP_CANDIDATE_BYTES {
            bail!("SDP ICE candidates exceed {MAX_TOTAL_SDP_CANDIDATE_BYTES} bytes in total");
        }
    }
    Ok(())
}

fn dispatch_input(
    application: &RemoteDesktopApplication,
    events: Vec<GshInputEvent>,
    width: u32,
    height: u32,
) {
    for event in events {
        if let Some(input) = into_gsh_input(event, width, height) {
            application.handle_input(&input);
        }
    }
}

fn into_gsh_input(event: GshInputEvent, width: u32, height: u32) -> Option<UserInput> {
    let (kind, input_event) = match event {
        GshInputEvent::Key { code, down } => {
            let key_code = browser_key_code(&code)?;
            (
                InputType::KeyEvent,
                user_input::InputEvent::KeyEvent(user_input::KeyEvent {
                    action: if down {
                        KeyAction::Press
                    } else {
                        KeyAction::Release
                    } as i32,
                    key_code,
                    modifiers: 0,
                }),
            )
        }
        GshInputEvent::Button { button, down, x, y } => {
            let button = match button {
                0 => MouseButton::Left,
                1 => MouseButton::Middle,
                2 => MouseButton::Right,
                _ => return None,
            };
            (
                InputType::MouseEvent,
                user_input::InputEvent::MouseEvent(user_input::MouseEvent {
                    action: if down {
                        MouseAction::Press
                    } else {
                        MouseAction::Release
                    } as i32,
                    x: normalized_to_pixels(x, width),
                    y: normalized_to_pixels(y, height),
                    button: button as i32,
                    delta_x: 0.0,
                    delta_y: 0.0,
                    relative: false,
                }),
            )
        }
        GshInputEvent::Pointer { x, y } => (
            InputType::MouseEvent,
            user_input::InputEvent::MouseEvent(user_input::MouseEvent {
                action: MouseAction::Move as i32,
                x: normalized_to_pixels(x, width),
                y: normalized_to_pixels(y, height),
                button: MouseButton::Left as i32,
                delta_x: 0.0,
                delta_y: 0.0,
                relative: false,
            }),
        ),
        GshInputEvent::RelativeMotion { dx, dy } => (
            InputType::MouseEvent,
            user_input::InputEvent::MouseEvent(user_input::MouseEvent {
                action: MouseAction::Move as i32,
                x: 0,
                y: 0,
                button: MouseButton::Left as i32,
                delta_x: dx as f32,
                delta_y: dy as f32,
                relative: true,
            }),
        ),
        GshInputEvent::Scroll { dx, dy, mode } => {
            let (dx, dy) = match mode {
                0 => (dx, dy),
                1 => (dx * CSS_LINE_HEIGHT_PX, dy * CSS_LINE_HEIGHT_PX),
                2 => (dx * width as f64, dy * height as f64),
                _ => return None,
            };
            (
                InputType::MouseEvent,
                user_input::InputEvent::MouseEvent(user_input::MouseEvent {
                    action: MouseAction::Scroll as i32,
                    x: 0,
                    y: 0,
                    button: MouseButton::Left as i32,
                    delta_x: dx as f32,
                    delta_y: dy as f32,
                    relative: false,
                }),
            )
        }
        GshInputEvent::ReleaseAll => return None,
    };
    Some(UserInput {
        kind: kind as i32,
        window_id: 0,
        input_event: Some(input_event),
    })
}

fn normalized_to_pixels(position: f32, extent: u32) -> i32 {
    if extent == 0 {
        return 0;
    }
    (position.clamp(0.0, 1.0) * (extent - 1) as f32).round() as i32
}

fn rgba_to_bgra(rgba: &[u8]) -> Result<Vec<u8>> {
    if rgba.len() % 4 != 0 {
        bail!("captured RGBA frame has incomplete pixel data");
    }
    let mut bgra = rgba.to_vec();
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    Ok(bgra)
}

fn browser_key_code(code: &str) -> Option<i32> {
    if let Some(letter) = code.strip_prefix("Key").filter(|s| s.len() == 1) {
        return Some(letter.as_bytes()[0].to_ascii_lowercase() as i32);
    }
    if let Some(digit) = code.strip_prefix("Digit").filter(|s| s.len() == 1) {
        return Some(digit.as_bytes()[0] as i32);
    }
    Some(match code {
        "Enter" => 13,
        "Escape" => 27,
        "Backspace" => 8,
        "Tab" => 9,
        "Space" => 32,
        "ArrowLeft" => 1_073_741_904,
        "ArrowRight" => 1_073_741_903,
        "ArrowUp" => 1_073_741_906,
        "ArrowDown" => 1_073_741_905,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::RemoteDesktopAuth;
    use libgsh::shared::protocol::{
        client_auth, client_hello::Os, server_auth_ack::AuthStatus, server_hello_ack::AuthMethod,
        server_message::ServerEvent, ClientAuth, ClientHello,
    };
    use libgsh::{
        rsa::{
            pkcs1::{EncodeRsaPublicKey, LineEnding},
            pkcs1v15::SigningKey,
            rand_core::OsRng,
            signature::{SignatureEncoding, Signer},
            RsaPrivateKey, RsaPublicKey,
        },
        sha2::Sha256,
    };
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::Message as ClientWsMessage;
    use xcap::Frame;

    #[test]
    fn rgba_capture_is_converted_to_webrtc_bgra_order() {
        assert_eq!(
            rgba_to_bgra(&[10, 20, 30, 40, 50, 60, 70, 80]).unwrap(),
            [30, 20, 10, 40, 70, 60, 50, 80]
        );
        assert!(rgba_to_bgra(&[1, 2, 3]).is_err());
    }

    #[test]
    fn browser_origin_must_match_configured_https_origin_exactly() {
        let origins = HashSet::from(["https://desktop.example.test".to_string()]);
        assert!(is_allowed_origin(
            &origins,
            Some("https://desktop.example.test")
        ));
        assert!(!is_allowed_origin(&origins, None));
        assert!(!is_allowed_origin(
            &origins,
            Some("https://attacker.example.test")
        ));
    }

    #[test]
    fn candidate_size_is_checked_before_queuing() {
        let value = json!({
            "candidate": {
                "candidate": "x".repeat(MAX_ICE_CANDIDATE_BYTES + 1),
                "sdpMid": "0",
                "sdpMLineIndex": 0
            }
        });
        assert!(parse_candidate(&value).is_err());

        let value = json!({
            "candidate": {
                "candidate": "candidate:1",
                "sdpMid": "x".repeat(MAX_SDP_MID_BYTES + 1),
                "sdpMLineIndex": 0
            }
        });
        assert!(parse_candidate(&value).is_err());
    }

    #[test]
    fn embedded_sdp_candidates_share_signaled_candidate_limits() {
        assert!(validate_offer_sdp("v=0\r\na=candidate:valid\r\n").is_ok());
        assert!(validate_offer_sdp(&format!(
            "a=candidate:{}",
            "x".repeat(MAX_SDP_CANDIDATE_BYTES)
        ))
        .is_err());
        let many_candidates = (0..=MAX_SDP_CANDIDATES)
            .map(|_| "a=candidate:x\r\n")
            .collect::<String>();
        assert!(validate_offer_sdp(&many_candidates).is_err());
    }

    #[test]
    fn h264_selection_requires_payload_on_video_media_line() {
        let sdp = "m=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=rtpmap:111 opus/48000/2\r\nm=video 9 UDP/TLS/RTP/SAVPF 96 102\r\na=rtpmap:96 VP8/90000\r\na=rtpmap:102 H264/90000\r\n";
        assert!(sdp_offers_h264(sdp));
        assert!(sdp_offers_vp8(sdp));
        assert!(!sdp_offers_h264(
            "m=video 9 UDP/TLS/RTP/SAVPF 96\r\na=rtpmap:96 VP8/90000\r\na=rtpmap:102 H264/90000\r\n"
        ));
        let h264_only = "m=video 9 UDP/TLS/RTP/SAVPF 102\r\na=rtpmap:102 H264/90000\r\n";
        assert!(sdp_offers_h264(h264_only));
        assert!(!sdp_offers_vp8(h264_only));
    }

    #[test]
    fn auth_attempt_limit_reserves_concurrent_guesses_per_peer_ip() {
        let limiter = Arc::new(AuthRateLimiter::default());
        let attacker = "192.0.2.10".parse().unwrap();
        let other = "192.0.2.11".parse().unwrap();
        for _ in 0..MAX_AUTH_FAILURES_PER_MINUTE - 1 {
            limiter.reserve(attacker).unwrap().fail();
        }
        drop(limiter.reserve(attacker).unwrap());
        assert!(limiter.reserve(attacker).is_none());
        limiter.reserve(other).unwrap().succeed();

        let mut in_flight = (0..MAX_AUTH_FAILURES_PER_MINUTE)
            .map(|_| limiter.reserve(other).unwrap())
            .collect::<Vec<_>>();
        assert!(limiter.reserve(other).is_none());
        in_flight.pop().unwrap().succeed();
        assert!(limiter.reserve(other).is_some());
        drop(in_flight);
    }

    #[test]
    fn auth_limiter_ip_state_has_a_fixed_cap() {
        let limiter = Arc::new(AuthRateLimiter::default());
        for index in 0..=MAX_AUTH_RATE_LIMITER_IPS {
            let ip = IpAddr::V4(std::net::Ipv4Addr::from(0x0a00_0000 + index as u32));
            limiter.reserve(ip).unwrap().fail();
        }
        assert_eq!(
            limiter
                .windows
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len(),
            MAX_AUTH_RATE_LIMITER_IPS
        );
    }

    #[test]
    fn successful_auth_does_not_erase_recent_failed_attempts() {
        let limiter = Arc::new(AuthRateLimiter::default());
        let ip = "192.0.2.20".parse().unwrap();
        for _ in 0..MAX_AUTH_FAILURES_PER_MINUTE - 1 {
            limiter.reserve(ip).unwrap().fail();
        }
        limiter.reserve(ip).unwrap().succeed();
        limiter.reserve(ip).unwrap().fail();
        assert!(limiter.reserve(ip).is_none());
    }

    async fn authenticate_over_websocket(
        server_auth: RemoteDesktopAuth,
        make_client_auth: impl FnOnce(Option<&AuthMethod>) -> ClientAuth,
    ) -> (i32, String) {
        let (frame_tx, frame_rx) = std::sync::mpsc::channel();
        frame_tx
            .send(Frame::new(320, 240, vec![0; 320 * 240 * 4]))
            .unwrap();
        let (frame_hub, forwarder) = crate::frame_source::FrameHub::from_xcap(frame_rx);
        let application = RemoteDesktopApplication::new(frame_hub, server_auth, 320, 240, false);
        let (certified_key, private_key) =
            libgsh::shared::cert::self_signed(&["127.0.0.1"]).unwrap();
        let certificate = certified_key.cert.der().clone();
        let _ = libgsh::tokio_rustls::rustls::crypto::ring::default_provider().install_default();
        let server_config = Arc::new(
            libgsh::tokio_rustls::rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(vec![certificate.clone()], private_key)
                .unwrap(),
        );
        let mut roots = libgsh::tokio_rustls::rustls::RootCertStore::empty();
        roots.add(certificate).unwrap();
        let client_config = Arc::new(
            libgsh::tokio_rustls::rustls::ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth(),
        );
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let handle = axum_server::Handle::new();
        let server = axum_server::from_tcp_rustls(
            listener,
            axum_server::tls_rustls::RustlsConfig::from_config(server_config),
        )
        .unwrap()
        .handle(handle.clone())
        .serve(
            router(
                application,
                vec![format!("https://127.0.0.1:{}", address.port())],
            )
            .into_make_service_with_connect_info::<SocketAddr>(),
        );
        let server_task = tokio::spawn(server);
        let mut request = format!("wss://127.0.0.1:{}/signal", address.port())
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            axum::http::header::ORIGIN,
            HeaderValue::from_str(&format!("https://127.0.0.1:{}", address.port())).unwrap(),
        );
        let (mut client, _) = tokio_tungstenite::connect_async_tls_with_config(
            request,
            None,
            false,
            Some(tokio_tungstenite::Connector::Rustls(client_config)),
        )
        .await
        .unwrap();

        let hello = ClientMessage {
            client_event: Some(ClientEvent::ClientHello(ClientHello {
                protocol_version: PROTOCOL_VERSION,
                os: Os::Web as i32,
                os_version: "test browser".into(),
                monitors: Vec::new(),
            })),
        };
        client
            .send(ClientWsMessage::Binary(
                ProstMessage::encode_to_vec(&hello).into(),
            ))
            .await
            .unwrap();
        let Some(Ok(ClientWsMessage::Binary(bytes))) = client.next().await else {
            panic!("expected binary ServerHelloAck");
        };
        let ServerEvent::ServerHelloAck(server_hello) =
            ServerMessage::decode(bytes).unwrap().server_event.unwrap()
        else {
            panic!("expected ServerHelloAck event");
        };
        let auth = ClientMessage {
            client_event: Some(ClientEvent::ClientAuth(make_client_auth(
                server_hello.auth_method.as_ref(),
            ))),
        };
        client
            .send(ClientWsMessage::Binary(
                ProstMessage::encode_to_vec(&auth).into(),
            ))
            .await
            .unwrap();
        let Some(Ok(ClientWsMessage::Binary(bytes))) = client.next().await else {
            panic!("expected binary ServerAuthAck");
        };
        let ServerEvent::ServerAuthAck(ack) =
            ServerMessage::decode(bytes).unwrap().server_event.unwrap()
        else {
            panic!("expected ServerAuthAck event");
        };
        let result = (ack.status, ack.message);

        let _ = client.close(None).await;
        handle.graceful_shutdown(Some(Duration::from_secs(1)));
        server_task.await.unwrap().unwrap();
        drop(frame_tx);
        forwarder.stop();
        forwarder.join().await.unwrap();
        result
    }

    #[tokio::test]
    async fn wss_gsh_handshake_authenticates_before_webrtc_signaling() {
        let (status, _) =
            authenticate_over_websocket(RemoteDesktopAuth::password("expected"), |_| ClientAuth {
                auth_data: Some(client_auth::AuthData::Password(client_auth::Password {
                    password: "expected".into(),
                })),
            })
            .await;
        assert_eq!(status, AuthStatus::Success as i32);
    }

    #[tokio::test]
    async fn wss_gsh_handshake_rejects_bad_password_before_webrtc_signaling() {
        let (status, message) =
            authenticate_over_websocket(RemoteDesktopAuth::password("expected"), |_| ClientAuth {
                auth_data: Some(client_auth::AuthData::Password(client_auth::Password {
                    password: "wrong".into(),
                })),
            })
            .await;
        assert_eq!(status, AuthStatus::Failure as i32);
        assert_eq!(message, "invalid password");
    }

    #[tokio::test]
    async fn wss_gsh_handshake_accepts_rsa_identity_signature() {
        let private_key = RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
        let public_key = RsaPublicKey::from(&private_key);
        let server_auth = RemoteDesktopAuth::authorized_keys(vec![public_key.clone()]);
        let (status, _) = authenticate_over_websocket(server_auth, move |method| {
            let Some(AuthMethod::Signature(method)) = method else {
                panic!("server did not advertise signature authentication");
            };
            let signature = SigningKey::<Sha256>::new(private_key).sign(&method.sign_message);
            let public_key = public_key.to_pkcs1_pem(LineEnding::LF).unwrap();
            ClientAuth {
                auth_data: Some(client_auth::AuthData::Signature(client_auth::Signature {
                    signature: signature.to_bytes().to_vec(),
                    public_key: public_key.as_bytes().to_vec(),
                })),
            }
        })
        .await;
        assert_eq!(status, AuthStatus::Success as i32);
    }
}
