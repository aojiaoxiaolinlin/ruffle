use crate::context::RenderContext;
use crate::drawing::Drawing;
use crate::font::FontAtlasGlyph;
use crate::prelude::*;
use ruffle_render::backend::null::NullBitmapSource;
use ruffle_render::backend::{RenderBackend, ShapeHandle};
use ruffle_render::transform::Transform;
use std::cell::{Ref, RefCell};
#[cfg(any(feature = "memory_swf_glyph_cache", feature = "memory_font_face_cache"))]
use std::time::Duration;
#[cfg(any(feature = "memory_swf_glyph_cache", feature = "memory_font_face_cache"))]
use web_time::Instant;

#[derive(Debug, Clone)]
enum SwfGlyphOrShape {
    Glyph(swf::Glyph),
    Shape {
        shape: swf::Shape,
        // Handle to registered shape, loaded lazily on first render of this glyph.
        handle: Option<ShapeHandle>,
        // Keep policy metadata in this boxed cache, not in every Glyph slot.
        #[cfg(feature = "memory_swf_glyph_cache")]
        last_used: Option<Instant>,
    },
    Poisoned,
}

impl SwfGlyphOrShape {
    fn shape(&mut self) -> (&mut swf::Shape, &mut Option<ShapeHandle>) {
        if let Self::Glyph(_) = self
            && let Self::Glyph(glyph) = core::mem::replace(self, Self::Poisoned)
        {
            *self = Self::Shape {
                shape: ruffle_render::shape_utils::swf_glyph_to_shape(glyph),
                handle: None,
                #[cfg(feature = "memory_swf_glyph_cache")]
                last_used: None,
            };
        }

        match self {
            SwfGlyphOrShape::Shape { shape, handle, .. } => (shape, handle),
            _ => unreachable!(),
        }
    }

    #[cfg(feature = "memory_swf_glyph_cache")]
    fn mark_render_cache_used(&mut self, now: Instant) {
        if let Self::Shape {
            handle: Some(_),
            last_used,
            ..
        } = self
        {
            *last_used = Some(now);
        }
    }

    #[cfg(feature = "memory_swf_glyph_cache")]
    fn expire_render_cache(&mut self, now: Instant, idle_limit: Duration) -> usize {
        let Self::Shape {
            handle, last_used, ..
        } = self
        else {
            return 0;
        };
        if last_used.is_some_and(|last| now.saturating_duration_since(last) >= idle_limit) {
            *last_used = None;
            usize::from(handle.take().is_some())
        } else {
            0
        }
    }
}

#[derive(Clone, Debug)]
pub enum GlyphRenderData {
    Shape(ShapeHandle),
    AtlasGlyph(FontAtlasGlyph),
}

impl GlyphRenderData {
    pub fn from_shape(shape_handle: ShapeHandle) -> Self {
        Self::Shape(shape_handle)
    }

    pub fn from_atlas(atlas_glyph: FontAtlasGlyph) -> Self {
        Self::AtlasGlyph(atlas_glyph)
    }
}

/// Keep idle metadata in the allocated drawing cache, never in unused glyph slots
/// or ordinary display-object drawings. Only the FontFace sweep expires it.
#[cfg(feature = "memory_font_face_cache")]
#[derive(Debug, Clone)]
struct FontFaceDrawingCache {
    drawing: Drawing,
    last_used: Option<Instant>,
}

#[cfg(feature = "memory_font_face_cache")]
impl FontFaceDrawingCache {
    fn register(&mut self, renderer: &mut dyn RenderBackend) -> Option<GlyphRenderData> {
        let handle = self.drawing.register_or_replace(renderer)?;
        self.last_used = Some(Instant::now());
        Some(GlyphRenderData::from_shape(handle))
    }

    fn expire_render_cache(&mut self, now: Instant, idle_limit: Duration) -> usize {
        if self
            .last_used
            .is_some_and(|last| now.saturating_duration_since(last) >= idle_limit)
        {
            self.last_used = None;
            self.drawing.drop_render_cache()
        } else {
            0
        }
    }
}

