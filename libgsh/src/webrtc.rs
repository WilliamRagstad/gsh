//! Concrete v1 WebRTC session backend.
//!
//! Media is carried on native RTP tracks; reliable transitions/control and
//! replaceable realtime state use separate data channels. This module
//! intentionally does not define a generic transport trait. Build with
//! `--features webrtc` to include libwebrtc.

use crate::shared::protocol::UserInput;
use crate::telemetry::{PipelineTelemetry, Stage};
use prost::Message;
use reactor_webrtc::{
    AudioTrack, DataChannel, DataChannelState, EncodedVideoFrame as RtcEncodedVideoFrame,
    EncodedVideoTrack, EncoderFeedback as RtcEncoderFeedback, FrameAction, FrameDirection,
    FrameTransform, IceCandidate, IceCandidateType, LocalVideoTrack as RtcLocalVideoTrack,
    MediaKind, PeerConnection, PeerConnectionFactory, PeerConnectionObserver, PeerConnectionState,
    PreEncodedOptions, RemoteTrack, RtcConfiguration, SdpType, SessionDescription, StreamKind,
    TrackVideoEncoder, TransceiverDirection, VideoCodec as RtcVideoCodec,
    VideoFrame as RtcVideoFrame, VideoTrack as RtcVideoTrack, VideoTrackOptions,
};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    Arc, Mutex,
};
use thiserror::Error;
use tokio::sync::{mpsc, Notify};

const INPUT_CHANNEL: &str = "gsh-input-v1";
const CONTROL_CHANNEL: &str = "gsh-control-v1";
const GSH_CONTROL_CHANNEL: &str = "gsh.control";
const REALTIME_CHANNEL: &str = "gsh.realtime";
const INPUT_QUEUE_CAPACITY: usize = 256;
const CONTROL_QUEUE_CAPACITY: usize = 64;
const ICE_QUEUE_CAPACITY: usize = 256;
const MAX_CONTROL_MESSAGE: usize = 64 * 1024;
const MAX_INPUT_MESSAGE: usize = 4096;
const MAX_REALTIME_MESSAGE: usize = 4096;
const MAX_VIDEO_FRAME_BYTES: usize = 16 * 1024 * 1024;
const MAX_RAW_FRAME_BYTES: usize = 64 * 1024 * 1024;
const MAX_DECODED_FRAME_BYTES: usize = 64 * 1024 * 1024;
const MAX_INPUT_BUFFERED: u64 = 64 * 1024;
const MAX_REALTIME_BUFFERED: u64 = 2048;
const MAX_FRAME_DIMENSION: u32 = 8192;
const MAX_SDP_BYTES: usize = 1024 * 1024;
const MAX_ICE_CANDIDATE_BYTES: usize = 4096;
const MAX_VIDEO_FRAMES_IN_FLIGHT: usize = 2;
const AUDIO_QUEUE_CAPACITY: usize = 64;
const FEEDBACK_QUEUE_CAPACITY: usize = 32;
const MAX_AUDIO_FRAME_SAMPLES: usize = 96_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
    Vp8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoInputMode {
    /// The application owns encoding and pushes compressed H.264/VP8 access units.
    PreEncoded,
    /// The application pushes BGRA frames and libwebrtc selects its built-in encoder.
    RawBgra,
}

/// Owned encoded access unit. The codec must match the one selected during SDP.
#[derive(Debug)]
pub struct EncodedVideoFrame {
    pub data: Vec<u8>,
    pub codec: VideoCodec,
    pub is_key_frame: bool,
    /// Zero inherits the configured track width or height.
    pub width: u32,
    pub height: u32,
    /// RTP timestamp; zero allows libwebrtc to assign one.
    pub rtp_timestamp: u32,
}

#[derive(Debug, Clone)]
pub struct DecodedVideoFrame {
    /// BGRA pixel bytes. Shared ownership lets the receiver retain the latest
    /// frame without introducing another full-frame copy.
    pub bgra: Arc<[u8]>,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone)]
pub struct DecodedAudioFrame {
    /// Interleaved signed 16-bit PCM samples.
    pub pcm: Vec<i16>,
    pub sample_rate: u32,
    pub channels: u32,
    pub frames: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VideoEncoderFeedback {
    RequestKeyFrame,
    RateUpdate {
        bitrate_bps: u32,
        framerate_fps: f64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaStreamKind {
    Audio,
    Video,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct InboundMediaStats {
    pub kind: MediaStreamKind,
    pub codec: Option<String>,
    pub bytes_received: u64,
    pub packets_received: u32,
    pub packets_lost: i32,
    pub nack_count: u32,
    pub pli_count: u32,
    pub frames_per_second: f64,
    pub frames_decoded: u32,
    pub frames_dropped: u32,
    pub frame_width: u32,
    pub frame_height: u32,
    pub jitter_s: f64,
    pub average_jitter_buffer_delay_s: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct OutboundMediaStats {
    pub kind: MediaStreamKind,
    pub codec: Option<String>,
    pub bytes_sent: u64,
    pub packets_sent: u64,
    pub packets_lost: i32,
    pub fraction_lost: f64,
    pub nack_count: u32,
    pub pli_count: u32,
    pub frames_per_second: f64,
    pub frames_encoded: u32,
    pub target_bitrate_bps: f64,
    pub round_trip_time_s: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct SelectedPathStats {
    pub round_trip_time_s: Option<f64>,
    pub available_incoming_bitrate_bps: Option<f64>,
    pub available_outgoing_bitrate_bps: Option<f64>,
    pub is_relay: bool,
}

#[derive(Debug, Clone, Default)]
pub struct GshSessionStats {
    pub inbound: Vec<InboundMediaStats>,
    pub outbound: Vec<OutboundMediaStats>,
    pub selected_path: Option<SelectedPathStats>,
}

/// Submission failure. `frame` is returned for validation/backpressure errors;
/// on a native push failure it is `None` because the upstream call has already
/// consumed it and its acceptance status is ambiguous.
#[derive(Debug, Error)]
#[error("video frame submission failed: {error}")]
pub struct VideoSubmitError {
    pub error: SessionError,
    pub frame: Option<EncodedVideoFrame>,
}

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("WebRTC operation failed: {0}")]
    WebRtc(String),
    #[error("local or remote data channel is not open")]
    ChannelNotOpen,
    #[error("control message exceeds {MAX_CONTROL_MESSAGE} bytes")]
    ControlMessageTooLarge,
    #[error("encoded video frame exceeds {MAX_VIDEO_FRAME_BYTES} bytes")]
    VideoFrameTooLarge,
    #[error("encoded video frame payload is empty")]
    EmptyVideoFrame,
    #[error("video frame dimensions do not match the configured track")]
    VideoDimensionsMismatch,
    #[error("this session's video input mode does not accept this frame type")]
    WrongVideoInputMode,
    #[error("raw BGRA frame size does not match the configured dimensions")]
    InvalidRawVideoFrame,
    #[error("input backlog exceeded its bound; {0} input event(s) were dropped; reconcile remote input state")]
    InputQueueOverflow(u64),
    #[error("control backlog exceeded its bound; {0} control message(s) were dropped")]
    ControlQueueOverflow(u64),
    #[error("audio receive queue exceeded its bound; {0} audio chunk(s) were dropped")]
    AudioQueueOverflow(u64),
    #[error("video encoder feedback queue exceeded its bound; {0} feedback event(s) were dropped")]
    FeedbackQueueOverflow(u64),
    #[error("signaling backlog exceeded its bound; {0} ICE candidate(s) were dropped")]
    IceQueueOverflow(u64),
    #[error("input send is backpressured; retry or coalesce replaceable pointer updates")]
    InputBackpressure,
    #[error("realtime input message exceeds {MAX_REALTIME_MESSAGE} bytes")]
    RealtimeMessageTooLarge,
    #[error("realtime data channel is backpressured; coalesce to the newest state")]
    RealtimeBackpressure,
    #[error("this session role does not publish local media")]
    MediaNotAvailable,
    #[error("SDP did not negotiate a supported video codec (H.264 or VP8)")]
    VideoCodecUnavailable,
    #[error("encoded frame codec does not match the negotiated session codec")]
    VideoCodecMismatch,
    #[error("video packetizer is at its bounded in-flight frame limit")]
    VideoBackpressure,
    #[error("video packetizer failed after accepting frame ownership; recreate the session")]
    VideoPipelineFailed,
    #[error("incoming event queue closed")]
    QueueClosed,
    #[error("session description has an unexpected kind")]
    WrongDescriptionKind,
    #[error("signaling payload exceeds the configured size limit")]
    SignalingPayloadTooLarge,
    #[error("bitrate limits must satisfy min <= start <= max")]
    InvalidBitrateRange,
}

pub type Result<T> = std::result::Result<T, SessionError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRole {
    /// Creates the input and control channels before making an offer.
    Offerer,
    /// Offers receive-only media transceivers and creates GSH data channels.
    OffererSubscriber,
    /// Receives channels created by the offerer.
    Answerer,
    /// Answers the remote offer while publishing local media; data channels
    /// are created by the offerer and arrive via `on_data_channel`.
    AnswererPublisher,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescriptionKind {
    Offer,
    Answer,
}

/// Signaling payload; signaling itself remains an application concern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdpMessage {
    pub kind: DescriptionKind,
    pub sdp: String,
}

/// Serializable candidate value for an HTTP/WebSocket signaling channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateMessage {
    pub candidate: String,
    pub sdp_mid: Option<String>,
    pub sdp_mline_index: Option<u16>,
}

type ChannelSlot = Arc<Mutex<Option<DataChannel>>>;

enum SessionVideoTrack {
    PreEncoded(EncodedVideoTrack),
    Raw(RtcVideoTrack),
}

#[derive(Clone)]
struct ChannelRouter {
    input_channel: ChannelSlot,
    control_channel: ChannelSlot,
    realtime_channel: ChannelSlot,
    input_tx: mpsc::Sender<UserInput>,
    control_tx: mpsc::Sender<Vec<u8>>,
    realtime_messages: Arc<LatestBytesQueue>,
    realtime_coalesced: Arc<AtomicU64>,
    dropped_input: Arc<AtomicU64>,
    dropped_control: Arc<AtomicU64>,
    telemetry: PipelineTelemetry,
}

struct LatestBytesQueue {
    value: Mutex<Option<Vec<u8>>>,
    notify: Notify,
    closed: AtomicBool,
}

impl LatestBytesQueue {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            value: Mutex::new(None),
            notify: Notify::new(),
            closed: AtomicBool::new(false),
        })
    }

