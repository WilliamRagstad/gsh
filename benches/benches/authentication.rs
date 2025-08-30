use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use libgsh::{
    rsa::{
        pkcs1v15::{SigningKey, VerifyingKey},
        pkcs8::DecodePrivateKey,
        RsaPrivateKey, RsaPublicKey,
    },
    sha2::{Digest, Sha256},
    rsa::signature::{Signer, Verifier, RandomizedSigner},
};
use rand::rngs::OsRng;

fn generate_rsa_keypair(bits: usize) -> (RsaPrivateKey, RsaPublicKey) {
    let mut rng = OsRng;
    let private_key = RsaPrivateKey::new(&mut rng, bits).unwrap();
    let public_key = RsaPublicKey::from(&private_key);
    (private_key, public_key)
}

fn bench_keypair_generation(c: &mut Criterion) {
    let mut group = c.benchmark_group("keypair_generation");
    
    for key_size in [2048, 3072, 4096].iter() {
        group.bench_with_input(
            BenchmarkId::new("rsa_keygen", key_size),
            key_size,
            |b, &bits| {
                b.iter(|| {
                    let keypair = generate_rsa_keypair(black_box(bits));
                    black_box(keypair)
                })
            },
        );
    }
    
    group.finish();
}

fn bench_digital_signature(c: &mut Criterion) {
    let mut group = c.benchmark_group("digital_signature");
    
    let (private_key, public_key) = generate_rsa_keypair(2048);
    let signing_key = SigningKey::<Sha256>::new(private_key);
    let verifying_key = VerifyingKey::<Sha256>::new(public_key);
    
    let message = b"Hello, this is a test message for digital signature benchmarking!";
    let mut rng = OsRng;
    
    group.bench_function("rsa_sign", |b| {
        b.iter(|| {
            let signature = signing_key.sign_with_rng(&mut rng, black_box(message));
            black_box(signature)
        })
    });
    
    let signature = signing_key.sign_with_rng(&mut rng, message);
    
    group.bench_function("rsa_verify", |b| {
        b.iter(|| {
            let result = verifying_key.verify(black_box(message), black_box(&signature));
            black_box(result)
        })
    });
    
    group.finish();
}

fn bench_password_hashing(c: &mut Criterion) {
    let mut group = c.benchmark_group("password_hashing");
    
    let passwords = [
        "password123",
        "very_long_password_with_special_chars!@#$%",
        "short",
    ];
    
    for password in passwords.iter() {
        group.bench_with_input(
            BenchmarkId::new("sha256_hash", password.len()),
            password,
            |b, &pwd| {
                b.iter(|| {
                    let mut hasher = Sha256::new();
                    hasher.update(black_box(pwd.as_bytes()));
                    let hash = hasher.finalize();
                    black_box(hash)
                })
            },
        );
    }
    
    group.finish();
}

fn bench_challenge_response(c: &mut Criterion) {
    let mut group = c.benchmark_group("challenge_response");
    
    let (private_key, public_key) = generate_rsa_keypair(2048);
    let signing_key = SigningKey::<Sha256>::new(private_key);
    let verifying_key = VerifyingKey::<Sha256>::new(public_key);
    
    // Simulate authentication challenge-response
    group.bench_function("auth_challenge_response", |b| {
        b.iter(|| {
            // Generate challenge
            let challenge: [u8; 32] = rand::random();
            
            // Sign challenge (client side)
            let mut rng = OsRng;
            let signature = signing_key.sign_with_rng(&mut rng, &challenge);
            
            // Verify signature (server side)
            let result = verifying_key.verify(&challenge, &signature);
            
            black_box((challenge, signature, result))
        })
    });
    
    group.finish();
}

fn bench_auth_session_setup(c: &mut Criterion) {
    c.bench_function("full_auth_session", |b| {
        b.iter(|| {
            // Simulate full authentication session setup
            
            // 1. Generate server challenge
            let challenge: [u8; 32] = rand::random();
            
            // 2. Generate client keypair (first-time setup)
            let (private_key, public_key) = generate_rsa_keypair(2048);
            
            // 3. Client signs challenge
            let signing_key = SigningKey::<Sha256>::new(private_key);
            let mut rng = OsRng;
            let signature = signing_key.sign_with_rng(&mut rng, &challenge);
            
            // 4. Server verifies signature
            let verifying_key = VerifyingKey::<Sha256>::new(public_key);
            let verification_result = verifying_key.verify(&challenge, &signature);
            
            black_box((challenge, signature, verification_result))
        })
    });
}

criterion_group!(
    auth_benches,
    bench_keypair_generation,
    bench_digital_signature,
    bench_password_hashing,
    bench_challenge_response,
    bench_auth_session_setup
);
criterion_main!(auth_benches);