#[derive(Debug, Clone)]
enum GlyphShape {
    Swf(Box<RefCell<SwfGlyphOrShape>>),
    #[cfg(not(feature = "memory_font_face_cache"))]
    Drawing(Box<Drawing>),
    #[cfg(feature = "memory_font_face_cache")]
    Drawing(Box<RefCell<FontFaceDrawingCache>>),
    AtlasGlyph(FontAtlasGlyph),
    None,
}

impl GlyphShape {
    pub fn hit_test(&self, point: Point<Twips>, local_matrix: &Matrix) -> bool {
        match self {
            GlyphShape::Swf(glyph) => {
                let mut glyph = glyph.borrow_mut();
                let (shape, _) = glyph.shape();
                shape.shape_bounds.contains(point)
                    && ruffle_render::shape_utils::shape_hit_test(shape, point, local_matrix)
            }
            #[cfg(not(feature = "memory_font_face_cache"))]
            GlyphShape::Drawing(drawing) => drawing.hit_test(point, local_matrix),
            #[cfg(feature = "memory_font_face_cache")]
            GlyphShape::Drawing(cache) => cache.borrow().drawing.hit_test(point, local_matrix),
            GlyphShape::AtlasGlyph(_) => {
                // TODO Implement this.
                true
            }
            GlyphShape::None => false,
        }
    }

