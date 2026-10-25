mod about_dialog;
mod bookmarks_dialog;
pub mod export_bundle_dialog;
pub mod filesystem_access_dialog;
mod kabu_login_dialog;
pub mod message_dialog;
pub mod network_access_dialog;
mod open_dialog;
mod open_url_dialog;
mod preferences_dialog;
pub mod select_path_dialog;
mod volume_controls;

use crate::custom_event::RuffleEvent;
use crate::game_sessions::KabuCredentialStore;
use crate::player::LaunchOptions;
use crate::preferences::GlobalPreferences;
use bookmarks_dialog::{BookmarkAddDialog, BookmarksDialog};
use export_bundle_dialog::{ExportBundleDialog, ExportBundleDialogConfiguration};
use filesystem_access_dialog::{FilesystemAccessDialog, FilesystemAccessDialogConfiguration};
use kabu_login_dialog::KabuLoginDialog;
use message_dialog::{MessageDialog, MessageDialogConfiguration};
use network_access_dialog::{NetworkAccessDialog, NetworkAccessDialogConfiguration};
use open_dialog::OpenDialog;
use open_url_dialog::OpenUrlDialog;
use preferences_dialog::PreferencesDialog;
use ruffle_core::Player;
use ruffle_frontend_utils::content::ContentDescriptor;
use select_path_dialog::{SelectPathDialog, SelectPathDialogConfiguration};
use std::{collections::VecDeque, sync::Weak};
use unic_langid::LanguageIdentifier;
use volume_controls::VolumeControls;
use winit::event_loop::EventLoopProxy;

use super::FilePicker;

pub struct Dialogs {
    event_loop: EventLoopProxy<RuffleEvent>,
    window: Weak<winit::window::Window>,

    picker: FilePicker,
    kabu_login_dialog: Option<KabuLoginDialog>,
    preferences_dialog: Option<PreferencesDialog>,
    bookmarks_dialog: Option<BookmarksDialog>,
    bookmark_add_dialog: Option<BookmarkAddDialog>,
    open_url_dialog: Option<OpenUrlDialog>,
    message_dialog: Option<MessageDialog>,
    export_bundle_dialog: Option<ExportBundleDialog>,
    pick_path_dialog: Option<SelectPathDialog>,

    // Use a queue for the following dialogs in order to:
    //  1. support handling multiple instances of them,
    //  2. prevent new instances popping up over existing ones
    //     (and possibly stealing a click).
    network_access_dialog_queue: VecDeque<NetworkAccessDialog>,
    filesystem_access_dialog: Option<FilesystemAccessDialog>,
    filesystem_access_dialog_queue: VecDeque<FilesystemAccessDialogConfiguration>,

    game_settings_dialog: Option<(crate::game_sessions::Game, OpenDialog)>,
    open_dialog: OpenDialog,
    is_open_dialog_visible: bool,

    pub volume_controls: VolumeControls,
    is_volume_visible: bool,

    is_about_visible: bool,

    preferences: GlobalPreferences,
}

pub enum DialogDescriptor {
    OpenUrl(url::Url),
    ShowMessage(MessageDialogConfiguration),
    NetworkAccess(NetworkAccessDialogConfiguration),
    FilesystemAccess(FilesystemAccessDialogConfiguration),
    ExportBundle(Box<ExportBundleDialogConfiguration>),
    SelectPath(SelectPathDialogConfiguration),
}

impl Dialogs {
    pub fn new(
        preferences: GlobalPreferences,
        player_options: LaunchOptions,
        default_content: Option<ContentDescriptor>,
        window: Weak<winit::window::Window>,
        event_loop: EventLoopProxy<RuffleEvent>,
    ) -> Self {
        let picker = FilePicker::new(window.clone(), preferences.clone(), event_loop.clone());
        Self {
            window,
            kabu_login_dialog: None,
            preferences_dialog: None,
            bookmarks_dialog: None,
            bookmark_add_dialog: None,
            open_url_dialog: None,
            message_dialog: None,
            export_bundle_dialog: None,
            pick_path_dialog: None,

            network_access_dialog_queue: VecDeque::new(),
            filesystem_access_dialog: None,
            filesystem_access_dialog_queue: VecDeque::new(),

            game_settings_dialog: None,
            open_dialog: OpenDialog::new(
                player_options,
                default_content,
                picker.clone(),
                event_loop.clone(),
            ),
            is_open_dialog_visible: false,

            volume_controls: VolumeControls::new(&preferences),
            is_volume_visible: false,

            is_about_visible: false,

            event_loop,
            picker,
            preferences,
        }
    }

    pub fn file_picker(&self) -> FilePicker {
        self.picker.clone()
    }

    /// Close all dialogs that have someone waiting for an answer.
    ///
    /// This method may be used when the original receiver is closed,
    /// e.g. by loading a new movie or destroying the existing one.
    pub fn close_dialogs_with_notifiers(&mut self) {
        self.network_access_dialog_queue.clear();
        self.filesystem_access_dialog = None;
        self.filesystem_access_dialog_queue.clear();
    }