    fn publish(&self, value: Vec<u8>) -> bool {
        if self.closed.load(Ordering::Relaxed) {
            return false;
        }
        let replaced = self
            .value
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .replace(value)
            .is_some();
        self.notify.notify_one();
        replaced
    }

    async fn recv(&self) -> Option<Vec<u8>> {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(value) = self
                .value
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
            {
                return Some(value);
            }
            if self.closed.load(Ordering::Relaxed) {
                return None;
            }
            notified.await;
        }
    }

    fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
        self.notify.notify_waiters();
    }
}

/// One-slot latest-frame mailbox. It replaces stale decoded frames and owns at
/// most one full-resolution CPU frame for backpressured consumers.
struct LatestVideoQueue {
    frame: Mutex<Option<Arc<DecodedVideoFrame>>>,
    notify: Notify,
    closed: AtomicBool,
    published_frames: AtomicU64,
}

impl LatestVideoQueue {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            frame: Mutex::new(None),
            notify: Notify::new(),
            closed: AtomicBool::new(false),
            published_frames: AtomicU64::new(0),
        })
    }

    fn publish(&self, frame: DecodedVideoFrame) -> bool {
        if self.closed.load(Ordering::Relaxed) {
            return false;
        }
        let replaced = self
            .frame
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .replace(Arc::new(frame))
            .is_some();
        self.published_frames.fetch_add(1, Ordering::Relaxed);
        self.notify.notify_one();
        replaced
    }

    fn published_frames(&self) -> u64 {
        self.published_frames.load(Ordering::Relaxed)
    }

    async fn recv(&self) -> Option<Arc<DecodedVideoFrame>> {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(frame) = self.try_recv() {
                return Some(frame);
            }
            if self.closed.load(Ordering::Relaxed) {
                return None;
            }
            notified.await;
        }
    }

    fn try_recv(&self) -> Option<Arc<DecodedVideoFrame>> {
        self.frame
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }

    fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
        self.notify.notify_waiters();
    }
}

/// One client session. App-facing methods operate on GSH inputs/control and
/// encoded media, not RTP packets or WebRTC data-channel internals.
pub struct GshSession {
    _factory: PeerConnectionFactory,
    peer: PeerConnection,
    role: SessionRole,
    video: Option<SessionVideoTrack>,
    _audio: Option<AudioTrack>,
    input_channel: ChannelSlot,
    control_channel: ChannelSlot,
    realtime_channel: ChannelSlot,
    input_rx: tokio::sync::Mutex<mpsc::Receiver<UserInput>>,
    control_rx: tokio::sync::Mutex<mpsc::Receiver<Vec<u8>>>,
    realtime_messages: Arc<LatestBytesQueue>,
    realtime_coalesced: Arc<AtomicU64>,
    video_frames: Arc<LatestVideoQueue>,
    audio_rx: tokio::sync::Mutex<mpsc::Receiver<DecodedAudioFrame>>,
    feedback_rx: tokio::sync::Mutex<mpsc::Receiver<VideoEncoderFeedback>>,
    encoder_feedback_available: bool,
    ice_rx: tokio::sync::Mutex<mpsc::Receiver<CandidateMessage>>,
    dropped_input: Arc<AtomicU64>,
    dropped_control: Arc<AtomicU64>,
    dropped_ice: Arc<AtomicU64>,
    connected: Arc<AtomicBool>,
    telemetry: PipelineTelemetry,
    width: u32,
    height: u32,
    negotiated_video_codec: Mutex<Option<VideoCodec>>,
    video_frames_in_flight: Arc<AtomicUsize>,
    _video_sender_transform: Mutex<Option<FrameTransform>>,
    video_failed: AtomicBool,
    dropped_audio: Arc<AtomicU64>,
    decoded_video_callbacks: Arc<AtomicU64>,
    dropped_feedback: Arc<AtomicU64>,
    dropped_video: Arc<AtomicU64>,
    _remote_tracks: Arc<Mutex<Vec<RemoteTrack>>>,
    codec_preferences: Vec<VideoCodec>,
}

impl GshSession {
    pub fn new(
        factory: PeerConnectionFactory,
        config: RtcConfiguration,
        role: SessionRole,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        Self::new_with_video_codecs(
            factory,
            config,
            role,
            width,
            height,
            &[VideoCodec::H264, VideoCodec::Vp8],
        )
    }

    /// Create a session with explicit preferred codecs in fallback order.
    pub fn new_with_video_codecs(
        factory: PeerConnectionFactory,
        config: RtcConfiguration,
        role: SessionRole,
        width: u32,
        height: u32,
        codec_preferences: &[VideoCodec],
    ) -> Result<Self> {
        Self::new_with_video_input(
            factory,
            config,
            role,
            width,
            height,
            VideoInputMode::PreEncoded,
            codec_preferences,
        )
    }

    /// Build a software-capable path where the application supplies BGRA and
    /// libwebrtc performs the negotiated video encoding.
    pub fn new_raw_video(
        factory: PeerConnectionFactory,
        config: RtcConfiguration,
        role: SessionRole,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        Self::new_with_video_input(
            factory,
            config,
            role,
            width,
            height,
            VideoInputMode::RawBgra,
            &[VideoCodec::Vp8],
        )
    }

