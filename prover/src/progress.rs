//! Live progress for an in-flight proving job.
//!
//! The prover is the only place that knows how long a job will take: the
//! session gives the segment count before proving starts, and risc0 fires a
//! hook around each segment as it is proven. Together those turn "processing"
//! into an ETA measured on this GPU, for this job — rather than extrapolated
//! from a synthetic calibration run on some other pod.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use risc0_zkvm::{Segment, SessionEvents};
use serde::Serialize;

/// A snapshot of a job's progress, safe to serialize into a status response.
#[derive(Debug, Clone, Serialize)]
pub struct JobProgress {
    /// User cycles executed by the guest.
    pub total_cycles: u64,
    /// Padded proving cycles across the segments proven so far. Equals the
    /// job's true total once `segments_done == segments`.
    pub proving_cycles_done: u64,
    /// Segments this job will prove. Known before proving starts.
    pub segments: usize,
    /// Segments proven so far.
    pub segments_done: usize,
    /// Wall-clock since proving began.
    pub elapsed_seconds: f64,
    /// Projected total wall-clock. `None` until a segment completes — there is
    /// nothing to extrapolate from before that.
    pub estimated_total_seconds: Option<f64>,
    /// Wall-clock of the first segment.
    ///
    /// Worth reporting separately: only sm_80 gets native SASS in the shipped
    /// image, so every other GPU JIT-compiles PTX on its first kernel launch
    /// and pays for it here. Comparing this against the mean is what tells an
    /// operator whether baking in their card's SASS is worth the build time.
    pub first_segment_seconds: Option<f64>,
}

/// Shared progress counters, written by the proving hook and read by the
/// status endpoint while the job runs.
pub struct ProgressSink {
    started: Instant,
    total_cycles: AtomicU64,
    segments: AtomicUsize,
    segments_done: AtomicUsize,
    proving_cycles_done: AtomicU64,
    /// Millis, so it fits an atomic without a lock.
    first_segment_millis: AtomicU64,
    elapsed_at_first_millis: AtomicU64,
}

impl ProgressSink {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            started: Instant::now(),
            total_cycles: AtomicU64::new(0),
            segments: AtomicUsize::new(0),
            segments_done: AtomicUsize::new(0),
            proving_cycles_done: AtomicU64::new(0),
            first_segment_millis: AtomicU64::new(0),
            elapsed_at_first_millis: AtomicU64::new(0),
        })
    }

    /// Record what the session tells us before any segment is proven.
    pub fn on_session_ready(&self, total_cycles: u64, segments: usize) {
        self.total_cycles.store(total_cycles, Ordering::Relaxed);
        self.segments.store(segments, Ordering::Relaxed);
    }

    fn on_segment_proven(&self, po2: usize) {
        self.proving_cycles_done
            .fetch_add(1u64 << po2, Ordering::Relaxed);
        let done = self.segments_done.fetch_add(1, Ordering::Relaxed) + 1;
        if done == 1 {
            let ms = self.started.elapsed().as_millis() as u64;
            self.first_segment_millis.store(ms, Ordering::Relaxed);
            self.elapsed_at_first_millis.store(ms, Ordering::Relaxed);
        }
    }

    pub fn snapshot(&self) -> JobProgress {
        let segments = self.segments.load(Ordering::Relaxed);
        let done = self.segments_done.load(Ordering::Relaxed);
        let elapsed = self.started.elapsed().as_secs_f64();
        let first_ms = self.first_segment_millis.load(Ordering::Relaxed);

        // Extrapolate from observed segments. Remaining segments' po2 is not
        // known without resolving them, so the best available projection is the
        // mean rate so far — which is exactly what a po2-weighted estimate
        // reduces to when the remaining work is unknown.
        //
        // The first segment carries GPU warm-up and any PTX JIT, so once a
        // second segment lands it is excluded from the rate and only the steady
        // -state segments are extrapolated.
        let estimated_total_seconds = if done == 0 || segments == 0 {
            None
        } else if done == 1 {
            Some(elapsed / done as f64 * segments as f64)
        } else {
            let first = first_ms as f64 / 1000.0;
            let steady_rate = (elapsed - first) / (done - 1) as f64;
            Some(first + steady_rate * (segments - 1) as f64)
        };

        JobProgress {
            total_cycles: self.total_cycles.load(Ordering::Relaxed),
            proving_cycles_done: self.proving_cycles_done.load(Ordering::Relaxed),
            segments,
            segments_done: done,
            elapsed_seconds: elapsed,
            estimated_total_seconds,
            first_segment_seconds: (first_ms > 0).then_some(first_ms as f64 / 1000.0),
        }
    }
}

/// Bridges risc0's per-segment proving hook into a [`ProgressSink`].
pub struct SegmentProgress {
    sink: Arc<ProgressSink>,
}

impl SegmentProgress {
    pub fn new(sink: Arc<ProgressSink>) -> Self {
        Self { sink }
    }
}

impl SessionEvents for SegmentProgress {
    fn on_post_prove_segment(&self, segment: &Segment) {
        self.sink.on_segment_proven(segment.po2());
    }
}