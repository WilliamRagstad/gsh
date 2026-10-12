use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use futures_util::{SinkExt, StreamExt};
use libgsh::{
    pacing::FramePacer,
    shared::protocol::{
        user_input::{
            self, key_event::KeyAction, mouse_event::MouseAction, mouse_event::MouseButton,
            InputType,
        },
        UserInput,
    },
    webrtc::{CandidateMessage, GshSession, SessionRole, VideoCodec},
    webrtc_input::{GshInputEvent, GshInputRouter},
};
use reactor_webrtc::{PeerConnectionFactory, RtcConfiguration};
use serde_json::{json, Value};
use std::{
    collections::{HashSet, VecDeque},
    net::SocketAddr,
    sync::Arc,
    time::Duration,
};
use tokio::{net::TcpListener, sync::mpsc};

const WIDTH: u32 = 320;
const HEIGHT: u32 = 240;
const CSS_LINE_HEIGHT_PX: f64 = 16.0;
const SIGNAL_QUEUE_CAPACITY: usize = 64;

#[cfg(test)]
// Retained as a known-valid fixture for the encoded-frame path.
const VP8_KEYFRAME: &[u8] = &[
    0x30, 0x12, 0x00, 0x9d, 0x01, 0x2a, 0x40, 0x01, 0xf0, 0x00, 0x00, 0x47, 0x08, 0x85, 0x85, 0x88,
    0x99, 0x84, 0x88, 0x02, 0x02, 0x00, 0x06, 0x16, 0x04, 0xf7, 0x06, 0x81, 0x64, 0x9f, 0x6b, 0xdb,
    0x9b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b,
    0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27,
    0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38,
    0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b,
    0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27,
    0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38,
    0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b,
    0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27,
    0x38, 0x7b, 0x27, 0x38, 0x7b, 0x7a, 0xf4, 0x00, 0xfe, 0xff, 0xab, 0x50, 0x80,
];

struct DemoScene {
    cursor_x: f32,
    cursor_y: f32,
    color_phase: u8,
    buttons_down: HashSet<i32>,
    keys_down: HashSet<i32>,
}

impl DemoScene {
    fn apply_input(&mut self, input: UserInput) {
        match input.input_event {
            Some(user_input::InputEvent::MouseEvent(mouse)) => {
                match MouseAction::try_from(mouse.action).ok() {
                    Some(MouseAction::Move) if mouse.relative => {
                        self.cursor_x += mouse.delta_x;
                        self.cursor_y += mouse.delta_y;
                    }
                    Some(MouseAction::Move | MouseAction::Press | MouseAction::Release) => {
                        self.cursor_x = mouse.x as f32;
                        self.cursor_y = mouse.y as f32;
                    }
                    Some(MouseAction::Scroll) => {
                        self.color_phase = self
                            .color_phase
                            .wrapping_add(mouse.delta_y.round() as i8 as u8);
                    }
                    Some(MouseAction::Enter | MouseAction::Exit) => {}
                    None => return,
                }
                match MouseAction::try_from(mouse.action).ok() {
                    Some(MouseAction::Press) => {
                        self.buttons_down.insert(mouse.button);
                    }
                    Some(MouseAction::Release) => {
                        self.buttons_down.remove(&mouse.button);
                    }
                    _ => {}
                }
            }
            Some(user_input::InputEvent::KeyEvent(key)) => {
                if KeyAction::try_from(key.action).ok() == Some(KeyAction::Press) {
                    self.keys_down.insert(key.key_code);
                    self.color_phase = self.color_phase.wrapping_add(7);
                } else {
                    self.keys_down.remove(&key.key_code);
                }
            }
            _ => {}
        }
        self.cursor_x = self.cursor_x.clamp(0.0, WIDTH.saturating_sub(1) as f32);
        self.cursor_y = self.cursor_y.clamp(0.0, HEIGHT.saturating_sub(1) as f32);
    }

