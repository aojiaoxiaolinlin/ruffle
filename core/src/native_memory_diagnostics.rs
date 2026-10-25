//! Diagnostic ownership accounting, not a heap profiler. No object references
//! are retained. Totals are process-wide and counts include unswept garbage.
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
struct Totals {
    created: AtomicU64,
    dropped: AtomicU64,
    bytes: AtomicU64,
}

static TOTALS: [Totals; 8] = [const {
    Totals {
        created: AtomicU64::new(0),
        dropped: AtomicU64::new(0),
        bytes: AtomicU64::new(0),
    }
}; 8];

// Only updated after a VTable has been built, never on property lookup. These
// histograms are cumulative construction counts, not current live entries.
static VTABLE_LAYOUT_BUILDS: [AtomicU64; 7] = [const { AtomicU64::new(0) }; 7];
static VTABLE_ENTRY_SIZE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn record_vtable_layout(layout: [usize; 7], entry_size: usize) {
    for (total, value) in VTABLE_LAYOUT_BUILDS.iter().zip(layout) {
        total.fetch_add(value as u64, Ordering::Relaxed);
    }
    VTABLE_ENTRY_SIZE.store(entry_size as u64, Ordering::Relaxed);
}

/// Counts only the explicitly supplied owned payload, excluding allocator
/// overhead and the owner's inline data. Never stores addresses or values.
#[derive(Debug)]
pub(crate) struct Allocation<const KIND: usize> {
    bytes: usize,
}

impl<const KIND: usize> Allocation<KIND> {
    pub fn new(bytes: usize) -> Self {
        TOTALS[KIND].created.fetch_add(1, Ordering::Relaxed);
        TOTALS[KIND]
            .bytes
            .fetch_add(bytes as u64, Ordering::Relaxed);
        Self { bytes }
    }

    pub fn resize(&mut self, bytes: usize) {
        if bytes >= self.bytes {
            TOTALS[KIND]
                .bytes
                .fetch_add((bytes - self.bytes) as u64, Ordering::Relaxed);
        } else {
            TOTALS[KIND]
                .bytes
                .fetch_sub((self.bytes - bytes) as u64, Ordering::Relaxed);
        }
        self.bytes = bytes;
    }
}

impl<const KIND: usize> Default for Allocation<KIND> {
    fn default() -> Self {
        Self::new(0)
    }
}

impl<const KIND: usize> Clone for Allocation<KIND> {
    fn clone(&self) -> Self {
        Self::new(self.bytes)
    }
}

impl<const KIND: usize> Drop for Allocation<KIND> {
    fn drop(&mut self) {
        TOTALS[KIND]
            .bytes
            .fetch_sub(self.bytes as u64, Ordering::Relaxed);
        TOTALS[KIND].dropped.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) fn summary() -> String {
    let mut output = String::from("native-memory: scope=process");
    use std::fmt::Write;
    for (name, index) in [
        ("script_slots", 0),
        ("bytearray_capacity", 1),
        ("bitmap_pixels", 2),
        ("vtable_tables", 3),
        ("vtable_traits_lower_bound", 6),
        ("vtable_metadata_lower_bound", 7),
    ] {
        let totals = &TOTALS[index];
        // Different atomics need not form a simultaneous snapshot.
        let dropped = totals.dropped.load(Ordering::Relaxed);
        let created = totals.created.load(Ordering::Relaxed);
        let bytes = totals.bytes.load(Ordering::Relaxed);
        let _ = write!(
            output,
            " {name}_live={} {name}_created={created} {name}_dropped={dropped} {name}_bytes={bytes}",
            created.saturating_sub(dropped)
        );
    }
    output.push_str("; selected owned capacities only, excludes inline objects, dynamic properties, temporary buffers and allocator overhead");
    output.push_str("; vtable-layout: scope=process totals=cumulative-builds");
    for (name, total) in [
        "builds",
        "local_names",
        "single_namespace_names",
        "multi_namespace_names",
        "spilled_names",
        "namespace_entries",
        "inherited_namespace_entries",
    ]
    .into_iter()
    .zip(&VTABLE_LAYOUT_BUILDS)
    {
        let _ = write!(output, " {name}={}", total.load(Ordering::Relaxed));
    }
    let _ = write!(
        output,
        " entry_size={}",
        VTABLE_ENTRY_SIZE.load(Ordering::Relaxed)
    );
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gc_sweep_releases_accounting_without_diagnostic_roots() {
        use gc_arena::{Arena, Collect, Gc, Rootable};
        #[derive(Collect)]
        #[collect(no_drop)]
        struct Payload {
            #[collect(require_static)]
            _memory: Allocation<5>,
        }
        let mut arena = Arena::<Rootable![Option<Gc<'_, Payload>>]>::new(|mc| {
            Some(Gc::new(
                mc,
                Payload {
                    _memory: Allocation::new(128),
                },
            ))
        });
        assert_eq!(TOTALS[5].bytes.load(Ordering::Relaxed), 128);
        arena.mutate_root(|_, root| *root = None);
        arena.finish_cycle();
        assert_eq!(TOTALS[5].bytes.load(Ordering::Relaxed), 0);
        assert_eq!(TOTALS[5].created.load(Ordering::Relaxed), 1);
        assert_eq!(TOTALS[5].dropped.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn ownership_tracks_clone_resize_transfer_and_drop_without_retaining_buffers() {
        let totals = &TOTALS[4];
        let initial_created = totals.created.load(Ordering::Relaxed);
        let initial_dropped = totals.dropped.load(Ordering::Relaxed);
        let initial_bytes = totals.bytes.load(Ordering::Relaxed);
        {
            let mut a = Allocation::<4>::new(100);
            let b = a.clone();
            a.resize(25);
            assert_eq!(totals.bytes.load(Ordering::Relaxed), initial_bytes + 125);
            let moved = b;
            drop(moved);
            assert_eq!(totals.bytes.load(Ordering::Relaxed), initial_bytes + 25);
            a.resize(0);
            assert_eq!(totals.bytes.load(Ordering::Relaxed), initial_bytes);
        }
        assert_eq!(totals.created.load(Ordering::Relaxed), initial_created + 2);
        assert_eq!(totals.dropped.load(Ordering::Relaxed), initial_dropped + 2);
        assert_eq!(totals.bytes.load(Ordering::Relaxed), initial_bytes);
    }
}