    pub fn new_with_video_input(
        factory: PeerConnectionFactory,
        config: RtcConfiguration,
        role: SessionRole,
        width: u32,
        height: u32,
        video_input_mode: VideoInputMode,
        codec_preferences: &[VideoCodec],
    ) -> Result<Self> {
        if width == 0 || height == 0 || width > MAX_FRAME_DIMENSION || height > MAX_FRAME_DIMENSION
        {
            return Err(SessionError::WebRtc(format!(
                "video dimensions must be in 1..={MAX_FRAME_DIMENSION}"
            )));
        }
        let rtc_codec_preferences = rtc_codec_preferences(codec_preferences);
        if rtc_codec_preferences.is_empty() {
            return Err(SessionError::VideoCodecUnavailable);
        }

        let (video, audio) = if role != SessionRole::Answerer
            && role != SessionRole::OffererSubscriber
        {
            let video = match video_input_mode {
                VideoInputMode::PreEncoded => {
                    let mut options = VideoTrackOptions::default();
                    options.encoder = Some(TrackVideoEncoder::PreEncoded(PreEncodedOptions::new(
                        width, height,
                    )));
                    match factory
                        .create_video_track_with_options("gsh-screen", options)
                        .map_err(web_rtc_error)?
                    {
                        RtcLocalVideoTrack::Encoded(track) => SessionVideoTrack::PreEncoded(track),
                        RtcLocalVideoTrack::Raw(_) => {
                            return Err(SessionError::WebRtc(
                                "pre-encoded video track was not created".into(),
                            ));
                        }
                    }
                }
                VideoInputMode::RawBgra => SessionVideoTrack::Raw(
                    factory
                        .create_video_track("gsh-screen")
                        .map_err(web_rtc_error)?,
                ),
            };
            let audio = factory
                .create_audio_track("gsh-audio")
                .map_err(web_rtc_error)?;
            (Some(video), Some(audio))
        } else {
            (None, None)
        };

        let (input_tx, input_rx) = mpsc::channel(INPUT_QUEUE_CAPACITY);
        let (control_tx, control_rx) = mpsc::channel(CONTROL_QUEUE_CAPACITY);
        let realtime_messages = LatestBytesQueue::new();
        let realtime_coalesced = Arc::new(AtomicU64::new(0));
        let video_frames = LatestVideoQueue::new();
        let (audio_tx, audio_rx) = mpsc::channel(AUDIO_QUEUE_CAPACITY);
        let (feedback_tx, feedback_rx) = mpsc::channel(FEEDBACK_QUEUE_CAPACITY);
        let (ice_tx, ice_rx) = mpsc::channel(ICE_QUEUE_CAPACITY);
        let dropped_input = Arc::new(AtomicU64::new(0));
        let dropped_control = Arc::new(AtomicU64::new(0));
        let dropped_ice = Arc::new(AtomicU64::new(0));
        let dropped_audio = Arc::new(AtomicU64::new(0));
        let decoded_video_callbacks = Arc::new(AtomicU64::new(0));
        let dropped_feedback = Arc::new(AtomicU64::new(0));
        let dropped_video = Arc::new(AtomicU64::new(0));
        let remote_tracks = Arc::new(Mutex::new(Vec::new()));
        let connected = Arc::new(AtomicBool::new(false));
        let telemetry = PipelineTelemetry::default();
        let input_channel: ChannelSlot = Arc::new(Mutex::new(None));
        let control_channel: ChannelSlot = Arc::new(Mutex::new(None));
        let realtime_channel: ChannelSlot = Arc::new(Mutex::new(None));
        let channel_router = ChannelRouter {
            input_channel: input_channel.clone(),
            control_channel: control_channel.clone(),
            realtime_channel: realtime_channel.clone(),
            input_tx: input_tx.clone(),
            control_tx: control_tx.clone(),
            realtime_messages: realtime_messages.clone(),
            realtime_coalesced: realtime_coalesced.clone(),
            dropped_input: dropped_input.clone(),
            dropped_control: dropped_control.clone(),
            telemetry: telemetry.clone(),
        };
        let encoder_feedback_available =
            matches!(video.as_ref(), Some(SessionVideoTrack::PreEncoded(_)));
        if let Some(SessionVideoTrack::PreEncoded(video)) = video.as_ref() {
            let dropped_feedback = dropped_feedback.clone();
            video.on_encoder_feedback(move |feedback| {
                let event = match feedback {
                    RtcEncoderFeedback::KeyFrameRequest => VideoEncoderFeedback::RequestKeyFrame,
                    RtcEncoderFeedback::RateUpdate {
                        bitrate_bps,
                        framerate_fps,
                    } => VideoEncoderFeedback::RateUpdate {
                        bitrate_bps,
                        framerate_fps,
                    },
                    _ => return,
                };
                if feedback_tx.try_send(event).is_err() {
                    dropped_feedback.fetch_add(1, Ordering::Relaxed);
                }
            });
        }

        let video_frames_observer = video_frames.clone();
        let observer = PeerConnectionObserver::new()
            .on_connection_state_change({
                let connected = connected.clone();
                move |state| {
                    connected.store(state == PeerConnectionState::Connected, Ordering::Relaxed);
                }
            })
            .on_ice_candidate({
                let dropped_ice = dropped_ice.clone();
                move |candidate| {
                    if ice_tx
                        .try_send(CandidateMessage {
                            candidate: candidate.candidate,
                            sdp_mid: candidate.sdp_mid,
                            sdp_mline_index: candidate.sdp_mline_index,
                        })
                        .is_err()
                    {
                        dropped_ice.fetch_add(1, Ordering::Relaxed);
                    }
                }
            })
            .on_data_channel({
                let channel_router = channel_router.clone();
                move |channel| {
                    route_data_channel(channel, &channel_router);
                }
            })
            .on_track({
                let telemetry = telemetry.clone();
                let dropped_audio = dropped_audio.clone();
                let dropped_video = dropped_video.clone();
                let decoded_video_callbacks = decoded_video_callbacks.clone();
                let remote_tracks = remote_tracks.clone();
                move |track| {
                    match &track {
                        RemoteTrack::Video(video) => {
                            let video_frames = video_frames_observer.clone();
                            let telemetry = telemetry.clone();
                            let dropped_video = dropped_video.clone();
                            let decoded_video_callbacks = decoded_video_callbacks.clone();
                            video.on_frame(move |frame| {
                                let _timer = telemetry.time(Stage::Decode);
                                decoded_video_callbacks.fetch_add(1, Ordering::Relaxed);
                                let expected = (frame.width as usize)
                                    .checked_mul(frame.height as usize)
                                    .and_then(|pixels| pixels.checked_mul(4));
                                if frame.width == 0
                                    || frame.height == 0
                                    || frame.width > MAX_FRAME_DIMENSION
                                    || frame.height > MAX_FRAME_DIMENSION
                                    || expected != Some(frame.bgra.len())
                                    || frame.bgra.len() > MAX_DECODED_FRAME_BYTES
                                {
                                    dropped_video.fetch_add(1, Ordering::Relaxed);
                                    return;
                                }
                                let overwritten = video_frames.publish(DecodedVideoFrame {
                                    bgra: Arc::from(frame.bgra),
                                    width: frame.width,
                                    height: frame.height,
                                });
                                if overwritten {
                                    dropped_video.fetch_add(1, Ordering::Relaxed);
                                }
                            });
                        }
                        RemoteTrack::Audio(audio) => {
                            let audio_tx = audio_tx.clone();
                            let dropped_audio = dropped_audio.clone();
                            let telemetry = telemetry.clone();
                            audio.on_frame(move |frame| {
                                let _timer = telemetry.time(Stage::Decode);
                                if frame.pcm.len() > MAX_AUDIO_FRAME_SAMPLES {
                                    dropped_audio.fetch_add(1, Ordering::Relaxed);
                                    return;
                                }
                                if audio_tx
                                    .try_send(DecodedAudioFrame {
                                        pcm: frame.pcm.to_vec(),
                                        sample_rate: frame.sample_rate,
                                        channels: frame.channels,
                                        frames: frame.frames,
                                    })
                                    .is_err()
                                {
                                    dropped_audio.fetch_add(1, Ordering::Relaxed);
                                }
                            });
                        }
                    }
                    remote_tracks
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .push(track);
                }
            });
        let peer = factory
            .create_peer_connection(&config, observer)
            .map_err(web_rtc_error)?;
        let video_frames_in_flight = Arc::new(AtomicUsize::new(0));
        let mut video_sender_transform = None;
        if role == SessionRole::Offerer {
            let (Some(video), Some(audio)) = (&video, &audio) else {
                return Err(SessionError::MediaNotAvailable);
            };
            let video_transceiver = peer
                .add_transceiver(MediaKind::Video, TransceiverDirection::SendOnly)
                .map_err(web_rtc_error)?;
            video_transceiver
                .set_track(match video {
                    SessionVideoTrack::PreEncoded(track) => track.track(),
                    SessionVideoTrack::Raw(track) => track,
                })
                .map_err(web_rtc_error)?;
            video_transceiver
                .set_codec_preferences(&rtc_codec_preferences)
                .map_err(web_rtc_error)?;
            let transform = frame_release_transform(video_frames_in_flight.clone());
            video_transceiver
                .set_sender_transform(&transform)
                .map_err(web_rtc_error)?;
            video_sender_transform = Some(transform);
            let audio_transceiver = peer
                .add_transceiver(MediaKind::Audio, TransceiverDirection::SendOnly)
                .map_err(web_rtc_error)?;
            audio_transceiver.set_track(audio).map_err(web_rtc_error)?;
        } else if role == SessionRole::OffererSubscriber {
            let video_transceiver = peer
                .add_transceiver(MediaKind::Video, TransceiverDirection::RecvOnly)
                .map_err(web_rtc_error)?;
            video_transceiver
                .set_codec_preferences(&rtc_codec_preferences)
                .map_err(web_rtc_error)?;
            peer.add_transceiver(MediaKind::Audio, TransceiverDirection::RecvOnly)
                .map_err(web_rtc_error)?;
        }

        if role == SessionRole::Offerer || role == SessionRole::OffererSubscriber {
            let input = peer
                .create_data_channel(INPUT_CHANNEL)
                .map_err(web_rtc_error)?;
            route_data_channel(input, &channel_router);
            let control = peer
                .create_data_channel(CONTROL_CHANNEL)
                .map_err(web_rtc_error)?;
            route_data_channel(control, &channel_router);
        }

        Ok(Self {
            _factory: factory,
            peer,
            role,
            video,
            _audio: audio,
            input_channel,
            control_channel,
            realtime_channel,
            input_rx: tokio::sync::Mutex::new(input_rx),
            control_rx: tokio::sync::Mutex::new(control_rx),
            realtime_messages: realtime_messages.clone(),
            realtime_coalesced,
            video_frames: video_frames.clone(),
            audio_rx: tokio::sync::Mutex::new(audio_rx),
            feedback_rx: tokio::sync::Mutex::new(feedback_rx),
            encoder_feedback_available,
            ice_rx: tokio::sync::Mutex::new(ice_rx),
            dropped_input,
            dropped_control,
            dropped_ice,
            connected,
            telemetry,
            width,
            height,
            negotiated_video_codec: Mutex::new(None),
            video_frames_in_flight,
            _video_sender_transform: Mutex::new(video_sender_transform),
            video_failed: AtomicBool::new(false),
            dropped_audio,
            decoded_video_callbacks,
            dropped_feedback,
            dropped_video,
            _remote_tracks: remote_tracks,
            codec_preferences: codec_preferences.to_vec(),
        })
    }