    pub fn recreate_open_dialog(
        &mut self,
        opt: LaunchOptions,
        content_descriptor: Option<ContentDescriptor>,
        event_loop: EventLoopProxy<RuffleEvent>,
    ) {
        self.kabu_login_dialog = None;
        self.is_open_dialog_visible = false;
        self.open_dialog =
            OpenDialog::new(opt, content_descriptor, self.picker.clone(), event_loop);
    }

    pub fn open_game_settings(&mut self, game: crate::game_sessions::Game) {
        let mut options = LaunchOptions::from(&self.preferences);
        match crate::game_settings::DisplaySettings::load(&self.preferences.cli.config, game) {
            Ok(Some(settings)) => settings.apply(&mut options),
            Ok(None) => {}
            Err(_) => {
                rfd::MessageDialog::new()
                    .set_title("无法读取游戏设置")
                    .set_description("游戏显示设置文件无法读取，请检查配置文件后重试。")
                    .set_level(rfd::MessageLevel::Error)
                    .show();
                return;
            }
        }
        self.game_settings_dialog = Some((
            game,
            OpenDialog::new(options, None, self.picker.clone(), self.event_loop.clone()),
        ));
    }

    pub fn open_file_advanced(&mut self) {
        self.is_open_dialog_visible = true;
    }

    pub fn open_kabu_login(&mut self, attempt: u64) -> u64 {
        // Keep an existing attempt when the menu is clicked repeatedly.
        if self.kabu_login_dialog.is_none() {
            let credentials = KabuCredentialStore::new(&self.preferences.cli.config)
                .load()
                .ok()
                .flatten();
            self.kabu_login_dialog = Some(KabuLoginDialog::new(
                attempt,
                self.event_loop.clone(),
                credentials.map(|credentials| (credentials.username, credentials.password)),
            ));
        }
        self.kabu_login_dialog
            .as_ref()
            .expect("login dialog exists")
            .attempt
    }

    pub fn cancel_kabu_login(&mut self) {
        self.kabu_login_dialog = None;
    }

    pub fn open_preferences(&mut self) {
        self.preferences_dialog = Some(PreferencesDialog::new(
            self.preferences.clone(),
            self.window.upgrade(),
        ));
    }

    pub fn open_bookmarks(&mut self) {
        self.bookmarks_dialog = Some(BookmarksDialog::new(
            self.preferences.clone(),
            self.picker.clone(),
            self.event_loop.clone(),
        ));
    }

    pub fn open_add_bookmark(&mut self, content_descriptor: Option<ContentDescriptor>) {
        self.bookmark_add_dialog = Some(BookmarkAddDialog::new(
            self.preferences.clone(),
            content_descriptor,
            self.picker.clone(),
        ))
    }

    pub fn open_volume_controls(&mut self) {
        self.is_volume_visible = true;
    }

    pub fn open_about_screen(&mut self) {
        self.is_about_visible = true;
    }

    pub fn open_feedback(&mut self) {
        self.message_dialog = Some(MessageDialog::new(MessageDialogConfiguration::new(
            super::LocalizableText::LocalizedText("feedback-qq-group-title"),
            super::LocalizableText::LocalizedText("feedback-qq-group-body"),
        )));
    }

    pub fn saved_content_descriptor(&self) -> Option<ContentDescriptor> {
        self.open_dialog.content_descriptor()
    }

    pub fn saved_launch_options(&self) -> &LaunchOptions {
        self.open_dialog.options()
    }

    pub fn open_dialog(&mut self, event: DialogDescriptor) {
        match event {
            DialogDescriptor::OpenUrl(url) => {
                self.open_url_dialog = Some(OpenUrlDialog::new(url));
            }
            DialogDescriptor::ShowMessage(config) => {
                self.message_dialog = Some(MessageDialog::new(config));
            }
            DialogDescriptor::NetworkAccess(config) => self
                .network_access_dialog_queue
                .push_back(NetworkAccessDialog::new(config)),
            DialogDescriptor::FilesystemAccess(config) => {
                self.filesystem_access_dialog_queue.push_back(config)
            }
            DialogDescriptor::ExportBundle(config) => {
                self.export_bundle_dialog =
                    Some(ExportBundleDialog::new(*config, self.file_picker()))
            }
            DialogDescriptor::SelectPath(config) => {
                self.pick_path_dialog = Some(SelectPathDialog::new(config));
            }
        }
    }

    pub fn show(
        &mut self,
        locale: &LanguageIdentifier,
        egui_ctx: &egui::Context,
        player: Option<&mut Player>,
    ) {
        if let Some(dialog) = &mut self.kabu_login_dialog
            && !dialog.show(egui_ctx)
        {
            self.kabu_login_dialog = None;
        }
        if let Some((game, dialog)) = &mut self.game_settings_dialog
            && !dialog.show_game_settings(*game, locale, egui_ctx)
        {
            self.game_settings_dialog = None;
        }
        self.show_open_dialog(locale, egui_ctx);
        self.show_preferences_dialog(locale, egui_ctx);
        self.show_bookmarks_dialog(locale, egui_ctx);
        self.show_bookmark_add_dialog(locale, egui_ctx);
        self.show_volume_controls(locale, egui_ctx, player);
        self.show_about_dialog(locale, egui_ctx);
        self.show_open_url_dialog(locale, egui_ctx);
        self.show_message_dialog(locale, egui_ctx);
        self.show_network_access_dialog(locale, egui_ctx);
        self.show_filesystem_access_dialog(locale, egui_ctx);
        self.show_export_bundle_dialog(locale, egui_ctx);
        self.show_pick_path_dialog(locale, egui_ctx);
    }

