# WebRTC session backend — implementation status

## Current code

`libgsh` has an opt-in `webrtc` feature backed by `reactor-webrtc 0.22.1`, with `GshSession` in `libgsh/src/webrtc.rs`.

- Offerer publishes pre-encoded video track and Opus audio track, and creates separate reliable `gsh-input-v1` and `gsh-control-v1` data channels. The default codec preference is H.264 first, VP8 fallback; applications can supply an explicit codec order. The negotiated payload is parsed and exposed as `negotiated_video_codec()`, and `send_video` rejects frames labeled with a different codec.
- Answerer configures receive-only video/audio transceivers and consumes data channels created by the offerer.
- The application-facing signaling values are `SdpMessage` and `CandidateMessage`; the application owns WebSocket/HTTP signaling.
- `send_video` accepts an owned `EncodedVideoFrame` tagged H.264 or VP8. Encode the payload outside the session; the payload must match the negotiated codec. H.264 is used when both peers/build support it; VP8 is a functional compatibility fallback. `send_audio` accepts PCM and feeds the factory's audio source.
- The receiver exposes the latest decoded BGRA frame through a one-slot `watch` channel and decoded Opus PCM through a bounded 64-chunk queue. These are CPU-owned buffers; the application still performs GPU upload/presentation and audio device playback.
- The external encoder receives bounded feedback events through `recv_encoder_feedback()`: keyframe requests and congestion bitrate/frame-rate updates. Applications must apply these to their encoder; the session does not implement rate control or recovery itself.
- Input/control use protobuf payloads over data channels. Input and control receive queues have fixed capacities. Overflow is surfaced as `InputQueueOverflow(n)` or `ControlQueueOverflow(n)` on receive, requiring the application to reconcile input state or control state. Input send also applies a buffered-amount guard.
- An opt-in native loopback test creates two sessions, negotiates SDP, trickles host ICE candidates, sends a generated VP8 keyframe and synthetic PCM, receives a software-decoded BGRA frame and Opus PCM, and delivers protobuf input over the data channel.

Basic compile/test commands:

```sh
cargo check -p libgsh --features webrtc
cargo test -p libgsh --features webrtc --lib
cargo bench -p gsh-benchmarks --features webrtc --bench webrtc_loopback -- --sample-size 20
```

The loopback Criterion harness separately measures input data-channel dispatch and VP8 pre-encoded-to-software-decoded-frame latency after session setup. These are same-host component measurements, not input-to-photon or presentation measurements; see [`baseline.md`](baseline.md) for current host/results and limitations.

## Material gaps; do not claim end-to-end support yet

- The feature is opt-in and is not yet wired into `client`, `server`, or the remote-desktop example. The existing TCP/TLS/Prost path remains the default application path.
- No signaling server/client or browser client has been implemented.
- Receiver video/audio decoding is exercised by the loopback test, but display/audio-device presentation, hardware decoder selection, local cursor, and GPU-surface handoff are not wired.
- The local WebRTC loopback test exercises one valid VP8 access unit, Opus PCM, and input data-channel delivery. It does not test browser interop, TURN, congestion/loss, hardware codec paths, or latency distributions.
- `reactor-webrtc`'s pre-encoded sender implementation queues frame ownership in its own FIFO. The session limits accepted frames to two until the sender transform observes the encoded frame immediately before RTP packetization; when full, `send_video` returns the original frame for retry. It never drops an accepted reference frame by design. A generated VP8 keyframe passed through the transform in the loopback test. The limit does not bound the WebRTC network sender queue; keyframe/rate feedback is exposed but app-level encoder recovery has not been tested under packet loss.
- If the upstream `push_frame` call fails after consuming frame ownership, the session is marked unusable for video and returns an error without the consumed frame; callers must recreate the session rather than blindly retrying and risking duplicated reference frames.
- `send_audio` is PCM submission to the stack; audio capture, receiver device output, A/V clock synchronization, and drift correction are not implemented.
- Current pointer updates share the reliable ordered input channel. Callers should coalesce replaceable pointer positions before sending; keyboard/button transitions must not be coalesced away. Distinct unreliable high-rate pointer transport is not yet supported by the selected wrapper surface.
- The built native libwebrtc artifact was available in this host's Cargo build output and local-runtime tested. This does not establish target portability or reproducible artifact fetching.

## Selection status

`reactor-webrtc` remains the provisional choice due to its source-inspected pre-encoded RTP path and standard libwebrtc SDP/ICE/data-channel APIs. The dependency's Rust feature is optional to keep non-WebRTC consumers and baseline library tests from requiring a large native artifact while the backend is a prototype. Before making it default, record exact artifact source/checksum and license, verify Linux and Windows supported targets, measure latency and allocations, and compare with the pure-Rust webrtc-rs stack on the same workload.

The implementation deliberately does not claim GPU-native surface transport: encoded bytes are the boundary, not GPU handles. Hardware encode/decode adapters and platform capture/presentation remain separate future milestones.
