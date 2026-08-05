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
    /// Which phase the job is in, 1-based.
    ///
    /// 1. Proving segments. Determinate: `segments_done` of `segments`.
    /// 2. Lift/join/resolve, turning the composite segment receipts into the
    ///    succinct receipt. risc0 exposes no `SessionEvents` hook for this, so
    ///    nothing can be observed while it runs — it is genuinely
    ///    indeterminate, not merely unmeasured.
    ///
    /// Reporting it matters because phase 2 is not a tail: on a measured
    /// single-segment step proof, segments finished at 10.7s of 38.8s, so 72%
    /// of the wall-clock happened in phase 2 with the bar already full.
    pub phase: u8,
    /// Total phases, so the UI need not hardcode it.
    pub phase_total: u8,
    /// Fraction of phase 1 complete (0.0–1.0), interpolated within the segment
    /// currently being proven so the bar advances between completions.
    ///
    /// `None` while the first segment is proving — there is no measured segment
    /// duration to interpolate against yet — and throughout phase 2.
    pub phase_one_fraction: Option<f64>,
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
    /// When the most recent segment landed.
    ///
    /// Estimates must extrapolate from segment *completions*, not from current
    /// elapsed time: between two completions no new information arrives, so an
    /// elapsed-based projection inflates on every poll and "remaining" tracks
    /// "elapsed" exactly.
    last_segment_millis: AtomicU64,
    /// Elapsed at the moment proving stopped, or 0 while it runs.
    ///
    /// Without this, `snapshot()` keeps deriving elapsed from `started`, so a
    /// finished job's /jobs/:id keeps ageing — reporting an ever-growing
    /// duration for work that ended. subs stops polling once the receipt is
    /// pulled, but anything else reading the prover API sees it.
    finished_millis: AtomicU64,
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
            last_segment_millis: AtomicU64::new(0),
            finished_millis: AtomicU64::new(0),
        })
    }

    /// Stop the clock. Idempotent — the first call wins, so a job that is
    /// finished twice (completed then cancelled, say) keeps its real duration.
    pub fn finish(&self) {
        let ms = self.started.elapsed().as_millis() as u64;
        let _ = self
            .finished_millis
            .compare_exchange(0, ms.max(1), Ordering::Relaxed, Ordering::Relaxed);
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
        let ms = self.started.elapsed().as_millis() as u64;
        self.last_segment_millis.store(ms, Ordering::Relaxed);
        if done == 1 {
            self.first_segment_millis.store(ms, Ordering::Relaxed);
        }
    }

    pub fn snapshot(&self) -> JobProgress {
        let segments = self.segments.load(Ordering::Relaxed);
        let done = self.segments_done.load(Ordering::Relaxed);
        // Frozen once the job ends, so a finished job reports the duration it
        // actually took rather than time since it started.
        let finished_ms = self.finished_millis.load(Ordering::Relaxed);
        let elapsed = if finished_ms > 0 {
            finished_ms as f64 / 1000.0
        } else {
            self.started.elapsed().as_secs_f64()
        };
        let first_ms = self.first_segment_millis.load(Ordering::Relaxed);

        // Extrapolate from observed segments. Remaining segments' po2 is not
        // known without resolving them, so the best available projection is the
        // mean rate so far — which is exactly what a po2-weighted estimate
        // reduces to when the remaining work is unknown.
        //
        // Crucially this extrapolates from segment *completion* timestamps, not
        // from current elapsed time. Between two completions no new information
        // arrives, so an elapsed-based projection grows on every poll: with one
        // of two segments done it computed `elapsed / 1 * 2`, making "remaining"
        // exactly equal "elapsed" and climb forever. The first segment's
        // duration is already measured — the estimate must hold still until the
        // next one actually lands.
        //
        // The first segment carries GPU warm-up and any PTX JIT, so once a
        // second segment lands it is excluded from the rate and only the steady
        // -state segments are extrapolated.
        //
        // Phase 2 (lift/join/resolve) is unobservable, so once every segment is
        // proven there is nothing left to extrapolate from. The old code fell
        // through to the `done == segments` case and reported
        // `total == elapsed` on every poll, which claims "finishing now" for
        // the entire phase -- 28.1s of a measured 38.8s proof. No estimate is
        // the honest answer; the UI shows phase 2 as indeterminate.
        let in_phase_two = segments > 0 && done >= segments;
        let first = first_ms as f64 / 1000.0;
        let last = self.last_segment_millis.load(Ordering::Relaxed) as f64 / 1000.0;

        let estimated_total_seconds = if in_phase_two || done == 0 || segments == 0 {
            None
        } else if done == 1 {
            // Only the first segment has been timed, and it includes warm-up,
            // so this runs high. It is at least stable.
            Some(first * segments as f64)
        } else {
            let steady_rate = (last - first) / (done - 1) as f64;
            Some(first + steady_rate * (segments - 1) as f64)
        };

        // If elapsed has already overtaken the projection, the current segment
        // is slower than the ones it was extrapolated from and the estimate is
        // simply wrong. Dropping it shows no ETA; clamping it to `elapsed`
        // would report "~0 seconds remaining" while work continues, which is
        // the same falsehood this pass set out to remove.
        let estimated_total_seconds = estimated_total_seconds.filter(|est| *est > elapsed);

        // Fraction of phase 1 complete, interpolated inside the segment being
        // proven so the bar moves continuously instead of stepping once per
        // completion. Segments here run over a minute, so a step-only bar looks
        // stalled for most of the job.
        //
        // The current segment is capped at 90% of its share: it must not look
        // finished before it is. Interpolation needs a segment duration to
        // scale against, so it only starts once one has been measured — the
        // very first segment has no basis and reports None, which the UI shows
        // as an indeterminate bar rather than a fabricated position.
        let phase_one_fraction = if in_phase_two || segments == 0 || done == 0 || last <= 0.0 {
            None
        } else {
            let mean_segment = last / done as f64;
            let within = ((elapsed - last).max(0.0) / mean_segment).min(0.9);
            Some(((done as f64 + within) / segments as f64).min(1.0))
        };

        JobProgress {
            total_cycles: self.total_cycles.load(Ordering::Relaxed),
            proving_cycles_done: self.proving_cycles_done.load(Ordering::Relaxed),
            segments,
            segments_done: done,
            elapsed_seconds: elapsed,
            estimated_total_seconds,
            phase: if in_phase_two { 2 } else { 1 },
            phase_total: 2,
            phase_one_fraction,
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
