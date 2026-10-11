# libgsh

Rust SDK for services that render graphics and handle input from the `gsh` client.

## Quick Install

Add the library to your application:

```bash
cargo add libgsh
```

## Build a Server

Implement `GshService` to provide window settings, optional authentication, and a per-client event loop. `GshServiceExt` supplies startup, tick, input, and exit hooks. Pass your service and a Rustls `ServerConfig` to `GshServer::new`, then call `serve().await` (port 1122) or `serve_port(port).await`.

See the [examples](../examples/) and [API reference](https://docs.rs/libgsh/latest/libgsh/).

## Protocol

The client and server negotiate window settings over TLS, authenticate if requested, then exchange input events and frame updates. Message definitions are in [protocol.proto](src/shared/protocol.proto).

## Build Requirements

- [Rust](https://www.rust-lang.org/tools/install) and [`protoc`](https://github.com/protocolbuffers/protobuf/releases/).
- Windows: MSVC C++ build tools.
- Linux workspace builds: native dependencies listed in [the CI workflow](../.github/workflows/rust.yml).
