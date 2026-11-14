use crate::font::{Font, FontWeak};
use fnv::FnvHashSet;
use gc_arena::{Collect, Mutation};
use std::collections::VecDeque;
use std::time::Duration;
use web_time::Instant;

const MAX_GLYPH_CHECKS: usize = 1024;
const MAX_HANDLE_RELEASES: usize = 128;
const MAX_FONT_VISITS: usize = 64;

/// Aggregate limits for both policies, including disabled weak-header cleanup.
#[derive(Clone, Copy)]
pub(super) struct BatchBudget {
    fonts: usize,
    glyphs: usize,
    releases: usize,
}

impl BatchBudget {
    pub const FULL: Self = Self {
        fonts: MAX_FONT_VISITS,
        glyphs: MAX_GLYPH_CHECKS,
        releases: MAX_HANDLE_RELEASES,
    };

    #[cfg(all(feature = "memory_swf_glyph_cache", feature = "memory_font_face_cache"))]
    fn half(self) -> Self {
        Self {
            fonts: self.fonts / 2,
            glyphs: self.glyphs / 2,
            releases: self.releases / 2,
        }
    }

    #[cfg(all(feature = "memory_swf_glyph_cache", feature = "memory_font_face_cache"))]
    fn after(self, stats: &BatchStats) -> Self {
        Self {
            fonts: self.fonts.saturating_sub(stats.fonts_visited),
            glyphs: self.glyphs.saturating_sub(stats.glyphs_checked),
            releases: self.releases.saturating_sub(stats.handles_released),
        }
    }
}

#[derive(Clone, Copy)]
enum CacheKind {
    #[cfg(feature = "memory_swf_glyph_cache")]
    Swf,
    #[cfg(feature = "memory_font_face_cache")]
    FontFace,
}

#[derive(Clone, Copy, Collect)]
#[collect(no_drop)]
struct FontCursor<'gc> {
    font: FontWeak<'gc>,
    #[collect(require_static)]
    next_glyph: usize,
}

#[derive(Default, Debug)]
pub(super) struct BatchStats {
    pub fonts_visited: usize,
    pub glyphs_checked: usize,
    pub handles_released: usize,
    pub dead_fonts_removed: usize,
    pub cycle_started: bool,
    pub cycle_completed: bool,
}

/// Player/Library-local weak rotation. Registry mutations append at the back;
/// an in-progress font resumes at the front without a borrowed Font reference.
#[derive(Collect)]
#[collect(no_drop)]
pub(super) struct GlyphCacheQueue<'gc> {
    #[collect(require_static)]
    kind: CacheKind,
    fonts: VecDeque<FontCursor<'gc>>,
    #[collect(require_static)]
    keys: FnvHashSet<usize>,
    #[collect(require_static)]
    idle_limit: Option<Duration>,
    #[collect(require_static)]
    last_cycle_completed: Instant,
    #[collect(require_static)]
    remaining: usize,
}

impl<'gc> GlyphCacheQueue<'gc> {
    #[cfg(feature = "memory_swf_glyph_cache")]
    pub fn new() -> Self {
        Self::with_kind(CacheKind::Swf)
    }

    #[cfg(feature = "memory_font_face_cache")]
    pub fn new_font_face() -> Self {
        Self::with_kind(CacheKind::FontFace)
    }

    fn with_kind(kind: CacheKind) -> Self {
        Self {
            kind,
            fonts: VecDeque::new(),
            keys: FnvHashSet::default(),
            idle_limit: None,
            last_cycle_completed: Instant::now(),
            remaining: 0,
        }
    }

    pub fn register(&mut self, font: Font<'gc>) {
        let weak = match self.kind {
            #[cfg(feature = "memory_swf_glyph_cache")]
            CacheKind::Swf => font.swf_cache_weak(),
            #[cfg(feature = "memory_font_face_cache")]
            CacheKind::FontFace => font.font_face_cache_weak(),
        };
        let Some(font) = weak else {
            return;
        };
        if self.keys.insert(font.key()) {
            self.fonts.push_back(FontCursor {
                font,
                next_glyph: 0,
            });
        }
    }