    pub fn register(&self, renderer: &mut dyn RenderBackend) -> Option<GlyphRenderData> {
        match self {
            GlyphShape::Swf(glyph) => {
                let mut glyph = glyph.borrow_mut();
                let (shape, handle) = glyph.shape();
                handle.get_or_insert_with(|| {
                    renderer.register_shape((&*shape).into(), &NullBitmapSource)
                });
                let render_data = handle.clone().map(GlyphRenderData::from_shape);
                // Creating eager ASCII handles also starts their idle clock.
                // Lookup, measurement and hit testing do not use this path.
                #[cfg(feature = "memory_swf_glyph_cache")]
                glyph.mark_render_cache_used(Instant::now());
                render_data
            }
            #[cfg(not(feature = "memory_font_face_cache"))]
            GlyphShape::Drawing(drawing) => drawing
                .register_or_replace(renderer)
                .map(GlyphRenderData::from_shape),
            #[cfg(feature = "memory_font_face_cache")]
            GlyphShape::Drawing(cache) => cache.borrow_mut().register(renderer),
            GlyphShape::AtlasGlyph(atlas_glyph) => atlas_glyph
                .atlas_handle(renderer)
                .as_ref()
                .map(|_| GlyphRenderData::from_atlas(atlas_glyph.clone())),
            GlyphShape::None => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Glyph {
    shape: GlyphShape,
    advance: Twips,

    // The character this glyph represents.
    character: char,
}

impl Glyph {
    /// Returns an empty glyph with zero advance.
    pub fn empty(character: char) -> Self {
        Self {
            shape: GlyphShape::None,
            advance: Twips::ZERO,
            character,
        }
    }

    pub fn whitespace(character: char, advance: Twips) -> Self {
        Self {
            shape: GlyphShape::None,
            advance,
            character,
        }
    }

    pub fn from_drawing(character: char, advance: Twips, drawing: Drawing) -> Self {
        Self {
            #[cfg(not(feature = "memory_font_face_cache"))]
            shape: GlyphShape::Drawing(Box::new(drawing)),
            #[cfg(feature = "memory_font_face_cache")]
            shape: GlyphShape::Drawing(Box::new(RefCell::new(FontFaceDrawingCache {
                drawing,
                last_used: None,
            }))),
            advance,
            character,
        }
    }

    pub fn from_swf(character: char, swf_glyph: swf::Glyph) -> Self {
        Self {
            advance: Twips::new(swf_glyph.advance.into()),
            shape: GlyphShape::Swf(Box::new(RefCell::new(SwfGlyphOrShape::Glyph(swf_glyph)))),
            character,
        }
    }

    pub fn from_atlas(character: char, atlas_glyph: FontAtlasGlyph, advance: Twips) -> Self {
        Self {
            shape: GlyphShape::AtlasGlyph(atlas_glyph),
            advance,
            character,
        }
    }

    pub fn glyph_render_data(&self, renderer: &mut dyn RenderBackend) -> Option<GlyphRenderData> {
        self.shape.register(renderer)
    }

    /// Release only reconstructible SWF geometry. Commands and cloned glyphs
    /// retain their own strong handles until they finish using them.
    #[cfg(feature = "memory_swf_glyph_cache")]
    pub(crate) fn expire_render_cache(&self, now: Instant, idle_limit: Duration) -> usize {
        match &self.shape {
            GlyphShape::Swf(glyph) => glyph.borrow_mut().expire_render_cache(now, idle_limit),
            _ => 0,
        }
    }

    /// Called only while traversing initialized slots owned by a FontFace.
    /// Other drawing glyph sources retain their existing cache lifetime.
    #[cfg(feature = "memory_font_face_cache")]
    pub(super) fn expire_font_face_render_cache(
        &self,
        now: Instant,
        idle_limit: Duration,
    ) -> usize {
        match &self.shape {
            GlyphShape::Drawing(cache) => cache.borrow_mut().expire_render_cache(now, idle_limit),
            _ => 0,
        }
    }

    #[cfg(all(test, feature = "memory_font_face_cache"))]
    pub(super) fn set_font_face_cache_last_used_for_test(&self, now: Instant) {
        let GlyphShape::Drawing(cache) = &self.shape else {
            panic!("expected drawing glyph");
        };
        cache.borrow_mut().last_used = Some(now);
    }

    pub fn hit_test(&self, point: Point<Twips>, local_matrix: &Matrix) -> bool {
        self.shape.hit_test(point, local_matrix)
    }

    pub fn advance(&self) -> Twips {
        self.advance
    }

    pub fn character(&self) -> char {
        self.character
    }

    pub fn as_ref(&self) -> GlyphRef<'_> {
        GlyphRef::Direct(self)
    }

    pub fn rendered_at_baseline(&self) -> bool {
        match self.shape {
            GlyphShape::Swf(_) => true,
            GlyphShape::Drawing(_) => true,
            GlyphShape::AtlasGlyph(_) => false,
            GlyphShape::None => false,
        }
    }

    pub fn renderable<'gc>(&self, context: &mut RenderContext<'_, 'gc>) -> bool {
        self.glyph_render_data(context.renderer).is_some()
    }

    pub fn render<'gc>(&self, context: &mut RenderContext<'_, 'gc>) {
        use ruffle_render::commands::CommandHandler;

        let Some(render_data) = self.glyph_render_data(context.renderer) else {
            return;
        };

        match render_data {
            GlyphRenderData::Shape(shape_handle) => {
                context
                    .commands
                    .render_shape(shape_handle, context.transform_stack.transform());
            }
            GlyphRenderData::AtlasGlyph(atlas_glyph) => {
                let handle = atlas_glyph.atlas_handle(context.renderer);
                let Some(handle) = handle else {
                    return;
                };

                context.transform_stack.push(&Transform {
                    matrix: Matrix::translate(atlas_glyph.tx(), atlas_glyph.ty()),
                    ..Default::default()
                });

                context.commands.render_bitmap(
                    handle,
                    context.transform_stack.transform(),
                    true,
                    ruffle_render::bitmap::PixelSnapping::Auto,
                    atlas_glyph.atlas_region(),
                );

                context.transform_stack.pop();
            }
        }
    }
}

