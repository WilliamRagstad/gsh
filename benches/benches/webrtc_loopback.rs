use criterion::{black_box, criterion_group, criterion_main, Criterion};
use libgsh::{
    shared::protocol::UserInput,
    webrtc::{EncodedVideoFrame, GshSession, SessionRole, VideoCodec, VideoInputMode},
};
use reactor_webrtc::{PeerConnectionFactory, RtcConfiguration};
use std::time::Duration;

const VP8_KEYFRAME: &[u8] = &[
    0x30, 0x12, 0x00, 0x9d, 0x01, 0x2a, 0x40, 0x01, 0xf0, 0x00, 0x00, 0x47, 0x08, 0x85, 0x85, 0x88,
    0x99, 0x84, 0x88, 0x02, 0x02, 0x00, 0x06, 0x16, 0x04, 0xf7, 0x06, 0x81, 0x64, 0x9f, 0x6b, 0xdb,
    0x9b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b,
    0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27,
    0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38,
    0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b,
    0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27,
    0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38,
    0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b,
    0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27, 0x38, 0x7b, 0x27,
    0x38, 0x7b, 0x27, 0x38, 0x7b, 0x7a, 0xf4, 0x00, 0xfe, 0xff, 0xab, 0x50, 0x80,
];

async fn connect_local_peers(raw_bgra: bool, width: u32, height: u32) -> (GshSession, GshSession) {
    let offerer_factory = PeerConnectionFactory::builder().build().unwrap();
    let answerer_factory = PeerConnectionFactory::builder().build().unwrap();
    let offerer = if raw_bgra {
        GshSession::new_raw_video(
            offerer_factory,
            RtcConfiguration::default(),
            SessionRole::Offerer,
            width,
            height,
        )
    } else {
        GshSession::new(
            offerer_factory,
            RtcConfiguration::default(),
            SessionRole::Offerer,
            width,
            height,
        )
    }
    .unwrap();
    let answerer = if raw_bgra {
        GshSession::new_with_video_input(
            answerer_factory,
            RtcConfiguration::default(),
            SessionRole::Answerer,
            width,
            height,
            VideoInputMode::RawBgra,
            &[VideoCodec::Vp8],
        )
    } else {
        GshSession::new(
            answerer_factory,
            RtcConfiguration::default(),
            SessionRole::Answerer,
            width,
            height,
        )
    }
    .unwrap();

    let offer = offerer.create_offer().unwrap();
    let answer = answerer.accept_offer(offer).unwrap();
    offerer.accept_answer(answer).unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    loop {
        for candidate in offerer.drain_ice_candidates().await.unwrap() {
            answerer.add_ice_candidate(candidate).unwrap();
        }
        for candidate in answerer.drain_ice_candidates().await.unwrap() {
            offerer.add_ice_candidate(candidate).unwrap();
        }
        if offerer.is_connected() && answerer.is_connected() {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "local ICE failed");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Ensure the SCTP stream is open before Criterion starts its timed region.
    let input = UserInput::default();
    loop {
        if answerer.send_input(&input).is_ok() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "input channel did not open"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let _ = tokio::time::timeout(Duration::from_secs(2), offerer.recv_input())
        .await
        .expect("warm-up input timed out")
        .unwrap();

    (offerer, answerer)
}

async fn send_raw_until_accepted(session: &GshSession, bgra: &[u8]) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        match session.send_raw_video(black_box(bgra)) {
            Ok(()) => return,
            Err(libgsh::webrtc::SessionError::VideoBackpressure) => {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "raw sender queue stalled"
                );
                tokio::time::sleep(Duration::from_micros(100)).await;
            }
            Err(error) => panic!("raw video path failed: {error}"),
        }
    }
}

fn bench_loopback(c: &mut Criterion) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (offerer, answerer) = runtime.block_on(connect_local_peers(false, 320, 240));
    let mut group = c.benchmark_group("webrtc_local_loopback");

    let input = UserInput::default();
    group.bench_function("input_to_server_dispatch", |b| {
        b.iter_custom(|iterations| {
            runtime.block_on(async {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iterations {
                    let started = tokio::time::Instant::now();
                    answerer.send_input(black_box(&input)).unwrap();
                    black_box(offerer.recv_input().await.unwrap());
                    elapsed += started.elapsed();
                }
                elapsed
            })
        })
    });

    if offerer.negotiated_video_codec() == Some(VideoCodec::Vp8) {
        group.bench_function("vp8_packetize_to_software_decode", |b| {
            b.iter_custom(|iterations| {
                runtime.block_on(async {
                    let mut elapsed = Duration::ZERO;
                    for _ in 0..iterations {
                        let frame = EncodedVideoFrame {
                            data: VP8_KEYFRAME.to_vec(),
                            codec: VideoCodec::Vp8,
                            is_key_frame: true,
                            width: 320,
                            height: 240,
                            rtp_timestamp: 0,
                        };
                        let mut pending = Some(frame);
                        let started = tokio::time::Instant::now();
                        while let Some(frame) = pending.take() {
                            if let Err(error) = offerer.send_video(frame) {
                                pending = error.frame;
                                assert!(pending.is_some(), "native video pipeline failed");
                                tokio::time::sleep(Duration::from_micros(100)).await;
                            }
                        }
                        let decoded = answerer.recv_video().await.unwrap();
                        black_box((decoded.width, decoded.height, decoded.bgra.len()));
                        elapsed += started.elapsed();
                    }
                    elapsed
                })
            })
        });
    }

    let (raw_offer, raw_answer) = runtime.block_on(connect_local_peers(true, 320, 240));
    let raw_bgra = vec![0x30_u8; 320 * 240 * 4];
    group.bench_function("bgra_to_vp8_encode_and_software_decode", |b| {
        b.iter_custom(|iterations| {
            runtime.block_on(async {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iterations {
                    let started = tokio::time::Instant::now();
                    send_raw_until_accepted(&raw_offer, &raw_bgra).await;
                    let decoded =
                        tokio::time::timeout(Duration::from_secs(3), raw_answer.recv_video())
                            .await
                            .expect("raw decode timed out")
                            .unwrap();
                    black_box((decoded.width, decoded.height, decoded.bgra.len()));
                    elapsed += started.elapsed();
                }
                elapsed
            })
        })
    });

    group.finish();
}

criterion_group!(webrtc_benches, bench_loopback);
criterion_main!(webrtc_benches);