    /// Create and install a local offer; exchange the returned SDP out of band.
    pub fn create_offer(&self) -> Result<SdpMessage> {
        let offer = self.peer.create_offer().map_err(web_rtc_error)?;
        self.peer
            .set_local_description(&offer)
            .map_err(web_rtc_error)?;
        Ok(SdpMessage {
            kind: DescriptionKind::Offer,
            sdp: offer.sdp,
        })
    }

    /// Apply a remote offer and create/install a local answer.
    pub fn accept_offer(&self, offer: SdpMessage) -> Result<SdpMessage> {
        if offer.kind != DescriptionKind::Offer {
            return Err(SessionError::WrongDescriptionKind);
        }
        if offer.sdp.len() > MAX_SDP_BYTES {
            return Err(SessionError::SignalingPayloadTooLarge);
        }
        self.peer
            .set_remote_description(&SessionDescription {
                kind: SdpType::Offer,
                sdp: offer.sdp,
            })
            .map_err(web_rtc_error)?;
        let rtc_codec_preferences = rtc_codec_preferences(&self.codec_preferences);
        if self.role == SessionRole::AnswererPublisher {
            let local_video = self.video.as_ref().ok_or(SessionError::MediaNotAvailable)?;
            let local_audio = self
                ._audio
                .as_ref()
                .ok_or(SessionError::MediaNotAvailable)?;
            let transceivers = self.peer.transceivers();
            let video = transceivers
                .iter()
                .find(|transceiver| transceiver.kind() == MediaKind::Video)
                .ok_or_else(|| SessionError::WebRtc("remote offer has no video m-line".into()))?;
            video
                .set_track(match local_video {
                    SessionVideoTrack::PreEncoded(track) => track.track(),
                    SessionVideoTrack::Raw(track) => track,
                })
                .map_err(web_rtc_error)?;
            video
                .set_direction(TransceiverDirection::SendOnly)
                .map_err(web_rtc_error)?;
            video
                .set_codec_preferences(&rtc_codec_preferences)
                .map_err(web_rtc_error)?;

            let audio = transceivers
                .iter()
                .find(|transceiver| transceiver.kind() == MediaKind::Audio)
                .ok_or_else(|| SessionError::WebRtc("remote offer has no audio m-line".into()))?;
            audio.set_track(local_audio).map_err(web_rtc_error)?;
            audio
                .set_direction(TransceiverDirection::SendOnly)
                .map_err(web_rtc_error)?;

            let transform = frame_release_transform(self.video_frames_in_flight.clone());
            video
                .set_sender_transform(&transform)
                .map_err(web_rtc_error)?;
            *self
                ._video_sender_transform
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(transform);
        }
        let answer = self.peer.create_answer().map_err(web_rtc_error)?;
        let codec =
            negotiated_video_codec(&answer.sdp).ok_or(SessionError::VideoCodecUnavailable)?;
        self.peer
            .set_local_description(&answer)
            .map_err(web_rtc_error)?;
        *self
            .negotiated_video_codec
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(codec);
        Ok(SdpMessage {
            kind: DescriptionKind::Answer,
            sdp: answer.sdp,
        })
    }

    pub fn accept_answer(&self, answer: SdpMessage) -> Result<()> {
        if answer.kind != DescriptionKind::Answer {
            return Err(SessionError::WrongDescriptionKind);
        }
        if answer.sdp.len() > MAX_SDP_BYTES {
            return Err(SessionError::SignalingPayloadTooLarge);
        }
        let codec =
            negotiated_video_codec(&answer.sdp).ok_or(SessionError::VideoCodecUnavailable)?;
        self.peer
            .set_remote_description(&SessionDescription {
                kind: SdpType::Answer,
                sdp: answer.sdp,
            })
            .map_err(web_rtc_error)?;
        *self
            .negotiated_video_codec
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(codec);
        Ok(())
    }

    pub fn add_ice_candidate(&self, candidate: CandidateMessage) -> Result<()> {
        if candidate.candidate.len() > MAX_ICE_CANDIDATE_BYTES
            || candidate
                .sdp_mid
                .as_ref()
                .is_some_and(|mid| mid.len() > 256)
        {
            return Err(SessionError::SignalingPayloadTooLarge);
        }
        self.peer
            .add_ice_candidate(&IceCandidate {
                candidate: candidate.candidate,
                sdp_mid: candidate.sdp_mid,
                sdp_mline_index: candidate.sdp_mline_index,
            })
            .map_err(web_rtc_error)
    }

    pub async fn next_ice_candidate(&self) -> Result<Option<CandidateMessage>> {
        let lost = self.dropped_ice.swap(0, Ordering::Relaxed);
        if lost > 0 {
            return Err(SessionError::IceQueueOverflow(lost));
        }
        Ok(self.ice_rx.lock().await.recv().await)
    }

