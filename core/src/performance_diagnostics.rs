//! Opt-in, bounded main-thread phase and frame-pacing diagnostics.
//! All times are CPU wall time; nested scopes overlap and are not additive.
//! Only static phase names and numbers are logged, never script values or URLs.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::time::Duration;
use web_time::Instant;

#[derive(Default)]
struct Timing {
    count: u64,
    total_us: u128,
    max_us: u128,
    over_42ms: u64,
    over_83ms: u64,
    histogram: Vec<u64>,
}

impl Timing {
    fn record(&mut self, elapsed: Duration) {
        if self.histogram.is_empty() {
            self.histogram.resize(1001, 0);
        }
        let us = elapsed.as_nanos().div_ceil(1000);
        self.count += 1;
        self.total_us += us;
        self.max_us = self.max_us.max(us);
        self.over_42ms += u64::from(us > 42_000);
        self.over_83ms += u64::from(us > 83_000);
        self.histogram[us.div_ceil(1000).min(1000) as usize] += 1;
    }

    fn p95_ms(&self) -> f64 {
        let rank = (u128::from(self.count) * 95).div_ceil(100) as u64;
        let mut count = 0;
        for (ms, bucket) in self.histogram.iter().enumerate() {
            count += bucket;
            if count >= rank {
                return if ms == 1000 {
                    self.max_us as f64 / 1000.0
                } else {
                    ms as f64
                };
            }
        }
        0.0
    }
}

struct Statistics {
    started: Instant,
    phases: BTreeMap<&'static str, Timing>,
    gaps: BTreeMap<&'static str, Instant>,
    frames: u64,
    catchup_ticks: u64,
    discarded_ticks: u64,
    audio_corrections: u64,
    max_audio_advance_ms: f64,
    max_audio_delay_ms: f64,
    max_busy_loop_offset_ms: u32,
    stall_messages: u32,
}

impl Default for Statistics {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            phases: BTreeMap::new(),
            gaps: BTreeMap::new(),
            frames: 0,
            catchup_ticks: 0,
            discarded_ticks: 0,
            audio_corrections: 0,
            max_audio_advance_ms: 0.0,
            max_audio_delay_ms: 0.0,
            max_busy_loop_offset_ms: 0,
            stall_messages: 0,
        }
    }
}

thread_local! {
    static STATISTICS: RefCell<Statistics> = RefCell::new(Statistics::default());
}

/// Scope labels must be static program-defined strings, not user/game data.
pub struct Scope {
    phase: &'static str,
    started: Instant,
}

impl Scope {
    pub fn new(phase: &'static str) -> Self {
        Self {
            phase,
            started: Instant::now(),
        }
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        record(self.phase, self.started.elapsed());
    }
}

pub fn record(phase: &'static str, elapsed: Duration) {
    STATISTICS.with_borrow_mut(|stats| {
        stats.phases.entry(phase).or_default().record(elapsed);
        if elapsed >= Duration::from_millis(83) && stats.stall_messages < 12 {
            stats.stall_messages += 1;
            tracing::info!("Perf stall: phase={phase} elapsed_ms={:.3}; overlapping CPU wall time, not GPU duration", elapsed.as_secs_f64() * 1000.0);
        }
    });
}

pub fn gap(phase: &'static str) {
    let now = Instant::now();
    let elapsed = STATISTICS.with_borrow_mut(|stats| {
        stats
            .gaps
            .insert(phase, now)
            .map(|previous| now.duration_since(previous))
    });
    if let Some(elapsed) = elapsed {
        record(phase, elapsed);
    }
}

pub fn tick_progress(frames: u32, discarded: bool, audio_skew: f64, busy_offset_ms: u32) {
    STATISTICS.with_borrow_mut(|stats| {
        stats.frames += u64::from(frames);
        stats.catchup_ticks += u64::from(frames > 1);
        stats.discarded_ticks += u64::from(discarded);
        stats.audio_corrections += u64::from(audio_skew != 0.0);
        stats.max_audio_advance_ms = stats.max_audio_advance_ms.max(audio_skew * 1000.0);
        stats.max_audio_delay_ms = stats.max_audio_delay_ms.max(-audio_skew * 1000.0);
        stats.max_busy_loop_offset_ms = stats.max_busy_loop_offset_ms.max(busy_offset_ms);
    });
    report(false);
}

pub fn report(force: bool) {
    let snapshot = STATISTICS.with_borrow_mut(|stats| {
        if !force && stats.started.elapsed() < Duration::from_secs(5) {
            return None;
        }
        if stats.phases.is_empty() {
            return None;
        }
        let mut snapshot = std::mem::take(stats);
        // Keep the last timestamp so report boundaries do not hide frame gaps.
        stats.gaps = std::mem::take(&mut snapshot.gaps);
        Some(snapshot)
    });
    if let Some(stats) = snapshot {
        let seconds = stats.started.elapsed().as_secs_f64();
        tracing::info!(
            "Perf progress: window_s={seconds:.3} frame_steps={} catchup_ticks={} discarded_ticks={} audio_corrections={} max_audio_advance_ms={:.3} max_audio_delay_ms={:.3} max_busy_offset_ms={}; frame_steps counts run_frame attempts; phases overlap and cannot be summed",
            stats.frames,
            stats.catchup_ticks,
            stats.discarded_ticks,
            stats.audio_corrections,
            stats.max_audio_advance_ms,
            stats.max_audio_delay_ms,
            stats.max_busy_loop_offset_ms
        );
        for (phase, timing) in stats.phases {
            tracing::info!(
                "Perf phase: phase={phase} count={} avg_ms={:.3} p95_le_ms={:.3} max_ms={:.3} over_42ms={} over_83ms={}; CPU wall time or interval, not GPU duration",
                timing.count,
                timing.total_us as f64 / timing.count.max(1) as f64 / 1000.0,
                timing.p95_ms(),
                timing.max_us as f64 / 1000.0,
                timing.over_42ms,
                timing.over_83ms
            );
        }
    }
}

pub fn reset() {
    report(true);
    STATISTICS.with_borrow_mut(|stats| *stats = Statistics::default());
    tracing::info!(
        "Perf diagnostics: new movie; phase timings are inclusive CPU wall time; gap metrics are intervals"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_preserves_large_stalls_and_threshold_counts() {
        let mut timing = Timing::default();
        for _ in 0..94 {
            timing.record(Duration::from_millis(4));
        }
        for _ in 0..6 {
            timing.record(Duration::from_millis(1500));
        }
        assert_eq!(timing.p95_ms(), 1500.0);
        assert_eq!(timing.over_42ms, 6);
        assert_eq!(timing.over_83ms, 6);
        assert_eq!(timing.histogram.len(), 1001);
    }

    #[test]
    fn report_boundary_preserves_gap_but_new_movie_clears_it() {
        reset();
        gap("test.gap");
        record("test.phase", Duration::from_millis(5));
        report(true);
        STATISTICS.with_borrow(|stats| {
            assert!(stats.phases.is_empty());
            assert!(stats.gaps.contains_key("test.gap"));
        });
        reset();
        STATISTICS.with_borrow(|stats| assert!(stats.gaps.is_empty()));
    }
}
