//! Fixed target-frame pacing based on absolute monotonic deadlines.
//!
//! This deliberately implements no adaptive scheduler. Callers choose a fixed
//! frame rate and can inspect skipped slots rather than accumulating drift.

use std::time::Duration;
use tokio::time::{sleep_until, Instant};

const NANOS_PER_SECOND: u64 = 1_000_000_000;
const MAX_TARGET_FPS: u32 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameTick {
    /// Intended monotonic deadline for this tick.
    pub deadline: Instant,
    /// Actual monotonic wake time.
    pub woke_at: Instant,
    /// Target slots skipped because work/scheduling overran the deadline.
    pub skipped: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PacingError {
    #[error("target FPS must be in 1..={MAX_TARGET_FPS}, got {0}")]
    InvalidFps(u32),
}

/// A periodic scheduler that advances from its previous deadline, not from
/// completion time. Late wakeups skip missed slots rather than burst catch-up.
#[derive(Debug, Clone)]
pub struct FramePacer {
    period: Duration,
    next_deadline: Instant,
}

impl FramePacer {
    pub fn new(target_fps: u32) -> Result<Self, PacingError> {
        if !(1..=MAX_TARGET_FPS).contains(&target_fps) {
            return Err(PacingError::InvalidFps(target_fps));
        }
        let period = Duration::from_nanos(NANOS_PER_SECOND / u64::from(target_fps));
        Ok(Self {
            period,
            next_deadline: Instant::now() + period,
        })
    }

    pub const fn period(&self) -> Duration {
        self.period
    }

    pub fn next_deadline(&self) -> Instant {
        self.next_deadline
    }

    pub async fn wait_next(&mut self) -> FrameTick {
        let deadline = self.next_deadline;
        sleep_until(deadline).await;
        let woke_at = Instant::now();
        let first_next = deadline + self.period;
        let late = woke_at.saturating_duration_since(first_next);
        let skipped = if woke_at >= first_next {
            (late.as_nanos() / self.period.as_nanos()) as u64 + 1
        } else {
            0
        };
        self.next_deadline = first_next + self.period.saturating_mul(skipped as u32);
        FrameTick {
            deadline,
            woke_at,
            skipped,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_rate_and_uses_integer_period() {
        assert!(matches!(
            FramePacer::new(0),
            Err(PacingError::InvalidFps(0))
        ));
        assert!(FramePacer::new(MAX_TARGET_FPS + 1).is_err());
        assert_eq!(
            FramePacer::new(60).unwrap().period(),
            Duration::from_nanos(16_666_666)
        );
        assert_eq!(
            FramePacer::new(144).unwrap().period(),
            Duration::from_nanos(6_944_444)
        );
    }
}
