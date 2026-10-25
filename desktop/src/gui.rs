mod context_menu;
mod controller;
pub mod dialogs;
mod locale;
mod menu_bar;
mod movie;
mod picker;
mod theme;
mod widgets;

pub use controller::GuiController;
pub use dialogs::DialogDescriptor;
pub use locale::LocalizableText;
pub use locale::available_languages;
pub use locale::optional_text;
pub use locale::text;
pub use locale::text_with_args;
pub use movie::MovieView;
pub use picker::FilePicker;
use ruffle_frontend_utils::content::ContentDescriptor;
pub use theme::ThemePreference;

use crate::custom_event::RuffleEvent;
use crate::gui::context_menu::ContextMenu;
use crate::player::LaunchOptions;
use crate::preferences::GlobalPreferences;
use dialogs::Dialogs;
use egui::*;
use menu_bar::MenuBar;
use ruffle_core::{Player, PlayerEvent};
use std::mem;
use std::sync::{MutexGuard, Weak};
use winit::event_loop::EventLoopProxy;

/// Size of the top menu bar in pixels.
/// This is the offset at which the movie will be shown,
/// and added to the window size if trying to match a movie.
pub const MENU_HEIGHT: u32 = 24;

/// The main controller for the Ruffle GUI.
pub struct RuffleGui {
    event_loop: EventLoopProxy<RuffleEvent>,
    context_menu: Option<ContextMenu>,
    dialogs: Dialogs,
    menu_bar: MenuBar,

    preferences: GlobalPreferences,
}

impl RuffleGui {
    fn new(
        window: Weak<winit::window::Window>,
        event_loop: EventLoopProxy<RuffleEvent>,
        default_content: Option<ContentDescriptor>,
        default_launch_options: LaunchOptions,
        preferences: GlobalPreferences,
    ) -> Self {
        Self {
            context_menu: None,
            dialogs: Dialogs::new(
                preferences.clone(),
                default_launch_options.clone(),
                default_content,
                window,
                event_loop.clone(),
            ),
            menu_bar: MenuBar::new(event_loop.clone()),

            event_loop,
            preferences,
        }
    }

    /// Renders all of the main Ruffle UI, including the main menu and context menus.
    fn update(
        &mut self,
        egui_ctx: &egui::Context,
        egui_ui: &mut egui::Ui,
        show_menu: bool,
        mut player: Option<&mut Player>,
        _menu_height_offset: f64,
    ) {
        let locale = self.preferences.language();

        self.menu_bar
            .consume_shortcuts(egui_ctx, &mut self.dialogs, player.as_deref_mut());
        if show_menu {
            self.menu_bar
                .show(&locale, egui_ui, &mut self.dialogs, player.as_deref_mut());
        }

        if player.is_none() {
            egui::CentralPanel::default().show(egui_ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(40.0);
                        ui.heading("游戏列表");
                        ui.add_space(8.0);
                        ui.label("选择游戏，开始游玩");
                        ui.add_space(28.0);
                        for &game in crate::game_sessions::Game::available() {
                            egui::Frame::group(ui.style()).show(ui, |ui| {
                                ui.set_min_width(280.0);
                                ui.vertical_centered(|ui| {
                                    ui.heading(game.name());
                                    ui.label(game.login_description());
                                    ui.add_space(12.0);
                                    if ui
                                        .add_sized([240.0, 40.0], egui::Button::new("进入游戏"))
                                        .clicked()
                                    {
                                        let _ = self
                                            .event_loop
                                            .send_event(RuffleEvent::GameLaunch(game));
                                    }
                                    if ui.small_button("游戏设置").clicked() {
                                        self.dialogs.open_game_settings(game);
                                    }
                                    if ui.small_button("切换账号 / 重新登录").clicked() {
                                        let _ = self
                                            .event_loop
                                            .send_event(RuffleEvent::GameRelogin(game));
                                    }
                                });
                            });
                            ui.add_space(16.0);
                        }
                        ui.label("自动沿用已保存的登录状态；失效或切换账号时重新登录。");
                    });
                });
            });
        }

        self.dialogs.show(&locale, egui_ctx, player.as_deref_mut());

        if let Some(player) = player {
            if let Some(context_menu) = &mut self.context_menu
                && !context_menu.show(&locale, egui_ctx, &self.event_loop, player.is_fullscreen())
            {
                self.close_context_menu(player);
            }
        };
    }

    pub fn show_context_menu(
        &mut self,
        menu: Vec<ruffle_core::ContextMenuItem>,
        close_event: PlayerEvent,
    ) {
        if !menu.is_empty() {
            self.context_menu = Some(ContextMenu::new(menu, close_event));
        }
    }

    pub fn close_context_menu(&mut self, player: &mut Player) {
        if let Some(context_menu) = mem::take(&mut self.context_menu) {
            player.handle_event(context_menu.close_event());
        }
    }

    pub fn is_context_menu_visible(&self) -> bool {
        self.context_menu.is_some()
    }

    /// Notifies the GUI that the player has been destroyed.
    fn on_player_destroyed(&mut self) {
        self.dialogs.close_dialogs_with_notifiers();
    }

    /// Notifies the GUI that a new player was created.
    fn on_player_created(
        &mut self,
        opt: LaunchOptions,
        content_descriptor: ContentDescriptor,
        mut player: MutexGuard<Player>,
    ) {
        self.menu_bar.currently_opened = Some((content_descriptor.clone(), opt.clone()));

        // Update dialog state to reflect the newly-opened movie's options.
        self.dialogs
            .recreate_open_dialog(opt, Some(content_descriptor), self.event_loop.clone());

        player.set_volume(self.dialogs.volume_controls.get_volume());
    }
}
