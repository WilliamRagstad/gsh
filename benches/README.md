# Benchmarks

Criterion suites measure server setup, frame processing, authentication, and service simulations. Network and concurrent-load benchmarks remain unregistered placeholders; the integration suite does not exercise real client-server connections.

From the repository root:

```bash
./run_benchmarks.sh
cargo bench -p gsh-benchmarks --bench '*' -- --quick
cargo bench -p gsh-benchmarks --bench authentication
cargo bench -p libgsh --bench libgsh_internals
```

| Target | Coverage |
| --- | --- |
| `server_performance` | Server creation, hello messages, frame generation |
| `frame_processing` | Zstd compression/decompression, segmentation, processing pipeline |
| `authentication` | RSA keys/signatures, password hashing, challenge-response |
| `integration_tests` | Service metadata, server setup, repeated frame generation |
| `libgsh_internals` | Certificates, segmentation, compression |

Reports are written to `target/criterion/`. CI runs the implemented suites in quick mode and uploads results; it does not enforce performance thresholds.

Add benchmarks to an existing suite, or register a new `[[bench]]` target in `Cargo.toml` with `harness = false`. See the [Criterion documentation](https://docs.rs/criterion/) for measurement and reporting options.
