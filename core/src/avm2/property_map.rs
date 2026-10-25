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