    pub fn set_idle_limit(&mut self, idle_limit: Option<Duration>, now: Instant) {
        self.idle_limit = idle_limit;
        self.last_cycle_completed = now;
        // Retain bounded progress through a partially examined font. A changed
        // policy must not reset a huge font to slot zero and starve its tail.
        if idle_limit.is_none() {
            self.remaining = 0;
        }
    }

    #[cfg(all(
        feature = "memory_swf_glyph_cache",
        any(test, not(feature = "memory_font_face_cache"))
    ))]
    pub fn step(&mut self, now: Instant, mc: &Mutation<'gc>) -> BatchStats {
        self.step_with_budget(now, mc, BatchBudget::FULL)
    }

    pub fn step_with_budget(
        &mut self,
        now: Instant,
        mc: &Mutation<'gc>,
        budget: BatchBudget,
    ) -> BatchStats {
        let mut stats = BatchStats::default();
        let Some(idle_limit) = self.idle_limit else {
            // Disabled policies never upgrade or inspect a font body. Rotate a
            // bounded number of weak headers so dropped bodies are not kept
            // allocated indefinitely by an unused observer index.
            for _ in 0..self.fonts.len().min(budget.fonts) {
                let cursor = self.fonts.pop_front().unwrap();
                stats.fonts_visited += 1;
                if cursor.font.is_dropped() {
                    self.keys.remove(&cursor.font.key());
                    stats.dead_fonts_removed += 1;
                } else {
                    self.fonts.push_back(cursor);
                }
            }
            return stats;
        };
        if self.remaining == 0 {
            if now.saturating_duration_since(self.last_cycle_completed) < Duration::from_secs(5) {
                return stats;
            }
            self.remaining = self.fonts.len();
            stats.cycle_started = true;
            if self.remaining == 0 {
                self.last_cycle_completed = now;
                stats.cycle_completed = true;
                return stats;
            }
        }
        while self.remaining != 0
            && stats.fonts_visited < budget.fonts
            && stats.glyphs_checked < budget.glyphs
            && stats.handles_released < budget.releases
        {
            let Some(mut cursor) = self.fonts.pop_front() else {
                debug_assert_eq!(self.remaining, 0);
                self.remaining = 0;
                break;
            };
            stats.fonts_visited += 1;
            let Some(font) = cursor.font.upgrade(mc) else {
                // Failed upgrade can include pending WhiteWeak during sweep.
                // No cast/dereference or resurrection is performed.
                self.keys.remove(&cursor.font.key());
                self.remaining -= 1;
                stats.dead_fonts_removed += 1;
                continue;
            };
            let max_checked = budget.glyphs - stats.glyphs_checked;
            let max_released = budget.releases - stats.handles_released;
            let batch = match self.kind {
                #[cfg(feature = "memory_swf_glyph_cache")]
                CacheKind::Swf => font.expire_render_cache_batch(
                    now,
                    idle_limit,
                    cursor.next_glyph,
                    max_checked,
                    max_released,
                ),
                #[cfg(feature = "memory_font_face_cache")]
                CacheKind::FontFace => font.expire_font_face_render_cache_batch(
                    now,
                    idle_limit,
                    cursor.next_glyph,
                    max_checked,
                    max_released,
                ),
            };
            stats.glyphs_checked += batch.checked;
            stats.handles_released += batch.released;
            cursor.next_glyph = batch.next_glyph;
            if batch.finished {
                cursor.next_glyph = 0;
                self.remaining -= 1;
                self.fonts.push_back(cursor);
            } else {
                self.fonts.push_front(cursor);
            }
        }
        if self.remaining == 0 {
            self.last_cycle_completed = now;
            stats.cycle_completed = true;
        }
        stats
    }
}

/// Each populated policy is guaranteed a budget even when the other owns a
/// huge sparse font. Unused work from the first policy is available to the
/// second; alternating priority avoids a permanent preference.
#[cfg(all(feature = "memory_swf_glyph_cache", feature = "memory_font_face_cache"))]
pub(super) fn step_pair<'gc>(
    first: &mut GlyphCacheQueue<'gc>,
    second: &mut GlyphCacheQueue<'gc>,
    now: Instant,
    mc: &Mutation<'gc>,
) -> (BatchStats, BatchStats) {
    let first_budget = if second.fonts.is_empty() {
        BatchBudget::FULL
    } else {
        BatchBudget::FULL.half()
    };
    let first_stats = first.step_with_budget(now, mc, first_budget);
    let second_stats = second.step_with_budget(now, mc, BatchBudget::FULL.after(&first_stats));
    (first_stats, second_stats)
}

