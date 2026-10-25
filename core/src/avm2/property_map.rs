//! Property map

use crate::avm2::AvmString;
use crate::avm2::Multiname;
use crate::avm2::Namespace;
use crate::avm2::QName;
use fnv::FnvBuildHasher;
use gc_arena::Collect;
use smallvec::SmallVec;
use std::collections::HashMap;

/// Type which represents named properties on an object.
///
/// This type exposes interfaces akin to `HashMap<QName<'gc>, V>`, and is
/// intended to serve as a drop-in replacement optimized for objects where few
/// properties have overlapping local names. However, we have made slight
/// changes to the API in the following cases:
///
///  * Iterators return tuples of namespace, local-name, and value; rather than
///    a qualified name and value pair.
///  * Only `HashMap` methods and traits that we need are implemented.
///
/// The internal structure of the `PropertyMap` technically allows storage of
/// multiple values per `QName`. It's implementation enforces the invariant
/// that each `QName` only have one associated `V`.
#[derive(Clone, Debug, Collect)]
#[collect(no_drop)]
pub struct PropertyMap<'gc, V, const INLINE: usize = 2>(
    HashMap<AvmString<'gc>, SmallVec<[(Namespace<'gc>, V); INLINE]>, FnvBuildHasher>,
)
where
    [(Namespace<'gc>, V); INLINE]: smallvec::Array<Item = (Namespace<'gc>, V)>;

#[cfg(feature = "performance_diagnostics")]
#[derive(Default)]
pub(super) struct PropertyMapMemory {
    pub payload_lower_bound_bytes: usize,
    pub local_names: usize,
    pub single_namespace_names: usize,
    pub multi_namespace_names: usize,
    pub spilled_names: usize,
    pub namespace_entries: usize,
    pub entry_size: usize,
}

#[cfg(all(test, feature = "performance_diagnostics"))]
mod memory_tests {
    use super::*;
    use crate::avm2::api_version::ApiVersion;
    use crate::string::{AvmStringInterner, StringContext};

    #[test]
    fn census_preserves_namespaces_and_observes_spilled_and_empty_buckets() {
        let mut arena = gc_arena::Arena::<gc_arena::Rootable![AvmStringInterner<'_>]>::new(|mc| {
            AvmStringInterner::new(mc)
        });
        arena.mutate_root(|mc, interner| {
            let mut strings = StringContext::from_parts(mc, interner);
            let mut map: PropertyMap<'_, i32> = PropertyMap::new();
            assert_eq!(map.memory_diagnostics().payload_lower_bound_bytes, 0);
            let name = AvmString::new_utf8(mc, "shared");
            let single_name = AvmString::new_utf8(mc, "single");
            let mut keys = Vec::new();
            for index in 0..3 {
                let uri = AvmString::new_utf8(mc, format!("ns{index}"));
                let ns = Namespace::package(uri, ApiVersion::AllVersions, &mut strings);
                let key = QName::new(ns, name);
                keys.push(key);
                map.insert(key, index);
            }
            let single_key = QName::new(keys[0].namespace(), single_name);
            map.insert(single_key, 100);
            let before: Vec<_> = map
                .iter()
                .map(|(name, ns, value)| (name, ns, *value))
                .collect();
            let census = map.memory_diagnostics();
            assert_eq!(census.local_names, 2);
            assert_eq!(census.namespace_entries, 4);
            assert_eq!(census.single_namespace_names, 1);
            assert_eq!(census.multi_namespace_names, 1);
            assert_eq!(census.spilled_names, 1);
            assert_eq!(
                before,
                map.iter()
                    .map(|(name, ns, value)| (name, ns, *value))
                    .collect::<Vec<_>>()
            );
            let clone = map.clone();
            for (index, key) in keys.into_iter().enumerate() {
                assert_eq!(map.get(key), Some(&(index as i32)));
                assert_eq!(map.remove(key), Some(index as i32));
                assert_eq!(clone.get(key), Some(&(index as i32)));
            }
            assert_eq!(map.get(single_key), Some(&100));
            let after = map.memory_diagnostics();
            assert_eq!(after.local_names, 2); // Removal keeps the empty name bucket.
            assert_eq!(after.namespace_entries, 1);
            assert_eq!(after.single_namespace_names, 1);
            assert_eq!(after.multi_namespace_names, 0);
            assert!(after.payload_lower_bound_bytes > 0);
        });
    }

    /// Manual release-mode comparison; never gates correctness on noisy timing.
    #[test]
    #[ignore = "manual property layout capacity and timing experiment"]
    fn layout_capacity_and_lookup_probe() {
        use crate::avm2::property::Property;
        use std::{hint::black_box, time::Instant};
        fn run<const INLINE: usize>(round: usize)
        where
            for<'gc> [(Namespace<'gc>, Property); INLINE]:
                smallvec::Array<Item = (Namespace<'gc>, Property)>,
        {
            let mut arena =
                gc_arena::Arena::<gc_arena::Rootable![AvmStringInterner<'_>]>::new(|mc| {
                    AvmStringInterner::new(mc)
                });
            arena.mutate_root(|mc, interner| {
                let mut strings = StringContext::from_parts(mc, interner);
                let first = Namespace::package(AvmString::new_utf8(mc,"first"), ApiVersion::AllVersions, &mut strings);
                let second = Namespace::package(AvmString::new_utf8(mc,"second"), ApiVersion::AllVersions, &mut strings);
                let mut base: PropertyMap<'_, Property, INLINE> = PropertyMap::new();
                let mut keys = Vec::new();
                for index in 0..64 {
                    let name = AvmString::new_utf8(mc, format!("field{index}"));
                    let key = QName::new(first, name);
                    base.insert(key, Property::new_slot(index));
                    keys.push(key);
                    if index % 13 == 0 {
                        let key = QName::new(second, name);
                        base.insert(key, Property::new_slot(index+100));
                        keys.push(key);
                    }
                }
                let begin = Instant::now();
                let maps: Vec<_> = (0..2048).map(|_| base.clone()).collect();
                let build_ms = begin.elapsed().as_secs_f64()*1000.0;
                let payload: usize = maps.iter().map(|map| map.memory_diagnostics().payload_lower_bound_bytes).sum();
                let begin = Instant::now();
                let mut checksum = 0usize;
                for index in 0..2_000_000 {
                    let key = black_box(keys[index % keys.len()]);
                    let map = black_box(&maps[(index / keys.len()) % maps.len()]);
                    match black_box(map.get(key)) {
                        Some(Property::Slot { slot_id }) => checksum = checksum.wrapping_add(*slot_id),
                        _ => panic!("lost property"),
                    }
                }
                let lookup_ms = begin.elapsed().as_secs_f64()*1000.0;
                let entry_size = base.memory_diagnostics().entry_size;
                let begin = Instant::now();
                drop(maps);
                let drop_ms = begin.elapsed().as_secs_f64()*1000.0;
                println!("layout-probe round={round} inline={INLINE} entry_size={entry_size} payload_bytes={payload} build_ms={build_ms:.3} lookup_ms={lookup_ms:.3} drop_ms={drop_ms:.3} checksum={checksum}");
            });
        }
        for round in 0..5 {
            if round % 2 == 0 {
                run::<2>(round);
                run::<1>(round);
            } else {
                run::<1>(round);
                run::<2>(round);
            }
        }
    }
}