    fn render_bgra(&self, frame_number: u32, pixels: &mut [u8]) {
        debug_assert_eq!(pixels.len(), WIDTH as usize * HEIGHT as usize * 4);
        let pressed = !self.buttons_down.is_empty() || !self.keys_down.is_empty();
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let offset = ((y * WIDTH + x) * 4) as usize;
                let band = ((x / 32 + y / 24 + frame_number / 3) % 2) as u8;
                pixels[offset] = 40_u8.wrapping_add(self.color_phase).wrapping_add(band * 24);
                pixels[offset + 1] = 70_u8.wrapping_add(band * 30);
                pixels[offset + 2] = 120_u8.wrapping_add(frame_number as u8 / 2);
                pixels[offset + 3] = 255;
            }
        }

        let cursor_x = self.cursor_x.round() as i32;
        let cursor_y = self.cursor_y.round() as i32;
        let radius_sq = if pressed { 100 } else { 49 };
        for y in (cursor_y - 10).max(0)..=(cursor_y + 10).min(HEIGHT as i32 - 1) {
            for x in (cursor_x - 10).max(0)..=(cursor_x + 10).min(WIDTH as i32 - 1) {
                if (x - cursor_x).pow(2) + (y - cursor_y).pow(2) <= radius_sq {
                    let offset = ((y as u32 * WIDTH + x as u32) * 4) as usize;
                    pixels[offset..offset + 4].copy_from_slice(&[32, 230, 250, 255]);
                }
            }
        }
    }
}

impl Default for DemoScene {
    fn default() -> Self {
        Self {
            cursor_x: WIDTH as f32 * 0.5,
            cursor_y: HEIGHT as f32 * 0.5,
            color_phase: 0,
            buttons_down: HashSet::new(),
            keys_down: HashSet::new(),
        }
    }
}

async fn index() -> Html<&'static str> {
    Html(include_str!("../static/index.html"))
}

