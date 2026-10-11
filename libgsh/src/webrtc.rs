//! Concrete v1 WebRTC session backend.
//!
//! Media is carried on native RTP tracks; input and control use separate
//! reliable data channels. This module intentionally does not define a generic
//! transport trait. Build with `--features webrtc` to include libwebrtc.

use crate::shared::protocol::UserInput;
use crate::telemetry::{PipelineTelemetry, Stage};
use prost::Message;
use reactor_webrtc::{
    AudioTrack, DataChannel, EncodedVideoFrame as RtcEncodedVideoFrame, EncodedVideoTrack,
    FrameAction, FrameDirection, FrameTransform, IceCandidate, LocalVideoTrack, MediaKind,
    PeerConnection, PeerConnectionFactory, PeerConnectionObserver, PeerConnectionState,
    PreEncodedOptions, RtcConfiguration, SdpType, SessionDescription, TrackVideoEncoder,
    TransceiverDirection, VideoCodec as RtcVideoCodec, VideoTrackOptions,
};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    Arc, Mutex,
};
use thiserror::Error;
use tokio::sync::mpsc;

const INPUT_CHANNEL: &str = "gsh-input-v1";
const CONTROL_CHANNEL: &str = "gsh-control-v1";
const INPUT_QUEUE_CAPACITY: usize = 256;
const CONTROL_QUEUE_CAPACITY: usize = 64;
const ICE_QUEUE_CAPACITY: usize = 256;
const MAX_CONTROL_MESSAGE: usize = 64 * 1024;
const MAX_INPUT_MESSAGE: usize = 4096;
const MAX_VIDEO_FRAME_BYTES: usize = 16 * 1024 * 1024;
const MAX_INPUT_BUFFERED: u64 = 64 * 1024;
const MAX_FRAME_DIMENSION: u32 = 8192;
const MAX_VIDEO_FRAMES_IN_FLIGHT: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
    Vp8,
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
    #[error("input backlog exceeded its bound; {0} input event(s) were dropped; reconcile remote input state")]
    InputQueueOverflow(u64),
    #[error("control backlog exceeded its bound; {0} control message(s) were dropped")]
    ControlQueueOverflow(u64),
    #[error("signaling backlog exceeded its bound; {0} ICE candidate(s) were dropped")]
    IceQueueOverflow(u64),
    #[error("input send is backpressured; retry or coalesce replaceable pointer updates")]
    InputBackpressure,
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
}

