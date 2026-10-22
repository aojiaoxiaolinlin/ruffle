//! Allocation-free per-layer counters. Pixels describe source target area,
//! not bytes, GPU time, simultaneous residency, or scheduler deferral hits.
#[derive(Debug, Clone, Copy)]
#[repr(usize)]
pub enum BlendFallback {
    Trivial,
    Ancestor,
    Shader,
    Disabled,
    ActiveMask,
    Nested,
    MaskCommands,
    ThreeD,
    Lines,
    InvalidBounds,
    LargeBounds,
    EmptyOrOffscreen,
}

impl BlendFallback {
    pub const COUNT: usize = 12;
    const NAMES: [&'static str; Self::COUNT] = [
        "trivial",
        "ancestor",
        "shader",
        "disabled",
        "active_mask",
        "nested",
        "mask_commands",
        "3d",
        "lines",
        "invalid_bounds",
        "large_bounds",
        "empty_offscreen",
    ];
}

#[derive(Debug, Default, Clone, Copy)]
pub struct BlendRegionStats {
    pub prepared: u64,
    pub candidates: u64,
    pub cropped: u64,
    pub original_pixels: u64,
    pub actual_pixels: u64,
    pub candidate_original_pixels: u64,
    pub candidate_actual_pixels: u64,
    /// One first-failure reason per non-cropped prepared layer, including
    /// modes outside the seven complex blend candidates.
    pub fallback: [u64; BlendFallback::COUNT],
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SoftBlurStats {
    pub requests: u64,
    pub reduced: u64,
    pub original_pixels: u64,
    pub actual_pixels: u64,
}

impl SoftBlurStats {
    pub(crate) fn accumulate(&mut self, other: Self) {
        self.requests += other.requests;
        self.reduced += other.reduced;
        self.original_pixels += other.original_pixels;
        self.actual_pixels += other.actual_pixels;
    }
}

impl BlendRegionStats {
    pub(crate) fn record(
        &mut self,
        candidate: bool,
        original: u64,
        actual: u64,
        reason: Option<BlendFallback>,
    ) {
        self.prepared += 1;
        self.candidates += u64::from(candidate);
        self.original_pixels += original;
        self.actual_pixels += actual;
        if candidate {
            self.candidate_original_pixels += original;
            self.candidate_actual_pixels += actual;
        }
        if let Some(reason) = reason {
            self.fallback[reason as usize] += 1;
        } else {
            self.cropped += 1;
        }
    }

    pub(crate) fn accumulate(&mut self, other: Self) {
        self.prepared += other.prepared;
        self.candidates += other.candidates;
        self.cropped += other.cropped;
        self.original_pixels += other.original_pixels;
        self.actual_pixels += other.actual_pixels;
        self.candidate_original_pixels += other.candidate_original_pixels;
        self.candidate_actual_pixels += other.candidate_actual_pixels;
        for (total, count) in self.fallback.iter_mut().zip(other.fallback) {
            *total += count;
        }
    }

    pub(crate) fn fallback_summary(&self) -> String {
        BlendFallback::NAMES
            .iter()
            .zip(self.fallback)
            .filter(|(_, count)| *count != 0)
            .map(|(name, count)| format!("{name}:{count}"))
            .collect::<Vec<_>>()
            .join(",")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn candidate_denominator_excludes_ancestor_layers_and_snapshots_accumulate() {
        let mut frame = BlendRegionStats::default();
        frame.record(true, 10000, 1000, None);
        frame.record(true, 10000, 10000, Some(BlendFallback::Nested));
        frame.record(false, 10000, 10000, Some(BlendFallback::Ancestor));
        let mut interval = BlendRegionStats::default();
        interval.accumulate(frame);
        interval.accumulate(frame);
        assert_eq!(
            (interval.prepared, interval.candidates, interval.cropped),
            (6, 4, 2)
        );
        assert_eq!(
            (
                interval.candidate_original_pixels,
                interval.candidate_actual_pixels
            ),
            (40000, 22000)
        );
        assert_eq!(
            interval.fallback.iter().sum::<u64>() + interval.cropped,
            interval.prepared
        );
        assert_eq!(interval.fallback_summary(), "ancestor:2,nested:2");
    }
}
