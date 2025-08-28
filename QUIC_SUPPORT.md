# QUIC Support in GSH

This document describes the QUIC+TLS implementation in the Graphical Shell (GSH) project.

## Overview

GSH now supports both traditional TCP+TLS connections and modern QUIC+TLS connections for improved networking performance. QUIC provides:

- **Built-in TLS 1.3 encryption** - No separate TLS layer needed
- **Lower latency** - Reduced connection establishment overhead
- **Better performance on unreliable networks** - Advanced congestion control
- **Multi-stream support** - Multiple independent streams over a single connection
- **Connection migration** - Maintain connections when network changes

## Architecture

### Protocol Layers

```
Traditional:  GSH Protocol -> MessageCodec -> TLS -> TCP -> IP
QUIC:         GSH Protocol -> MessageCodec -> QUIC (TLS built-in) -> UDP -> IP
```

### Multi-Stream Design

QUIC supports multiple streams over a single connection:

- **Stream 0 (bidirectional)**: Control messages, handshake, status updates
- **Stream 1+ (unidirectional)**: Frame data for optimized performance

This allows frame data to be sent independently from control messages, improving performance when the GSH protocol doesn't require ordered delivery.

## Implementation

### Server Side

```rust
use libgsh::r#async::{quic_server::AsyncQuicServer, service::AsyncService};
use libgsh::quic::create_server_config;

// Create QUIC server configuration
let quic_config = create_server_config(cert_chain, private_key)?;

// Create server with your service
let server = AsyncQuicServer::new(your_service, quic_config);

// Run on custom port
server.serve_port(1123).await?;
```

### Client Side

```rust
use libgsh::client::network::connect_quic;

// Connect using QUIC
let (hello, messages) = connect_quic(
    "localhost", 
    1123,
    false, // secure connection
    monitors,
    known_hosts,
    id_files,
    None
).await?;
```

## Key Features

### 1. **Dual Protocol Support**
Both TLS and QUIC servers can run simultaneously on different ports:
- TLS server: `AsyncServer` on port 1122 (default)
- QUIC server: `AsyncQuicServer` on port 1123

### 2. **Compatible Service Interface**
Existing services work with both protocols through the `AsyncService` trait:

```rust
#[async_trait]
impl AsyncService for YourService {
    fn server_hello(&self) -> ServerHelloAck { /* ... */ }
    async fn main(self, messages: Messages) -> Result<()> { /* ... */ }
}
```

### 3. **Seamless Stream Abstraction**
The `AsyncMessageCodec<S>` works with any stream type that implements `AsyncRead + AsyncWrite + Send + Unpin`:
- `TlsStream<TcpStream>` for TCP+TLS
- `QuicStreamWrapper` for QUIC streams

### 4. **Performance Optimizations**
- QUIC's 0-RTT connection establishment for returning clients
- Multiplexed streams prevent head-of-line blocking
- Advanced congestion control algorithms
- Connection migration support

## Configuration

### Server Configuration

```rust
// Create certificate (same for both TLS and QUIC)
let (cert_key, private_key) = self_signed(&["localhost"])?;

// QUIC server config
let quic_config = create_server_config(
    vec![cert_key.cert.der().clone()], 
    private_key.clone_key()
)?;
```

### Client Configuration

```rust
// Insecure (skip certificate verification)
let client_config = create_client_config(true)?;

// Secure (verify certificates)
let client_config = create_client_config(false)?;
```

## Examples

### Running the Demo

1. **Start both servers**:
   ```bash
   cd examples/quic_demo
   cargo run
   ```

2. **Test QUIC connection**:
   ```bash
   cargo run --bin quic_client
   ```

### Integration in Existing Code

Existing GSH applications can add QUIC support with minimal changes:

```rust
// Before (TLS only)
let tls_server = AsyncServer::new(service, tls_config);
tls_server.serve().await?;

// After (Both TLS and QUIC)
let tls_server = AsyncServer::new(service.clone(), tls_config);
let quic_server = AsyncQuicServer::new(service, quic_config);

tokio::select! {
    _ = tls_server.serve_port(1122) => {},
    _ = quic_server.serve_port(1123) => {},
}
```

## Performance Benefits

1. **Reduced Latency**: QUIC's 0-RTT establishment vs TCP's 3-way handshake + TLS handshake
2. **Better Congestion Control**: Modern algorithms vs traditional TCP
3. **Connection Resilience**: Survives IP address changes (mobile networks)
4. **Multiplexed Streams**: No head-of-line blocking between different message types
5. **Efficient Retransmission**: Stream-level vs connection-level recovery

## Migration Path

1. **Phase 1**: Add QUIC support alongside existing TLS (✅ Complete)
2. **Phase 2**: Update services to leverage multi-stream capabilities
3. **Phase 3**: Optimize frame transmission using dedicated QUIC streams
4. **Phase 4**: Consider deprecating TCP+TLS in favor of QUIC

## Future Enhancements

- [ ] Implement frame data streaming on dedicated QUIC streams
- [ ] Add connection pooling and reuse
- [ ] Implement QUIC-specific error handling
- [ ] Add metrics and monitoring for QUIC connections
- [ ] Support for QUIC connection migration
- [ ] Integration with existing GSH authentication mechanisms

## Testing

The implementation includes:
- Basic connectivity tests
- Protocol handshake verification
- Multi-stream capability demonstration
- Performance comparison tools (planned)

## Dependencies

- `quinn`: Rust QUIC implementation
- `tokio`: Async runtime
- `rustls`: TLS implementation (used by quinn)
- `anyhow`: Error handling

## Compatibility

- **Protocol Version**: Compatible with existing GSH protocol
- **Authentication**: Works with all existing auth methods
- **Services**: All existing `AsyncService` implementations supported
- **Certificates**: Uses the same certificate infrastructure as TLS