pub enum GlyphRef<'a> {
    Direct(&'a Glyph),
    Ref(Ref<'a, Glyph>),
}

impl<'a> std::ops::Deref for GlyphRef<'a> {
    type Target = Glyph;

    fn deref(&self) -> &Self::Target {
        match self {
            GlyphRef::Direct(r) => r,
            GlyphRef::Ref(r) => r.deref(),
        }
    }
}

#[cfg(all(test, feature = "memory_font_face_cache"))]
mod font_face_cache_tests {
    use super::*;
    use ruffle_render::backend::ViewportDimensions;
    use ruffle_render::backend::null::NullRenderer;
    use ruffle_render::commands::{CommandHandler, CommandList};
    use ruffle_render::shape_utils::DrawCommand;
    use std::sync::Arc;

    fn renderer() -> NullRenderer {
        NullRenderer::new(ViewportDimensions {
            width: 64,
            height: 64,
            scale_factor: 1.0,
        })
    }

    fn glyph() -> Glyph {
        let mut drawing = Drawing::new();
        drawing.set_fill_style(Some(swf::FillStyle::Color(swf::Color::WHITE)));
        drawing.draw_command(DrawCommand::MoveTo(Point::ZERO));
        for (x, y) in [(200, 0), (200, 200), (0, 200), (0, 0)] {
            drawing.draw_command(DrawCommand::LineTo(Point::new(
                Twips::new(x),
                Twips::new(y),
            )));
        }
        drawing.set_fill_style(None);
        Glyph::from_drawing('A', Twips::new(240), drawing)
    }

    fn cache(glyph: &Glyph) -> &RefCell<FontFaceDrawingCache> {
        let GlyphShape::Drawing(cache) = &glyph.shape else {
            panic!("expected drawing glyph");
        };
        cache
    }

    fn shape_handle(glyph: &Glyph, renderer: &mut dyn RenderBackend) -> ShapeHandle {
        match glyph.glyph_render_data(renderer).expect("renderable glyph") {
            GlyphRenderData::Shape(handle) => handle,
            GlyphRenderData::AtlasGlyph(_) => panic!("expected drawing handle"),
        }
    }

    #[test]
    fn font_face_expiry_preserves_drawing_and_pending_commands() {
        let glyph = glyph();
        let mut renderer = renderer();
        let before = cache(&glyph).borrow().drawing.self_bounds(true);
        let handle = shape_handle(&glyph, &mut renderer);
        let weak = Arc::downgrade(&handle.0);
        let last_used = cache(&glyph).borrow().last_used.unwrap();
        let mut commands = CommandList::new();
        commands.render_shape(handle, Transform::default());
        assert_eq!(
            glyph.expire_font_face_render_cache(
                last_used + Duration::from_secs(90),
                Duration::from_secs(90),
            ),
            1
        );
        assert_eq!(
            glyph
                .expire_font_face_render_cache(last_used + Duration::from_secs(90), Duration::ZERO),
            0
        );
        assert_eq!(cache(&glyph).borrow().drawing.self_bounds(true), before);
        assert_eq!(glyph.advance(), Twips::new(240));
        assert_eq!(glyph.character(), 'A');
        assert!(glyph.hit_test(
            Point::new(Twips::new(100), Twips::new(100)),
            &Matrix::IDENTITY,
        ));
        assert!(
            weak.upgrade().is_some(),
            "pending commands own the old handle"
        );
        let rebuilt = shape_handle(&glyph, &mut renderer);
        assert_ne!(Arc::as_ptr(&rebuilt.0), weak.as_ptr());
        drop(commands);
        assert!(weak.upgrade().is_none());
        drop(rebuilt);
        for _ in 0..4 {
            let handle = shape_handle(&glyph, &mut renderer);
            let weak = Arc::downgrade(&handle.0);
            drop(handle);
            let used = cache(&glyph).borrow().last_used.unwrap();
            assert_eq!(glyph.expire_font_face_render_cache(used, Duration::ZERO), 1);
            assert!(weak.upgrade().is_none());
        }
    }

    #[test]
    fn font_face_render_refreshes_clock_and_clone_expires_independently() {
        let glyph = glyph();
        let mut renderer = renderer();
        let first = shape_handle(&glyph, &mut renderer);
        let weak = Arc::downgrade(&first.0);
        drop(first);
        let old = Instant::now() - Duration::from_secs(120);
        cache(&glyph).borrow_mut().last_used = Some(old);
        let cloned = glyph.clone();
        assert!(!std::ptr::eq(cache(&glyph), cache(&cloned)));
        drop(shape_handle(&cloned, &mut renderer));
        let refreshed = cache(&cloned).borrow().last_used.unwrap();
        assert!(refreshed > old);
        assert_eq!(cache(&glyph).borrow().last_used, Some(old));
        assert_eq!(
            glyph.expire_font_face_render_cache(Instant::now(), Duration::from_secs(90)),
            1
        );
        assert!(
            weak.upgrade().is_some(),
            "clone owns its own cached reference"
        );
        assert_eq!(
            cloned.expire_font_face_render_cache(
                refreshed + Duration::from_secs(89),
                Duration::from_secs(90)
            ),
            0
        );
        assert_eq!(
            cloned.expire_font_face_render_cache(
                refreshed - Duration::from_secs(1),
                Duration::from_secs(90)
            ),
            0
        );
        assert_eq!(
            cloned.expire_font_face_render_cache(
                refreshed + Duration::from_secs(90),
                Duration::from_secs(90)
            ),
            1
        );
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn font_face_hit_test_and_metrics_do_not_refresh_render_clock() {
        let glyph = glyph();
        let mut renderer = renderer();
        assert!(glyph.hit_test(
            Point::new(Twips::new(100), Twips::new(100)),
            &Matrix::IDENTITY,
        ));
        assert_eq!(cache(&glyph).borrow().last_used, None);
        drop(shape_handle(&glyph, &mut renderer));
        let old = Instant::now() - Duration::from_secs(120);
        cache(&glyph).borrow_mut().last_used = Some(old);
        assert_eq!(glyph.advance(), Twips::new(240));
        assert_eq!(glyph.character(), 'A');
        assert!(glyph.rendered_at_baseline());
        assert!(glyph.hit_test(
            Point::new(Twips::new(100), Twips::new(100)),
            &Matrix::IDENTITY,
        ));
        assert_eq!(cache(&glyph).borrow().last_used, Some(old));
        assert_eq!(
            glyph.expire_font_face_render_cache(Instant::now(), Duration::from_secs(90)),
            1
        );
        let empty = Glyph::from_drawing(' ', Twips::new(100), Drawing::new());
        assert!(empty.glyph_render_data(&mut renderer).is_none());
        assert_eq!(cache(&empty).borrow().last_used, None);
    }
}

#[cfg(all(test, feature = "memory_swf_glyph_cache"))]
mod render_cache_tests {
    use super::*;
    use ruffle_render::backend::ViewportDimensions;
    use ruffle_render::backend::null::NullRenderer;
    use ruffle_render::commands::{CommandHandler, CommandList};
    use std::sync::Arc;

    fn renderer() -> NullRenderer {
        NullRenderer::new(ViewportDimensions {
            width: 64,
            height: 64,
            scale_factor: 1.0,
        })
    }

    fn glyph() -> Glyph {
        let mut shape_records = vec![swf::ShapeRecord::StyleChange(Box::new(
            swf::StyleChangeData {
                move_to: Some(Point::ZERO),
                fill_style_0: Some(1),
                fill_style_1: None,
                line_style: None,
                new_styles: None,
            },
        ))];
        for (x, y) in [(200, 0), (0, 200), (-200, 0), (0, -200)] {
            shape_records.push(swf::ShapeRecord::StraightEdge {
                delta: swf::PointDelta::new(Twips::new(x), Twips::new(y)),
            });
        }
        Glyph::from_swf(
            'A',
            swf::Glyph {
                shape_records,
                code: u16::from(b'A'),
                advance: 240,
                bounds: None,
            },
        )
    }

    fn shape_handle(glyph: &Glyph, renderer: &mut dyn RenderBackend) -> ShapeHandle {
        match glyph.glyph_render_data(renderer).expect("renderable glyph") {
            GlyphRenderData::Shape(handle) => handle,
            GlyphRenderData::AtlasGlyph(_) => panic!("expected SWF shape"),
        }
    }

    fn with_cache<T>(glyph: &Glyph, f: impl FnOnce(&mut SwfGlyphOrShape) -> T) -> T {
        let GlyphShape::Swf(cache) = &glyph.shape else {
            panic!("expected SWF glyph");
        };
        f(&mut cache.borrow_mut())
    }

    fn last_used(glyph: &Glyph) -> Instant {
        with_cache(glyph, |cache| match cache {
            SwfGlyphOrShape::Shape { last_used, .. } => last_used.expect("registered cache"),
            _ => panic!("expected converted shape"),
        })
    }

    fn outline(glyph: &Glyph) -> swf::Shape {
        with_cache(glyph, |cache| cache.shape().0.clone())
    }

    #[test]
    fn swf_glyph_expiry_preserves_outline_and_queued_handles() {
        let glyph = glyph();
        let mut renderer = renderer();
        let handle = shape_handle(&glyph, &mut renderer);
        let weak = Arc::downgrade(&handle.0);
        let before = outline(&glyph);
        let mut commands = CommandList::new();
        commands.render_shape(handle, Transform::default());
        let expires = last_used(&glyph) + Duration::from_secs(90);

        assert_eq!(
            glyph.expire_render_cache(expires, Duration::from_secs(90)),
            1
        );
        assert_eq!(
            glyph.expire_render_cache(expires, Duration::from_secs(90)),
            0
        );
        assert_eq!(outline(&glyph), before);
        assert_eq!(glyph.character(), 'A');
        assert_eq!(glyph.advance(), Twips::new(240));
        assert!(glyph.hit_test(
            Point::new(Twips::new(100), Twips::new(100)),
            &Matrix::IDENTITY
        ));
        assert!(
            weak.upgrade().is_some(),
            "queued commands must keep the old mesh alive"
        );

        let rebuilt = shape_handle(&glyph, &mut renderer);
        assert_ne!(Arc::as_ptr(&rebuilt.0), weak.as_ptr());
        assert_eq!(outline(&glyph), before);
        drop(commands);
        assert!(
            weak.upgrade().is_none(),
            "dropping old commands releases their old mesh"
        );

        // Repeated expiration must not accumulate a chain of old handles.
        drop(rebuilt);
        for _ in 0..4 {
            let handle = shape_handle(&glyph, &mut renderer);
            let weak = Arc::downgrade(&handle.0);
            drop(handle);
            assert_eq!(
                glyph.expire_render_cache(last_used(&glyph), Duration::ZERO),
                1
            );
            assert!(weak.upgrade().is_none());
        }
    }

    #[test]
    fn swf_glyph_gpu_handle_use_refreshes_idle_deadline() {
        let glyph = glyph();
        let mut renderer = renderer();
        let handle = shape_handle(&glyph, &mut renderer);
        let old = Instant::now() - Duration::from_secs(120);
        with_cache(&glyph, |cache| cache.mark_render_cache_used(old));
        let reused = shape_handle(&glyph, &mut renderer);
        assert!(Arc::ptr_eq(&handle.0, &reused.0));
        let used = last_used(&glyph);
        assert!(used > old);
        assert_eq!(
            glyph.expire_render_cache(used - Duration::from_secs(1), Duration::from_secs(90)),
            0
        );
        assert_eq!(
            glyph.expire_render_cache(used + Duration::from_secs(89), Duration::from_secs(90)),
            0
        );
        assert_eq!(
            glyph.expire_render_cache(used + Duration::from_secs(90), Duration::from_secs(90)),
            1
        );
    }

    #[test]
    fn swf_glyph_metrics_and_hit_test_do_not_keep_gpu_cache_alive() {
        let cached = glyph();
        let mut renderer = renderer();
        drop(shape_handle(&cached, &mut renderer));
        let old = Instant::now() - Duration::from_secs(120);
        with_cache(&cached, |cache| cache.mark_render_cache_used(old));
        assert_eq!(cached.advance(), Twips::new(240));
        assert_eq!(cached.character(), 'A');
        assert!(cached.rendered_at_baseline());
        assert!(cached.hit_test(
            Point::new(Twips::new(100), Twips::new(100)),
            &Matrix::IDENTITY
        ));
        assert_eq!(last_used(&cached), old);
        assert_eq!(
            cached.expire_render_cache(old + Duration::from_secs(90), Duration::from_secs(90)),
            1
        );

        // Hit testing an unregistered glyph converts the outline but must not
        // create a GPU cache or an idle deadline.
        let untouched = glyph();
        assert!(untouched.hit_test(
            Point::new(Twips::new(100), Twips::new(100)),
            &Matrix::IDENTITY
        ));
        assert_eq!(
            untouched.expire_render_cache(Instant::now(), Duration::ZERO),
            0
        );
        with_cache(&untouched, |cache| {
            assert!(matches!(
                cache,
                SwfGlyphOrShape::Shape {
                    handle: None,
                    last_used: None,
                    ..
                }
            ));
        });
    }

    #[test]
    fn swf_glyph_clone_has_independent_expiry_and_keeps_shared_handle_alive() {
        let glyph = glyph();
        let mut renderer = renderer();
        let handle = shape_handle(&glyph, &mut renderer);
        let weak = Arc::downgrade(&handle.0);
        drop(handle);
        let old = Instant::now() - Duration::from_secs(120);
        with_cache(&glyph, |cache| cache.mark_render_cache_used(old));
        let cloned = glyph.clone();
        drop(shape_handle(&cloned, &mut renderer));
        assert_eq!(last_used(&glyph), old);
        assert!(last_used(&cloned) > old);
        assert_eq!(
            glyph.expire_render_cache(Instant::now(), Duration::from_secs(90)),
            1
        );
        assert!(
            weak.upgrade().is_some(),
            "clone owns a separate cached strong reference"
        );
        assert_eq!(
            cloned.expire_render_cache(last_used(&cloned), Duration::from_secs(90)),
            0
        );
        drop(cloned);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn swf_glyph_sweep_does_not_evict_drawing_handles() {
        let source = glyph();
        let drawing = Drawing::from_swf_shape(&outline(&source));
        let glyph = Glyph::from_drawing('A', Twips::new(240), drawing);
        let mut renderer = renderer();
        let first = shape_handle(&glyph, &mut renderer);
        assert_eq!(
            glyph.expire_render_cache(Instant::now() + Duration::from_secs(3600), Duration::ZERO),
            0
        );
        let second = shape_handle(&glyph, &mut renderer);
        assert!(Arc::ptr_eq(&first.0, &second.0));
    }

    #[test]
    fn swf_font_eager_handles_expire_without_access_and_measurement_does_not_refresh() {
        use crate::font::{EvalParameters, Font, FontLike, FontType};
        use crate::string::WStr;

        let player = crate::PlayerBuilder::new().build();
        player
            .lock()
            .unwrap()
            .mutate_with_update_context(|context| {
                let font = Font::from_swf_tag(
                    context.gc_context,
                    context.renderer,
                    swf::Font {
                        version: 2,
                        id: 1,
                        name: "IdleGlyphTest".into(),
                        language: swf::Language::Unknown,
                        layout: Some(swf::FontLayout {
                            ascent: 800,
                            descent: 200,
                            leading: 50,
                            kerning: vec![swf::KerningRecord {
                                left_code: u16::from(b'A'),
                                right_code: u16::from(b'A'),
                                adjustment: Twips::new(-20),
                            }],
                        }),
                        glyphs: vec![swf::Glyph {
                            shape_records: outline(&glyph()).shape,
                            code: u16::from(b'A'),
                            advance: 240,
                            bounds: None,
                        }],
                        flags: swf::FontFlag::HAS_LAYOUT,
                    },
                    swf::UTF_8,
                    FontType::Embedded,
                );
                let identity = font.as_ptr();
                let alias = font;
                // from_swf_tag eagerly registers ASCII. No glyph API was called
                // before this sweep, so a creation timestamp is necessary.
                assert_eq!(
                    font.expire_render_cache(
                        Instant::now() + Duration::from_secs(95),
                        Duration::from_secs(90)
                    ),
                    1
                );
                assert_eq!(alias.as_ptr(), identity);

                let cached = font.get_glyph_for_char('A').unwrap();
                drop(shape_handle(&cached, context.renderer));
                let old = Instant::now() - Duration::from_secs(120);
                with_cache(&cached, |cache| cache.mark_render_cache_used(old));
                let params = EvalParameters {
                    height: Twips::new(1024),
                    letter_spacing: Twips::ZERO,
                    kerning: true,
                    color: swf::Color::BLACK,
                };
                let text = WStr::from_units(b"AA");
                let before = font.measure(text, params);
                assert_eq!(before, Twips::new(460));
                assert!(font.has_glyphs_for_str(text));
                assert_eq!(font.get_glyph(0).unwrap().advance(), Twips::new(240));
                assert_eq!(font.get_kerning_offset('A', 'A'), Twips::new(-20));
                assert_eq!(font.metrics().ascent, 800);
                assert!(cached.hit_test(
                    Point::new(Twips::new(100), Twips::new(100)),
                    &Matrix::IDENTITY
                ));
                assert_eq!(last_used(&cached), old);
                assert_eq!(
                    alias.expire_render_cache(Instant::now(), Duration::from_secs(90)),
                    1
                );
                assert_eq!(font.measure(text, params), before);
                assert_eq!(font.as_ptr(), identity);
                assert_eq!(font.expire_render_cache(Instant::now(), Duration::ZERO), 0);
            });
    }
}
