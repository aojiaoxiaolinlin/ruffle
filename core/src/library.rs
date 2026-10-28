use crate::avm1::{PropertyMap as Avm1PropertyMap, PropertyMap};
use crate::avm2::{Class as Avm2Class, Domain as Avm2Domain};
use crate::backend::audio::SoundHandle;
use crate::character::Character;

use crate::display_object::{Bitmap, Graphic, MorphShape, Text};
use crate::font::{Font, FontDescriptor, FontLike, FontQuery, FontType};
use crate::prelude::*;
use crate::string::AvmString;
use crate::tag_utils::SwfMovie;
use gc_arena::barrier::unlock;
use gc_arena::collect::Trace;
use gc_arena::lock::RefLock;
use gc_arena::{Collect, Gc, GcWeak, Mutation};
use ruffle_render::backend::RenderBackend;
use ruffle_render::bitmap::BitmapHandle;
use ruffle_render::utils::remove_invalid_jpeg_data;
use ruffle_wstr::{WStr, WString};
use std::cell::{Ref, RefMut};

use crate::backend::ui::{FontDefinition, UiBackend};
use crate::font::DefaultFont;
use fnv::{FnvHashMap, FnvHashSet};
use std::collections::HashMap;
use std::sync::{Arc, Weak};
use std::time::Duration;
use weak_table::PtrWeakKeyHashMap;
use web_time::Instant;

#[derive(Collect)]
#[collect(no_drop)]
struct WeakClassSymbol<'gc> {
    class: GcWeak<'gc, ()>,
    #[collect(require_static)]
    movie: Weak<SwfMovie>,
    symbol: CharacterId,
}

/// The mappings between class objects and library characters defined by
/// `SymbolClass`.
#[derive(Collect)]
#[collect(no_drop)]
pub struct Avm2ClassRegistry<'gc> {
    /// A list of AVM2 class objects and the character IDs they are expected to
    /// instantiate.
    // Weak identities are traced as weak GC edges. Strong keys here would
    // retain Class -> Method -> TranslationUnit -> movie even after unload.
    class_map: FnvHashMap<usize, WeakClassSymbol<'gc>>,
    next_cleanup: usize,
}

impl Default for Avm2ClassRegistry<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'gc> Avm2ClassRegistry<'gc> {
    pub fn new() -> Self {
        Self {
            class_map: FnvHashMap::default(),
            next_cleanup: 256,
        }
    }

    /// Retrieve the library symbol for a given AVM2 class object.
    ///
    /// A value of `None` indicates that this AVM2 class is not associated with
    /// a library symbol.
    pub fn class_symbol(&self, class_def: Avm2Class<'gc>) -> Option<(Arc<SwfMovie>, CharacterId)> {
        let entry = self.class_map.get(&(class_def.as_ptr() as usize))?;
        Some((entry.movie.upgrade()?, entry.symbol))
    }

    /// Associate an AVM2 class definition with a given library symbol.
    pub fn set_class_symbol(
        &mut self,
        class_def: Avm2Class<'gc>,
        movie: Arc<SwfMovie>,
        symbol: CharacterId,
        definition: MovieDefinition<'gc>,
        mc: &Mutation<'gc>,
    ) {
        if let Some((old_movie, old_symbol)) = self.class_symbol(class_def) {
            if Arc::ptr_eq(&movie, &old_movie) && symbol != old_symbol {
                // Flash player actually allows using the same class in multiple SymbolClass
                // entries in the same swf, with *different* symbol ids. Whichever one
                // is processed first will *win*, and the second one will be ignored.
                // We still log a warning, since we wouldn't expect this to happen outside
                // of deliberately crafted SWFs.
                tracing::warn!(
                    "Tried to overwrite class {:?} id={:?} with symbol id={:?} from same movie",
                    class_def,
                    old_symbol,
                    symbol,
                );
            }
            // If we're trying to overwrite the class with a symbol from a *different* SwfMovie,
            // then just ignore it. This handles the case where a Loader has a class that shadows
            // a class in the main swf (possibly with a different ApplicationDomain). This will
            // result in the original class from the parent being used, even when the child swf
            // instantiates the clip on the timeline.
            return;
        }
        // Amortize cleanup; scanning on every registration would be quadratic
        // while loading large libraries. No GC payload is accessed through an
        // erased pointer: is_dropped reads only the traced weak allocation.
        if self.class_map.len() >= self.next_cleanup {
            self.class_map
                .retain(|_, entry| !entry.class.is_dropped() && entry.movie.upgrade().is_some());
            self.next_cleanup = self.class_map.len().saturating_mul(2).max(256);
        }
        class_def.retain_symbol_definition(mc, movie.clone(), definition);
        self.class_map.insert(
            class_def.as_ptr() as usize,
            WeakClassSymbol {
                class: class_def.weak_identity(),
                movie: Arc::downgrade(&movie),
                symbol,
            },
        );
    }
}

/// Symbol library for a single given SWF.
#[derive(Collect)]
#[collect(no_drop)]
pub struct MovieLibrary<'gc> {
    // MovieDefinition owns the immutable movie; library borrows only need a
    // weak back-reference. The global index owns neither the movie nor library.
    #[collect(require_static)]
    swf: Weak<SwfMovie>,
    characters: HashMap<CharacterId, Character<'gc>>,
    export_characters: Avm1PropertyMap<'gc, CharacterId>,
    imported_assets: HashMap<AvmString<'gc>, CharacterId>,
    jpeg_tables: Option<Vec<u8>>,
    fonts: FontMap<'gc>,
    avm2_domain: Option<Avm2Domain<'gc>>,
    owner: Option<MovieDefinitionWeak<'gc>>,
    /// Importers must retain source definitions, not just copied characters.
    dependencies: Vec<MovieDefinition<'gc>>,
}

impl<'gc> MovieLibrary<'gc> {
    pub fn new(swf: Arc<SwfMovie>) -> Self {
        Self {
            swf: Arc::downgrade(&swf),
            characters: HashMap::new(),
            imported_assets: HashMap::new(),
            export_characters: Avm1PropertyMap::new(),
            jpeg_tables: None,
            fonts: Default::default(),
            avm2_domain: None,
            owner: None,
            dependencies: Vec::new(),
        }
    }

    pub fn add_dependency(&mut self, definition: MovieDefinition<'gc>) {
        if !self
            .dependencies
            .iter()
            .any(|existing| Gc::ptr_eq(existing.0, definition.0))
        {
            self.dependencies.push(definition);
        }
    }