#[cfg(all(test, feature = "memory_font_face_cache"))]
mod font_face_tests {
    use super::*;
    use crate::font::{FontDescriptor, FontFileData, FontType};
    use flate2::read::DeflateDecoder;
    use gc_arena::{Arena, Rootable};
    use ruffle_render::backend::ViewportDimensions;
    use ruffle_render::backend::null::NullRenderer;
    use std::io::Read;

    fn face_font<'gc>(mc: &Mutation<'gc>) -> Font<'gc> {
        let mut data = Vec::new();
        DeflateDecoder::new(include_bytes!("../../assets/notosans.subset.ttf.gz").as_slice())
            .read_to_end(&mut data)
            .expect("test font decompression");
        Font::from_font_file(
            mc,
            FontDescriptor::from_parts("FontFaceCacheTest", false, false),
            FontFileData::new(data),
            0,
            FontType::Device,
        )
        .expect("valid test font")
    }

    fn renderer() -> NullRenderer {
        NullRenderer::new(ViewportDimensions {
            width: 64,
            height: 64,
            scale_factor: 1.0,
        })
    }

    #[test]
    fn font_face_queue_obeys_all_budgets_and_does_not_retain_fonts() {
        let mut renderer = renderer();
        let mut arena = Arena::<Rootable![(Option<Font<'_>>, GlyphCacheQueue<'_>)]>::new(|mc| {
            let font = face_font(mc);
            drop(
                font.get_glyph_for_char('A')
                    .unwrap()
                    .glyph_render_data(&mut renderer),
            );
            let mut queue = GlyphCacheQueue::new_font_face();
            queue.register(font);
            queue.register(font);
            assert_eq!(queue.fonts.len(), 1);
            (Some(font), queue)
        });
        let now = Instant::now();
        arena.mutate_root(|mc, (font, queue)| {
            queue.set_idle_limit(Some(Duration::from_secs(90)), now);
            let budget = BatchBudget {
                fonts: 1,
                glyphs: 13,
                releases: 1,
            };
            let stats = queue.step_with_budget(now + Duration::from_secs(95), mc, budget);
            assert!(
                stats.fonts_visited <= 1
                    && stats.glyphs_checked <= 13
                    && stats.handles_released <= 1
            );
            assert_eq!(stats.glyphs_checked, 13);
            assert_eq!(queue.fonts.front().unwrap().next_glyph, 13);
            queue.set_idle_limit(None, now);
            let off = queue.step_with_budget(now, mc, BatchBudget::FULL);
            assert_eq!(off.glyphs_checked, 0);
            assert_eq!(off.handles_released, 0);
            *font = None;
        });
        arena.finish_cycle();
        arena.mutate_root(|mc, (_, queue)| {
            assert!(queue.fonts.front().unwrap().font.is_dropped());
            let stats = queue.step_with_budget(now, mc, BatchBudget::FULL);
            assert_eq!(stats.dead_fonts_removed, 1);
            assert!(queue.fonts.is_empty() && queue.keys.is_empty());
        });
    }

    #[test]
    #[cfg(feature = "memory_swf_glyph_cache")]
    fn sparse_font_face_cannot_starve_swf_queue_or_double_total_budget() {
        let mut renderer = renderer();
        let mut arena = Arena::<
            Rootable![(Font<'_>, Font<'_>, GlyphCacheQueue<'_>, GlyphCacheQueue<'_>)],
        >::new(|mc| {
            let face = face_font(mc);
            let swf = Font::from_swf_tag(
                mc,
                &mut renderer,
                swf::Font {
                    version: 2,
                    id: 1,
                    name: "SharedBudgetTest".into(),
                    language: swf::Language::Unknown,
                    layout: None,
                    flags: swf::FontFlag::HAS_WIDE_CODES,
                    glyphs: (0..2050)
                        .map(|i| swf::Glyph {
                            shape_records: Vec::new(),
                            code: (0x100 + i) as u16,
                            advance: 100,
                            bounds: None,
                        })
                        .collect(),
                },
                swf::UTF_8,
                FontType::Embedded,
            );
            let mut faces = GlyphCacheQueue::new_font_face();
            let mut swfs = GlyphCacheQueue::new();
            faces.register(face);
            faces.register(swf);
            swfs.register(swf);
            swfs.register(face);
            assert_eq!(faces.fonts.len(), 1);
            assert_eq!(swfs.fonts.len(), 1);
            (face, swf, faces, swfs)
        });
        arena.mutate_root(|mc, (_, _, faces, swfs)| {
            let start = Instant::now();
            faces.set_idle_limit(Some(Duration::from_secs(90)), start);
            swfs.set_idle_limit(Some(Duration::from_secs(90)), start);
            let (face, swf) = step_pair(faces, swfs, start + Duration::from_secs(95), mc);
            assert_eq!(face.glyphs_checked, 512);
            assert_eq!(swf.glyphs_checked, 512);
            assert!(face.fonts_visited + swf.fonts_visited <= MAX_FONT_VISITS);
            assert!(face.handles_released + swf.handles_released <= MAX_HANDLE_RELEASES);
            let (swf, face) = step_pair(swfs, faces, start + Duration::from_secs(95), mc);
            assert_eq!(face.glyphs_checked + swf.glyphs_checked, 1024);
            assert_eq!(faces.fonts.front().unwrap().next_glyph, 1024);
            assert_eq!(swfs.fonts.front().unwrap().next_glyph, 1024);
            faces.set_idle_limit(None, start);
            let (face, swf) = step_pair(faces, swfs, start + Duration::from_secs(95), mc);
            assert_eq!(face.glyphs_checked, 0);
            assert_eq!(swf.glyphs_checked, 1024);
            assert_eq!(swfs.idle_limit, Some(Duration::from_secs(90)));
        });
    }
}