impl<'gc, V, const INLINE: usize> Default for PropertyMap<'gc, V, INLINE>
where
    [(Namespace<'gc>, V); INLINE]: smallvec::Array<Item = (Namespace<'gc>, V)>,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<'gc, V, const INLINE: usize> PropertyMap<'gc, V, INLINE>
where
    [(Namespace<'gc>, V); INLINE]: smallvec::Array<Item = (Namespace<'gc>, V)>,
{
    /// Estimates owned payload only. HashMap capacity excludes some buckets,
    /// control bytes and allocator overhead, so this is deliberately a lower
    /// bound rather than an exact heap size. Strings and values are not followed.
    #[cfg(feature = "performance_diagnostics")]
    pub(super) fn memory_diagnostics(&self) -> PropertyMapMemory {
        let entry_size =
            std::mem::size_of::<(AvmString<'gc>, SmallVec<[(Namespace<'gc>, V); INLINE]>)>();
        let mut result = PropertyMapMemory {
            payload_lower_bound_bytes: self.0.capacity() * entry_size,
            local_names: self.0.len(),
            entry_size,
            ..Default::default()
        };
        for bucket in self.0.values() {
            result.namespace_entries += bucket.len();
            result.single_namespace_names += usize::from(bucket.len() == 1);
            result.multi_namespace_names += usize::from(bucket.len() > 1);
            if bucket.spilled() {
                result.spilled_names += 1;
                result.payload_lower_bound_bytes +=
                    bucket.capacity() * std::mem::size_of::<(Namespace<'gc>, V)>();
            }
        }
        result
    }

    pub fn new() -> Self {
        Self(Default::default())
    }

    pub fn get(&self, name: QName<'gc>) -> Option<&V> {
        self.0.get(&name.local_name()).iter().find_map(|v| {
            v.iter()
                .filter(|(n, _)| n.matches_ns(name.namespace()))
                .map(|(_, v)| v)
                .next()
        })
    }

    pub fn get_for_multiname(&self, name: &Multiname<'gc>) -> Option<&V> {
        if name.has_lazy_component() {
            unreachable!("Lookup on lazy Multiname should never happen ({:?})", name);
        }
        if let Some(local_name) = name.local_name() {
            self.0.get(&local_name).iter().find_map(|v| {
                v.iter()
                    .filter(|(n, _)| name.namespace_set().iter().any(|ns| n.matches_ns(*ns)))
                    .map(|(_, v)| v)
                    .next()
            })
        } else {
            None
        }
    }

    pub fn get_with_ns_for_multiname(&self, name: &Multiname<'gc>) -> Option<(Namespace<'gc>, &V)> {
        if name.has_lazy_component() {
            unreachable!("Lookup on lazy Multiname should never happen ({:?})", name);
        }
        if let Some(local_name) = name.local_name() {
            self.0.get(&local_name).iter().find_map(|v| {
                v.iter()
                    .filter(|(n, _)| name.namespace_set().iter().any(|ns| n.matches_ns(*ns)))
                    .map(|(ns, v)| (*ns, v))
                    .next()
            })
        } else {
            None
        }
    }

    pub fn get_mut(&mut self, name: QName<'gc>) -> Option<&mut V> {
        if let Some(bucket) = self.0.get_mut(&name.local_name())
            && let Some((_, old_value)) = bucket
                .iter_mut()
                .find(|(n, _)| n.matches_ns(name.namespace()))
        {
            return Some(old_value);
        }

        None
    }

    pub fn contains_key(&self, name: QName<'gc>) -> bool {
        self.0
            .get(&name.local_name())
            .iter()
            .any(|v| v.iter().any(|(n, _)| n.matches_ns(name.namespace())))
    }

    pub fn iter(&self) -> impl Iterator<Item = (AvmString<'gc>, Namespace<'gc>, &V)> {
        self.0
            .iter()
            .flat_map(|(k, vs)| vs.iter().map(|(ns, v)| (*k, *ns, v)))
    }

    pub fn insert(&mut self, name: QName<'gc>, value: V) -> Option<V> {
        let bucket = self.0.entry(name.local_name()).or_default();

        if let Some(position) = bucket
            .iter()
            .position(|(n, _)| n.matches_ns(name.namespace()))
        {
            let (_, old_value) = bucket.remove(position);
            bucket.insert(0, (name.namespace(), value));

            Some(old_value)
        } else {
            bucket.insert(0, (name.namespace(), value));

            None
        }
    }

    /// Preserves Flash Player ordering in scope caches and application domains.
    pub fn insert_at_end(&mut self, name: QName<'gc>, value: V) -> Option<V> {
        let bucket = self.0.entry(name.local_name()).or_default();

        if let Some((_, old_value)) = bucket
            .iter_mut()
            .find(|(n, _)| n.matches_ns(name.namespace()))
        {
            Some(std::mem::replace(old_value, value))
        } else {
            bucket.push((name.namespace(), value));
            None
        }
    }

    pub fn remove(&mut self, name: QName<'gc>) -> Option<V> {
        let bucket = self.0.get_mut(&name.local_name());

        if let Some(bucket) = bucket {
            let position = bucket
                .iter_mut()
                .enumerate()
                .find(|(_, (n, _))| n.matches_ns(name.namespace()));
            if let Some((position, _)) = position {
                return Some(bucket.remove(position).1);
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::avm2::api_version::ApiVersion;
    use crate::string::{AvmStringInterner, StringContext};

    #[test]
    fn both_layouts_preserve_multi_namespace_priority_and_clone_independence() {
        fn check<const INLINE: usize>()
        where
            for<'gc> [(Namespace<'gc>, i32); INLINE]: smallvec::Array<Item = (Namespace<'gc>, i32)>,
        {
            let mut arena =
                gc_arena::Arena::<gc_arena::Rootable![AvmStringInterner<'_>]>::new(|mc| {
                    AvmStringInterner::new(mc)
                });
            arena.mutate_root(|mc, interner| {
                let mut strings = StringContext::from_parts(mc, interner);
                let name = AvmString::new_utf8(mc, "same_name");
                let mut keys = Vec::new();
                let mut map: PropertyMap<'_, i32, INLINE> = PropertyMap::new();
                for index in 0..3 {
                    let ns = Namespace::package(
                        AvmString::new_utf8(mc, format!("ns{index}")),
                        ApiVersion::AllVersions,
                        &mut strings,
                    );
                    let key = QName::new(ns, name);
                    keys.push(key);
                    map.insert(key, index);
                }
                let clone = map.clone();
                assert_eq!(map.insert(keys[0], 10), Some(0));
                assert_eq!(
                    map.iter().map(|(_, _, value)| *value).collect::<Vec<_>>(),
                    [10, 2, 1]
                );
                *map.get_mut(keys[1]).unwrap() = 11;
                assert_eq!(map.remove(keys[2]), Some(2));
                assert_eq!(map.get(keys[0]), Some(&10));
                assert_eq!(map.get(keys[1]), Some(&11));
                assert_eq!(clone.get(keys[0]), Some(&0));
                assert_eq!(clone.get(keys[1]), Some(&1));
                assert_eq!(clone.get(keys[2]), Some(&2));
            });
        }
        check::<1>();
        check::<2>();
    }
}