    /// Registers a character; returns `true` if successful, or `false` if a character with
    /// the given ID already exists.
    pub fn register_character(
        &mut self,
        id: CharacterId,
        character: Character<'gc>,
        mc: &Mutation<'gc>,
    ) -> bool {
        use std::collections::hash_map::Entry;
        match self.characters.entry(id) {
            Entry::Vacant(e) => {
                if let Some(prototype) = character.as_display_object()
                    && prototype.definition().is_none()
                    && let Some(owner) = self.owner.and_then(|owner| owner.upgrade(mc))
                {
                    prototype.set_definition(mc, owner);
                }
                if let Character::Font(font) = character {
                    if let Some(owner) = self.owner.and_then(|owner| owner.upgrade(mc)) {
                        font.set_definition(mc, owner);
                    }
                    self.fonts.register(font);
                }
                e.insert(character);
                true
            }
            Entry::Occupied(_) => {
                tracing::error!("Character ID collision: Tried to register ID {} twice", id);
                false
            }
        }
    }

    /// Registers an export name for a given character ID.
    /// This character will then be instantiable from AVM1.
    pub fn register_export(&mut self, id: CharacterId, export_name: AvmString<'gc>) {
        let character_exists = self.contains_character(id);
        debug_assert!(character_exists);
        if !character_exists {
            tracing::error!(
                "Tried to register export '{export_name}' for a non-existent character {id}"
            );
            return;
        }

        self.export_characters.insert(export_name, id, false);
    }

    pub fn register_font_name(&mut self, character_id: u16, font_name: &str) {
        let Some(Character::Font(font)) = self.character_by_id(character_id) else {
            return;
        };

        let descriptor = FontDescriptor::from_parts(
            font_name,
            font.descriptor().bold(),
            font.descriptor().italic(),
        );
        self.fonts.register_with_descriptor(font, &descriptor);
    }

    pub fn characters(&self) -> &HashMap<CharacterId, Character<'gc>> {
        &self.characters
    }

    pub fn export_characters(&self) -> &PropertyMap<'gc, CharacterId> {
        &self.export_characters
    }

    pub fn contains_character(&self, id: CharacterId) -> bool {
        self.characters.contains_key(&id)
    }

    pub fn character_by_id(&self, id: CharacterId) -> Option<Character<'gc>> {
        self.characters.get(&id).copied()
    }

    pub fn character_by_export_name(&self, name: &WStr) -> Option<(CharacterId, Character<'gc>)> {
        if let Some(id) = self.export_characters.get(name, false)
            && let Some(character) = self.characters.get(id)
        {
            return Some((*id, *character));
        }
        None
    }

    pub fn character_id_by_import_name(&self, name: &WStr) -> Option<CharacterId> {
        self.imported_assets.get(name).copied()
    }

    pub fn register_import(&mut self, name: AvmString<'gc>, id: CharacterId) {
        self.imported_assets.insert(name, id);
    }

    /// Instantiates the library item with the given character ID into a display object.
    /// The object must then be post-instantiated before being used.
    pub fn instantiate_by_id(
        &self,
        id: CharacterId,
        mc: &Mutation<'gc>,
    ) -> Option<DisplayObject<'gc>> {
        if let Some(&character) = self.characters.get(&id) {
            self.instantiate_display_object(id, character, mc)
        } else {
            tracing::error!("Tried to instantiate a non-registered character ID {id}");
            None
        }
    }

    /// Instantiates the library item with the given export name into a display object.
    /// The object must then be post-instantiated before being used.
    pub fn instantiate_by_export_name(
        &self,
        export_name: &WStr,
        mc: &Mutation<'gc>,
    ) -> Option<DisplayObject<'gc>> {
        if let Some((id, character)) = self.character_by_export_name(export_name) {
            self.instantiate_display_object(id, character, mc)
        } else {
            tracing::error!("Tried to instantiate a non-registered character {export_name}");
            None
        }
    }

    /// Instantiates the given character into a display object.
    /// The object must then be post-instantiated before being used.
    fn instantiate_display_object(
        &self,
        id: CharacterId,
        character: Character<'gc>,
        mc: &Mutation<'gc>,
    ) -> Option<DisplayObject<'gc>> {
        let instance: Option<DisplayObject<'gc>> = match character {
            Character::Bitmap(bitmap) => {
                let avm2_class = bitmap.avm2_class();
                let bitmap = bitmap.compressed().decode().unwrap();
                let movie = self.swf.upgrade()?;
                let bitmap = Bitmap::new(mc, id, bitmap, movie);
                bitmap.set_avm2_bitmapdata_class(mc, avm2_class);
                Some(bitmap.instantiate(mc).into())
            }
            Character::EditText(edit_text) => Some(edit_text.instantiate(mc).into()),
            Character::Graphic(graphic) => Some(graphic.instantiate(mc).into()),
            Character::MorphShape(morph_shape) => Some(morph_shape.instantiate(mc).into()),
            Character::MovieClip(movie_clip) => Some(movie_clip.instantiate(mc).into()),
            Character::Avm1Button(button) => Some(button.instantiate(mc).into()),
            Character::Avm2Button(button) => Some(button.instantiate(mc).into()),
            Character::Text(text) => Some(text.instantiate(mc).into()),
            Character::Video(video) => Some(video.instantiate(mc).into()),
            _ => {
                // Cannot instantiate non-display object
                None
            }
        };
        let instance = instance?;
        if instance.definition().is_none()
            && let Some(definition) = self.owner.and_then(|owner| owner.upgrade(mc))
        {
            instance.set_definition(mc, definition);
        }
        Some(instance)
    }

    pub fn get_font(&self, id: CharacterId) -> Option<Font<'gc>> {
        if let Some(&Character::Font(font)) = self.characters.get(&id) {
            Some(font)
        } else {
            None
        }
    }

    pub fn embedded_fonts(&self) -> Vec<Font<'gc>> {
        self.fonts.all()
    }

    /// Returns the `Graphic` with the given character ID.
    /// Returns `None` if the ID does not exist or is not a `Graphic`.
    pub fn get_graphic(&self, id: CharacterId) -> Option<Graphic<'gc>> {
        if let Some(&Character::Graphic(graphic)) = self.characters.get(&id) {
            Some(graphic)
        } else {
            None
        }
    }

    /// Returns the `MorphShape` with the given character ID.
    /// Returns `None` if the ID does not exist or is not a `MorphShape`.
    pub fn get_morph_shape(&self, id: CharacterId) -> Option<MorphShape<'gc>> {
        if let Some(&Character::MorphShape(morph_shape)) = self.characters.get(&id) {
            Some(morph_shape)
        } else {
            None
        }
    }

    pub fn get_sound(&self, id: CharacterId) -> Option<SoundHandle> {
        if let Some(Character::Sound(sound)) = self.characters.get(&id) {
            Some(*sound)
        } else {
            None
        }
    }

    /// Returns the `Text` with the given character ID.
    /// Returns `None` if the ID does not exist or is not a `Text`.
    pub fn get_text(&self, id: CharacterId) -> Option<Text<'gc>> {
        if let Some(&Character::Text(text)) = self.characters.get(&id) {
            Some(text)
        } else {
            None
        }
    }

    pub fn set_jpeg_tables(&mut self, data: &[u8]) {
        if self.jpeg_tables.is_some() {
            // SWF spec says there should only be one JPEGTables tag.
            // TODO: What is the behavior when there are multiples?
            tracing::warn!("SWF contains multiple JPEGTables tags");
            return;
        }
        // Some SWFs have a JPEGTables tag with 0 length; ignore these.
        // (Does this happen when there is only a single DefineBits tag?)
        self.jpeg_tables = if data.is_empty() {
            None
        } else {
            Some(remove_invalid_jpeg_data(data).to_vec())
        }
    }

    pub fn jpeg_tables(&self) -> Option<&[u8]> {
        self.jpeg_tables.as_ref().map(|data| &data[..])
    }

    pub fn set_avm2_domain(&mut self, avm2_domain: Avm2Domain<'gc>) {
        self.avm2_domain = Some(avm2_domain);
    }

    /// Get the AVM2 domain this movie runs under.
    ///
    /// Note that the presence of an AVM2 domain does *not* indicate that this
    /// movie provides AVM2 code. For example, a movie may have been loaded by
    /// AVM2 code into a particular domain, even though it turned out to be
    /// an AVM1 movie, and thus this domain is unused.
    pub fn avm2_domain(&self) -> Avm2Domain<'gc> {
        self.avm2_domain.unwrap()
    }

    pub fn try_avm2_domain(&self) -> Option<Avm2Domain<'gc>> {
        self.avm2_domain
    }
}

