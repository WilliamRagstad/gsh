# Remote Desktop

Captures the primary monitor once and shares a bounded latest-frame source across peers. WebRTC is the default transport. The browser completes the existing GSH protobuf password or RSA-signature handshake over trusted WSS before sending SDP/ICE. WebRTC encrypts media and data channels with DTLS/SRTP; signaling integrity binds the peer fingerprints to the authenticated GSH session. ICE uses direct candidates only; no STUN/TURN service is configured.

The server requires a certificate chain and key. Install the issuing private CA in browser and native-client trust stores. The browser RSA flow accepts the existing combined GSH PEM through a local file picker; the private key remains in browser memory, and only its public key and challenge signature are sent. `ServerHelloAck` advertises one authentication method per deployment, matching existing GSH protocol semantics.

Configure exactly one authentication method and an exact HTTPS origin allowlist:

```sh
export GSH_TLS_CERT_FILE=/etc/gsh/server-chain.pem
export GSH_TLS_KEY_FILE=/etc/gsh/server-key.pem
export GSH_ALLOWED_ORIGINS=https://desktop.example.test:8787
export GSH_PASSWORD_FILE=/etc/gsh/remote-desktop.password
# Or, instead of GSH_PASSWORD_FILE:
# export GSH_AUTHORIZED_KEYS_FILE=/etc/gsh/authorized-rsa-public-keys.pem
cargo run --release -p remote_desktop
```

Open `https://desktop.example.test:8787/`. Set `GSH_WSS_ADDR` to change bind address; set `GSH_ALLOWED_ORIGINS` to comma-separated trusted browser origins. WSS rejects missing/unlisted `Origin`, allows at most 4 unauthenticated and 16 total sessions, limits each peer IP to 5 in-flight/failed auth attempts per minute, and applies a 60-second total GSH-auth and 30-second WebRTC-negotiation deadline. ICE candidates and SDP candidate lists are size/count capped. Native clients must provide an allowlisted `Origin` for this endpoint. Deprecated TCP/TLS transport remains opt-in: `cargo run --release -p remote_desktop -- --legacy-tcp`.

Current media path uses libwebrtc VP8 software encoding, up to 60 FPS. Linux/NVIDIA NVENC integration remains in progress. The application input hook currently logs normalized GSH events; OS-level input injection is not implemented. Do not expose this service without private-CA trust, strong credentials, and network access controls.
