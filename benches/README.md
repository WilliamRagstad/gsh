# GSH Benchmarks and Performance Testing

This directory contains comprehensive benchmarks and performance tests for the GSH (Graphical Shell) project. The benchmark suite is designed to measure performance across different components and usage scenarios to guide optimization efforts.

## 📋 Overview

The benchmark suite covers the following areas:

- **Server Performance**: Async vs Simple server implementations under load
- **Frame Processing**: Graphics data compression, segmentation, and throughput
- **Authentication**: RSA operations, password hashing, and challenge-response
- **Client-Server Communication**: Protocol performance and latency
- **Concurrent Load**: Multiple client connections and resource usage
- **Integration Tests**: End-to-end performance scenarios

## 🚀 Quick Start

### Running All Benchmarks
```bash
# From project root
./run_benchmarks.sh
```

### Running Individual Benchmark Categories
```bash
cd benches

# Server performance benchmarks
cargo bench --bench server_performance

# Frame processing benchmarks  
cargo bench --bench frame_processing

# Authentication benchmarks
cargo bench --bench authentication

# Integration tests
cargo bench --bench integration_tests
```

### Quick Development Testing
```bash
# Fast benchmarks for development (less accurate but faster)
cargo bench -- --quick
```

## 📊 Benchmark Categories

### 1. Server Performance (`server_performance.rs`)
Tests the core server implementation performance:
- Async server creation and startup
- Frame generation at different resolutions
- Server hello message creation
- Resource allocation patterns

**Key Metrics**: Server creation time, frame generation throughput, memory allocation

### 2. Frame Processing (`frame_processing.rs`)
Measures graphics and data processing performance:
- Frame compression (Zstd) at various resolutions and patterns
- Frame segmentation algorithms
- Complete processing pipelines (segment → compress)

**Key Metrics**: Compression ratio, processing latency, throughput MB/s

### 3. Authentication (`authentication.rs`)
Evaluates security operation performance:
- RSA keypair generation (2048, 3072, 4096 bit)
- Digital signature creation and verification
- Password hashing (SHA256)
- Challenge-response authentication flows

**Key Metrics**: Key generation time, signature ops/sec, hash operations/sec

### 4. Integration Tests (`integration_tests.rs`)
End-to-end performance scenarios:
- Complete server lifecycle simulation
- High-frequency frame generation
- Multi-resolution server configuration

**Key Metrics**: End-to-end latency, sustained throughput, resource efficiency

## 🔧 Configuration

### Benchmark Server
The `BenchmarkServer` in `src/benchmark_server.rs` provides a configurable test server that:
- Generates predictable frame patterns for consistent testing
- Supports various resolutions and data sizes
- Implements the full GSH async service interface
- Provides controlled load scenarios

### Custom Test Scenarios
Create new benchmarks by:

1. Adding new benchmark functions in the appropriate category file
2. Using the `BenchmarkServer` or creating specialized test fixtures
3. Following the criterion.rs patterns for measurement

Example:
```rust
fn bench_custom_scenario(c: &mut Criterion) {
    c.bench_function("my_scenario", |b| {
        let server = BenchmarkServer::new(1920, 1080);
        b.iter(|| {
            // Your benchmark code here
            black_box(server.server_hello())
        })
    });
}
```

## 📈 Interpreting Results

### HTML Reports
Benchmarks generate detailed HTML reports in `target/criterion/`:
- Performance graphs over time
- Statistical analysis (mean, median, std dev)
- Comparison with previous runs
- Regression detection

### Key Performance Indicators

**Server Performance**:
- Server creation: < 1ms (target)
- Frame generation: > 60 FPS for 1080p
- Memory efficiency: minimal allocations

**Frame Processing**:
- Compression ratio: > 50% for typical content
- Processing latency: < 16ms (60 FPS budget)
- Throughput: > 100 MB/s sustained

**Authentication**:
- Key generation: < 100ms for 2048-bit RSA
- Signature verification: < 1ms
- Hash operations: > 10,000 ops/sec

## 🏗️ Adding New Benchmarks

1. **Identify the component** to benchmark
2. **Choose the appropriate category** file or create a new one
3. **Write focused benchmarks** that test specific operations
4. **Use realistic data sizes** and scenarios
5. **Add documentation** for the benchmark purpose

Example workflow:
```bash
# Add benchmark to existing file
vim benches/server_performance.rs

# Or create new benchmark category
cargo new --name my_benchmark benches/my_benchmark.rs

# Update Cargo.toml
vim Cargo.toml  # Add [[bench]] entry

# Test the benchmark
cargo bench --bench my_benchmark -- --quick
```

## 🔄 Continuous Integration

Benchmarks run automatically in CI on every PR:
- Quick mode for fast feedback
- Full benchmarks on main branch
- Results uploaded as artifacts
- Performance regression detection

## 🎯 Performance Targets

### Current Performance Goals

| Component | Metric | Target | Current |
|-----------|--------|--------|---------|
| Server Startup | Time | < 10ms | TBD |
| Frame Processing | 1080p@60fps | < 16ms/frame | TBD |
| Authentication | RSA Sign | < 5ms | TBD |
| Compression | Ratio | > 60% | TBD |

*TBD: To Be Determined through initial benchmark runs*

## 🐛 Troubleshooting

### Common Issues

**Benchmark compilation errors**:
```bash
# Ensure all dependencies are available
cargo check --benches
```

**Port conflicts in integration tests**:
```bash
# Run benchmarks sequentially
cargo bench --jobs 1
```

**Insufficient permissions**:
```bash
# Some benchmarks may need additional system permissions
sudo ./run_benchmarks.sh
```

### Performance Debugging

1. **Profile individual operations** using criterion's built-in profiling
2. **Use flamegraph integration** for CPU profiling
3. **Monitor memory usage** with valgrind or heaptrack
4. **Check system resources** during benchmark runs

## 📚 Resources

- [Criterion.rs Documentation](https://docs.rs/criterion/)
- [Rust Performance Book](https://nnethercote.github.io/perf-book/)
- [GSH Architecture Documentation](../libgsh/README.md)

## 🤝 Contributing

When adding benchmarks:
- Follow existing patterns and naming conventions
- Document the purpose and expected performance characteristics
- Include both micro-benchmarks and integration scenarios
- Test on representative hardware configurations

For questions or suggestions about the benchmark suite, please open an issue or discussion on the GSH repository.