pub struct MovieLibrarySource<'a, 'gc> {
    pub library: &'a MovieLibrary<'gc>,
}

impl ruffle_render::bitmap::BitmapSource for MovieLibrarySource<'_, '_> {
    fn bitmap_size(&self, id: u16) -> Option<ruffle_render::bitmap::BitmapSize> {
        if let Some(Character::Bitmap(bitmap)) = self.library.characters.get(&id) {
            Some(bitmap.compressed().size())
        } else {
            None
        }
    }

    fn bitmap_handle(&self, id: u16, backend: &mut dyn RenderBackend) -> Option<BitmapHandle> {
        let Some(Character::Bitmap(bitmap)) = self.library.characters.get(&id) else {
            return None;
        };

        match bitmap.bitmap_handle(backend) {
            Ok(handle) => Some(handle),
            Err(e) => {
                tracing::error!("Failed to register bitmap character {id}: {e}");
                None
            }
        }
    }
}

/// A traced owner of a movie's definitions, independent of its byte storage.
/// Consumers retain this handle; the global index traces only weak GC edges.
#[derive(Clone, Copy, Collect)]
#[collect(no_drop)]
pub struct MovieDefinition<'gc>(Gc<'gc, MovieDefinitionData<'gc>>);

#[derive(Collect)]
#[collect(no_drop)]
struct MovieDefinitionData<'gc> {
    movie: Arc<SwfMovie>,
    library: RefLock<MovieLibrary<'gc>>,
}

impl std::fmt::Debug for MovieDefinition<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("MovieDefinition")
            .field(&Gc::as_ptr(self.0))
            .finish()
    }
}

#[derive(Clone, Copy, Collect)]
#[collect(no_drop)]
pub struct MovieDefinitionWeak<'gc>(GcWeak<'gc, MovieDefinitionData<'gc>>);

impl<'gc> MovieDefinitionWeak<'gc> {
    pub fn upgrade(self, mc: &Mutation<'gc>) -> Option<MovieDefinition<'gc>> {
        GcWeak::upgrade(self.0, mc).map(MovieDefinition)
    }
}

impl<'gc> MovieDefinition<'gc> {
    pub fn movie(self) -> Arc<SwfMovie> {
        self.0.movie.clone()
    }

    pub fn downgrade(self) -> MovieDefinitionWeak<'gc> {
        MovieDefinitionWeak(Gc::downgrade(self.0))
    }

    pub fn new(movie: Arc<SwfMovie>, mc: &Mutation<'gc>) -> Self {
        let definition = Self(Gc::new(
            mc,
            MovieDefinitionData {
                movie: movie.clone(),
                library: RefLock::new(MovieLibrary::new(movie)),
            },
        ));
        definition.library_mut(mc).owner = Some(definition.downgrade());
        definition
    }

    pub fn library(self) -> Ref<'gc, MovieLibrary<'gc>> {
        Gc::as_ref(self.0).library.borrow()
    }

    pub fn library_mut(self, mc: &Mutation<'gc>) -> RefMut<'gc, MovieLibrary<'gc>> {
        unlock!(Gc::write(mc, self.0), MovieDefinitionData, library).borrow_mut()
    }
}

struct MovieLibraries<'gc>(PtrWeakKeyHashMap<Weak<SwfMovie>, MovieDefinitionWeak<'gc>>);

unsafe impl<'gc> Collect<'gc> for MovieLibraries<'gc> {
    #[inline]
    fn trace<C: Trace<'gc>>(&self, cc: &mut C) {
        // Trace weak GC edges as weak edges. The index must not own definitions.
        for (_, val) in self.0.iter() {
            cc.trace(val);
        }
    }
}

impl<'gc> MovieLibraries<'gc> {
    fn new() -> Self {
        Self(PtrWeakKeyHashMap::new())
    }