async fn signal_route(
    State(allow_remote_input): State<bool>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<impl IntoResponse, StatusCode> {
    if headers
        .get(axum::http::header::ORIGIN)
        .and_then(|origin| origin.to_str().ok())
        .is_some_and(|origin| {
            origin != "http://127.0.0.1:8787" && origin != "http://localhost:8787"
        })
    {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(upgrade
        .max_message_size(1024 * 1024)
        .max_frame_size(1024 * 1024)
        .on_upgrade(move |socket| run_session(socket, allow_remote_input)))
}

async fn run_session(socket: WebSocket, allow_remote_input: bool) {
    if let Err(error) = run_session_inner(socket, allow_remote_input).await {
        eprintln!("WebRTC demo session ended: {error}");
    }
}

async fn run_session_inner(socket: WebSocket, allow_remote_input: bool) -> anyhow::Result<()> {
    let factory = PeerConnectionFactory::builder().build()?;
    let session = Arc::new(GshSession::new_raw_video(
        factory,
        RtcConfiguration::default(),
        SessionRole::AnswererPublisher,
        WIDTH,
        HEIGHT,
    )?);

    let (mut socket_tx, mut socket_rx) = socket.split();
    let (signal_tx, mut signal_rx) = mpsc::channel::<Message>(SIGNAL_QUEUE_CAPACITY);
    tokio::spawn(async move {
        while let Some(message) = signal_rx.recv().await {
            if socket_tx.send(message).await.is_err() {
                break;
            }
        }
    });

    send_signal(
        &signal_tx,
        json!({ "type": "status", "message": "Waiting for browser offer" }),
    )
    .await?;

    let mut remote_description_set = false;
    let mut pending_candidates = VecDeque::new();
    let mut pacer = FramePacer::new(30)?;
    let mut connection_poll = tokio::time::interval(Duration::from_millis(100));
    connection_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut frame_number = 0_u32;
    let mut frames_submitted = 0_u64;
    let mut frames_backpressured = 0_u64;
    let mut last_status_frame = 0_u32;
    let mut input_count = 0_u64;
    let mut realtime_count = 0_u64;
    let mut input_router = GshInputRouter::default();
    let mut authorization_notice_sent = false;
    let mut was_connected = false;
    let (app_input_tx, mut app_input_rx) = mpsc::channel::<UserInput>(256);
    let mut demo_scene = DemoScene::default();
    let mut scene_frame = vec![0_u8; WIDTH as usize * HEIGHT as usize * 4];
    let mut app_input_count = 0_u64;

    let session_result: anyhow::Result<()> = async {
        loop {
        tokio::select! {
            _ = connection_poll.tick() => {
                if session.is_connected() {
                    was_connected = true;
                } else if was_connected {
                    eprintln!("WebRTC disconnected; releasing held input state");
                    break;
                }
            }
            app_input = app_input_rx.recv() => {
                if let Some(input) = app_input {
                    demo_scene.apply_input(input);
                    app_input_count += 1;
                    if app_input_count.is_power_of_two() {
                        eprintln!("demo scene applied input #{app_input_count}");
                    }
                }
            }
            incoming = socket_rx.next() => {
                let Some(incoming) = incoming else { break; };
                match incoming? {
                    Message::Text(text) => {
                        let value: Value = serde_json::from_str(text.as_str())?;
                        match value.get("type").and_then(Value::as_str) {
                            Some("offer") => {
                                if remote_description_set {
                                    anyhow::bail!("duplicate remote offer");
                                }
                                let sdp = value.get("sdp").and_then(Value::as_str)
                                    .ok_or_else(|| anyhow::anyhow!("offer has no SDP string"))?;
                                let answer = session.accept_offer(libgsh::webrtc::SdpMessage {
                                    kind: libgsh::webrtc::DescriptionKind::Offer,
                                    sdp: sdp.to_owned(),
                                })?;
                                remote_description_set = true;
                                while let Some(candidate) = pending_candidates.pop_front() {
                                    session.add_ice_candidate(candidate)?;
                                }
                                send_signal(&signal_tx, json!({
                                    "type": "answer",
                                    "sdp": answer.sdp
                                })).await?;
                                send_signal(&signal_tx, json!({
                                    "type": "status",
                                    "message": format!("Answer sent, codec: {:?}", session.negotiated_video_codec())
                                })).await?;
                            }
                            Some("candidate") => {
                                let candidate = parse_candidate(&value)?;
                                if remote_description_set {
                                    session.add_ice_candidate(candidate)?;
                                } else {
                                    if pending_candidates.len() == ICE_QUEUE_LIMIT {
                                        anyhow::bail!("too many candidates before remote SDP");
                                    }
                                    pending_candidates.push_back(candidate);
                                }
                            }
                            other => anyhow::bail!("unexpected signaling message type: {other:?}"),
                        }
                    }
                    Message::Ping(payload) => {
                        signal_tx.send(Message::Pong(payload)).await?;
                    }
                    Message::Close(_) => break,
                    Message::Binary(_) | Message::Pong(_) => {}
                }
            }
            candidate = session.next_ice_candidate() => {
                if let Some(candidate) = candidate? {
                    send_signal(&signal_tx, json!({
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
                if !allow_remote_input {
                    if !authorization_notice_sent {
                        authorization_notice_sent = true;
                        send_signal(&signal_tx, json!({
                            "type": "status",
                            "message": "Remote input disabled; start the local demo with --allow-remote-input"
                        })).await?;
                    }
                    continue;
                }
                match input_router.handle_control(&control?) {
                    Ok(events) => {
                        for input in normalize_events(events) {
                            app_input_tx.try_send(input.clone())
                                .map_err(|_| anyhow::anyhow!("GSH input dispatcher is overloaded"))?;
                                input_count += 1;
                                if input_count == 1 {
                                    send_signal(&signal_tx, json!({
                                        "type": "status",
                                        "message": "First browser control input reached GSH"
                                    })).await?;
                                }
                                if input_count.is_power_of_two() { eprintln!("dispatched input #{input_count}: {input:?}"); }
                        }
                    }
                    Err(error) => eprintln!("rejected control input: {error}"),
                }
            }
            realtime = session.recv_realtime() => {
                if !allow_remote_input {
                    if !authorization_notice_sent {
                        authorization_notice_sent = true;
                        send_signal(&signal_tx, json!({
                            "type": "status",
                            "message": "Remote input disabled; start the local demo with --allow-remote-input"
                        })).await?;
                    }
                    continue;
                }
                match input_router.handle_realtime(&realtime?) {
                    Ok(events) => {
                        for input in normalize_events(events) {
                            app_input_tx.try_send(input.clone())
                                .map_err(|_| anyhow::anyhow!("GSH input dispatcher is overloaded"))?;
                                realtime_count += 1;
                                if realtime_count == 1 {
                                    send_signal(&signal_tx, json!({
                                        "type": "status",
                                        "message": "First browser realtime input reached GSH"
                                    })).await?;
                                }
                                input_count += 1;
                                if input_count.is_power_of_two() { eprintln!("dispatched input #{input_count}: {input:?}"); }
                        }
                    }
                    Err(error) => eprintln!("rejected realtime input: {error}"),
                }
            }
            feedback = session.recv_encoder_feedback(), if session.encoder_feedback_available() => {
                match feedback? {
                    libgsh::webrtc::VideoEncoderFeedback::RequestKeyFrame => {
                        // The raw-video track uses libwebrtc's built-in encoder;
                        // RTCP keyframe recovery is owned by that media pipeline.
                        eprintln!("encoder feedback: keyframe requested by receiver");
                    }
                    libgsh::webrtc::VideoEncoderFeedback::RateUpdate {
                        bitrate_bps,
                        framerate_fps,
                    } => {
                        eprintln!("encoder feedback: {bitrate_bps} bps @ {framerate_fps:.1} fps");
                    }
                }
            }
            tick = pacer.wait_next() => {
                if session.negotiated_video_codec() == Some(VideoCodec::Vp8) {
                    demo_scene.render_bgra(frame_number, &mut scene_frame);
                    let result = session.send_raw_video(&scene_frame);
                    match result {
                        Ok(()) => frames_submitted += 1,
                        Err(libgsh::webrtc::SessionError::VideoBackpressure) => {
                            frames_backpressured += 1;
                        }
                        Err(error) => anyhow::bail!("video session failed: {error}"),
                    }
                    frame_number = frame_number.wrapping_add(1);
                    if frame_number.saturating_sub(last_status_frame) >= 30 {
                        last_status_frame = frame_number;
                        send_signal(&signal_tx, json!({
                            "type": "status",
                            "message": format!(
                                "video submitted={frames_submitted} backpressure={frames_backpressured} pending={}",
                                session.video_frames_pending()
                            )
                        })).await?;
                    }
                }
                if tick.skipped > 0 {
                    eprintln!("frame pacer skipped {} deadline(s)", tick.skipped);
                }
            }
        }
        }
        Ok(())
    }
    .await;

    for event in input_router.release_all() {
        for input in normalize_events(vec![event]) {
            let _ = app_input_tx.try_send(input);
        }
    }
    drop(app_input_tx);
    while let Ok(input) = app_input_rx.try_recv() {
        demo_scene.apply_input(input);
    }

    session_result
}

fn normalize_events(events: Vec<GshInputEvent>) -> Vec<UserInput> {
    events.into_iter().filter_map(into_gsh_input).collect()
}

fn into_gsh_input(event: GshInputEvent) -> Option<UserInput> {
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
                    x: normalized_to_pixels(x, WIDTH),
                    y: normalized_to_pixels(y, HEIGHT),
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
                x: normalized_to_pixels(x, WIDTH),
                y: normalized_to_pixels(y, HEIGHT),
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
            // Browser WheelEvent values are pixels (0), lines (1), or pages (2).
            // Convert the latter two into the demo's logical video-pixel units.
            let (dx, dy) = match mode {
                0 => (dx, dy),
                1 => (dx * CSS_LINE_HEIGHT_PX, dy * CSS_LINE_HEIGHT_PX),
                2 => (dx * WIDTH as f64, dy * HEIGHT as f64),
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
        window_id: 1,
        input_event: Some(input_event),
    })
}

fn normalized_to_pixels(position: f32, extent: u32) -> i32 {
    if extent == 0 {
        return 0;
    }
    (position.clamp(0.0, 1.0) * (extent - 1) as f32).round() as i32
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

const ICE_QUEUE_LIMIT: usize = 256;

async fn send_signal(sender: &mpsc::Sender<Message>, value: Value) -> anyhow::Result<()> {
    sender.send(Message::Text(value.to_string().into())).await?;
    Ok(())
}

fn parse_candidate(value: &Value) -> anyhow::Result<CandidateMessage> {
    let candidate = value
        .get("candidate")
        .filter(|value| value.is_object())
        .unwrap_or(value);
    Ok(CandidateMessage {
        candidate: candidate
            .get("candidate")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("candidate payload has no candidate string"))?
            .to_owned(),
        sdp_mid: candidate
            .get("sdpMid")
            .and_then(Value::as_str)
            .map(str::to_owned),
        sdp_mline_index: candidate
            .get("sdpMLineIndex")
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok()),
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let address = SocketAddr::from(([127, 0, 0, 1], 8787));
    let allow_remote_input = std::env::args().any(|arg| arg == "--allow-remote-input");
    let app = Router::new()
        .route("/", get(index))
        .route("/signal", get(signal_route))
        .with_state(allow_remote_input);
    let listener = TcpListener::bind(address).await?;
    eprintln!("Local-only GSH WebRTC demo: http://{address}/");
    eprintln!("Signaling is unauthenticated and plain WS; keep it on loopback.");
    eprintln!(
        "Remote input authorization: {allow_remote_input} (pass --allow-remote-input to enable)"
    );
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_motion_keeps_delta_semantics() {
        let input = into_gsh_input(GshInputEvent::RelativeMotion { dx: 4, dy: -3 }).unwrap();
        let Some(user_input::InputEvent::MouseEvent(mouse)) = input.input_event else {
            panic!("expected mouse event");
        };
        assert_eq!(mouse.action, MouseAction::Move as i32);
        assert!(mouse.relative);
        assert_eq!((mouse.delta_x, mouse.delta_y), (4.0, -3.0));
    }

    #[test]
    fn normalized_input_changes_demo_scene_state() {
        let mut scene = DemoScene::default();
        scene.apply_input(into_gsh_input(GshInputEvent::Pointer { x: 0.75, y: 0.25 }).unwrap());
        assert_eq!(scene.cursor_x, ((WIDTH - 1) as f32 * 0.75).round());
        assert_eq!(scene.cursor_y, ((HEIGHT - 1) as f32 * 0.25).round());

        let mut frame = vec![0; WIDTH as usize * HEIGHT as usize * 4];
        scene.render_bgra(0, &mut frame);
        let cursor_offset =
            ((scene.cursor_y as usize * WIDTH as usize + scene.cursor_x as usize) * 4) as usize;
        assert_eq!(
            &frame[cursor_offset..cursor_offset + 4],
            &[32, 230, 250, 255]
        );
    }

    #[test]
    fn encoded_frame_fixture_has_vp8_keyframe_header() {
        assert_eq!(&VP8_KEYFRAME[..6], &[0x30, 0x12, 0x00, 0x9d, 0x01, 0x2a]);
    }

    #[test]
    fn scroll_modes_convert_into_logical_video_pixels() {
        let scroll = |dx, dy, mode| {
            let input = into_gsh_input(GshInputEvent::Scroll { dx, dy, mode }).unwrap();
            let Some(user_input::InputEvent::MouseEvent(mouse)) = input.input_event else {
                panic!("expected mouse event");
            };
            (mouse.delta_x, mouse.delta_y)
        };

        assert_eq!(scroll(2.0, -3.0, 0), (2.0, -3.0));
        assert_eq!(scroll(2.0, -3.0, 1), (32.0, -48.0));
        assert_eq!(scroll(1.0, -1.0, 2), (WIDTH as f32, -(HEIGHT as f32)));
    }
}