    /// Drain locally discovered ICE candidates without awaiting signaling I/O.
    pub async fn drain_ice_candidates(&self) -> Result<Vec<CandidateMessage>> {
        let lost = self.dropped_ice.swap(0, Ordering::Relaxed);
        if lost > 0 {
            return Err(SessionError::IceQueueOverflow(lost));
        }
        let mut candidates = Vec::new();
        let mut receiver = self.ice_rx.lock().await;
        while let Ok(candidate) = receiver.try_recv() {
            candidates.push(candidate);
        }
        Ok(candidates)
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    pub fn input_channel_open(&self) -> bool {
        self.input_channel
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .is_some_and(|channel| channel.state() == DataChannelState::Open)
    }

    pub fn encoder_feedback_available(&self) -> bool {
        self.encoder_feedback_available
    }

    /// Set libwebrtc's aggregate congestion-control bounds (bits per second).
    /// `None` leaves that bound at the native default. Call outside the media
    /// hot path; congestion feedback remains authoritative for the encoder.
    pub fn set_bitrate_limits(
        &self,
        minimum_bps: Option<u32>,
        start_bps: Option<u32>,
        maximum_bps: Option<u32>,
    ) -> Result<()> {
        if minimum_bps
            .zip(start_bps)
            .is_some_and(|(min, start)| min > start)
            || start_bps
                .zip(maximum_bps)
                .is_some_and(|(start, max)| start > max)
            || minimum_bps
                .zip(maximum_bps)
                .is_some_and(|(min, max)| min > max)
        {
            return Err(SessionError::InvalidBitrateRange);
        }
        let convert = |value: Option<u32>| -> Result<Option<i32>> {
            value
                .map(|value| i32::try_from(value).map_err(|_| SessionError::InvalidBitrateRange))
                .transpose()
        };
        self.peer
            .set_bitrate(
                convert(minimum_bps)?,
                convert(start_bps)?,
                convert(maximum_bps)?,
            )
            .map_err(web_rtc_error)
    }

    /// Collect a compact WebRTC transport/media stats snapshot. The native
    /// stats request is synchronous (typically a few milliseconds, with a
    /// bounded upstream timeout); call it off the frame/input hot path.
    pub fn stats(&self) -> Result<GshSessionStats> {
        let report = self.peer.get_stats().map_err(web_rtc_error)?;
        let inbound = report
            .inbound_rtp
            .into_iter()
            .map(|stats| InboundMediaStats {
                kind: media_stream_kind(stats.kind),
                codec: stats.codec_mime_type.clone(),
                bytes_received: stats.bytes_received,
                packets_received: stats.packets_received,
                packets_lost: stats.packets_lost,
                nack_count: stats.nack_count,
                pli_count: stats.pli_count,
                frames_per_second: stats.frames_per_second,
                frames_decoded: stats.frames_decoded,
                frames_dropped: stats.frames_dropped,
                frame_width: stats.frame_width,
                frame_height: stats.frame_height,
                jitter_s: stats.jitter_s,
                average_jitter_buffer_delay_s: stats
                    .average_jitter_buffer_delay()
                    .map(|delay| delay.as_secs_f64()),
            })
            .collect();
        let outbound = report
            .outbound_rtp
            .into_iter()
            .map(|stats| OutboundMediaStats {
                kind: media_stream_kind(stats.kind),
                codec: stats.codec_mime_type,
                bytes_sent: stats.bytes_sent,
                packets_sent: stats.packets_sent,
                packets_lost: stats.packets_lost,
                fraction_lost: stats.fraction_lost,
                nack_count: stats.nack_count,
                pli_count: stats.pli_count,
                frames_per_second: stats.frames_per_second,
                frames_encoded: stats.frames_encoded,
                target_bitrate_bps: stats.target_bitrate_bps,
                round_trip_time_s: nonzero(stats.round_trip_time_s),
            })
            .collect();
        let selected_path = report
            .candidate_pairs
            .into_iter()
            .find(|pair| pair.nominated)
            .map(|pair| SelectedPathStats {
                round_trip_time_s: nonzero(pair.current_round_trip_time_s),
                available_incoming_bitrate_bps: nonzero(pair.available_incoming_bitrate_bps),
                available_outgoing_bitrate_bps: nonzero(pair.available_outgoing_bitrate_bps),
                is_relay: pair.local_candidate_type == IceCandidateType::Relay,
            });
        Ok(GshSessionStats {
            inbound,
            outbound,
            selected_path,
        })
    }

    pub fn negotiated_video_codec(&self) -> Option<VideoCodec> {
        *self
            .negotiated_video_codec
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Video frames accepted by the session but not yet observed at the RTP
    /// packetizer. Raw frames count while they are awaiting encoder output.
    /// The session caps this count at two.
    pub fn video_frames_pending(&self) -> usize {
        self.video_frames_in_flight.load(Ordering::Relaxed)
    }

    fn acquire_video_slot(&self) -> bool {
        self.video_frames_in_flight
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                (count < MAX_VIDEO_FRAMES_IN_FLIGHT).then_some(count + 1)
            })
            .is_ok()
    }

    fn release_video_slot(&self) {
        let _ = self.video_frames_in_flight.fetch_update(
            Ordering::Relaxed,
            Ordering::Relaxed,
            |count| Some(count.saturating_sub(1)),
        );
    }

    /// Shared opt-in telemetry collector for application capture/render stages
    /// and the session's measured send/receive boundaries.
    pub fn telemetry(&self) -> &PipelineTelemetry {
        &self.telemetry
    }

    /// Push externally encoded H.264/VP8 access units to the RTP track.
    pub fn send_video(
        &self,
        frame: EncodedVideoFrame,
    ) -> std::result::Result<(), VideoSubmitError> {
        let _timer = self.telemetry.time(Stage::WebRtcSend);
        if self.video_failed.load(Ordering::Relaxed) {
            return Err(video_submit_error(SessionError::VideoPipelineFailed, frame));
        }
        if self.negotiated_video_codec() != Some(frame.codec) {
            return Err(video_submit_error(SessionError::VideoCodecMismatch, frame));
        }
        if frame.data.is_empty() {
            return Err(video_submit_error(SessionError::EmptyVideoFrame, frame));
        }
        if frame.data.len() > MAX_VIDEO_FRAME_BYTES {
            return Err(video_submit_error(SessionError::VideoFrameTooLarge, frame));
        }
        if (frame.width != 0 && frame.width != self.width)
            || (frame.height != 0 && frame.height != self.height)
        {
            return Err(video_submit_error(
                SessionError::VideoDimensionsMismatch,
                frame,
            ));
        }
        let video = match self.video.as_ref() {
            Some(SessionVideoTrack::PreEncoded(video)) => video,
            Some(SessionVideoTrack::Raw(_)) => {
                return Err(video_submit_error(SessionError::WrongVideoInputMode, frame));
            }
            None => return Err(video_submit_error(SessionError::MediaNotAvailable, frame)),
        };
        if !self.acquire_video_slot() {
            return Err(video_submit_error(SessionError::VideoBackpressure, frame));
        }
        match video.push_frame(RtcEncodedVideoFrame {
            data: frame.data,
            is_key_frame: frame.is_key_frame,
            width: frame.width,
            height: frame.height,
            rtp_timestamp: frame.rtp_timestamp,
        }) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.release_video_slot();
                self.video_failed.store(true, Ordering::Relaxed);
                Err(VideoSubmitError {
                    error: web_rtc_error(error),
                    frame: None,
                })
            }
        }
    }

    /// Push CPU-resident BGRA pixels to libwebrtc's built-in video encoder.
    /// This is the portable software-fallback path; platform GPU-native surface
    /// handoff is a separate adapter and is not implied by this call.
    pub fn send_raw_video(&self, bgra: &[u8]) -> Result<()> {
        let _timer = self.telemetry.time(Stage::WebRtcSend);
        if self.video_failed.load(Ordering::Relaxed) {
            return Err(SessionError::VideoPipelineFailed);
        }
        let video = match self.video.as_ref() {
            Some(SessionVideoTrack::Raw(video)) => video,
            Some(SessionVideoTrack::PreEncoded(_)) => {
                return Err(SessionError::WrongVideoInputMode)
            }
            None => return Err(SessionError::MediaNotAvailable),
        };
        let expected = (self.width as usize)
            .checked_mul(self.height as usize)
            .and_then(|pixels| pixels.checked_mul(4));
        if expected != Some(bgra.len()) || bgra.len() > MAX_RAW_FRAME_BYTES {
            return Err(SessionError::InvalidRawVideoFrame);
        }
        if self.negotiated_video_codec().is_none() {
            return Err(SessionError::VideoCodecUnavailable);
        }
        if !self.acquire_video_slot() {
            return Err(SessionError::VideoBackpressure);
        }
        match video.push_frame(RtcVideoFrame::new(bgra, self.width, self.height)) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.release_video_slot();
                Err(web_rtc_error(error))
            }
        }
    }

    /// Push mono/stereo PCM to libwebrtc's Opus audio sender.
    pub fn send_audio(&self, pcm: &[i16], sample_rate: u32, channels: u32) -> Result<()> {
        let _timer = self.telemetry.time(Stage::WebRtcSend);
        if self._audio.is_none() {
            return Err(SessionError::MediaNotAvailable);
        }
        self._factory.push_audio_frame(pcm, sample_rate, channels);
        Ok(())
    }

    /// Send a protobuf-encoded input transition over the reliable input channel.
    pub fn send_input(&self, input: &UserInput) -> Result<()> {
        let _timer = self.telemetry.time(Stage::WebRtcSend);
        let bytes = input.encode_to_vec();
        let channel = self
            .input_channel
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let channel = channel.as_ref().ok_or(SessionError::ChannelNotOpen)?;
        if channel.buffered_amount() > MAX_INPUT_BUFFERED {
            return Err(SessionError::InputBackpressure);
        }
        channel.send(&bytes, true).map_err(web_rtc_error)
    }

    pub async fn recv_input(&self) -> Result<UserInput> {
        let lost = self.dropped_input.swap(0, Ordering::Relaxed);
        if lost > 0 {
            return Err(SessionError::InputQueueOverflow(lost));
        }
        self.input_rx
            .lock()
            .await
            .recv()
            .await
            .ok_or(SessionError::QueueClosed)
    }

    /// Send a control-plane protobuf message. High-frequency media must not use
    /// this path; it belongs on the RTP video/audio tracks.
    pub fn send_control(&self, message: &impl Message) -> Result<()> {
        let _timer = self.telemetry.time(Stage::WebRtcSend);
        let encoded_len = message.encoded_len();
        if encoded_len > MAX_CONTROL_MESSAGE {
            return Err(SessionError::ControlMessageTooLarge);
        }
        let mut bytes = Vec::with_capacity(encoded_len);
        message.encode(&mut bytes).map_err(web_rtc_error)?;
        let channel = self
            .control_channel
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let channel = channel.as_ref().ok_or(SessionError::ChannelNotOpen)?;
        channel.send(&bytes, true).map_err(web_rtc_error)
    }

    /// Send a bounded latest-state payload over `gsh.realtime`. The channel's
    /// reliability/ordering policy is selected by its creator; browser offers
    /// should use unordered delivery with zero retransmissions.
    pub fn send_realtime(&self, payload: &[u8]) -> Result<()> {
        if payload.len() > MAX_REALTIME_MESSAGE {
            return Err(SessionError::RealtimeMessageTooLarge);
        }
        let channel = self
            .realtime_channel
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let channel = channel.as_ref().ok_or(SessionError::ChannelNotOpen)?;
        if channel.buffered_amount() > MAX_REALTIME_BUFFERED {
            return Err(SessionError::RealtimeBackpressure);
        }
        channel.send(payload, true).map_err(web_rtc_error)
    }

    pub async fn recv_control(&self) -> Result<Vec<u8>> {
        let lost = self.dropped_control.swap(0, Ordering::Relaxed);
        if lost > 0 {
            return Err(SessionError::ControlQueueOverflow(lost));
        }
        self.control_rx
            .lock()
            .await
            .recv()
            .await
            .ok_or(SessionError::QueueClosed)
    }

    /// Receive the newest replaceable pointer/gamepad state. Older queued
    /// realtime snapshots are coalesced instead of being applied late.
    pub async fn recv_realtime(&self) -> Result<Vec<u8>> {
        self.realtime_messages
            .recv()
            .await
            .ok_or(SessionError::QueueClosed)
    }

    pub fn realtime_messages_coalesced(&self) -> u64 {
        self.realtime_coalesced.load(Ordering::Relaxed)
    }

    /// Receive the latest decoded video frame. Intermediate frames are replaced
    /// rather than queued, preventing presentation lag from growing unbounded.
    pub async fn recv_video(&self) -> Option<Arc<DecodedVideoFrame>> {
        self.video_frames.recv().await
    }

    /// Non-blocking latest-frame read for render loops that already have a
    /// frame-pacing clock of their own.
    pub fn try_recv_video(&self) -> Option<Arc<DecodedVideoFrame>> {
        self.video_frames.try_recv()
    }

    pub fn dropped_decoded_video_frames(&self) -> u64 {
        self.dropped_video.load(Ordering::Relaxed)
    }

    pub fn decoded_video_frames_published(&self) -> u64 {
        self.video_frames.published_frames()
    }

    pub fn decoded_video_callbacks(&self) -> u64 {
        self.decoded_video_callbacks.load(Ordering::Relaxed)
    }

    /// Receive a decoded PCM audio chunk from the bounded native audio queue.
    pub async fn recv_audio(&self) -> Result<DecodedAudioFrame> {
        let lost = self.dropped_audio.swap(0, Ordering::Relaxed);
        if lost > 0 {
            return Err(SessionError::AudioQueueOverflow(lost));
        }
        self.audio_rx
            .lock()
            .await
            .recv()
            .await
            .ok_or(SessionError::QueueClosed)
    }

    pub async fn recv_encoder_feedback(&self) -> Result<VideoEncoderFeedback> {
        let lost = self.dropped_feedback.swap(0, Ordering::Relaxed);
        if lost > 0 {
            return Err(SessionError::FeedbackQueueOverflow(lost));
        }
        self.feedback_rx
            .lock()
            .await
            .recv()
            .await
            .ok_or(SessionError::QueueClosed)
    }
}

