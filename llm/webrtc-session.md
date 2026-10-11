# WebRTC session backend — implementation status

## Current code

`libgsh` has an opt-in `webrtc` feature backed by `reactor-webrtc 0.22.1`, with `GshSession` in `libgsh/src/webrtc.rs`.

- Offerer publishes pre-encoded video track and Opus audio track, and creates separate reliable `gsh-input-v1` and `gsh-control-v1` data channels.
- Answerer configures receive-only video/audio transceivers and consumes data channels created by the offerer.
- The application-facing signaling values are `SdpMessage` and `CandidateMessage`; the application owns WebSocket/HTTP signaling.
- `send_video` accepts `EncodedVideoFrame` as owned H.264 Annex-B bitstream bytes. Encode those frames outside the session. `send_audio` accepts PCM and feeds the factory's audio source.
- Input/control use protobuf payloads over data channels. Input and control receive queues have fixed capacities. Overflow is surfaced as `InputQueueOverflow(n)` or `ControlQueueOverflow(n)` on receive, requiring the application to reconcile input state or control state. Input send also applies a buffered-amount guard.
- An opt-in native loopback test creates two sessions, negotiates SDP, trickles host ICE candidates, confirms connection state, and delivers a protobuf input message over the data channel.

Basic compile/test commands:

```sh
cargo check -p libgsh --features webrtc
cargo test -p libgsh --features webrtc --lib
```

## Material gaps; do not claim end-to-end support yet

- The feature is opt-in and is not yet wired into `client`, `server`, or the remote-desktop example. The existing TCP/TLS/Prost path remains the default application path.
- No signaling server/client or browser client has been implemented.
- Receive media presentation, audio playback, decoder selection, local cursor, and remote-track frame callbacks are not wired.
- The local WebRTC loopback test only verifies ICE/SDP/data-channel input. It does not exercise a valid encoded media bitstream, audio, receiver decode/presentation, browser interop, TURN, congestion/loss, or GPU/codec performance.
- `reactor-webrtc`'s pre-encoded sender implementation queues frame ownership in its own FIFO and is not bounded by this wrapper. This session adds no safe drop policy for encoded reference frames; frame backpressure/recovery must be implemented and tested before deployment.
- `send_audio` is PCM submission to the stack; audio capture, receiver output, clocking, and synchronization are not implemented.
- Current pointer updates share the reliable ordered input channel. Callers should coalesce replaceable pointer positions before sending; keyboard/button transitions must not be coalesced away. Distinct unreliable high-rate pointer transport is not yet supported by the selected wrapper surface.
- The built native libwebrtc artifact was available in this host's Cargo build output, but the runtime has not been exercised. This does not establish target portability or reproducible artifact fetching.

## Selection status

`reactor-webrtc` remains the provisional choice due to its source-inspected pre-encoded RTP path and standard libwebrtc SDP/ICE/data-channel APIs. The dependency's Rust feature is optional to keep non-WebRTC consumers and baseline library tests from requiring a large native artifact while the backend is a prototype. Before making it default, record exact artifact source/checksum and license, verify Linux and Windows supported targets, run two-peer loopback tests, measure latency and allocations, and compare with the pure-Rust webrtc-rs stack on the same workload.

The implementation deliberately does not claim GPU-native surface transport: encoded bytes are the boundary, not GPU handles. Hardware encode/decode adapters and platform capture/presentation remain separate future milestones.
