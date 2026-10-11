# Baseline performance report

## Baseline identity

- Source commit: `f929965` (`main` before rewrite branch).
- Workspace: `libgsh`, `client`, `examples/*`, `benches`.
- Host: `williams-pc-cachyos`, x86_64 Linux kernel `7.2.9-2-cachyos`; Rust `1.98.0` (LLVM 22.1.8). CPU/GPU model, graphics driver, power profile, and network topology were not captured at measurement time.
- Method available in tree: Criterion CPU benchmarks in `benches/benches/frame_processing.rs` (zstd, full-frame segment copy, combined segment+zstd); these do not measure streaming, WebRTC, capture, GPU, or display.

## Baseline run

Command: `cargo test --workspace`

Result: **blocked before project tests**. Build failed compiling cached `libspa v0.8.0` (transitive PipeWire dependency) at `pod/builder.rs:38-39`, because generated `spa_pod_builder` exposes `_address` rather than fields `data` and `size`. SDL3 and PipeWire sys crates were being compiled. No project test result is available from this run.

Command: `cargo bench -p gsh-benchmarks --bench frame_processing`

Executed 2026-10-11. Criterion recorded the following CPU-only intervals before the imposed 120-second timeout terminated the process (suite had reached 720p; 1080p/full-frame stages did not finish):

| Existing operation/input | Criterion interval |
|---|---:|
| zstd compress, QVGA 320x240 solid | 15.224–15.270 µs |
| zstd decompress, QVGA solid | 9.049–9.105 µs |
| zstd compress, VGA 640x480 solid | 68.612–68.777 µs |
| zstd compress, VGA 640x480 random | 104.15–104.77 µs |

These characterize isolated CPU compression only, on a host whose CPU model was not captured at measurement time. They are not end-to-end baseline results or evidence of WebRTC performance. The command ended due to tool timeout rather than Criterion failure.

## Reproduction and next steps

1. Record `rustc -Vv`, `cargo -V`, `uname -a`, CPU/GPU/driver, SDL/PipeWire versions, and network topology.
2. Resolve the dependency/build environment issue (do not patch registry source as a project fix); rerun `cargo test --workspace`.
3. Run `cargo bench -p gsh-benchmarks --bench frame_processing -- --sample-size 30` (or use a longer timeout) and preserve Criterion output.
4. The feature-gated WebRTC input loopback now validates SDP/ICE/data-channel behavior. Add media loopback and server/client presentation before reporting frame-to-display or input-to-photon percentiles. Cross-host measurements require synchronized clocks or a one-way latency method with uncertainty.

## Results

Instrumentation overhead microbenchmark (same host, release Criterion, 30 samples; `cargo bench -p gsh-benchmarks --bench frame_processing -- pipeline_stage_timer --sample-size 30`): disabled timer scope **7.968–7.987 ns**, enabled empty scope **56.628–56.678 ns**. This is a framework microbenchmark, not end-to-end stage overhead under contention. The timer is opt-in, records logarithmic histogram percentiles, and is wired into WebRTC send/receive and application-shared pipeline boundaries; render/capture/encoder/display call sites remain application responsibilities.

| Metric | Baseline |
|---|---|
| End-to-end input-to-photon p50/p95/p99 | Not measured; no instrumentation/test rig |
| Frame production/encode/network/decode/presentation percentiles | Not measured |
| FPS/drops/CPU/GPU/bandwidth/memory | Not measured |
| Workspace unit/integration tests | Blocked at dependency compile; standalone libgsh unit suite passed (see latest count below) |
| Criterion frame processing | Partial CPU-only results above; process timed out before finishing suite |
| Telemetry/framework tests | Default library test suite: 19 passed; WebRTC-feature suite: 20 passed (includes a real local SDP/ICE/data-channel input loopback) |
| Telemetry scope overhead | Disabled 7.968–7.987 ns; enabled 56.628–56.678 ns in the synthetic empty scope benchmark |
| WebRTC feature compile | `cargo check -p libgsh --features webrtc`: passed |
| WebRTC loopback | Two local peers negotiated SDP/ICE and exchanged input; 1 test passed (included in 20 feature-suite tests). This is not media/rendering performance. |

This report is deliberately explicit about unverified items. Update it with host metadata and comparable measurements at each verified milestone.
