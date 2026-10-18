use crate::descriptors::Descriptors;
use crate::globals::Globals;
use fnv::FnvHashMap;
use std::fmt::{Debug, Formatter};
use std::ops::Deref;
use std::sync::{Arc, Mutex, Weak};

type PoolInner<T> = Mutex<Vec<T>>;
type Constructor<Type, Description> = Box<dyn Fn(&Descriptors, &Description) -> Type>;

#[derive(Debug, Default)]
pub struct TexturePool {
    pools: FnvHashMap<TextureKey, BufferPool<(wgpu::Texture, wgpu::TextureView), AlwaysCompatible>>,
    globals_cache: FnvHashMap<GlobalsKey, Arc<Globals>>,
}

impl TexturePool {
    pub fn new() -> Self {
        Default::default()
    }

    /// Evict only idle scratch textures. Movie bitmaps and checked-out entries
    /// remain owned by their callers and are never destroyed by this operation.
    pub fn trim_idle(&mut self, mut remaining_bytes: u64) {
        self.pools.retain(|key, pool| {
            let bytes = key.estimated_bytes();
            pool.retain_idle(|_| {
                if bytes <= remaining_bytes {
                    remaining_bytes -= bytes;
                    true
                } else {
                    false
                }
            })
        });
        // Dimension-specific uniforms should not accumulate across every scene.
        if self.globals_cache.len() > 128 {
            self.globals_cache.clear();
        }
    }

    #[track_caller]
    pub fn get_texture(
        &mut self,
        descriptors: &Descriptors,
        size: wgpu::Extent3d,
        usage: wgpu::TextureUsages,
        format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> PoolEntry<(wgpu::Texture, wgpu::TextureView), AlwaysCompatible> {
        let key = TextureKey {
            size,
            usage,
            format,
            sample_count,
        };
        let texture_bytes =
            u64::from(size.width) * u64::from(size.height) * u64::from(sample_count) * 4;
        if texture_bytes >= 4 * 1024 * 1024 {
            let caller = std::panic::Location::caller();
            tracing::info!(
                "requesting pooled large texture {}x{} samples {} ({:.1} MiB) at {}:{}",
                size.width,
                size.height,
                sample_count,
                texture_bytes as f64 / (1024.0 * 1024.0),
                caller.file(),
                caller.line()
            );
        }
        let pool = self.pools.entry(key).or_insert_with(|| {
            let label = if cfg!(feature = "render_debug_labels") {
                use std::sync::atomic::{AtomicU32, Ordering};
                static ID_COUNT: AtomicU32 = AtomicU32::new(0);
                let id = ID_COUNT.fetch_add(1, Ordering::Relaxed);
                create_debug_label!("Pooled texture {}", id)
            } else {
                None
            };
            BufferPool::new(Box::new(move |descriptors, _description| {
                tracing::debug!(
                    "creating pooled texture {}x{} format {:?} samples {}",
                    size.width,
                    size.height,
                    format,
                    sample_count
                );
                let texture_bytes =
                    u64::from(size.width) * u64::from(size.height) * u64::from(sample_count) * 4;
                if texture_bytes >= 4 * 1024 * 1024 {
                    tracing::info!(
                        "creating pooled large texture {}x{} samples {} ({:.1} MiB)",
                        size.width,
                        size.height,
                        sample_count,
                        texture_bytes as f64 / (1024.0 * 1024.0)
                    );
                }
                let texture = descriptors.device.create_texture(&wgpu::TextureDescriptor {
                    label: label.as_deref(),
                    size,
                    mip_level_count: 1,
                    sample_count,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    view_formats: &[format],
                    usage,
                });
                let view = texture.create_view(&Default::default());
                (texture, view)
            }))
        });
        pool.take(descriptors, AlwaysCompatible)
    }

    /// Bytes retained in idle scratch textures. Used for diagnostics and to
    /// decide when to trim the pool.
    pub fn estimated_bytes(&self) -> u64 {
        self.pools
            .iter()
            .map(|(key, pool)| key.estimated_bytes() * pool.idle_len() as u64)
            .sum()
    }

    /// Number of idle scratch textures currently retained.
    pub fn idle_len(&self) -> usize {
        self.pools.values().map(BufferPool::idle_len).sum()
    }

    /// Number of distinct texture sizes currently tracked.
    pub fn pool_count(&self) -> usize {
        self.pools.len()
    }

    pub fn get_globals(
        &mut self,
        descriptors: &Descriptors,
        viewport_width: u32,
        viewport_height: u32,
    ) -> Arc<Globals> {
        self.globals_cache
            .entry(GlobalsKey {
                viewport_width,
                viewport_height,
            })
            .or_insert_with(|| {
                Arc::new(Globals::new(
                    &descriptors.device,
                    &descriptors.bind_layouts.globals,
                    viewport_width,
                    viewport_height,
                ))
            })
            .clone()
    }
}

#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq)]
struct TextureKey {
    size: wgpu::Extent3d,
    usage: wgpu::TextureUsages,
    format: wgpu::TextureFormat,
    sample_count: u32,
}

impl TextureKey {
    fn estimated_bytes(&self) -> u64 {
        let (block_width, block_height) = self.format.block_dimensions();
        u64::from(self.size.width.div_ceil(block_width))
            * u64::from(self.size.height.div_ceil(block_height))
            * u64::from(self.size.depth_or_array_layers)
            * u64::from(self.sample_count)
            * u64::from(self.format.block_copy_size(None).unwrap_or(16))
    }
}

#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq)]
struct GlobalsKey {
    viewport_width: u32,
    viewport_height: u32,
}

pub trait BufferDescription: Clone + Debug {
    type Cost: Ord;

