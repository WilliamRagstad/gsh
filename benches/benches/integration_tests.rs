use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use gsh_benchmarks::BenchmarkServer;
use libgsh::{
    server::GshService,
    shared::cert,
    tokio,
    tokio_rustls::rustls::{crypto::ring, ServerConfig},
};

fn setup_server_config() -> ServerConfig {
    let (key, private_key) = cert::self_signed(&["localhost"]).unwrap();
    // A process-wide provider can only be installed once.
    let _ = ring::default_provider().install_default();
    ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![key.cert.der().clone()], private_key)
        .unwrap()
}

fn bench_server_hello_performance(c: &mut Criterion) {
    let mut group = c.benchmark_group("server_hello_performance");

    for &(width, height, name) in &[
        (320, 240, "QVGA"),
        (640, 480, "VGA"),
        (1280, 720, "720p"),
        (1920, 1080, "1080p"),
    ] {
        group.bench_with_input(
            BenchmarkId::new("server_hello_creation", name),
            &(width, height),
            |b, &(w, h)| {
                let server = BenchmarkServer::new(w, h);
                b.iter(|| {
                    let hello = server.server_hello();
                    black_box(hello)
                })
            },
        );
    }

    group.finish();
}

fn bench_end_to_end_server_lifecycle(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("complete_server_setup", |b| {
        b.to_async(&rt).iter(|| async {
            let config = setup_server_config();
            let server = BenchmarkServer::new(640, 480);
            let hello = server.server_hello();
            let async_server = server.create_async_server(config);

            black_box((async_server, hello))
        })
    });
}

fn bench_high_frequency_frame_simulation(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("high_frequency_frames", |b| {
        b.to_async(&rt).iter(|| async {
            let mut server = BenchmarkServer::new(300, 300);
            let mut frame_count = 0;

            for _ in 0..10 {
                server.frame_count += 1;
                frame_count += 1;

                let width = server.width;
                let height = server.height;
                let mut frame_data = vec![0u8; width * height * 4];

                for y in 0..height {
                    for x in 0..width {
                        let idx = (y * width + x) * 4;
                        frame_data[idx] = ((x + frame_count) * 255 / width) as u8; // R
                        frame_data[idx + 1] = ((y + frame_count) * 255 / height) as u8; // G
                        frame_data[idx + 2] = 128; // B
                        frame_data[idx + 3] = 255; // A
                    }
                }

                black_box(frame_data);
            }

            black_box(server)
        })
    });
}

criterion_group!(
    integration_benches,
    bench_server_hello_performance,
    bench_end_to_end_server_lifecycle,
    bench_high_frequency_frame_simulation
);
criterion_main!(integration_benches);
