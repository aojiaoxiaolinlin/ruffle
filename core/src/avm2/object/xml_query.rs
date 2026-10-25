//! Capacity policy for freshly constructed XML query results. Each query keeps
//! its own buffer and its normal target links; this does not cache XMLLists.

pub(super) fn finish<T>(mut children: Vec<T>) -> Vec<T> {
    #[cfg(feature = "xml_query_diagnostics")]
    let before = children.capacity() * std::mem::size_of::<T>();

    if cfg!(feature = "xml_query_memory_experiment") && should_compact(&children) {
        #[cfg(feature = "performance_diagnostics")]
        let _scope = crate::performance_diagnostics::Scope::new("core.xml-query-compact");
        children.shrink_to_fit();
    }

    #[cfg(feature = "xml_query_diagnostics")]
    diagnostics::record(
        children.len() * std::mem::size_of::<T>(),
        before,
        children.capacity() * std::mem::size_of::<T>(),
    );
    children
}

fn should_compact<T>(children: &Vec<T>) -> bool {
    let unused = children.capacity() - children.len();
    // Avoid an allocator call for small or nearly full results. A query has
    // finished growing here, but the resulting list remains mutable as usual.
    unused * std::mem::size_of::<T>() >= 4096 && unused >= children.capacity() / 4
}

#[cfg(feature = "xml_query_diagnostics")]
pub(crate) mod diagnostics {
    use std::sync::atomic::{AtomicU64, Ordering};
    static QUERIES: AtomicU64 = AtomicU64::new(0);
    static ELEMENT_BYTES: AtomicU64 = AtomicU64::new(0);
    static BEFORE_BYTES: AtomicU64 = AtomicU64::new(0);
    static AFTER_BYTES: AtomicU64 = AtomicU64::new(0);
    static COMPACTED: AtomicU64 = AtomicU64::new(0);

    pub(super) fn record(elements: usize, before: usize, after: usize) {
        QUERIES.fetch_add(1, Ordering::Relaxed);
        ELEMENT_BYTES.fetch_add(elements as u64, Ordering::Relaxed);
        BEFORE_BYTES.fetch_add(before as u64, Ordering::Relaxed);
        AFTER_BYTES.fetch_add(after as u64, Ordering::Relaxed);
        if after < before {
            COMPACTED.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn summary() -> String {
        format!(
            "xml-query-buffers: scope=process compact={} queries={} element_bytes={} capacity_before_bytes={} capacity_after_bytes={} compacted_queries={}; cumulative construction totals, not live memory or physical RAM",
            cfg!(feature = "xml_query_memory_experiment"),
            QUERIES.load(Ordering::Relaxed),
            ELEMENT_BYTES.load(Ordering::Relaxed),
            BEFORE_BYTES.load(Ordering::Relaxed),
            AFTER_BYTES.load(Ordering::Relaxed),
            COMPACTED.load(Ordering::Relaxed),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compaction_preserves_query_order_and_subsequent_mutation() {
        let mut values = Vec::with_capacity(2048);
        values.extend(0..1201_u64);
        let before = values.capacity();
        assert!(should_compact(&values));
        let mut result = finish(values);
        assert!(result.iter().copied().eq(0..1201));
        if cfg!(feature = "xml_query_memory_experiment") {
            assert!(result.capacity() < before);
        } else {
            assert_eq!(result.capacity(), before);
        }
        result[0] = 99;
        result.push(1201);
        assert_eq!(result[0], 99);
        assert_eq!(result.pop(), Some(1201));
    }

    #[test]
    fn small_and_nearly_full_queries_avoid_allocator_calls() {
        assert!(!should_compact(&Vec::<u64>::new()));
        assert!(!should_compact(&Vec::<u64>::with_capacity(64)));
        let mut nearly_full = Vec::with_capacity(2048);
        nearly_full.resize(2000, 0_u64);
        assert!(!should_compact(&nearly_full));
        let capacity = nearly_full.capacity();
        assert_eq!(finish(nearly_full).capacity(), capacity);
    }
}