    fn get(&self, key: &Arc<SwfMovie>, mc: &Mutation<'gc>) -> Option<MovieDefinition<'gc>> {
        self.0.get(key)?.upgrade(mc)
    }

    fn get_or_insert(&mut self, movie: Arc<SwfMovie>, mc: &Mutation<'gc>) -> MovieDefinition<'gc> {
        if let Some(definition) = self.get(&movie, mc) {
            return definition;
        }
        let definition = MovieDefinition::new(movie.clone(), mc);
        self.0.insert(movie, definition.downgrade());
        definition
    }

    fn known_movies(&self) -> impl Iterator<Item = Arc<SwfMovie>> {
        self.0
            .iter()
            .filter(|(_, definition)| !definition.0.is_dropped())
            .map(|(movie, _)| movie)
    }
}

#[cfg(test)]
mod movie_library_lifetime_tests {
    use super::{MovieLibraries, SwfMovie};
    use crate::character::Character;
    use crate::display_object::MovieClip;
    use gc_arena::{Arena, Rootable};
    use std::sync::Arc;

    #[test]
    fn weak_index_does_not_keep_movie_or_empty_definition_alive() {
        let movie = Arc::new(SwfMovie::empty(10, None));
        let weak = Arc::downgrade(&movie);
        let mut arena = Arena::<Rootable![MovieLibraries<'_>]>::new(|mc| {
            let mut libraries = MovieLibraries::new();
            libraries.get_or_insert(movie.clone(), mc);
            assert_eq!(libraries.known_movies().count(), 1);
            libraries
        });
        drop(movie);
        for _ in 0..3 {
            arena.finish_cycle();
        }
        assert!(weak.upgrade().is_none());
        arena.mutate(|_, libraries| assert_eq!(libraries.known_movies().count(), 0));
    }

    #[test]
    fn bare_movie_bytes_do_not_root_symbols_and_dead_entries_can_be_replaced() {
        let movie = Arc::new(SwfMovie::empty(10, None));
        let mut arena = Arena::<Rootable![MovieLibraries<'_>]>::new(|mc| {
            let mut libraries = MovieLibraries::new();
            libraries
                .get_or_insert(movie.clone(), mc)
                .library_mut(mc)
                .register_character(
                    1,
                    Character::MovieClip(MovieClip::new(movie.clone(), mc)),
                    mc,
                );
            libraries
        });
        for _ in 0..3 {
            arena.finish_cycle();
        }
        arena.mutate_root(|mc, libraries| {
            assert_eq!(libraries.known_movies().count(), 0);
            assert!(libraries.get(&movie, mc).is_none());
            let replacement = libraries.get_or_insert(movie.clone(), mc);
            assert!(!replacement.library().contains_character(1));
            assert_eq!(libraries.known_movies().count(), 1);
        });
    }

