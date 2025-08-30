use criterion::{black_box, criterion_group, criterion_main, Criterion};
use libgsh::{
    cert::self_signed,
    frame::full_frame_segment,
    zstd,
};

fn bench_cert_generation(c: &mut Criterion) {
    c.bench_function("self_signed_cert_generation", |b| {
        b.iter(|| {
            let result = self_signed(&["localhost", "127.0.0.1"]);
            black_box(result)
        })
    });
}

fn bench_frame_segmentation(c: &mut Criterion) {
    let frame_data = vec![128u8; 640 * 480 * 4]; // VGA RGBA
    
    c.bench_function("frame_segmentation", |b| {
        b.iter(|| {
            let segments = full_frame_segment(
                black_box(&frame_data),
                black_box(640),
                black_box(480),
            );
            black_box(segments)
        })
    });
}

fn bench_compression(c: &mut Criterion) {
    let data = vec![128u8; 640 * 480 * 4]; // VGA RGBA
    
    c.bench_function("zstd_compression", |b| {
        b.iter(|| {
            let compressed = zstd::encode_all(black_box(&data[..]), 1);
            black_box(compressed)
        })
    });
    
    let compressed = zstd::encode_all(&data[..], 1).unwrap();
    c.bench_function("zstd_decompression", |b| {
        b.iter(|| {
            let decompressed = zstd::decode_all(black_box(&compressed[..]));
            black_box(decompressed)
        })
    });
}

criterion_group!(
    internal_benches,
    bench_cert_generation,
    bench_frame_segmentation,
    bench_compression
);
criterion_main!(internal_benches);