pub type Result<T> = std::result::Result<T, SessionError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRole {
    /// Creates the input and control channels before making an offer.
    Offerer,
    /// Receives channels created by the offerer.
    Answerer,
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

#[derive(Clone)]
struct ChannelRouter {
    input_channel: ChannelSlot,
    control_channel: ChannelSlot,
    input_tx: mpsc::Sender<UserInput>,
    control_tx: mpsc::Sender<Vec<u8>>,
    dropped_input: Arc<AtomicU64>,
    dropped_control: Arc<AtomicU64>,
    telemetry: PipelineTelemetry,
}

/// One client session. App-facing methods operate on GSH inputs/control and
/// encoded media, not RTP packets or WebRTC data-channel internals.
pub struct GshSession {
    _factory: PeerConnectionFactory,
    peer: PeerConnection,
    video: Option<EncodedVideoTrack>,
    _audio: Option<AudioTrack>,
    input_channel: ChannelSlot,
    control_channel: ChannelSlot,
    input_rx: mpsc::Receiver<UserInput>,
    control_rx: mpsc::Receiver<Vec<u8>>,
    ice_rx: mpsc::Receiver<CandidateMessage>,
    dropped_input: Arc<AtomicU64>,
    dropped_control: Arc<AtomicU64>,
    dropped_ice: Arc<AtomicU64>,
    connected: Arc<AtomicBool>,
    telemetry: PipelineTelemetry,
    width: u32,
    height: u32,
    negotiated_video_codec: Mutex<Option<VideoCodec>>,
    video_frames_in_flight: Arc<AtomicUsize>,
    _video_sender_transform: Option<FrameTransform>,
    video_failed: AtomicBool,
}

impl GshSession {
    pub fn new(
        factory: PeerConnectionFactory,
        config: RtcConfiguration,
        role: SessionRole,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        if width == 0 || height == 0 || width > MAX_FRAME_DIMENSION || height > MAX_FRAME_DIMENSION
        {
            return Err(SessionError::WebRtc(format!(
                "video dimensions must be in 1..={MAX_FRAME_DIMENSION}"
            )));
        }

        let (video, audio) = if role == SessionRole::Offerer {
            let mut video_options = VideoTrackOptions::default();
            video_options.encoder = Some(TrackVideoEncoder::PreEncoded(PreEncodedOptions::new(
                width, height,
            )));
            let video = match factory
                .create_video_track_with_options("gsh-screen", video_options)
                .map_err(web_rtc_error)?
            {
                LocalVideoTrack::Encoded(track) => track,
                LocalVideoTrack::Raw(_) => {
                    return Err(SessionError::WebRtc(
                        "pre-encoded video track was not created".into(),
                    ));
                }
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
        let (ice_tx, ice_rx) = mpsc::channel(ICE_QUEUE_CAPACITY);
        let dropped_input = Arc::new(AtomicU64::new(0));
        let dropped_control = Arc::new(AtomicU64::new(0));
        let dropped_ice = Arc::new(AtomicU64::new(0));
        let connected = Arc::new(AtomicBool::new(false));
        let telemetry = PipelineTelemetry::default();
        let input_channel: ChannelSlot = Arc::new(Mutex::new(None));
        let control_channel: ChannelSlot = Arc::new(Mutex::new(None));
        let channel_router = ChannelRouter {
            input_channel: input_channel.clone(),
            control_channel: control_channel.clone(),
            input_tx: input_tx.clone(),
            control_tx: control_tx.clone(),
            dropped_input: dropped_input.clone(),
            dropped_control: dropped_control.clone(),
            telemetry: telemetry.clone(),
        };

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
            });
        let peer = factory
            .create_peer_connection(&config, observer)
            .map_err(web_rtc_error)?;
        let video_frames_in_flight = Arc::new(AtomicUsize::new(0));
        let mut video_sender_transform = None;
        if let (Some(video), Some(audio)) = (&video, &audio) {
            let video_transceiver = peer
                .add_transceiver(MediaKind::Video, TransceiverDirection::SendOnly)
                .map_err(web_rtc_error)?;
            video_transceiver
                .set_track(video.track())
                .map_err(web_rtc_error)?;
            video_transceiver
                .set_codec_preferences(&[RtcVideoCodec::H264, RtcVideoCodec::Vp8])
                .map_err(web_rtc_error)?;
            let frames_in_flight = video_frames_in_flight.clone();
            let transform = FrameTransform::new(move |frame| {
                if frame.direction == FrameDirection::Send {
                    let _ = frames_in_flight.fetch_update(
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                        |count| Some(count.saturating_sub(1)),
                    );
                }
                FrameAction::Forward
            });
            video_transceiver
                .set_sender_transform(&transform)
                .map_err(web_rtc_error)?;
            video_sender_transform = Some(transform);
            let audio_transceiver = peer
                .add_transceiver(MediaKind::Audio, TransceiverDirection::SendOnly)
                .map_err(web_rtc_error)?;
            audio_transceiver.set_track(audio).map_err(web_rtc_error)?;
        } else {
            peer.add_transceiver(MediaKind::Video, TransceiverDirection::RecvOnly)
                .map_err(web_rtc_error)?;
            peer.add_transceiver(MediaKind::Audio, TransceiverDirection::RecvOnly)
                .map_err(web_rtc_error)?;
        }

        if role == SessionRole::Offerer {
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
            video,
            _audio: audio,
            input_channel,
            control_channel,
            input_rx,
            control_rx,
            ice_rx,
            dropped_input,
            dropped_control,
            dropped_ice,
            connected,
            telemetry,
            width,
            height,
            negotiated_video_codec: Mutex::new(None),
            video_frames_in_flight,
            _video_sender_transform: video_sender_transform,
            video_failed: AtomicBool::new(false),
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
        self.peer
            .set_remote_description(&SessionDescription {
                kind: SdpType::Offer,
                sdp: offer.sdp,
            })
            .map_err(web_rtc_error)?;
        if let Some(video) = self
            .peer
            .transceivers()
            .into_iter()
            .find(|transceiver| transceiver.kind() == MediaKind::Video)
        {
            video
                .set_codec_preferences(&[RtcVideoCodec::H264, RtcVideoCodec::Vp8])
                .map_err(web_rtc_error)?;
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
        self.peer
            .add_ice_candidate(&IceCandidate {
                candidate: candidate.candidate,
                sdp_mid: candidate.sdp_mid,
                sdp_mline_index: candidate.sdp_mline_index,
            })
            .map_err(web_rtc_error)
    }

    pub async fn next_ice_candidate(&mut self) -> Result<Option<CandidateMessage>> {
        let lost = self.dropped_ice.swap(0, Ordering::Relaxed);
        if lost > 0 {
            return Err(SessionError::IceQueueOverflow(lost));
        }
        Ok(self.ice_rx.recv().await)
    }

    /// Drain locally discovered ICE candidates without awaiting signaling I/O.
    pub fn drain_ice_candidates(&mut self) -> Result<Vec<CandidateMessage>> {
        let lost = self.dropped_ice.swap(0, Ordering::Relaxed);
        if lost > 0 {
            return Err(SessionError::IceQueueOverflow(lost));
        }
        let mut candidates = Vec::new();
        while let Ok(candidate) = self.ice_rx.try_recv() {
            candidates.push(candidate);
        }
        Ok(candidates)
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    pub fn negotiated_video_codec(&self) -> Option<VideoCodec> {
        *self
            .negotiated_video_codec
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Encoded frames accepted by the session but not yet observed at the RTP
    /// packetizer. The session caps this count at two.
    pub fn video_frames_pending(&self) -> usize {
        self.video_frames_in_flight.load(Ordering::Relaxed)
    }

    /// Shared opt-in telemetry collector for application capture/render stages
    /// and the session's measured send/receive boundaries.
    pub fn telemetry(&self) -> &PipelineTelemetry {
        &self.telemetry
    }

    /// Push externally hardware-encoded H.264 Annex-B frames to the RTP track.
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
        let Some(video) = self.video.as_ref() else {
            return Err(video_submit_error(SessionError::MediaNotAvailable, frame));
        };
        if self
            .video_frames_in_flight
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                if count < MAX_VIDEO_FRAMES_IN_FLIGHT {
                    Some(count + 1)
                } else {
                    None
                }
            })
            .is_err()
        {
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
                self.video_failed.store(true, Ordering::Relaxed);
                Err(VideoSubmitError {
                    error: web_rtc_error(error),
                    frame: None,
                })
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

    pub async fn recv_input(&mut self) -> Result<UserInput> {
        let lost = self.dropped_input.swap(0, Ordering::Relaxed);
        if lost > 0 {
            return Err(SessionError::InputQueueOverflow(lost));
        }
        self.input_rx.recv().await.ok_or(SessionError::QueueClosed)
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

    pub async fn recv_control(&mut self) -> Result<Vec<u8>> {
        let lost = self.dropped_control.swap(0, Ordering::Relaxed);
        if lost > 0 {
            return Err(SessionError::ControlQueueOverflow(lost));
        }
        self.control_rx
            .recv()
            .await
            .ok_or(SessionError::QueueClosed)
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
        CONTROL_CHANNEL => {
            let control_tx = router.control_tx.clone();
            let dropped_control = router.dropped_control.clone();
            let telemetry = router.telemetry.clone();
            channel.on_message(move |data, binary| {
                let _timer = telemetry.time(Stage::WebRtcReceive);
                if binary
                    && data.len() <= MAX_CONTROL_MESSAGE
                    && control_tx.try_send(data.to_vec()).is_err()
                {
                    dropped_control.fetch_add(1, Ordering::Relaxed);
                }
            });
            *router
                .control_channel
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(channel);
        }
        _ => {}
    }
}

fn web_rtc_error(error: impl std::fmt::Display) -> SessionError {
    SessionError::WebRtc(error.to_string())
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

#[cfg(test)]
mod tests {
    use super::*;
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
            let mut offerer = GshSession::new(
                factory_offer,
                RtcConfiguration::default(),
                SessionRole::Offerer,
                320,
                240,
            )
            .unwrap();
            let mut answerer = GshSession::new(
                factory_answer,
                RtcConfiguration::default(),
                SessionRole::Answerer,
                320,
                240,
            )
            .unwrap();

            let offer = offerer.create_offer().unwrap();
            let answer = answerer.accept_offer(offer).unwrap();
            offerer.accept_answer(answer).unwrap();

            let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
            loop {
                for candidate in offerer.drain_ice_candidates().unwrap() {
                    answerer.add_ice_candidate(candidate).unwrap();
                }
                for candidate in answerer.drain_ice_candidates().unwrap() {
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
                let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
                while offerer.video_frames_pending() != 0 {
                    assert!(
                        tokio::time::Instant::now() < deadline,
                        "encoded frame did not reach the RTP packetizer"
                    );
                    tokio::time::sleep(Duration::from_millis(10)).await;
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
        });
    }
}