    fn show_open_dialog(&mut self, locale: &LanguageIdentifier, egui_ctx: &egui::Context) {
        if self.is_open_dialog_visible {
            let keep_open = self.open_dialog.show(locale, egui_ctx);
            self.is_open_dialog_visible = keep_open;
        }
    }

    fn show_preferences_dialog(&mut self, locale: &LanguageIdentifier, egui_ctx: &egui::Context) {
        let keep_open = if let Some(dialog) = &mut self.preferences_dialog {
            dialog.show(locale, egui_ctx)
        } else {
            true
        };
        if !keep_open {
            self.preferences_dialog = None;
        }
    }

    fn show_bookmarks_dialog(&mut self, locale: &LanguageIdentifier, egui_ctx: &egui::Context) {
        let keep_open = if let Some(dialog) = &mut self.bookmarks_dialog {
            dialog.show(locale, egui_ctx)
        } else {
            true
        };
        if !keep_open {
            self.bookmarks_dialog = None;
        }
    }

    fn show_bookmark_add_dialog(&mut self, locale: &LanguageIdentifier, egui_ctx: &egui::Context) {
        let keep_open = if let Some(dialog) = &mut self.bookmark_add_dialog {
            dialog.show(locale, egui_ctx)
        } else {
            true
        };
        if !keep_open {
            self.bookmark_add_dialog = None;
        }
    }

    fn show_volume_controls(
        &mut self,
        locale: &LanguageIdentifier,
        egui_ctx: &egui::Context,
        player: Option<&mut Player>,
    ) {
        if self.is_volume_visible {
            let keep_open = self
                .volume_controls
                .show(locale, egui_ctx, player, &self.preferences);
            self.is_volume_visible = keep_open;
        }
    }

    fn show_about_dialog(&mut self, locale: &LanguageIdentifier, egui_ctx: &egui::Context) {
        if self.is_about_visible {
            let keep_open = about_dialog::show_about_dialog(locale, egui_ctx);
            self.is_about_visible = keep_open;
        }
    }

    fn show_open_url_dialog(&mut self, locale: &LanguageIdentifier, egui_ctx: &egui::Context) {
        let keep_open = if let Some(dialog) = &mut self.open_url_dialog {
            dialog.show(locale, egui_ctx)
        } else {
            true
        };
        if !keep_open {
            self.open_url_dialog = None;
        }
    }

    fn show_message_dialog(&mut self, locale: &LanguageIdentifier, egui_ctx: &egui::Context) {
        let keep_open = if let Some(dialog) = &mut self.message_dialog {
            dialog.show(locale, egui_ctx)
        } else {
            true
        };
        if !keep_open {
            self.message_dialog = None;
        }
    }

    fn show_network_access_dialog(
        &mut self,
        locale: &LanguageIdentifier,
        egui_ctx: &egui::Context,
    ) {
        let keep_open = if let Some(dialog) = &mut self.network_access_dialog_queue.front_mut() {
            dialog.show(locale, egui_ctx)
        } else {
            true
        };
        if !keep_open {
            self.network_access_dialog_queue.pop_front();
        }
    }

    fn show_filesystem_access_dialog(
        &mut self,
        locale: &LanguageIdentifier,
        egui_ctx: &egui::Context,
    ) {
        let keep_open = if let Some(dialog) = &mut self.filesystem_access_dialog {
            dialog.show(locale, egui_ctx)
        } else {
            true
        };
        if !keep_open {
            self.filesystem_access_dialog = None;
        }

        if self.filesystem_access_dialog.is_none() {
            self.filesystem_access_dialog = self
                .filesystem_access_dialog_queue
                .pop_front()
                .map(FilesystemAccessDialog::new);
        }
    }

    fn show_export_bundle_dialog(&mut self, locale: &LanguageIdentifier, egui_ctx: &egui::Context) {
        let keep_open = if let Some(dialog) = &mut self.export_bundle_dialog {
            dialog.show(locale, egui_ctx)
        } else {
            true
        };
        if !keep_open {
            self.export_bundle_dialog = None;
        }
    }

    fn show_pick_path_dialog(&mut self, locale: &LanguageIdentifier, egui_ctx: &egui::Context) {
        let keep_open = if let Some(dialog) = &mut self.pick_path_dialog {
            dialog.show(locale, egui_ctx)
        } else {
            true
        };
        if !keep_open {
            self.pick_path_dialog = None;
        }
    }
}
