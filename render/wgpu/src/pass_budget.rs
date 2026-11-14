//! Offline experiment, deliberately separate from the production draw guard.
//!
//! Each owned 2D command encoder supplies its count on submission. Context3D
//! and copy-only external encoders are excluded; this is not a HAL inventory.

/// Actual 2D render passes in this ActiveFrame's submitted encoders. These
/// numbers are not Vulkan raw-buffer counts or a driver-memory estimate.
#[derive(Clone, Debug, Default)]
pub struct PassBudgetStats {
    pub budget: u64,
    pub observed_passes: u64,
    pub submissions: u64,
    pub max_observed_passes_per_submission: u64,
    pub last_observed_passes_per_submission: u64,
    pub budget_flushes: u64,
    pub draw_guard_flushes: u64,
}

#[derive(Default)]
pub(crate) struct PassBudget {
    pub stats: PassBudgetStats,
    #[cfg(test)]
    pub submission_passes: Vec<u64>,
}

impl PassBudget {
    pub fn should_flush(&self, passes: u64) -> bool {
        self.stats.budget != 0 && passes >= self.stats.budget
    }

    pub fn submitted(&mut self, observed: u64) {
        #[cfg(test)]
        self.submission_passes.push(observed);
        self.stats.observed_passes += observed;
        self.stats.submissions += 1;
        self.stats.last_observed_passes_per_submission = observed;
        self.stats.max_observed_passes_per_submission =
            self.stats.max_observed_passes_per_submission.max(observed);
    }
}
