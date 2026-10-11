# GSH architecture audit (pre-rewrite)

Audit scope: repository at baseline commit `f929965`; controlling rewrite decision: [`WebRTC_FIRST.md`](WebRTC_FIRST.md). This is a source-level audit, not a claim that GPU or end-to-end streaming was measured.

## Existing path

The workspace is a single `libgsh` library plus an SDL3 client, examples, and Criterion benchmarks. The server accepts TCP, wraps it in rustls, exchanges length-prefixed Prost messages, then dispatches a service callback loop. Frames are `Frame { segments: Vec<Segment> }` messages. The remote-desktop example captures CPU image data, segments and zstd-compresses it, then sends it as a reliable ordered protocol message. The client receives that stream, allocates a target texture per frame, decompresses/copies segment bytes, updates the texture, and presents it through SDL.

Input travels from SDL event handling through individual awaited Prost/TLS writes on the same connection. The client loop waits for one server message before polling input; so input responsiveness can be coupled to server message availability. Server input and ticks share one service loop.

## Hot-path inventory

| Stage | Current representation / ownership | Cost and risk |
|---|---|---|
| Capture | CPU `Vec<u8>` in remote desktop example | Full-frame readback/CPU residency; capture details are backend-dependent. |
| Diff/segment | `Vec<Segment>` with owned `Vec<u8>` per segment | Row-sized and segment copies; 50-segment fallback copies full frame. |
| Compression | zstd per segment/frame | General lossless compression, not video; CPU cost and latency may dominate motion. |
| Serialization | Prost encode to new `Vec`, then another length-prefixed `Vec` | At least two serialized buffers and one reliable stream write per message. |
| Transport | TCP + TLS ordered byte stream | Head-of-line blocking; no media deadline or frame discard semantics. |
| Receive | resize buffer from peer-supplied u32 length | No visible protocol maximum before allocation; read timeout is 10ms and partial-read handling needs review. |
| Decode | zstd to a new output Vec, or clone uncompressed segment | Per-segment allocation/copy. |
| Present | SDL target texture created for each frame; update then copy/present | Texture churn, CPU upload, full-canvas clear; no GPU decoder surface. |
| Input | Event writes await per event on the same stream | High-frequency pointer updates can queue/block behind media; no replacement/latest-state behavior. |
| Scheduling | fixed 60 FPS constants in service and client | Client polls server before events; loop sleeps after work (drift), no target configuration. |
| Queues/threads | Tokio connection task; synchronous rendering/decompression in client loop | Blocking media work shares async task; no bounded media pipeline/pool. |

`PrevFrame` has ping-pong storage, but `optimize_segments` creates row copies and clones segments, and the active remote-desktop path must be checked independently before treating it as an optimization.

## Architecture decision

Follow `WebRTC_FIRST.md`: one concrete WebRTC session backend, RTP video/audio, data channels for input/control, minimal separate signaling, H.264 baseline and Opus audio. Do not build generic transport traits or alternate networking backends in v1. Keep capture, encoded-frame ownership, codec, session semantics, and signaling decoupled. GPU-native resources cannot be carried through a byte-slice-only API; explicit CPU/encoded/GPU frame variants and synchronization/ownership must be designed before claiming zero-copy.

## WebRTC implementation evaluation (initial source review)

Candidates: `webrtc` (webrtc-rs pure Rust API), `str0m` (sans-I/O protocol engine), and `reactor-webrtc` (safe Rust wrapper over Google libwebrtc). `str0m` leaves socket/ICE orchestration to its caller. `reactor-webrtc 0.22.1` documents PeerConnection, RTP tracks, data channels, stats, ICE/DTLS/SCTP, browser-compatible SDP, and a pre-encoded track API. Its `examples/pre_encoded.rs` demonstrates injecting externally encoded H.264 without pixel encoding inside libwebrtc. This matches GPU encoder → RTP path more directly than a raw-pixel-only API.

The pre-encoded path accepts an owned `Vec<u8>`, queues it, and triggers libwebrtc's encoder thread with a dummy BGRA frame; it is not zero-copy and the inspected implementation uses a `VecDeque` without exposed bounded/latest-frame policy. Native GPU surfaces would require a custom encoder integration or platform adapter and cannot be inferred from the Rust API. The wrapper downloads a target-specific libwebrtc prebuilt at build time; supply-chain, licensing, supported target triples, reproducibility, package size, and fallback behavior must be evaluated before depending on it.

Provisional path: prototype `reactor-webrtc` first because it is the inspected candidate exposing pre-encoded RTP track injection and a safe libwebrtc API. This is not a final production dependency decision until reproducible builds and loopback/browser interop, latency, memory, and queue behavior are tested. Sources inspected: crates.io `reactor-webrtc 0.22.1` (`examples/pre_encoded.rs`, `src/encoded.rs`, `src/peer_connection.rs`) and its README API/target table. Pure Rust alternatives remain candidates for comparison, not parallel implementations.

## Known correctness/security concerns to address

- Bound protocol/frame sizes before allocation and validate dimensions/stride/segment ranges before decoding or presentation.
- Avoid `unwrap` in untrusted handshake and connection-task paths.
- Avoid a client loop that services input only after network receive returns.
- Keep reliable key/button transitions distinct from replaceable pointer positions; never lose relative deltas.
- Don't describe successful submission/present as photons observed. Cross-host one-way timing needs clock sync and uncertainty reporting.

## Baseline constraints and remediation

The original Criterion frame benchmark measures CPU zstd/segment helpers only, not capture → network → display. The first workspace build/test on baseline dependencies failed in cached `libspa 0.8.0` through `xcap 0.5.2`; upgrading the remote-desktop example to `xcap 0.9.8` (PipeWire/libspa 0.10.1) fixed this, and current workspace build/tests pass. The experimental WebRTC backend has a local peer test for SDP/ICE, VP8 RTP packetization and software decoding, Opus decoding to PCM, and data-channel input; a Criterion loopback measures those component paths. Native app integration, GPU display/audio output, browser, TURN/loss recovery, and input-to-photon performance remain unverified. See [`baseline.md`](baseline.md). No latency claim is made.
