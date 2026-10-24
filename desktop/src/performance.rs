//! Bounded CPU wall-time statistics; never waits for GPU completion.
use std::time::{Duration, Instant};

/// Compiles to a no-op unless the explicit diagnostic feature is enabled.
#[cfg(feature = "performance_diagnostics")]
pub fn phase(name: &'static str) -> ruffle_core::performance_diagnostics::Scope {
    ruffle_core::performance_diagnostics::Scope::new(name)
}

#[cfg(not(feature = "performance_diagnostics"))]
pub struct InactiveScope;

#[cfg(not(feature = "performance_diagnostics"))]
pub fn phase(_: &'static str) -> InactiveScope {
    InactiveScope
}

#[derive(Default)]
struct Timing {
    count: u64,
    sum_us: u128,
    max_us: u128,
    // Rounded up to 1 ms; the final bucket holds >= 1000 ms. If P95
    // lands there, max is a valid (but coarser) upper bound.
    histogram: Vec<u64>,
}

impl Timing {
    fn record(&mut self, elapsed: Duration) {
        if self.histogram.is_empty() {
            self.histogram.resize(1001, 0);
        }
        let us = elapsed.as_nanos().div_ceil(1000);
        self.count += 1;
        self.sum_us += us;
        self.max_us = self.max_us.max(us);
        self.histogram[us.div_ceil(1000).min(1000) as usize] += 1;
    }

    fn avg_ms(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.sum_us as f64 / self.count as f64 / 1000.0
        }
    }
    fn max_ms(&self) -> f64 {
        self.max_us as f64 / 1000.0
    }
    fn p95_le_ms(&self) -> f64 {
        if self.count == 0 {
            return 0.0;
        }
        let rank = (u128::from(self.count) * 95).div_ceil(100) as u64;
        let mut seen = 0;
        for (ms, count) in self.histogram.iter().enumerate() {
            seen += count;
            if seen >= rank {
                return if ms == 1000 { self.max_ms() } else { ms as f64 };
            }
        }
        self.max_ms()
    }

    fn clear(&mut self) {
        self.count = 0;
        self.sum_us = 0;
        self.max_us = 0;
        self.histogram.fill(0);
    }
}

pub struct DesktopPerformance {
    start: Instant,
    tick: Timing,
    render: Timing,
    gui: Timing,
}

impl Default for DesktopPerformance {
    fn default() -> Self {
        Self {
            start: Instant::now(),
            tick: Timing::default(),
            render: Timing::default(),
            gui: Timing::default(),
        }
    }
}

impl DesktopPerformance {
    pub fn tick(&mut self, elapsed: Duration) {
        self.tick.record(elapsed);
    }
    pub fn render(&mut self, elapsed: Duration) {
        self.render.record(elapsed);
    }
    pub fn gui(&mut self, elapsed: Duration) {
        self.gui.record(elapsed);
    }

    pub fn report_if_due(&mut self) {
        if self.start.elapsed() >= Duration::from_secs(5) {
            self.report();
        }
    }

    pub fn report(&mut self) {
        let seconds = self.start.elapsed().as_secs_f64();
        if self.tick.count + self.render.count + self.gui.count != 0 {
            tracing::info!(
                "Desktop perf: window_s={:.3} tick_calls={} tick_avg_ms={:.3} tick_p95_le_ms={:.3} tick_max_ms={:.3} render_calls={} render_hz={:.2} render_avg_ms={:.3} render_p95_le_ms={:.3} render_max_ms={:.3} gui_calls={} gui_avg_ms={:.3} gui_p95_le_ms={:.3} gui_max_ms={:.3}; CPU wall time, render_hz is redraw throughput, not SWF FPS or GPU duration",
                seconds,
                self.tick.count,
                self.tick.avg_ms(),
                self.tick.p95_le_ms(),
                self.tick.max_ms(),
                self.render.count,
                self.render.count as f64 / seconds.max(f64::EPSILON),
                self.render.avg_ms(),
                self.render.p95_le_ms(),
                self.render.max_ms(),
                self.gui.count,
                self.gui.avg_ms(),
                self.gui.p95_le_ms(),
                self.gui.max_ms(),
            );
        }
        self.tick.clear();
        self.render.clear();
        self.gui.clear();
        self.start = Instant::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn histogram_quantiles_include_stalls_and_clear_between_windows() {
        let mut timing = Timing::default();
        for _ in 0..94 {
            timing.record(Duration::from_micros(500));
        }
        for _ in 0..6 {
            timing.record(Duration::from_millis(1500));
        }
        assert_eq!(timing.p95_le_ms(), 1500.0);
        assert!((timing.avg_ms() - 90.47).abs() < 0.001);
        timing.clear();
        timing.record(Duration::from_micros(2300));
        assert_eq!(timing.count, 1);
        assert_eq!(timing.p95_le_ms(), 3.0);
        assert_eq!(timing.max_ms(), 2.3);
        assert_eq!(timing.histogram.len(), 1001);
    }
}
