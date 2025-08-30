use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use gsh_benchmarks::BenchmarkServer;
use libgsh::{
    cert,
    tokio_rustls::rustls::{crypto::ring, ServerConfig},
    tokio,
};
use std::time::Duration;

fn setup_server_config() -> ServerConfig {
    let (key, private_key) = cert::self_signed(&["localhost"]).unwrap();
    ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");
    ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![key.cert.der().clone()], private_key)
        .unwrap()
}

fn bench_server_creation(c: &mut Criterion) {    
    c.bench_function("async_server_creation", |b| {
        b.iter(|| {
            let config = setup_server_config();
            let server = BenchmarkServer::default();
            black_box(server.create_async_server(config))
        })
    });
}

fn bench_server_startup_performance(c: &mut Criterion) {
    let mut group = c.benchmark_group("server_startup");
    
    for &size in &[300, 640, 1280] {
        group.bench_with_input(
            BenchmarkId::new("async_server_creation", size),
            &size,
            |b, &dimension| {
                b.iter(|| {
                    let config = setup_server_config();
                    let server = BenchmarkServer::new(dimension, dimension);
                    let async_server = server.create_async_server(config);
                    
                    // Time server creation (but don't actually serve to avoid port conflicts)
                    black_box(async_server)
                })
            },
        );
    }
    
    group.finish();
}

fn bench_frame_generation(c: &mut Criterion) {
    let mut group = c.benchmark_group("frame_generation");
    
    for &(width, height, name) in &[
        (300, 300, "300x300"),
        (640, 480, "VGA"),
        (1280, 720, "720p"),
        (1920, 1080, "1080p"),
    ] {
        group.bench_with_input(
            BenchmarkId::new("generate_gradient_frame", name),
            &(width, height),
            |b, &(w, h)| {
                b.iter(|| {
                    // Simulate gradient frame generation like in benchmark server
                    let mut frame_data = vec![0u8; w * h * 4]; // RGBA
                    for y in 0..h {
                        for x in 0..w {
                            let idx = (y * w + x) * 4;
                            frame_data[idx] = (x * 255 / w) as u8;     // R
                            frame_data[idx + 1] = (y * 255 / h) as u8; // G
                            frame_data[idx + 2] = 128;                 // B
                            frame_data[idx + 3] = 255;                 // A
                        }
                    }
                    black_box(frame_data)
                })
            },
        );
    }
    
    group.finish();
}

fn bench_server_hello_creation(c: &mut Criterion) {
    let server = BenchmarkServer::default();
    
    c.bench_function("server_hello_creation", |b| {
        b.iter(|| {
            let hello = server.server_hello();
            black_box(hello)
        })
    });
}

criterion_group!(
    server_benches,
    bench_server_creation,
    bench_server_startup_performance,
    bench_frame_generation,
    bench_server_hello_creation
);
criterion_main!(server_benches);