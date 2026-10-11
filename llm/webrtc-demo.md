# Local browser WebRTC demo

Run:

```sh
cargo run -p webrtc_demo
```

Open `http://127.0.0.1:8787/`. The example serves a static browser client and WebSocket signaling endpoint. It sends a deterministic 320x240 VP8 keyframe at 30 FPS over the GSH WebRTC video track; the page presents the remote track and sends pointer movement over the input data channel. The server logs received protobuf input. The library loopback test additionally verifies local VP8 software decode to BGRA and Opus PCM decode.

This is a protocol/application integration smoke demo, not the remote desktop application: it does not capture a real app, render from a GPU, encode in hardware, play real audio, or dispatch input into an application. It must not be exposed as a production service. Signaling is plain WebSocket with no authentication, the server binds only to loopback, and the handler checks browser `Origin` when supplied. Native clients omitting Origin can still connect locally. WebRTC media/data is protected by DTLS/SRTP after negotiation.

Browser interoperability was not exercised in the current headless environment. A Node WebSocket smoke probe retrieved the offer; no browser `RTCPeerConnection` implementation was available to complete browser offer/answer, rendering, or input verification. The checked-in page uses the standard browser `RTCPeerConnection` API and queues remote ICE candidates until the offer is installed.
