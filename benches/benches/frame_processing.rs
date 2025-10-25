use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use libgsh::{frame::full_frame_segment, shared::protocol::Frame, zstd};
use rand::{Rng, SeedableRng};
use rand::rngs::StdRng;

fn generate_test_frame_data(size: usize, pattern: &str) -> Vec<u8> {
    match pattern {
        "random" => {
            let mut rng = StdRng::seed_from_u64(42);
            (0..size).map(|_| rng.gen::<u8>()).collect()
        },
        "gradient" => {
            (0..size).map(|i| (i % 256) as u8).collect()
        },
        "solid" => vec![128u8; size],
        _ => vec![0u8; size],
    }
}

fn bench_frame_compression(c: &mut Criterion) {
    let mut group = c.benchmark_group("frame_compression");
    
    let frame_sizes = [
        (320, 240, 4, "QVGA"),
        (640, 480, 4, "VGA"), 
        (1280, 720, 4, "720p"),
        (1920, 1080, 4, "1080p"),
    ];
    
    let patterns = ["solid", "gradient", "random"];
    
    for (width, height, channels, resolution) in frame_sizes.iter() {
        let size = width * height * channels;
        
        for pattern in patterns.iter() {
            let frame_data = generate_test_frame_data(size, pattern);
            
            group.bench_with_input(
                BenchmarkId::new(format!("zstd_compress_{}_{}", resolution, pattern), size),
                &frame_data,
                |b, data| {
                    b.iter(|| {
                        let compressed = zstd::encode_all(black_box(&data[..]), 1).unwrap();
                        black_box(compressed)
                    })
                },
            );
            
            // Also benchmark decompression
            let compressed = zstd::encode_all(&frame_data[..], 1).unwrap();
            group.bench_with_input(
                BenchmarkId::new(format!("zstd_decompress_{}_{}", resolution, pattern), size),
                &compressed,
                |b, data| {
                    b.iter(|| {
                        let decompressed = zstd::decode_all(black_box(&data[..])).unwrap();
                        black_box(decompressed)
                    })
                },
            );
        }
    }
    
    group.finish();
}

fn bench_frame_segmentation(c: &mut Criterion) {
    let mut group = c.benchmark_group("frame_segmentation");
    
    let frame_sizes = [
        (640, 480, 4, "VGA"),
        (1920, 1080, 4, "1080p"),
    ];
    
    for (width, height, channels, resolution) in frame_sizes.iter() {
        let size = width * height * channels;
        let frame_data = generate_test_frame_data(size, "gradient");
        
        let frame = Frame {
            window_id: 0,
            data: frame_data,
        };
        
        group.bench_with_input(
            BenchmarkId::new("full_frame_segment", resolution),
            &frame,
            |b, frame| {
                b.iter(|| {
                    let segment = full_frame_segment(black_box(frame.clone()));
                    black_box(segment)
                })
            },
        );
    }
    
    group.finish();
}

fn bench_frame_processing_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("frame_processing_pipeline");
    
    let frame_data = generate_test_frame_data(1920 * 1080 * 4, "gradient");
    
    group.bench_function("complete_pipeline", |b| {
        b.iter(|| {
            // Simulate complete frame processing: create frame -> segment -> compress
            let frame = Frame {
                window_id: 0,
                data: black_box(frame_data.clone()),
            };
            
            let segment = full_frame_segment(frame);
            let compressed = zstd::encode_all(&segment.data[..], 1).unwrap();
            black_box(compressed)
        })
    });
    
    group.finish();
}

criterion_group!(
    frame_benches,
    bench_frame_compression,
    bench_frame_segmentation,
    bench_frame_processing_pipeline
);
criterion_main!(frame_benches);