use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::time::Duration;
use web_time::Instant;

use crate::backend::audio::SoundHandle;
use crate::binary_data::BinaryData;
use crate::display_object::{
    Avm1Button, Avm2Button, BitmapClass, EditText, Graphic, MorphShape, MovieClip, Text, Video,
};
use crate::font::Font;
use gc_arena::barrier::unlock;
use gc_arena::lock::Lock;
use gc_arena::{Collect, Gc, Mutation};
use ruffle_render::backend::RenderBackend;
use ruffle_render::bitmap::{Bitmap as RenderBitmap, BitmapHandle, BitmapSize};
use ruffle_render::error::Error as RenderError;
use swf::DefineBitsLossless;

#[derive(Copy, Clone, Collect, Debug)]
#[collect(no_drop)]
pub enum Character<'gc> {
    EditText(EditText<'gc>),
    Graphic(Graphic<'gc>),
    MovieClip(MovieClip<'gc>),
    Bitmap(Gc<'gc, BitmapCharacter<'gc>>),
    Avm1Button(Avm1Button<'gc>),
    Avm2Button(Avm2Button<'gc>),
    Font(Font<'gc>),
    MorphShape(MorphShape<'gc>),
    Text(Text<'gc>),
    Sound(#[collect(require_static)] SoundHandle),
    Video(Video<'gc>),
    BinaryData(Gc<'gc, BinaryData>),
}

impl<'gc> Character<'gc> {
    pub(crate) fn as_display_object(self) -> Option<crate::display_object::DisplayObject<'gc>> {
        match self {
            Self::EditText(v) => Some(v.into()),
            Self::Graphic(v) => Some(v.into()),
            Self::MovieClip(v) => Some(v.into()),
            Self::Avm1Button(v) => Some(v.into()),
            Self::Avm2Button(v) => Some(v.into()),
            Self::MorphShape(v) => Some(v.into()),
            Self::Text(v) => Some(v.into()),
            Self::Video(v) => Some(v.into()),
            _ => None,
        }
    }
}

#[derive(Collect, Debug)]
#[collect(no_drop)]
pub struct BitmapCharacter<'gc> {
    #[collect(require_static)]
    compressed: CompressedBitmap,
    /// A lazily constructed GPU handle, used when performing fills with this bitmap
    #[collect(require_static)]
    handle: RefCell<Option<BitmapHandle>>,
    #[collect(require_static)]
    last_handle_use: Cell<Option<Instant>>,
    /// The bitmap class set by `SymbolClass` - this is used when we instantaite
    /// a `Bitmap` displayobject.
    avm2_class: Lock<BitmapClass<'gc>>,
}

impl<'gc> BitmapCharacter<'gc> {
    pub fn new(compressed: CompressedBitmap) -> Self {
        Self {
            compressed,
            handle: RefCell::new(None),
            last_handle_use: Cell::new(None),
            avm2_class: Lock::new(BitmapClass::NoSubclass),
        }
    }

    pub(crate) fn has_cached_handle(&self) -> bool {
        self.handle.borrow().is_some()
    }

    /// Tessellated meshes retain their bitmap handle. Release only an idle
    /// library reference with no other handle owner; the mesh can rebuild it.
    pub(crate) fn expire_idle_handle(&self, now: Instant, idle_limit: Duration) -> bool {
        let Some(last_use) = self.last_handle_use.get() else {
            return false;
        };
        if now.saturating_duration_since(last_use) < idle_limit {
            return false;
        }
        let mut cached = self.handle.borrow_mut();
        if cached
            .as_ref()
            .is_some_and(|handle| Arc::strong_count(&handle.0) == 1)
        {
            cached.take();
            self.last_handle_use.set(None);
            true
        } else {
            false
        }
    }

    pub fn compressed(&self) -> &CompressedBitmap {
        &self.compressed
    }

    pub fn avm2_class(&self) -> BitmapClass<'gc> {
        self.avm2_class.get()
    }

    pub fn set_avm2_class(this: Gc<'gc, Self>, bitmap_class: BitmapClass<'gc>, mc: &Mutation<'gc>) {
        unlock!(Gc::write(mc, this), Self, avm2_class).set(bitmap_class);
    }

    pub fn bitmap_handle(
        &self,
        backend: &mut dyn RenderBackend,
    ) -> Result<BitmapHandle, RenderError> {
        self.last_handle_use.set(Some(Instant::now()));
        if let Some(handle) = self.handle.borrow().as_ref() {
            return Ok(handle.clone());
        }
        let decoded = self.compressed.decode()?;
        let new_handle = backend.register_bitmap(decoded)?;
        *self.handle.borrow_mut() = Some(new_handle.clone());
        Ok(new_handle)
    }
}

#[cfg(test)]
mod bitmap_cache_tests {
    use super::{BitmapCharacter, BitmapHandle, CompressedBitmap};
    use ruffle_render::bitmap::BitmapHandleImpl;
    use std::sync::Arc;
    use std::time::Duration;
    use web_time::Instant;

    #[derive(Debug)]
    struct TestBitmapHandle;
    impl BitmapHandleImpl for TestBitmapHandle {}

    #[test]
    fn idle_bitmap_upload_waits_for_external_mesh_reference() {
        let character = BitmapCharacter::new(CompressedBitmap::Jpeg {
            data: vec![],
            alpha: None,
            width: 1,
            height: 1,
        });
        let mesh_reference = BitmapHandle(Arc::new(TestBitmapHandle));
        *character.handle.borrow_mut() = Some(mesh_reference.clone());
        let first_use = Instant::now();
        character.last_handle_use.set(Some(first_use));
        let idle_limit = Duration::from_secs(90);

        assert!(!character.expire_idle_handle(first_use + Duration::from_secs(89), idle_limit));
        assert!(!character.expire_idle_handle(first_use + idle_limit, idle_limit));
        drop(mesh_reference);
        assert!(character.expire_idle_handle(first_use + idle_limit, idle_limit));
        assert!(!character.has_cached_handle());
    }
}

/// Holds a bitmap from an SWF tag, plus the decoded width/height.
/// We avoid decompressing the image until it's actually needed - some pathological SWFS
/// like 'House' have thousands of highly-compressed (mostly empty) bitmaps, which can
/// take over 10GB of ram if we decompress them all during preloading.
#[derive(Clone, Debug)]
pub enum CompressedBitmap {
    Jpeg {
        data: Vec<u8>,
        alpha: Option<Vec<u8>>,
        width: u32,
        height: u32,
    },
    Lossless(DefineBitsLossless<'static>),
}

impl CompressedBitmap {
    pub fn size(&self) -> BitmapSize {
        match self {
            CompressedBitmap::Jpeg { width, height, .. } => BitmapSize {
                width: *width,
                height: *height,
            },
            CompressedBitmap::Lossless(define_bits_lossless) => BitmapSize {
                width: define_bits_lossless.width.into(),
                height: define_bits_lossless.height.into(),
            },
        }
    }
    pub fn decode(&self) -> Result<RenderBitmap<'static>, RenderError> {
        match self {
            CompressedBitmap::Jpeg {
                data,
                alpha,
                width: _,
                height: _,
            } => ruffle_render::utils::decode_define_bits_jpeg(data, alpha.as_deref()),
            CompressedBitmap::Lossless(define_bits_lossless) => {
                ruffle_render::utils::decode_define_bits_lossless(define_bits_lossless)
            }
        }
    }
}