    // An isolated dependency-graph control alongside the production owner
    // tests. Use real library/character data to verify weak GC edges cannot
    // root cyclic definition dependencies.
    #[derive(gc_arena::Collect)]
    #[collect(no_drop)]
    struct DefinitionOwner<'gc> {
        library: super::MovieLibrary<'gc>,
        dependencies: Vec<gc_arena::Gc<'gc, gc_arena::lock::RefLock<DefinitionOwner<'gc>>>>,
    }

    #[derive(gc_arena::Collect)]
    #[collect(no_drop)]
    struct OwnershipPrototype<'gc> {
        index: Vec<gc_arena::GcWeak<'gc, gc_arena::lock::RefLock<DefinitionOwner<'gc>>>>,
        consumer: Option<gc_arena::Gc<'gc, gc_arena::lock::RefLock<DefinitionOwner<'gc>>>>,
    }

    #[test]
    fn weak_definition_index_preserves_live_imports_and_collects_dependency_cycles() {
        use crate::display_object::TDisplayObject;
        use gc_arena::lock::RefLock;
        use gc_arena::{Gc, GcWeak};

        let first = Arc::new(SwfMovie::empty(10, None));
        let second = Arc::new(SwfMovie::empty(10, None));
        let weak_movies = [Arc::downgrade(&first), Arc::downgrade(&second)];
        let mut arena = Arena::<Rootable![OwnershipPrototype<'_>]>::new(|mc| {
            let mut first_library = super::MovieLibrary::new(first.clone());
            let mut second_library = super::MovieLibrary::new(second.clone());
            first_library.register_character(
                1,
                Character::MovieClip(MovieClip::new(first, mc)),
                mc,
            );
            let imported = Character::MovieClip(MovieClip::new(second, mc));
            second_library.register_character(2, imported, mc);
            // Match export_assets copying a source Character into an importer.
            first_library.register_character(7, imported, mc);
            let first = Gc::new(
                mc,
                RefLock::new(DefinitionOwner {
                    library: first_library,
                    dependencies: Vec::new(),
                }),
            );
            let second = Gc::new(
                mc,
                RefLock::new(DefinitionOwner {
                    library: second_library,
                    dependencies: vec![first],
                }),
            );
            first.borrow_mut(mc).dependencies.push(second);
            OwnershipPrototype {
                index: vec![Gc::downgrade(first), Gc::downgrade(second)],
                consumer: Some(first),
            }
        });
        for _ in 0..3 {
            arena.finish_cycle();
        }
        arena.mutate(|mc, root| {
            let first = GcWeak::upgrade(root.index[0], mc).unwrap();
            assert!(Gc::ptr_eq(first, root.consumer.unwrap()));
            let second = GcWeak::upgrade(root.index[1], mc).unwrap();
            let imported = first.borrow().library.instantiate_by_id(7, mc).unwrap();
            assert!(Arc::ptr_eq(
                &imported.movie(),
                &weak_movies[1].upgrade().unwrap()
            ));
            assert!(second.borrow().library.instantiate_by_id(2, mc).is_some());
        });
        assert!(weak_movies.iter().all(|movie| movie.upgrade().is_some()));
        arena.mutate_root(|_, root| root.consumer = None);
        for _ in 0..3 {
            arena.finish_cycle();
        }
        arena.mutate(|mc, root| {
            assert!(
                root.index
                    .iter()
                    .all(|owner| GcWeak::upgrade(*owner, mc).is_none())
            );
        });
        assert!(weak_movies.iter().all(|movie| movie.upgrade().is_none()));
    }

    #[test]
    fn idle_render_sweep_preserves_exports_and_rebuilds_shared_meshes() {
        use crate::context::{RenderContext, UpdateContext};
        use crate::display_object::{DisplayObject, Graphic, TDisplayObject};
        use crate::string::AvmString;
        use ruffle_render::backend::ShapeHandle;
        use ruffle_render::commands::{Command, CommandList};
        use ruffle_render::transform::TransformStack;
        use std::time::Duration;
        use web_time::Instant;

        fn draw<'gc>(graphic: Graphic<'gc>, context: &mut UpdateContext<'gc>) -> ShapeHandle {
            let mut transforms = TransformStack::new();
            let mut cache_draws = Vec::new();
            let mut render = RenderContext {
                renderer: &mut *context.renderer,
                commands: CommandList::new(),
                cache_draws: &mut cache_draws,
                gc_context: context.gc_context,
                library: &mut *context.library,
                ui: &*context.ui,
                transform_stack: &mut transforms,
                untransformed_object: None,
                is_offscreen: true,
                use_bitmap_cache: true,
                stage: context.stage,
            };
            graphic.render_self(&mut render);
            match render.commands.commands.remove(0) {
                Command::RenderShape { shape, .. } => shape,
                other => panic!("Expected a shape draw, got {other:?}"),
            }
        }

        let player = crate::PlayerBuilder::new().build();
        player
            .lock()
            .unwrap()
            .mutate_with_update_context(|context| {
                let movie = Arc::new(SwfMovie::empty(8, None));
                let graphic = Graphic::from_swf_tag(
                    context,
                    swf::Shape {
                        version: 1,
                        id: 1,
                        shape_bounds: Default::default(),
                        edge_bounds: Default::default(),
                        flags: swf::ShapeFlag::empty(),
                        styles: swf::ShapeStyles {
                            fill_styles: vec![],
                            line_styles: vec![],
                        },
                        shape: vec![],
                    },
                    movie.clone(),
                );
                let name = AvmString::new_utf8(context.gc(), "CachedGraphic");
                let mut library = context
                    .library
                    .library_for_movie_mut(movie.clone(), context.gc_context);
                library.register_character(1, Character::Graphic(graphic), context.gc_context);
                library.register_export(1, name);
                let DisplayObject::Graphic(instance) = library
                    .instantiate_by_export_name(&name, context.gc_context)
                    .unwrap()
                else {
                    panic!("Expected a graphic symbol")
                };
                drop(library);
                let queued_draw = draw(instance, context);
                let weak_handle = Arc::downgrade(&queued_draw.0);
                assert_eq!(graphic.cached_shape_count(), 1);
                let now = Instant::now();
                // The default policy must preserve existing cache behavior.
                context
                    .library
                    .sweep_render_caches_at(now + Duration::from_secs(10000), context.gc_context);
                assert_eq!(graphic.cached_shape_count(), 1);
                context
                    .library
                    .set_render_cache_idle_limit(Some(Duration::from_secs(90)));
                context
                    .library
                    .sweep_render_caches_at(now + Duration::from_secs(89), context.gc_context);
                assert_eq!(instance.cached_shape_count(), 1);
                // A sweep is rate-limited, even if an object just crossed its TTL.
                context
                    .library
                    .sweep_render_caches_at(now + Duration::from_secs(91), context.gc_context);
                assert_eq!(instance.cached_shape_count(), 1);
                context
                    .library
                    .sweep_render_caches_at(now + Duration::from_secs(95), context.gc_context);
                assert_eq!(graphic.cached_shape_count(), 0);
                assert_eq!(instance.cached_shape_count(), 0);
                assert!(
                    weak_handle.upgrade().is_some(),
                    "queued draws keep their handles"
                );
                drop(queued_draw);
                assert!(weak_handle.upgrade().is_none());
                // The movie, symbol and export remain valid for late instantiation.
                let library = context
                    .library
                    .library_for_movie(movie, context.gc_context)
                    .unwrap();
                let DisplayObject::Graphic(late_instance) = library
                    .instantiate_by_export_name(&name, context.gc_context)
                    .unwrap()
                else {
                    panic!("Expected a graphic symbol")
                };
                let rebuilt = draw(late_instance, context);
                assert_ne!(Arc::as_ptr(&rebuilt.0), weak_handle.as_ptr());
                assert_eq!(graphic.cached_shape_count(), 1);
            });
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Collect)]
#[collect(require_static)]
struct SoundDefinitionKey(SoundHandle);

/// Symbol library for multiple movies.
#[derive(Collect)]
#[collect(no_drop)]
pub struct Library<'gc> {
    /// All the movie libraries.
    movie_libraries: MovieLibraries<'gc>,
    sound_definitions: FnvHashMap<SoundDefinitionKey, MovieDefinitionWeak<'gc>>,
    next_sound_cleanup: usize,

    /// A cache of seen device fonts.
    // TODO: Descriptors shouldn't be stored in fonts. Fonts should be a list that we iterate and ask "do you match". A font can have zero or many names.
    device_fonts: FontMap<'gc>,

    /// "Global" embedded fonts, registered through AVM2 `Font.registerFont`.
    /// These should be checked before any Movie-specific library's own fonts.
    global_fonts: FontMap<'gc>,

    /// A set of which fonts we've asked from the backend already, to help with negative caching.
    /// If we've asked for a specific font, record it here and don't ask again.
    font_lookup_cache: FnvHashSet<FontQuery>,

    /// Cached font sort queries.
    font_sort_cache: FnvHashMap<FontQuery, Vec<Font<'gc>>>,

    /// The implementation names of each default font.
    default_font_names: FnvHashMap<DefaultFont, Vec<String>>,

    /// The cached list of implementations per default font.
    default_font_cache: FnvHashMap<(DefaultFont, bool, bool), Vec<Font<'gc>>>,

    /// A list of the symbols associated with specific AVM2 constructor
    /// prototypes.
    avm2_class_registry: Avm2ClassRegistry<'gc>,

    /// Optional frontend policy for regenerable shape caches. SWF definitions,
    /// script-visible objects and immutable bitmap fills are retained.
    #[collect(require_static)]
    render_cache_idle_limit: Option<Duration>,
    #[collect(require_static)]
    last_render_cache_sweep: Instant,
}

impl<'gc> Library<'gc> {
    pub fn empty() -> Self {
        Self {
            movie_libraries: MovieLibraries::new(),
            sound_definitions: FnvHashMap::default(),
            next_sound_cleanup: 256,
            device_fonts: Default::default(),
            global_fonts: Default::default(),
            font_lookup_cache: Default::default(),
            font_sort_cache: Default::default(),
            default_font_names: Default::default(),
            default_font_cache: Default::default(),
            avm2_class_registry: Default::default(),
            render_cache_idle_limit: None,
            last_render_cache_sweep: Instant::now(),
        }
    }

