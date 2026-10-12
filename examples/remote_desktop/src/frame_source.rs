use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, RecvTimeoutError},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::watch;
use xcap::Frame;

#[derive(Debug)]
pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
    pub sequence: u64,
    pub captured_at: Instant,
}

impl CapturedFrame {
    fn from_xcap(frame: Frame, sequence: u64) -> Self {
        Self {
            width: frame.width,
            height: frame.height,
            rgba: Arc::from(frame.raw),
            sequence,
            captured_at: Instant::now(),
        }
    }
}

/// Shared, latest-frame capture fan-out. Each peer receives an independent
/// subscription; slow peers skip obsolete frames instead of stealing frames
/// from other peers or accumulating a queue.
#[derive(Clone)]
pub struct FrameHub {
    frames: watch::Sender<Option<Arc<CapturedFrame>>>,
}

impl FrameHub {
    pub fn from_xcap(receiver: Receiver<Frame>) -> (Self, FrameForwarder) {
        let (frames, _) = watch::channel(None);
        let publisher = frames.clone();
        let stopping = Arc::new(AtomicBool::new(false));
        let worker_stopping = stopping.clone();
        let worker = tokio::task::spawn_blocking(move || {
            let mut sequence = 0_u64;
            loop {
                let mut frame = match receiver.recv_timeout(Duration::from_millis(20)) {
                    Ok(frame) => frame,
                    Err(RecvTimeoutError::Timeout) => {
                        if worker_stopping.load(Ordering::Acquire) {
                            break;
                        }
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                };
                // xcap 0.9.8 feeds this receiver from an unbounded queue. Drain
                // queued frames before publishing so slow consumers never build
                // a second, stale frame backlog upstream of the latest-frame hub.
                while let Ok(newest) = receiver.try_recv() {
                    frame = newest;
                }
                if worker_stopping.load(Ordering::Acquire) {
                    break;
                }
                sequence = sequence.wrapping_add(1);
                publisher.send_replace(Some(Arc::new(CapturedFrame::from_xcap(frame, sequence))));
            }
        });
        (
            Self { frames },
            FrameForwarder {
                stopping,
                worker: Some(worker),
            },
        )
    }

    pub fn subscribe(&self) -> FrameSource {
        FrameSource {
            frames: self.frames.subscribe(),
            checked_initial_value: false,
        }
    }

    #[cfg(test)]
    fn publish(&self, frame: CapturedFrame) {
        self.frames.send_replace(Some(Arc::new(frame)));
    }
}

pub struct FrameForwarder {
    stopping: Arc<AtomicBool>,
    worker: Option<tokio::task::JoinHandle<()>>,
}

impl FrameForwarder {
    pub fn stop(&self) {
        self.stopping.store(true, Ordering::Release);
    }

    pub async fn join(mut self) -> Result<(), tokio::task::JoinError> {
        self.worker
            .take()
            .expect("capture worker already joined")
            .await
    }
}

pub struct FrameSource {
    frames: watch::Receiver<Option<Arc<CapturedFrame>>>,
    checked_initial_value: bool,
}

impl FrameSource {
    pub async fn next(&mut self) -> Option<Arc<CapturedFrame>> {
        if !self.checked_initial_value {
            self.checked_initial_value = true;
            if let Some(frame) = self.frames.borrow_and_update().clone() {
                return Some(frame);
            }
        }

        loop {
            if self.frames.changed().await.is_err() {
                return None;
            }
            if let Some(frame) = self.frames.borrow_and_update().clone() {
                return Some(frame);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(value: u8) -> CapturedFrame {
        CapturedFrame {
            width: 2,
            height: 1,
            rgba: Arc::from([value; 8]),
            sequence: value as u64,
            captured_at: Instant::now(),
        }
    }

    #[tokio::test]
    async fn subscriptions_receive_same_latest_frame_independently() {
        let (frames, _) = watch::channel(None);
        let hub = FrameHub { frames };
        let mut first = hub.subscribe();
        let mut second = hub.subscribe();

        hub.publish(frame(1));
        let first_frame = first.next().await.unwrap();
        let second_frame = second.next().await.unwrap();
        assert!(Arc::ptr_eq(&first_frame, &second_frame));

        hub.publish(frame(2));
        let first_frame = first.next().await.unwrap();
        let second_frame = second.next().await.unwrap();
        assert_eq!(&*first_frame.rgba, &[2; 8]);
        assert_eq!(&*second_frame.rgba, &[2; 8]);
    }

    #[tokio::test]
    async fn late_subscriber_starts_at_latest_frame() {
        let (frames, _) = watch::channel(None);
        let hub = FrameHub { frames };
        hub.publish(frame(7));

        let mut source = hub.subscribe();
        let latest = source.next().await.unwrap();
        assert_eq!(&*latest.rgba, &[7; 8]);
    }

    #[tokio::test]
    async fn next_waits_for_a_new_frame_after_initial_value() {
        let (frames, _) = watch::channel(None);
        let hub = FrameHub { frames };
        hub.publish(frame(1));
        let mut source = hub.subscribe();
        assert_eq!(&*source.next().await.unwrap().rgba, &[1; 8]);

        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), source.next())
                .await
                .is_err()
        );

        hub.publish(frame(2));
        assert_eq!(&*source.next().await.unwrap().rgba, &[2; 8]);
    }

    #[tokio::test]
    async fn forwarder_stop_unblocks_empty_capture_receiver() {
        let (_sender, receiver) = std::sync::mpsc::channel();
        let (_hub, forwarder) = FrameHub::from_xcap(receiver);
        forwarder.stop();
        tokio::time::timeout(Duration::from_secs(1), forwarder.join())
            .await
            .expect("capture forwarding task did not stop")
            .unwrap();
    }

    #[tokio::test]
    async fn xcap_backlog_is_reduced_to_newest_frame() {
        let (sender, receiver) = std::sync::mpsc::channel();
        for value in 0..128_u8 {
            sender.send(Frame::new(2, 1, vec![value; 8])).unwrap();
        }
        let (hub, forwarder) = FrameHub::from_xcap(receiver);
        let mut source = hub.subscribe();
        let newest = tokio::time::timeout(Duration::from_secs(1), source.next())
            .await
            .expect("capture forwarder did not publish")
            .unwrap();
        assert_eq!(&*newest.rgba, &[127; 8]);

        drop(sender);
        forwarder.join().await.unwrap();
    }
}
