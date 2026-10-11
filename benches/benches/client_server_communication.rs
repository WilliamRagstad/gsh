use criterion::{criterion_group, criterion_main, Criterion};

fn bench_placeholder(c: &mut Criterion) {
    c.bench_function("placeholder", |b| {
        b.iter(|| {
        })
    });
}

criterion_group!(comm_benches, bench_placeholder);
criterion_main!(comm_benches);