    pub fn set_render_cache_idle_limit(&mut self, limit: Option<Duration>) {
        self.render_cache_idle_limit = limit;
        self.last_render_cache_sweep = Instant::now();
    }

    /// Called after render command construction. Stale handles may still be held
    /// by commands or the GPU until their submissions finish.
    pub fn sweep_render_caches(&mut self, mc: &Mutation<'gc>) {
        if self.render_cache_idle_limit.is_some() {
            self.sweep_render_caches_at(Instant::now(), mc);
        }
    }

    fn sweep_render_caches_at(&mut self, now: Instant, mc: &Mutation<'gc>) {
        let Some(idle_limit) = self.render_cache_idle_limit else {
            return;
        };
        if now.saturating_duration_since(self.last_render_cache_sweep) < Duration::from_secs(5) {
            return;
        }
        #[cfg(feature = "performance_diagnostics")]
        let _scope = crate::performance_diagnostics::Scope::new("core.render-cache-sweep");
        self.last_render_cache_sweep = now;
        let mut movies = 0usize;
        let mut characters = 0usize;
        let mut meshes = 0usize;
        let mut morph_frames = 0usize;
        let mut bitmap_fills = 0usize;
        for movie in self.known_movies() {
            movies += 1;
            if let Some(library) = self.library_for_movie(movie, mc) {
                for character in library.characters.values() {
                    characters += 1;
                    match character {
                        Character::Graphic(graphic) => {
                            meshes += graphic.expire_render_cache(now, idle_limit);
                        }
                        Character::MorphShape(morph) => {
                            morph_frames += morph.expire_render_cache(now, idle_limit);
                        }
                        Character::Bitmap(bitmap) => {
                            bitmap_fills += usize::from(bitmap.expire_idle_handle(now, idle_limit));
                        }
                        _ => {}
                    }
                }
            }
        }
        if cfg!(feature = "performance_diagnostics")
            || meshes != 0
            || morph_frames != 0
            || bitmap_fills != 0
        {
            tracing::info!(
                "SWF render cache sweep: idle_seconds={} movies={movies} characters={characters} released_meshes={meshes} released_morph_frames={morph_frames} released_bitmap_fills={bitmap_fills}; per-sweep releases, not live resource counts",
                idle_limit.as_secs(),
            );
        }
    }

    pub fn library_for_movie(
        &self,
        movie: Arc<SwfMovie>,
        mc: &Mutation<'gc>,
    ) -> Option<Ref<'gc, MovieLibrary<'gc>>> {
        self.movie_libraries
            .get(&movie, mc)
            .map(MovieDefinition::library)
    }

    pub fn register_sound_definition(
        &mut self,
        sound: SoundHandle,
        definition: MovieDefinition<'gc>,
    ) {
        if self.sound_definitions.len() >= self.next_sound_cleanup {
            self.sound_definitions
                .retain(|_, definition| !definition.0.is_dropped());
            self.next_sound_cleanup = self.sound_definitions.len().saturating_mul(2).max(256);
        }
        self.sound_definitions
            .insert(SoundDefinitionKey(sound), definition.downgrade());
    }

