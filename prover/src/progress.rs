//! Live progress for an in-flight proving job.
//!
//! The prover decides entirely what a client displays: the heading, whether
//! there is a bar and how full it is, which figures appear and in what order.
//! subs renders what it is given and computes nothing.
//!
//! That split matters because the phases are this prover's, not a universal
//! truth. A proxy that rents a pod has a boot phase; a prover that has profiled
//! lift/join can report a fraction where this one cannot. Encoding "there are
//! two phases, the second is unknowable" in the UI would make those
//! unexpressible.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use risc0_zkvm::{Segment, SessionEvents};
use serde::Serialize;

/// How a client should draw the progress bar.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Bar {
    /// Draw it filled to `fraction`.
    Determinate,
    /// Work is happening; its extent is unknown.
    Indeterminate,
    /// Draw no bar at all — for a status that is not progress, such as
    /// "queued". Distinct from Indeterminate, which still asserts that
    /// something is underway.
    None,
}

/// One figure to display, already formatted.
///
/// The prover formats rather than sending raw numbers: it is the only side that
/// knows whether a value is cycles, seconds, or dollars, and a client that
/// re-derives "1.6M" from 1567156 is guessing at units it was never told.
#[derive(Debug, Clone, Serialize)]
pub struct Stat {
    pub label: String,
    pub value: String,
    /// Emphasised. At most one is worth marking — normally whatever the
    /// operator is actually waiting on.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub accent: bool,
}

impl Stat {
    fn new(label: &str, value: impl Into<String>) -> Self {
        Self { label: label.into(), value: value.into(), accent: false }
    }

    fn accented(label: &str, value: impl Into<String>) -> Self {
        Self { label: label.into(), value: value.into(), accent: true }
    }
}

/// A snapshot of a job's progress, safe to serialize into a status response.
///
/// Every field is optional. A prover with nothing useful to say omits the whole
/// structure; one that only wants to report "booting" sends a `label` and
/// nothing else.
#[derive(Debug, Clone, Serialize)]
pub struct JobProgress {
    /// What is happening now, in the prover's own words.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Position in a sequence of phases, for an "N of M" indicator. Phases are
    /// whatever the prover says they are.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase_total: Option<u8>,
    /// 0.0–1.0. Only meaningful when `bar` is Determinate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fraction: Option<f64>,
    /// Omitted means "Determinate if `fraction` is set, else Indeterminate", so
    /// the ordinary cases need not send it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bar: Option<Bar>,
    /// Figures to display, in the order they should appear.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stats: Vec<Stat>,
    /// Lines to surface — pod boot output, queue notices. Replaced wholesale on
    /// every poll, so the prover decides how many to keep and how to format
    /// them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub log: Vec<String>,
}

/// Compact duration: "48s", "1m 32s", "2h 5m".
///
/// Spelled-out forms wrap inside a stat tile and bury the number.
fn fmt_duration(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as u64;
    if total < 60 {
        return format!("{}s", total);
    }
    let mins = total / 60;
    if mins < 60 {
        let secs = total % 60;
        return if secs == 0 { format!("{}m", mins) } else { format!("{}m {}s", mins, secs) };
    }
    let hours = mins / 60;
    let rem = mins % 60;
    if rem == 0 { format!("{}h", hours) } else { format!("{}h {}m", hours, rem) }
}

/// Cycle counts run to millions, where exact digits are noise.
fn fmt_cycles(n: u64) -> String {
    match n {
        n if n >= 1_000_000_000 => format!("{:.1}B", n as f64 / 1e9),
        n if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1e6),
        n if n >= 1_000 => format!("{:.1}K", n as f64 / 1e3),
        n => n.to_string(),
    }
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
        let fraction = if in_phase_two || segments == 0 || done == 0 || last <= 0.0 {
            None
        } else {
            let mean_segment = last / done as f64;
            let within = ((elapsed - last).max(0.0) / mean_segment).min(0.9);
            Some(((done as f64 + within) / segments as f64).min(1.0))
        };

        // Nothing has been reported yet: the executor is still running and even
        // the segment count is unknown. Saying so beats a bar at zero.
        if segments == 0 {
            return JobProgress {
                label: Some("Executing".into()),
                phase: None,
                phase_total: None,
                fraction: None,
                bar: None,
                stats: vec![Stat::new("elapsed", fmt_duration(elapsed))],
                log: Vec::new(),
            };
        }

        let mut stats = Vec::new();
        if let Some(total) = estimated_total_seconds {
            stats.push(Stat::accented("remaining", format!("~{}", fmt_duration(total - elapsed))));
        }
        stats.push(Stat::new("elapsed", fmt_duration(elapsed)));
        if !in_phase_two {
            stats.push(Stat::new("segments", format!("{}/{}", done, segments)));
        }
        let total_cycles = self.total_cycles.load(Ordering::Relaxed);
        if total_cycles > 0 {
            stats.push(Stat::new("cycles", fmt_cycles(total_cycles)));
        }
        // Only sm_80 gets native SASS in the shipped image, so every other GPU
        // JIT-compiles PTX on its first kernel launch and pays for it here.
        // Comparing this against the mean tells an operator whether baking in
        // their card's SASS is worth the build time.
        if first_ms > 0 {
            stats.push(Stat::new("first segment", fmt_duration(first_ms as f64 / 1000.0)));
        }

        JobProgress {
            label: Some(
                if in_phase_two { "Producing succinct receipt" } else { "Proving segments" }.into(),
            ),
            phase: Some(if in_phase_two { 2 } else { 1 }),
            phase_total: Some(2),
            fraction,
            // Left to the default rule: determinate when a fraction is present.
            // Phase 2 has none — lift/join/resolve fire no hook — so it draws
            // indeterminate, which is the honest shape for work of unknown
            // extent rather than a full bar sitting still.
            bar: None,
            stats,
            log: Vec::new(),
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