    /// If the potential buffer represented by this description (`self`)
    /// fits another existing buffer and its description (`other`),
    /// return the cost to use that buffer instead of making a new one.
    ///
    /// Cost is an arbitrary unit, but lower is better.
    /// None means that the other buffer cannot be used in place of this one.
    fn cost_to_use(&self, other: &Self) -> Option<Self::Cost>;
}

#[derive(Clone, Debug)]
pub struct AlwaysCompatible;

impl BufferDescription for AlwaysCompatible {
    type Cost = ();

    fn cost_to_use(&self, _other: &Self) -> Option<()> {
        Some(())
    }
}

pub struct BufferPool<Type, Description: BufferDescription> {
    available: Arc<PoolInner<(Type, Description)>>,
    constructor: Constructor<Type, Description>,
}

impl<Type, Description: BufferDescription> Debug for BufferPool<Type, Description> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BufferPool").finish()
    }
}

impl<Type, Description: BufferDescription> BufferPool<Type, Description> {
    fn retain_idle(&self, mut keep: impl FnMut(&Type) -> bool) -> bool {
        let mut available = self.available.lock().expect("Pool lock poisoned");
        available.retain(|(item, _)| keep(item));
        !available.is_empty()
    }

    fn idle_len(&self) -> usize {
        self.available.lock().expect("Pool lock poisoned").len()
    }

    pub fn new(constructor: Constructor<Type, Description>) -> Self {
        Self {
            available: Arc::new(Mutex::new(vec![])),
            constructor,
        }
    }

    pub fn take(
        &self,
        descriptors: &Descriptors,
        description: Description,
    ) -> PoolEntry<Type, Description> {
        let mut guard = self
            .available
            .lock()
            .expect("Should not be able to lock recursively");
        let mut best: Option<(Description::Cost, usize)> = None;
        for i in 0..guard.len() {
            if let Some(cost) = description.cost_to_use(&guard[i].1) {
                if let Some(best) = &mut best {
                    if best.0 > cost {
                        *best = (cost, i);
                    }
                } else if best.is_none() {
                    best = Some((cost, i));
                }
            }
        }

        let (item, used_description) = if let Some((_, best)) = best {
            guard.swap_remove(best)
        } else {
            let item = (self.constructor)(descriptors, &description);
            (item, description)
        };
        PoolEntry {
            item: Some(item),
            description: used_description,
            pool: Arc::downgrade(&self.available),
        }
    }
}

pub struct PoolEntry<Type, Description: BufferDescription> {
    item: Option<Type>,
    description: Description,
    pool: Weak<PoolInner<(Type, Description)>>,
}

impl<Type, Description: BufferDescription> Debug for PoolEntry<Type, Description>
where
    Type: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("PoolEntry").field(&self.item).finish()
    }
}

impl<Type, Description: BufferDescription> Drop for PoolEntry<Type, Description> {
    fn drop(&mut self) {
        if let Some(item) = self.item.take()
            && let Some(pool) = self.pool.upgrade()
        {
            pool.lock()
                .expect("Should not be able to lock recursively")
                .push((item, self.description.clone()))
        }
    }
}

impl<Type, Description: BufferDescription> Deref for PoolEntry<Type, Description> {
    type Target = Type;

    fn deref(&self) -> &Self::Target {
        self.item.as_ref().expect("Item should exist until dropped")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texture_budget_accounts_for_multisampling() {
        let key = TextureKey {
            size: wgpu::Extent3d {
                width: 1024,
                height: 512,
                depth_or_array_layers: 1,
            },
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: wgpu::TextureFormat::Rgba8Unorm,
            sample_count: 4,
        };
        assert_eq!(key.estimated_bytes(), 8 * 1024 * 1024);
    }

    #[test]
    fn evicting_idle_entries_does_not_destroy_checked_out_entries() {
        let pool = BufferPool::new(Box::new(|_, _: &AlwaysCompatible| 0u32));
        pool.available.lock().unwrap().push((1, AlwaysCompatible));
        let active = PoolEntry {
            item: Some(2),
            description: AlwaysCompatible,
            pool: Arc::downgrade(&pool.available),
        };
        assert!(!pool.retain_idle(|_| false));
        assert_eq!(*active, 2);
        drop(active);
        assert_eq!(pool.available.lock().unwrap().len(), 1);
    }
}