    pub fn definition_for_sound(
        &self,
        sound: SoundHandle,
        mc: &Mutation<'gc>,
    ) -> Option<MovieDefinition<'gc>> {
        self.sound_definitions
            .get(&SoundDefinitionKey(sound))?
            .upgrade(mc)
    }

    pub fn definition_for_movie(
        &mut self,
        movie: Arc<SwfMovie>,
        mc: &Mutation<'gc>,
    ) -> MovieDefinition<'gc> {
        self.movie_libraries.get_or_insert(movie, mc)
    }

    pub fn library_for_movie_mut(
        &mut self,
        movie: Arc<SwfMovie>,
        mc: &Mutation<'gc>,
    ) -> RefMut<'gc, MovieLibrary<'gc>> {
        self.definition_for_movie(movie, mc).library_mut(mc)
    }

    pub fn known_movies(&self) -> impl Iterator<Item = Arc<SwfMovie>> {
        self.movie_libraries.known_movies()
    }

    /// Library references to reconstructible render caches, not unique heap objects.
    /// Imported/shared symbols can be counted in multiple libraries; dynamic drawings,
    /// fonts and nine-slice gradient variants are not included.
    pub(crate) fn render_cache_diagnostic_summary(&self, mc: &Mutation<'gc>) -> String {
        let mut graphics = 0usize;
        let mut graphic_meshes = 0usize;
        let mut unused_graphics = 0usize;
        let mut morphs = 0usize;
        let mut morph_frames = 0usize;
        let mut morph_meshes = 0usize;
        let mut bitmap_fills = 0usize;
        for movie in self.known_movies() {
            if let Some(library) = self.library_for_movie(movie, mc) {
                for character in library.characters.values() {
                    match character {
                        Character::Graphic(graphic) => {
                            graphics += 1;
                            let count = graphic.cached_shape_count();
                            graphic_meshes += count;
                            unused_graphics += usize::from(count == 0);
                        }
                        Character::MorphShape(morph) => {
                            morphs += 1;
                            let (frames, meshes) = morph.cached_frame_stats();
                            morph_frames += frames;
                            morph_meshes += meshes;
                        }
                        Character::Bitmap(bitmap) => {
                            bitmap_fills += usize::from(bitmap.has_cached_handle());
                        }
                        _ => {}
                    }
                }
            }
        }
        format!(
            "library_graphics={graphics} untessellated_graphics={unused_graphics} graphic_mesh_refs={graphic_meshes} morph_symbols={morphs} morph_frames={morph_frames} morph_mesh_refs={morph_meshes} bitmap_fill_refs={bitmap_fills} font_lookup_entries={} font_sort_entries={} default_font_entries={}",
            self.font_lookup_cache.len(),
            self.font_sort_cache.len(),
            self.default_font_cache.len(),
        )
    }

    /// Returns the default Font implementations behind the built in names (ie `_sans`)
    pub fn default_font(
        &mut self,
        name: DefaultFont,
        is_bold: bool,
        is_italic: bool,
        ui: &dyn UiBackend,
        renderer: &mut dyn RenderBackend,
        gc_context: &Mutation<'gc>,
    ) -> Vec<Font<'gc>> {
        // Can't use entry api here as we want to use self for `load_device_font`.
        // Cache the value as this will be looked up a lot, and font lookup by name can be expensive if lots of fonts exist.
        if let Some(cache) = self.default_font_cache.get(&(name, is_bold, is_italic)) {
            return cache.clone();
        }

        let mut result = vec![];
        // First try to find any exactly matching fonts.
        for name in self.default_font_names.entry(name).or_default().clone() {
            let query = FontQuery::new(FontType::Device, name, is_bold, is_italic);
            if let Some(font) = self.get_or_load_exact_device_font(&query, ui, renderer, gc_context)
            {
                result.push(font);
                break; // TODO: Return multiple fonts when it's needed.
            }
        }

        // Nothing found, try a compatible font.
        if result.is_empty() {
            for name in self.default_font_names.entry(name).or_default().clone() {
                let query = FontQuery::new(FontType::Device, name, is_bold, is_italic);
                if let Some(font) = self.device_fonts.find(&query) {
                    result.push(font);
                    break; // TODO: Return multiple fonts when it's needed.
                }
            }
        }

        // Default aliases such as `_sans` need the same glyph fallbacks as
        // explicitly named device fonts. Keep the selected family as the main
        // font, but ask the backend for the rest of its font set as well.
        if let Some(font) = result.first() {
            let fonts = self.get_or_sort_device_fonts(
                font.descriptor().name(),
                font.descriptor().bold(),
                font.descriptor().italic(),
                ui,
                renderer,
                gc_context,
            );
            if !fonts.is_empty() {
                result = fonts;
            }
        }

        self.default_font_cache
            .insert((name, is_bold, is_italic), result.clone());
        result
    }

    /// Returns the device font exactly matching the requested options.
    fn get_or_load_exact_device_font(
        &mut self,
        query: &FontQuery,
        ui: &dyn UiBackend,
        renderer: &mut dyn RenderBackend,
        gc_context: &Mutation<'gc>,
    ) -> Option<Font<'gc>> {
        // If we have the exact matching font already, use that
        // TODO: We should instead ask each font if it matches a given name. Partial matches are allowed, and fonts may have any amount of names.
        if let Some(font) = self.device_fonts.get(query) {
            return Some(font);
        }

        // We don't have this font already. Did we ask for it before?
        let new_request = self.font_lookup_cache.insert(query.clone());
        if new_request {
            // First time asking for this font, see if our backend can provide anything relevant
            ui.load_device_font(query, &mut |definition| {
                self.register_device_font(gc_context, renderer, definition)
            });

            // Check again. A backend may or may not have provided some new fonts,
            // and they may or may not be relevant to the one we're asking for.
            if let Some(font) = self.device_fonts.get(query) {
                return Some(font);
            }

            let name = &query.name;
            let is_bold = query.is_bold;
            let is_italic = query.is_italic;
            warn!("Unknown device font \"{name}\" (bold: {is_bold}, italic: {is_italic})");
        }

        None
    }

    /// Returns the device font compatible with the requested options.
    pub fn get_or_load_device_font(
        &mut self,
        name: &str,
        is_bold: bool,
        is_italic: bool,
        ui: &dyn UiBackend,
        renderer: &mut dyn RenderBackend,
        gc_context: &Mutation<'gc>,
    ) -> Option<Font<'gc>> {
        let query = FontQuery::new(FontType::Device, name.to_owned(), is_bold, is_italic);

        // Try to find an exactly matching font.
        if let Some(font) = self.get_or_load_exact_device_font(&query, ui, renderer, gc_context) {
            return Some(font);
        }

        // Fallback: Try to find an existing font to re-use instead of giving up.
        self.device_fonts.find(&query)
    }

    fn sort_device_fonts(
        &mut self,
        query: &FontQuery,
        ui: &dyn UiBackend,
        renderer: &mut dyn RenderBackend,
        gc_context: &Mutation<'gc>,
    ) -> Vec<Font<'gc>> {
        // First, ask the backend to sort the fonts for us.
        let fonts = ui.sort_device_fonts(query, &mut |definition| {
            self.register_device_font(gc_context, renderer, definition)
        });

        let fonts: Vec<Font<'gc>> = fonts
            .iter()
            .filter_map(|font_query| self.device_fonts.get(font_query))
            .collect();

        if !fonts.is_empty() {
            return fonts;
        }

        // When the backend failed (or doesn't support sorting fonts), fall back
        // to loading one font only without sorting.
        let font = self.get_or_load_device_font(
            &query.name,
            query.is_bold,
            query.is_italic,
            ui,
            renderer,
            gc_context,
        );
        font.map(|font| vec![font]).unwrap_or_default()
    }

    pub fn get_or_sort_device_fonts(
        &mut self,
        name: &str,
        is_bold: bool,
        is_italic: bool,
        ui: &dyn UiBackend,
        renderer: &mut dyn RenderBackend,
        gc_context: &Mutation<'gc>,
    ) -> Vec<Font<'gc>> {
        // TODO We should be able to return a &Vec here, but (1) the borrow
        //   checker is too strict and doesn't allow if branching, and
        //   (2) there's no way to insert a value and get a reference to
        //   it at the same time.
        let query = FontQuery::new(FontType::Device, name.to_owned(), is_bold, is_italic);
        if let Some(fonts) = self.font_sort_cache.get(&query) {
            return fonts.clone();
        }

        let fonts = self.sort_device_fonts(&query, ui, renderer, gc_context);
        self.font_sort_cache.insert(query, fonts.clone());
        fonts
    }

    pub fn set_default_font(&mut self, font: DefaultFont, names: Vec<String>) {
        self.default_font_names.insert(font, names);
        self.default_font_cache.clear();
    }

    pub fn register_device_font(
        &mut self,
        gc_context: &Mutation<'gc>,
        renderer: &mut dyn RenderBackend,
        definition: FontDefinition<'_>,
    ) {
        match definition {
            FontDefinition::SwfTag(tag, encoding) => {
                let font =
                    Font::from_swf_tag(gc_context, renderer, tag, encoding, FontType::Device);
                let name = font.descriptor().name().to_owned();
                let is_bold = font.descriptor().bold();
                let is_italic = font.descriptor().italic();
                tracing::debug!(
                    "Loaded new device font \"{name}\" (bold: {is_bold}, italic: {is_italic}) from swf tag"
                );
                self.device_fonts.register(font);
            }
            FontDefinition::FontFile {
                name,
                is_bold,
                is_italic,
                data,
                index,
            } => {
                let descriptor = FontDescriptor::from_parts(&name, is_bold, is_italic);
                if let Ok(font) =
                    Font::from_font_file(gc_context, descriptor, data, index, FontType::Device)
                {
                    let name = font.descriptor().name().to_owned();
                    tracing::debug!(
                        "Loaded new device font \"{name}\" (bold: {is_bold}, italic: {is_italic}) from file"
                    );
                    self.device_fonts.register(font);
                } else {
                    warn!("Failed to load device font from file");
                }
            }
            FontDefinition::ExternalRenderer {
                name,
                is_bold,
                is_italic,
                font_renderer,
            } => {
                let descriptor = FontDescriptor::from_parts(&name, is_bold, is_italic);
                let font = Font::from_renderer(gc_context, descriptor, font_renderer);
                tracing::debug!(
                    "Loaded new externally rendered font \"{name}\" (bold: {is_bold}, italic: {is_italic})"
                );
                self.device_fonts.register(font);
            }
        }
        self.default_font_cache.clear();
    }

    /// Find a font by its name and parameters.
    pub fn get_embedded_font_by_name(
        &self,
        name: &str,
        font_type: FontType,
        is_bold: bool,
        is_italic: bool,
        movie: Option<Arc<SwfMovie>>,
        mc: &Mutation<'gc>,
    ) -> Option<Font<'gc>> {
        let query = FontQuery::new(font_type, name.to_owned(), is_bold, is_italic);
        if let Some(font) = self.global_fonts.find(&query) {
            return Some(font);
        }
        if let Some(movie) = movie
            && let Some(library) = self.library_for_movie(movie, mc)
        {
            if let Some((_, font)) = library.character_by_export_name(&WString::from_utf8(name)) {
                // Exporting a font seems to override font lookup completely.
                return if let Character::Font(font) = font {
                    Some(font)
                } else {
                    None
                };
            }
            if let Some(font) = library.fonts.find(&query) {
                return Some(font);
            }
        }
        None
    }

    pub fn global_fonts(&self) -> Vec<Font<'gc>> {
        self.global_fonts.all()
    }

    pub fn register_global_font(&mut self, font: Font<'gc>) {
        self.global_fonts.register(font);
    }

    pub fn set_class_symbol(
        &mut self,
        class: Avm2Class<'gc>,
        movie: Arc<SwfMovie>,
        symbol: CharacterId,
        mc: &Mutation<'gc>,
    ) {
        let definition = self.definition_for_movie(movie.clone(), mc);
        self.avm2_class_registry
            .set_class_symbol(class, movie, symbol, definition, mc);
    }

    /// Get the AVM2 class registry.
    pub fn avm2_class_registry(&self) -> &Avm2ClassRegistry<'gc> {
        &self.avm2_class_registry
    }

    /// Mutate the AVM2 class registry.
    pub fn avm2_class_registry_mut(&mut self) -> &mut Avm2ClassRegistry<'gc> {
        &mut self.avm2_class_registry
    }

    /// Evicts cached font resources that haven't been used, across all device
    /// fonts. Meant to be called once per rendered frame.
    pub fn sweep_font_caches(&self) {
        for font in self.device_fonts.iter_all() {
            font.sweep_caches();
        }
    }
}