impl Drop for GshSession {
    fn drop(&mut self) {
        self.video_frames.close();
        self.realtime_messages.close();
    }
}

fn route_data_channel(channel: DataChannel, router: &ChannelRouter) {
    match channel.label().as_str() {
        INPUT_CHANNEL => {
            let input_tx = router.input_tx.clone();
            let dropped_input = router.dropped_input.clone();
            let telemetry = router.telemetry.clone();
            channel.on_message(move |data, binary| {
                let _timer = telemetry.time(Stage::WebRtcReceive);
                if data.len() > MAX_INPUT_MESSAGE {
                    dropped_input.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                if !binary {
                    dropped_input.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                match UserInput::decode(data) {
                    Ok(input) if input_tx.try_send(input).is_ok() => {}
                    _ => {
                        dropped_input.fetch_add(1, Ordering::Relaxed);
                    }
                }
            });
            *router
                .input_channel
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(channel);
        }
        CONTROL_CHANNEL | GSH_CONTROL_CHANNEL => {
            let control_tx = router.control_tx.clone();
            let dropped_control = router.dropped_control.clone();
            let telemetry = router.telemetry.clone();
            channel.on_message(move |data, _binary| {
                let _timer = telemetry.time(Stage::WebRtcReceive);
                if data.len() > MAX_CONTROL_MESSAGE {
                    dropped_control.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                if control_tx.try_send(data.to_vec()).is_err() {
                    dropped_control.fetch_add(1, Ordering::Relaxed);
                }
            });
            *router
                .control_channel
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(channel);
        }
        REALTIME_CHANNEL => {
            let realtime_messages = router.realtime_messages.clone();
            let realtime_coalesced = router.realtime_coalesced.clone();
            let telemetry = router.telemetry.clone();
            channel.on_message(move |data, _binary| {
                let _timer = telemetry.time(Stage::WebRtcReceive);
                if data.len() <= MAX_REALTIME_MESSAGE && realtime_messages.publish(data.to_vec()) {
                    realtime_coalesced.fetch_add(1, Ordering::Relaxed);
                }
            });
            *router
                .realtime_channel
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(channel);
        }
        _ => {}
    }
}

fn web_rtc_error(error: impl std::fmt::Display) -> SessionError {
    SessionError::WebRtc(error.to_string())
}

fn frame_release_transform(frames_in_flight: Arc<AtomicUsize>) -> FrameTransform {
    FrameTransform::new(move |frame| {
        if frame.direction == FrameDirection::Send {
            let _ = frames_in_flight.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                Some(count.saturating_sub(1))
            });
        }
        FrameAction::Forward
    })
}

fn media_stream_kind(kind: StreamKind) -> MediaStreamKind {
    match kind {
        StreamKind::Audio => MediaStreamKind::Audio,
        StreamKind::Video => MediaStreamKind::Video,
        StreamKind::Unknown => MediaStreamKind::Unknown,
    }
}

fn nonzero(value: f64) -> Option<f64> {
    (value.is_finite() && value > 0.0).then_some(value)
}

fn video_submit_error(error: SessionError, frame: EncodedVideoFrame) -> VideoSubmitError {
    VideoSubmitError {
        error,
        frame: Some(frame),
    }
}

fn negotiated_video_codec(sdp: &str) -> Option<VideoCodec> {
    let mut video_payloads = Vec::new();
    let mut video_mappings = std::collections::HashMap::new();
    let mut in_video_section = false;
    for line in sdp.lines() {
        if let Some(media) = line.strip_prefix("m=") {
            in_video_section = media.starts_with("video ");
            if in_video_section {
                video_payloads.extend(media.split_whitespace().skip(3).map(str::to_owned));
            }
            continue;
        }
        if in_video_section {
            if let Some(mapping) = line.strip_prefix("a=rtpmap:") {
                if let Some((payload, codec)) = mapping.split_once(' ') {
                    video_mappings.insert(payload.to_owned(), codec.to_owned());
                }
            }
        }
    }

    video_payloads.into_iter().find_map(|payload| {
        match video_mappings.get(&payload)?.split('/').next()? {
            codec if codec.eq_ignore_ascii_case("H264") => Some(VideoCodec::H264),
            codec if codec.eq_ignore_ascii_case("VP8") => Some(VideoCodec::Vp8),
            _ => None,
        }
    })
}

fn rtc_codec_preferences(preferences: &[VideoCodec]) -> Vec<RtcVideoCodec> {
    let mut codecs = Vec::with_capacity(preferences.len());
    for codec in preferences {
        let native = match codec {
            VideoCodec::H264 => RtcVideoCodec::H264,
            VideoCodec::Vp8 => RtcVideoCodec::Vp8,
        };
        if !codecs.contains(&native) {
            codecs.push(native);
        }
    }
    codecs
}

#[cfg(test)]
mod tests {
    use super::*;
    use reactor_webrtc::DataChannelState;
    use std::time::Duration;

    // Single 320x240 VP8 keyframe produced by libvpx (black color source).
    const VP8_KEYFRAME: &[u8] = &[
        0x30, 0x12, 0x00, 0x9d, 0x01, 0x2a, 0x40, 0x01, 0xf0, 0x00, 0x00, 0x47, 0x08, 0x85, 0x85,
        0x88, 0x99, 0x84, 0x88, 0x02, 0x02, 0x00, 0x06, 0x16, 0x04, 0xf7, 0x06, 0x81, 0x64, 0x9f,
        0x6b, 0xdb, 0x9b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b,
        0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b,
        0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b,
        0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b,
        0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b,
        0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b,
        0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b,
        0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b,
        0x27, 0x38, 0x7b, 0x27, 0x38, 0x7a, 0xf4, 0x00, 0xfe, 0xff, 0xab, 0x50, 0x80,
    ];

    #[test]
    fn codec_selection_follows_video_payload_preference_order() {
        let sdp = "m=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=rtpmap:111 opus/48000/2\r\nm=video 9 UDP/TLS/RTP/SAVPF 96 102\r\na=rtpmap:102 H264/90000\r\na=rtpmap:96 VP8/90000\r\n";
        assert_eq!(negotiated_video_codec(sdp), Some(VideoCodec::Vp8));
        let h264_first = sdp.replace("SAVPF 96 102", "SAVPF 102 96");
        assert_eq!(negotiated_video_codec(&h264_first), Some(VideoCodec::H264));
        assert_eq!(negotiated_video_codec("m=video 9 RTP/AVP 100\r\n"), None);
    }

    #[test]
    fn two_sessions_negotiate_and_deliver_input_over_data_channel() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let factory_offer = PeerConnectionFactory::builder().build().unwrap();
            let factory_answer = PeerConnectionFactory::builder().build().unwrap();
            let offerer = GshSession::new(
                factory_offer,
                RtcConfiguration::default(),
                SessionRole::Offerer,
                320,
                240,
            )
            .unwrap();
            let answerer = GshSession::new(
                factory_answer,
                RtcConfiguration::default(),
                SessionRole::Answerer,
                320,
                240,
            )
            .unwrap();
            offerer
                .set_bitrate_limits(Some(100_000), Some(500_000), Some(2_000_000))
                .unwrap();
            assert!(matches!(
                offerer.set_bitrate_limits(Some(500), Some(100), Some(1_000)),
                Err(SessionError::InvalidBitrateRange)
            ));

            let offer = offerer.create_offer().unwrap();
            let answer = answerer.accept_offer(offer).unwrap();
            offerer.accept_answer(answer).unwrap();

            let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
            loop {
                for candidate in offerer.drain_ice_candidates().await.unwrap() {
                    answerer.add_ice_candidate(candidate).unwrap();
                }
                for candidate in answerer.drain_ice_candidates().await.unwrap() {
                    offerer.add_ice_candidate(candidate).unwrap();
                }
                if offerer.is_connected() && answerer.is_connected() {
                    break;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "ICE did not connect"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert_eq!(
                offerer.negotiated_video_codec(),
                answerer.negotiated_video_codec()
            );
            if offerer.negotiated_video_codec() == Some(VideoCodec::Vp8) {
                offerer
                    .send_video(EncodedVideoFrame {
                        data: VP8_KEYFRAME.to_vec(),
                        codec: VideoCodec::Vp8,
                        is_key_frame: true,
                        width: 320,
                        height: 240,
                        rtp_timestamp: 0,
                    })
                    .unwrap();
                let decoded = tokio::time::timeout(Duration::from_secs(2), answerer.recv_video())
                    .await
                    .expect("decoded video frame receive timed out")
                    .expect("remote video track closed");
                assert_eq!((decoded.width, decoded.height), (320, 240));
                assert_eq!(decoded.bgra.len(), 320 * 240 * 4);
                let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
                while offerer.video_frames_pending() != 0 {
                    assert!(
                        tokio::time::Instant::now() < deadline,
                        "encoded frame did not reach the RTP packetizer"
                    );
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }

                // A burst of independent keyframes can neither grow the native
                // encoded FIFO past its admission window nor lose ownership on
                // backpressure. These are all keyframes, so each is independently
                // decodable if accepted.
                for timestamp in 1..=16 {
                    let frame = EncodedVideoFrame {
                        data: VP8_KEYFRAME.to_vec(),
                        codec: VideoCodec::Vp8,
                        is_key_frame: true,
                        width: 320,
                        height: 240,
                        rtp_timestamp: timestamp * 3_000,
                    };
                    match offerer.send_video(frame) {
                        Ok(()) => {}
                        Err(error) if matches!(error.error, SessionError::VideoBackpressure) => {
                            assert!(error.frame.is_some(), "backpressure must return the frame");
                        }
                        Err(error) => panic!("unexpected video burst failure: {}", error.error),
                    }
                    assert!(offerer.video_frames_pending() <= MAX_VIDEO_FRAMES_IN_FLIGHT);
                }
            }
            let wrong_codec = match offerer.negotiated_video_codec().unwrap() {
                VideoCodec::H264 => VideoCodec::Vp8,
                VideoCodec::Vp8 => VideoCodec::H264,
            };
            let rejected = offerer
                .send_video(EncodedVideoFrame {
                    data: vec![0; 16],
                    codec: wrong_codec,
                    is_key_frame: true,
                    width: 320,
                    height: 240,
                    rtp_timestamp: 0,
                })
                .unwrap_err();
            assert!(matches!(rejected.error, SessionError::VideoCodecMismatch));
            assert!(rejected.frame.is_some());

            let input = UserInput::default();
            let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
            loop {
                if answerer.send_input(&input).is_ok() {
                    break;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "input channel did not open"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let received = tokio::time::timeout(Duration::from_secs(2), offerer.recv_input())
                .await
                .expect("input receive timed out")
                .unwrap();
            assert_eq!(received, input);

            let pcm = vec![0_i16; 480 * 2];
            for _ in 0..20 {
                offerer.send_audio(&pcm, 48_000, 2).unwrap();
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let audio = tokio::time::timeout(Duration::from_secs(2), answerer.recv_audio())
                .await
                .expect("decoded audio receive timed out")
                .unwrap();
            assert_eq!(audio.sample_rate, 48_000);
            assert!(audio.channels >= 1);
            assert!(!audio.pcm.is_empty());

            let stats = offerer.stats().unwrap();
            assert!(!stats.outbound.is_empty());
            assert!(stats.outbound.iter().any(|stream| stream.codec.is_some()));
            let remote_stats = answerer.stats().unwrap();
            assert!(!remote_stats.inbound.is_empty());
        });
    }

    #[test]
    fn subscriber_offerer_receives_remote_video_and_sends_input() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let subscriber = GshSession::new(
                PeerConnectionFactory::builder().build().unwrap(),
                RtcConfiguration::default(),
                SessionRole::OffererSubscriber,
                320,
                240,
            )
            .unwrap();
            let publisher = GshSession::new_raw_video(
                PeerConnectionFactory::builder().build().unwrap(),
                RtcConfiguration::default(),
                SessionRole::AnswererPublisher,
                320,
                240,
            )
            .unwrap();

            let offer = subscriber.create_offer().unwrap();
            let answer = publisher.accept_offer(offer).unwrap();
            subscriber.accept_answer(answer).unwrap();

            let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
            loop {
                for candidate in subscriber.drain_ice_candidates().await.unwrap() {
                    publisher.add_ice_candidate(candidate).unwrap();
                }
                for candidate in publisher.drain_ice_candidates().await.unwrap() {
                    subscriber.add_ice_candidate(candidate).unwrap();
                }
                if subscriber.is_connected() && publisher.is_connected() {
                    break;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "ICE did not connect"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }

            let input = UserInput::default();
            let input_deadline = tokio::time::Instant::now() + Duration::from_secs(2);
            loop {
                if subscriber.send_input(&input).is_ok() {
                    break;
                }
                assert!(
                    tokio::time::Instant::now() < input_deadline,
                    "subscriber input channel did not open"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let received = tokio::time::timeout(Duration::from_secs(2), publisher.recv_input())
                .await
                .expect("input receive timed out")
                .unwrap();
            assert_eq!(received, input);

            publisher
                .send_raw_video(&vec![0x40; 320 * 240 * 4])
                .unwrap();
            let decoded = tokio::time::timeout(Duration::from_secs(3), subscriber.recv_video())
                .await
                .expect("video receive timed out")
                .expect("remote video track closed");
            assert_eq!((decoded.width, decoded.height), (320, 240));
            assert_eq!(decoded.bgra.len(), 320 * 240 * 4);
        });
    }

    #[test]
    fn raw_bgra_path_uses_libwebrtc_software_fallback() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let offerer = GshSession::new_raw_video(
                PeerConnectionFactory::builder().build().unwrap(),
                RtcConfiguration::default(),
                SessionRole::Offerer,
                320,
                240,
            )
            .unwrap();
            assert!(!offerer.encoder_feedback_available());
            let mut bgra = vec![0; 320 * 240 * 4];
            assert!(matches!(
                offerer.send_raw_video(&bgra),
                Err(SessionError::VideoCodecUnavailable)
            ));
            assert!(offerer.acquire_video_slot());
            assert!(offerer.acquire_video_slot());
            assert!(!offerer.acquire_video_slot());
            assert_eq!(offerer.video_frames_pending(), MAX_VIDEO_FRAMES_IN_FLIGHT);
            offerer.release_video_slot();
            offerer.release_video_slot();

            let answerer = GshSession::new_with_video_input(
                PeerConnectionFactory::builder().build().unwrap(),
                RtcConfiguration::default(),
                SessionRole::Answerer,
                320,
                240,
                VideoInputMode::RawBgra,
                &[VideoCodec::Vp8],
            )
            .unwrap();
            let answer = answerer
                .accept_offer(offerer.create_offer().unwrap())
                .unwrap();
            offerer.accept_answer(answer).unwrap();

            let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
            loop {
                for candidate in offerer.drain_ice_candidates().await.unwrap() {
                    answerer.add_ice_candidate(candidate).unwrap();
                }
                for candidate in answerer.drain_ice_candidates().await.unwrap() {
                    offerer.add_ice_candidate(candidate).unwrap();
                }
                if offerer.is_connected() && answerer.is_connected() {
                    break;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "ICE did not connect"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }

            for pixel in bgra.chunks_exact_mut(4) {
                pixel.copy_from_slice(&[0x20, 0x80, 0xe0, 0xff]);
            }
            offerer.send_raw_video(&bgra).unwrap();
            let decoded = tokio::time::timeout(Duration::from_secs(2), answerer.recv_video())
                .await
                .expect("software-decoded frame timed out")
                .expect("video track closed");
            assert_eq!((decoded.width, decoded.height), (320, 240));
            assert_eq!(decoded.bgra.len(), 320 * 240 * 4);

            for frame_number in 1..=10 {
                tokio::time::sleep(Duration::from_millis(34)).await;
                let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
                loop {
                    match offerer.send_raw_video(&bgra) {
                        Ok(()) => break,
                        Err(SessionError::VideoBackpressure) => {
                            assert!(tokio::time::Instant::now() < deadline);
                            tokio::time::sleep(Duration::from_millis(1)).await;
                        }
                        Err(error) => panic!("raw video frame {frame_number} failed: {error}"),
                    }
                }
                let decoded = tokio::time::timeout(Duration::from_secs(2), answerer.recv_video())
                    .await
                    .expect("paced software-decoded frame timed out")
                    .expect("video track closed");
                assert_eq!((decoded.width, decoded.height), (320, 240));
            }
        });
    }

    #[test]
    fn answerer_publisher_streams_raw_video_to_browser_style_offer() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (ice_tx, mut ice_rx) = tokio::sync::mpsc::unbounded_channel();
            let decoded_frames = Arc::new(AtomicUsize::new(0));
            let frame_counter = decoded_frames.clone();
            let remote_video_tracks = Arc::new(AtomicUsize::new(0));
            let track_counter = remote_video_tracks.clone();
            let remote_tracks = Arc::new(Mutex::new(Vec::new()));
            let track_keepalive = remote_tracks.clone();
            let observer = PeerConnectionObserver::new()
                .on_ice_candidate(move |candidate| {
                    let _ = ice_tx.send(candidate);
                })
                .on_track(move |track| {
                    if let RemoteTrack::Video(video) = &track {
                        track_counter.fetch_add(1, Ordering::Relaxed);
                        let frame_counter = frame_counter.clone();
                        video.on_frame(move |_| {
                            frame_counter.fetch_add(1, Ordering::Relaxed);
                        });
                    }
                    track_keepalive
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .push(track);
                });
            let browser_factory = PeerConnectionFactory::builder().build().unwrap();
            let browser = browser_factory
                .create_peer_connection(&RtcConfiguration::default(), observer)
                .unwrap();
            browser
                .add_transceiver(MediaKind::Video, TransceiverDirection::RecvOnly)
                .unwrap();
            browser
                .add_transceiver(MediaKind::Audio, TransceiverDirection::RecvOnly)
                .unwrap();
            let control = browser.create_data_channel(GSH_CONTROL_CHANNEL).unwrap();
            let realtime = browser.create_data_channel(REALTIME_CHANNEL).unwrap();

            let session = GshSession::new_raw_video(
                PeerConnectionFactory::builder().build().unwrap(),
                RtcConfiguration::default(),
                SessionRole::AnswererPublisher,
                320,
                240,
            )
            .unwrap();
            let offer = browser.create_offer().unwrap();
            browser.set_local_description(&offer).unwrap();
            let answer = session
                .accept_offer(SdpMessage {
                    kind: DescriptionKind::Offer,
                    sdp: offer.sdp,
                })
                .unwrap();
            browser
                .set_remote_description(&SessionDescription {
                    kind: SdpType::Answer,
                    sdp: answer.sdp,
                })
                .unwrap();

            let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
            loop {
                while let Ok(candidate) = ice_rx.try_recv() {
                    session
                        .add_ice_candidate(CandidateMessage {
                            candidate: candidate.candidate,
                            sdp_mid: candidate.sdp_mid,
                            sdp_mline_index: candidate.sdp_mline_index,
                        })
                        .unwrap();
                }
                for candidate in session.drain_ice_candidates().await.unwrap() {
                    browser
                        .add_ice_candidate(&IceCandidate {
                            candidate: candidate.candidate,
                            sdp_mid: candidate.sdp_mid,
                            sdp_mline_index: candidate.sdp_mline_index,
                        })
                        .unwrap();
                }
                if session.is_connected() {
                    break;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "browser-style peer did not connect"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }

            let channel_deadline = tokio::time::Instant::now() + Duration::from_secs(2);
            while control.state() != DataChannelState::Open
                || realtime.state() != DataChannelState::Open
            {
                assert!(
                    tokio::time::Instant::now() < channel_deadline,
                    "browser-style input channels did not open"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            control.send(b"control-transition", true).unwrap();
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(1), session.recv_control())
                    .await
                    .expect("control message receive timed out")
                    .unwrap(),
                b"control-transition"
            );
            realtime.send(b"latest-state", true).unwrap();
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(1), session.recv_realtime())
                    .await
                    .expect("realtime message receive timed out")
                    .unwrap(),
                b"latest-state"
            );

            let bgra = vec![0x40; 320 * 240 * 4];
            for _ in 0..10 {
                let admission_deadline = tokio::time::Instant::now() + Duration::from_secs(2);
                loop {
                    match session.send_raw_video(&bgra) {
                        Ok(()) => break,
                        Err(SessionError::VideoBackpressure) => {
                            assert!(tokio::time::Instant::now() < admission_deadline);
                            tokio::time::sleep(Duration::from_millis(1)).await;
                        }
                        Err(error) => panic!("answerer raw video failed: {error}"),
                    }
                }
                tokio::time::sleep(Duration::from_millis(34)).await;
            }
            let frame_deadline = tokio::time::Instant::now() + Duration::from_secs(3);
            while decoded_frames.load(Ordering::Relaxed) == 0 {
                if tokio::time::Instant::now() >= frame_deadline {
                    let stats = browser.get_stats().unwrap();
                    panic!(
                        "answerer raw VP8 frame was not decoded (codec={:?}, pending={}, remote video tracks={}, inbound={:?}, outbound={:?})",
                        session.negotiated_video_codec(),
                        session.video_frames_pending(),
                        remote_video_tracks.load(Ordering::Relaxed),
                        stats.inbound_rtp,
                        stats.outbound_rtp,
                    );
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
    }
}
