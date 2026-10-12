//! Low-overhead, opt-in stage timing for local monotonic pipeline durations.
//!
//! Timings are not cross-host timestamps and do not measure photons. A stage is
//! measured with `Instant`; percentile buckets are logarithmic and approximate.

use std::{
    array,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

/// Pipeline stages. Keep indices stable if downstream telemetry consumes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum Stage {
    InputCapture = 0,
    InputDispatch = 1,
    Render = 2,
    Capture = 3,
    Encode = 4,
    WebRtcSend = 5,
    WebRtcReceive = 6,
    Decode = 7,
    Present = 8,
}

impl Stage {
    pub const COUNT: usize = 9;

    pub const fn name(self) -> &'static str {
        match self {
            Self::InputCapture => "input_capture",
            Self::InputDispatch => "input_dispatch",
            Self::Render => "render",
            Self::Capture => "capture",
            Self::Encode => "encode",
            Self::WebRtcSend => "webrtc_send",
            Self::WebRtcReceive => "webrtc_receive",
            Self::Decode => "decode",
            Self::Present => "present",
        }
    }
}

struct StageStats {
    count: AtomicU64,
    total_ns: AtomicU64,
    max_ns: AtomicU64,
    histogram: [AtomicU64; 64],
}

impl StageStats {
    fn new() -> Self {
        Self {
            count: AtomicU64::new(0),
            total_ns: AtomicU64::new(0),
            max_ns: AtomicU64::new(0),
            histogram: array::from_fn(|_| AtomicU64::new(0)),
        }
    }

    fn record(&self, duration: Duration) {
        let ns = duration.as_nanos().min(u64::MAX as u128) as u64;
        self.count.fetch_add(1, Ordering::Relaxed);
        self.total_ns.fetch_add(ns, Ordering::Relaxed);
        self.max_ns.fetch_max(ns, Ordering::Relaxed);
        let bucket = if ns == 0 {
            0
        } else {
            (u64::BITS - ns.leading_zeros()) as usize
        }
        .min(63);
        self.histogram[bucket].fetch_add(1, Ordering::Relaxed);
    }

    fn snapshot(&self, name: &'static str) -> StageSnapshot {
        let count = self.count.load(Ordering::Relaxed);
        let total_ns = self.total_ns.load(Ordering::Relaxed);
        StageSnapshot {
            name,
            count,
            total: Duration::from_nanos(total_ns),
            mean: (count > 0).then(|| Duration::from_nanos(total_ns / count)),
            max: Duration::from_nanos(self.max_ns.load(Ordering::Relaxed)),
            p50: self.percentile(count, 50),
            p95: self.percentile(count, 95),
            p99: self.percentile(count, 99),
        }
    }

    fn percentile(&self, count: u64, percentile: u64) -> Option<Duration> {
        if count == 0 {
            return None;
        }
        let rank = ((count - 1) * percentile / 100) + 1;
        let mut seen = 0;
        for (bucket, frequency) in self.histogram.iter().enumerate() {
            seen += frequency.load(Ordering::Relaxed);
            if seen >= rank {
                // Upper edge of logarithmic bucket; useful for comparing runs,
                // not a substitute for raw latency samples.
                let upper_ns = if bucket == 0 { 0 } else { 1u64 << bucket };
                return Some(Duration::from_nanos(upper_ns));
            }
        }
        Some(Duration::from_nanos(u64::MAX))
    }
}

struct Inner {
    enabled: AtomicBool,
    stages: [StageStats; Stage::COUNT],
}

/// Cheap-to-clone shared timing collector. Disabled by default; disabled scopes
/// do not read the clock. Histogram updates use relaxed atomics.
#[derive(Clone)]
pub struct PipelineTelemetry(Arc<Inner>);

impl Default for PipelineTelemetry {
    fn default() -> Self {
        Self::new(false)
    }
}

impl PipelineTelemetry {
    pub fn new(enabled: bool) -> Self {
        Self(Arc::new(Inner {
            enabled: AtomicBool::new(enabled),
            stages: array::from_fn(|_| StageStats::new()),
        }))
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.0.enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn is_enabled(&self) -> bool {
        self.0.enabled.load(Ordering::Relaxed)
    }

    /// Start a timer whose elapsed time is recorded when the guard is dropped.
    #[inline]
    pub fn time(&self, stage: Stage) -> StageTimer {
        let started = self.is_enabled().then(Instant::now);
        StageTimer {
            telemetry: self.clone(),
            stage,
            started,
        }
    }

    /// Record an externally measured local duration.
    #[inline]
    pub fn record(&self, stage: Stage, elapsed: Duration) {
        if self.is_enabled() {
            self.0.stages[stage as usize].record(elapsed);
        }
    }

    pub fn snapshot(&self) -> Vec<StageSnapshot> {
        self.0
            .stages
            .iter()
            .enumerate()
            .map(|(i, stats)| stats.snapshot(stage_name(i)))
            .collect()
    }
}

/// RAII timer for a single synchronous or async-scoped stage.
pub struct StageTimer {
    telemetry: PipelineTelemetry,
    stage: Stage,
    started: Option<Instant>,
}

impl Drop for StageTimer {
    fn drop(&mut self) {
        if let Some(started) = self.started {
            self.telemetry.0.stages[self.stage as usize].record(started.elapsed());
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageSnapshot {
    pub name: &'static str,
    pub count: u64,
    pub total: Duration,
    pub mean: Option<Duration>,
    pub max: Duration,
    pub p50: Option<Duration>,
    pub p95: Option<Duration>,
    pub p99: Option<Duration>,
}

const fn stage_name(index: usize) -> &'static str {
    match index {
        0 => "input_capture",
        1 => "input_dispatch",
        2 => "render",
        3 => "capture",
        4 => "encode",
        5 => "webrtc_send",
        6 => "webrtc_receive",
        7 => "decode",
        8 => "present",
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_telemetry_does_not_record() {
        let telemetry = PipelineTelemetry::default();
        telemetry.record(Stage::Encode, Duration::from_millis(1));
        assert_eq!(telemetry.snapshot()[Stage::Encode as usize].count, 0);
    }

    #[test]
    fn records_stage_and_percentiles() {
        let telemetry = PipelineTelemetry::new(true);
        telemetry.record(Stage::Encode, Duration::from_nanos(10));
        telemetry.record(Stage::Encode, Duration::from_nanos(100));
        let stage = &telemetry.snapshot()[Stage::Encode as usize];
        assert_eq!(stage.name, "encode");
        assert_eq!(stage.count, 2);
        assert_eq!(stage.total, Duration::from_nanos(110));
        assert!(stage.p50.is_some());
        assert!(stage.p99.is_some());
    }

    #[test]
    fn timer_records_on_drop() {
        let telemetry = PipelineTelemetry::new(true);
        {
            let _timer = telemetry.time(Stage::Present);
        }
        assert_eq!(telemetry.snapshot()[Stage::Present as usize].count, 1);
    }
}