#[derive(Collect, Default)]
#[collect(no_drop)]
struct FontMap<'gc>(FnvHashMap<FontQuery, Font<'gc>>);

impl<'gc> FontMap<'gc> {
    pub fn register(&mut self, font: Font<'gc>) {
        self.register_with_descriptor(font, font.descriptor());
    }

    pub fn register_with_descriptor(&mut self, font: Font<'gc>, descriptor: &FontDescriptor) {
        self.0
            .entry(FontQuery::from_descriptor(font.font_type(), descriptor))
            .or_insert(font);
    }

    pub fn get(&self, font_query: &FontQuery) -> Option<Font<'gc>> {
        self.0.get(font_query).copied()
    }

    pub fn find(&self, font_query: &FontQuery) -> Option<Font<'gc>> {
        // The order here is specific, and tested in `tests/swfs/fonts/embed_matching/fallback_preferences`

        // Exact match
        if let Some(font) = self.get(font_query) {
            return Some(font);
        }

        let is_italic = font_query.is_italic;
        let is_bold = font_query.is_bold;

        let mut fallback_query = font_query.clone();
        if is_italic ^ is_bold {
            // If one is set (but not both), then try upgrading to bold italic...
            fallback_query.is_bold = true;
            fallback_query.is_italic = true;
            if let Some(font) = self.get(&fallback_query) {
                return Some(font);
            }

            // and then downgrading to regular
            fallback_query.is_bold = false;
            fallback_query.is_italic = false;
            if let Some(font) = self.get(&fallback_query) {
                return Some(font);
            }

            // and then finally whichever one we don't have set
            fallback_query.is_bold = !is_bold;
            fallback_query.is_italic = !is_italic;
            if let Some(font) = self.get(&fallback_query) {
                return Some(font);
            }
        } else {
            // We don't have an exact match and we were either looking for regular or bold-italic

            if is_italic && is_bold {
                // Do we have regular? (unless we already looked for it)
                fallback_query.is_bold = false;
                fallback_query.is_italic = false;
                if let Some(font) = self.get(&fallback_query) {
                    return Some(font);
                }
            }

            // Do we have bold?
            fallback_query.is_bold = true;
            fallback_query.is_italic = false;
            if let Some(font) = self.get(&fallback_query) {
                return Some(font);
            }

            // Do we have italic?
            fallback_query.is_bold = false;
            fallback_query.is_italic = true;
            if let Some(font) = self.get(&fallback_query) {
                return Some(font);
            }

            if !is_bold && !is_italic {
                // Do we have bold italic? (unless we already looked for it)
                fallback_query.is_bold = true;
                fallback_query.is_italic = true;
                if let Some(font) = self.get(&fallback_query) {
                    return Some(font);
                }
            }
        }

        None
    }

    pub fn all(&self) -> Vec<Font<'gc>> {
        self.0.values().copied().collect()
    }

    pub fn iter_all(&self) -> impl Iterator<Item = Font<'gc>> {
        self.0.values().copied()
    }
}