#[cfg(all(test, feature = "memory_swf_glyph_cache"))]
mod tests {
    use super::*;
    use crate::font::FontType;
    use gc_arena::{Arena, Rootable};
    use ruffle_render::backend::ViewportDimensions;
    use ruffle_render::backend::null::NullRenderer;

    fn renderer() -> NullRenderer {
        NullRenderer::new(ViewportDimensions {
            width: 64,
            height: 64,
            scale_factor: 1.0,
        })
    }

    fn font<'gc>(
        mc: &Mutation<'gc>,
        renderer: &mut NullRenderer,
        count: usize,
        warm: bool,
    ) -> Font<'gc> {
        let font = Font::from_swf_tag(
            mc,
            renderer,
            swf::Font {
                version: 2,
                id: 1,
                name: "BoundedGlyphTest".into(),
                language: swf::Language::Unknown,
                layout: None,
                flags: swf::FontFlag::HAS_WIDE_CODES,
                glyphs: (0..count)
                    .map(|i| swf::Glyph {
                        shape_records: Vec::new(),
                        code: (0x100 + i) as u16,
                        advance: 100,
                        bounds: None,
                    })
                    .collect(),
            },
            swf::UTF_8,
            FontType::Embedded,
        );
        if warm {
            for i in 0..count {
                assert!(
                    font.get_glyph(i)
                        .unwrap()
                        .glyph_render_data(renderer)
                        .is_some()
                );
            }
        }
        font
    }

    fn bounded(s: &BatchStats) {
        assert!(s.fonts_visited <= MAX_FONT_VISITS);
        assert!(s.glyphs_checked <= MAX_GLYPH_CHECKS);
        assert!(s.handles_released <= MAX_HANDLE_RELEASES);
    }

    #[test]
    fn bounded_glyph_checks_include_inside_a_large_font() {
        let mut r = renderer();
        let mut arena = Arena::<Rootable![(Font<'_>, GlyphCacheQueue<'_>)]>::new(|mc| {
            let f = font(mc, &mut r, 2050, false);
            let mut q = GlyphCacheQueue::new();
            q.register(f);
            (f, q)
        });
        arena.mutate_root(|mc, (_, q)| {
            let start = Instant::now();
            q.set_idle_limit(Some(Duration::from_secs(90)), start);
            assert_eq!(q.step(start + Duration::from_secs(4), mc).glyphs_checked, 0);
            let a = q.step(start + Duration::from_secs(95), mc);
            bounded(&a);
            assert_eq!(a.glyphs_checked, 1024);
            assert!(!a.cycle_completed);
            let b = q.step(start + Duration::from_secs(95), mc);
            bounded(&b);
            assert_eq!(b.glyphs_checked, 1024);
            assert!(!b.cycle_completed);
            let c = q.step(start + Duration::from_secs(95), mc);
            bounded(&c);
            assert_eq!(c.glyphs_checked, 2);
            assert!(c.cycle_completed);
        });
    }

    #[test]
    fn bounded_glyph_release_resumes_and_handles_backward_injected_time() {
        let mut r = renderer();
        let mut arena = Arena::<Rootable![(Font<'_>, GlyphCacheQueue<'_>)]>::new(|mc| {
            let f = font(mc, &mut r, 300, true);
            let mut q = GlyphCacheQueue::new();
            q.register(f);
            (f, q)
        });
        arena.mutate_root(|mc, (f, q)| {
            let now = Instant::now();
            q.set_idle_limit(Some(Duration::from_secs(90)), now);
            let a = q.step(now + Duration::from_secs(95), mc);
            bounded(&a);
            assert_eq!(a.handles_released, 128);
            assert_eq!(q.fonts.front().unwrap().next_glyph, 128);
            // Return from the injected future to actual time. All remaining
            // timestamps must survive this backward-time case without panic.
            let tail = f.get_glyph(299).unwrap();
            assert!(tail.glyph_render_data(&mut r).is_some());
            let refresh = Instant::now();
            let b = q.step(refresh + Duration::from_secs(1), mc);
            bounded(&b);
            assert_eq!(b.handles_released, 0);
            assert!(b.cycle_completed);
            let c = q.step(refresh + Duration::from_secs(100), mc);
            bounded(&c);
            assert_eq!(c.handles_released, 128);
            let d = q.step(refresh + Duration::from_secs(100), mc);
            bounded(&d);
            assert_eq!(d.handles_released, 44);
            assert!(d.cycle_completed);
        });
    }

    #[test]
    fn bounded_glyph_rotation_does_not_starve_tail_when_new_fonts_arrive() {
        let mut r = renderer();
        let mut arena = Arena::<Rootable![(Vec<Font<'_>>, GlyphCacheQueue<'_>)]>::new(|mc| {
            let first = font(mc, &mut r, 2050, false);
            let tail = font(mc, &mut r, 1, true);
            let mut q = GlyphCacheQueue::new();
            q.register(first);
            q.register(tail);
            (vec![first, tail], q)
        });
        arena.mutate_root(|mc, (fonts, q)| {
            let now = Instant::now();
            q.set_idle_limit(Some(Duration::from_secs(90)), now);
            bounded(&q.step(now + Duration::from_secs(95), mc));
            let added = font(mc, &mut r, 1, true);
            q.register(added);
            fonts.push(added);
            bounded(&q.step(now + Duration::from_secs(95), mc));
            let last = q.step(now + Duration::from_secs(95), mc);
            bounded(&last);
            assert!(last.cycle_completed);
            assert_eq!(last.handles_released, 1);
            assert_eq!(q.fonts.len(), 3);
            // Added font was beyond this cycle's initial remaining frontier.
            let next = q.step(now + Duration::from_secs(101), mc);
            bounded(&next);
            assert_eq!(next.handles_released, 1);
        });
    }

    #[test]
    fn bounded_glyph_refresh_between_batches_preserves_only_the_used_tail() {
        let mut r = renderer();
        let mut arena = Arena::<Rootable![(Font<'_>, GlyphCacheQueue<'_>)]>::new(|mc| {
            let f = font(mc, &mut r, 300, true);
            let mut q = GlyphCacheQueue::new();
            q.register(f);
            (f, q)
        });
        std::thread::sleep(Duration::from_millis(30));
        arena.mutate_root(|mc, (f, q)| {
            let now = Instant::now();
            q.set_idle_limit(Some(Duration::from_millis(20)), now);
            let a = q.step(now + Duration::from_secs(6), mc);
            assert_eq!(a.handles_released, 128);
            let tail = f.get_glyph(299).unwrap();
            assert!(tail.glyph_render_data(&mut r).is_some());
            let refreshed = Instant::now();
            let b = q.step(refreshed, mc);
            bounded(&b);
            assert_eq!(b.handles_released, 128);
            let c = q.step(refreshed, mc);
            bounded(&c);
            assert_eq!(c.handles_released, 43);
            assert!(c.cycle_completed);
            let d = q.step(refreshed + Duration::from_secs(6), mc);
            bounded(&d);
            assert_eq!(d.handles_released, 1);
            assert!(d.cycle_completed);
        });
    }

    #[test]
    fn bounded_glyph_disabled_rotation_and_midcycle_toggle_keep_progress() {
        let mut r = renderer();
        let mut arena = Arena::<Rootable![(Vec<Font<'_>>, GlyphCacheQueue<'_>)]>::new(|mc| {
            let fonts = (0..70)
                .map(|_| font(mc, &mut r, 1, false))
                .collect::<Vec<_>>();
            let mut q = GlyphCacheQueue::new();
            for f in &fonts {
                q.register(*f);
            }
            (fonts, q)
        });
        arena.mutate_root(|mc, (_, q)| {
            let now = Instant::now();
            let off = q.step(now, mc);
            bounded(&off);
            assert_eq!(off.fonts_visited, 64);
            assert_eq!(off.glyphs_checked, 0);
            q.set_idle_limit(Some(Duration::from_secs(90)), now);
            let a = q.step(now + Duration::from_secs(6), mc);
            bounded(&a);
            assert_eq!(a.fonts_visited, 64);
            assert!(!a.cycle_completed);
            q.set_idle_limit(None, now);
            assert_eq!(q.step(now, mc).glyphs_checked, 0);
            q.set_idle_limit(Some(Duration::from_secs(90)), now);
            let b = q.step(now + Duration::from_secs(6), mc);
            bounded(&b);
            assert_eq!(b.fonts_visited, 64);
            let c = q.step(now + Duration::from_secs(6), mc);
            bounded(&c);
            assert_eq!(c.fonts_visited, 6);
            assert!(c.cycle_completed);
        });
    }

    #[test]
    fn bounded_glyph_registry_is_library_local_and_does_not_retain_fonts() {
        let mut r = renderer();
        let mut arena = Arena::<
            Rootable![(Option<Font<'_>>, GlyphCacheQueue<'_>, GlyphCacheQueue<'_>)],
        >::new(|mc| {
            let f = font(mc, &mut r, 1, true);
            let mut a = GlyphCacheQueue::new();
            let mut b = GlyphCacheQueue::new();
            a.register(f);
            a.register(f);
            b.register(f);
            assert_eq!(a.fonts.len(), 1);
            assert_eq!(b.fonts.len(), 1);
            (Some(f), a, b)
        });
        arena.mutate_root(|_, (f, _, _)| *f = None);
        arena.finish_cycle();
        arena.mutate_root(|mc, (_, a, b)| {
            assert!(a.fonts.front().unwrap().font.is_dropped());
            assert!(b.fonts.front().unwrap().font.is_dropped());
            // Disabled policy removes weak headers without upgrading bodies.
            let sa = a.step(Instant::now(), mc);
            let sb = b.step(Instant::now(), mc);
            assert_eq!(sa.dead_fonts_removed, 1);
            assert_eq!(sb.dead_fonts_removed, 1);
            assert!(
                a.fonts.is_empty() && a.keys.is_empty() && b.fonts.is_empty() && b.keys.is_empty()
            );
        });
    }

    #[test]
    fn bounded_glyph_active_cycle_gc_removes_partial_and_pending_fonts() {
        let mut r = renderer();
        let mut arena =
            Arena::<Rootable![(Vec<Option<Font<'_>>>, GlyphCacheQueue<'_>)]>::new(|mc| {
                let partial = font(mc, &mut r, 2050, false);
                let pending = font(mc, &mut r, 1, true);
                let tail = font(mc, &mut r, 1, true);
                let mut q = GlyphCacheQueue::new();
                for f in [partial, pending, tail] {
                    q.register(f);
                }
                (vec![Some(partial), Some(pending), Some(tail)], q)
            });
        let now = Instant::now();
        arena.mutate_root(|mc, (fonts, q)| {
            q.set_idle_limit(Some(Duration::from_secs(90)), now);
            let first = q.step(now + Duration::from_secs(95), mc);
            bounded(&first);
            assert_eq!(first.glyphs_checked, 1024);
            assert_eq!(q.remaining, 3);
            assert_eq!(q.fonts.front().unwrap().next_glyph, 1024);
            fonts[0] = None;
            fonts[1] = None;
        });
        arena.finish_cycle();
        arena.mutate_root(|mc, (fonts, q)| {
            assert!(q.fonts[0].font.is_dropped());
            assert!(q.fonts[1].font.is_dropped());
            let last = q.step(now + Duration::from_secs(95), mc);
            bounded(&last);
            assert_eq!(last.dead_fonts_removed, 2);
            assert_eq!(last.fonts_visited, 3);
            assert_eq!(last.glyphs_checked, 1);
            assert_eq!(last.handles_released, 1);
            assert!(last.cycle_completed);
            assert_eq!(q.remaining, 0);
            assert_eq!(q.fonts.len(), 1);
            assert_eq!(q.keys.len(), 1);
            assert_eq!(
                q.fonts.front().unwrap().font.key(),
                fonts[2].unwrap().as_ptr() as usize
            );
            let next = q.step(now + Duration::from_secs(101), mc);
            bounded(&next);
            assert_eq!(next.glyphs_checked, 1);
            assert_eq!(next.handles_released, 0);
            assert!(next.cycle_completed);
        });
    }

    #[test]
    fn bounded_glyph_upgrades_between_incremental_gc_steps_do_not_keep_font_alive() {
        use gc_arena::arena::CollectionPhase;
        let mut r = renderer();
        let mut arena = Arena::<
            Rootable![(
                Option<Font<'_>>,
                Font<'_>,
                FontWeak<'_>,
                GlyphCacheQueue<'_>
            )],
        >::new(|mc| {
            let lost = font(mc, &mut r, 20000, false);
            let tail = font(mc, &mut r, 1, false);
            let weak = lost.swf_cache_weak().unwrap();
            let mut q = GlyphCacheQueue::new();
            q.register(lost);
            q.register(tail);
            (Some(lost), tail, weak, q)
        });
        let now = Instant::now();
        arena.mutate_root(|mc, (strong, _, _, q)| {
            q.set_idle_limit(Some(Duration::from_secs(90)), now);
            assert_eq!(
                q.step(now + Duration::from_secs(95), mc).glyphs_checked,
                1024
            );
            *strong = None;
        });
        // Use the standard gc-arena phase API, without investigation-only steps.
        for _ in 0..4 {
            arena.finish_marking().unwrap();
            arena.mutate_root(|mc, (_, _, weak, q)| {
                assert!(weak.upgrade(mc).is_some());
                let stats = q.step(now + Duration::from_secs(95), mc);
                bounded(&stats);
            });
        }
        arena.finish_marking().unwrap().start_sweeping();
        assert_eq!(arena.collection_phase(), CollectionPhase::Sweeping);
        let mut removed = 0;
        arena.mutate_root(|mc, (_, _, weak, q)| {
            assert!(weak.upgrade(mc).is_none());
            for _ in 0..4 {
                let stats = q.step(now + Duration::from_secs(95), mc);
                bounded(&stats);
                removed += stats.dead_fonts_removed;
            }
        });
        arena.finish_cycle();
        assert_eq!(removed, 1);
        arena.mutate(|mc, (strong, _, weak, q)| {
            assert!(strong.is_none());
            assert!(weak.is_dropped() && weak.upgrade(mc).is_none());
            assert_eq!(q.fonts.len(), 1);
            assert_eq!(q.keys.len(), 1);
            assert_eq!(q.remaining, 0);
        });
    }
}
