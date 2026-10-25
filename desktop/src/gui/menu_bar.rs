use crate::custom_event::RuffleEvent;
use crate::game_sessions::Game;
use crate::gui::dialogs::Dialogs;
use crate::gui::text;
use crate::player::LaunchOptions;
use egui::{Button, Key, KeyboardShortcut, Modifiers, Widget};
use ruffle_core::config::Letterbox;
use ruffle_core::{Player, StageScaleMode};
use ruffle_frontend_utils::content::ContentDescriptor;
use ruffle_render::quality::StageQuality;
use unic_langid::LanguageIdentifier;
use winit::event_loop::EventLoopProxy;

pub struct MenuBar {
    event_loop: EventLoopProxy<RuffleEvent>,
    pub currently_opened: Option<(ContentDescriptor, LaunchOptions)>,
}

impl MenuBar {
    const SHORTCUT_FULLSCREEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F11);
    const SHORTCUT_FULLSCREEN_WINDOWS: KeyboardShortcut =
        KeyboardShortcut::new(Modifiers::ALT, Key::Enter);
    const SHORTCUT_OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
    const SHORTCUT_QUIT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Q);

    pub fn new(event_loop: EventLoopProxy<RuffleEvent>) -> Self {
        Self {
            event_loop,
            currently_opened: None,
        }
    }

    pub fn consume_shortcuts(
        &self,
        egui_ctx: &egui::Context,
        _dialogs: &mut Dialogs,
        mut player: Option<&mut Player>,
    ) {
        // TODO(mike): Make some MenuItem struct with shortcut info to handle this more cleanly.
        if egui_ctx.input_mut(|input| input.consume_shortcut(&Self::SHORTCUT_OPEN)) {
            let _ = self.event_loop.send_event(RuffleEvent::CloseFile);
        }
        if egui_ctx.input_mut(|input| input.consume_shortcut(&Self::SHORTCUT_QUIT)) {
            self.request_exit();
        }

        let mut fullscreen_pressed =
            egui_ctx.input_mut(|input| input.consume_shortcut(&Self::SHORTCUT_FULLSCREEN));
        if cfg!(windows) && !fullscreen_pressed {
            // TODO We can remove this shortcut when we add some kind of preferences.
            fullscreen_pressed = egui_ctx
                .input_mut(|input| input.consume_shortcut(&Self::SHORTCUT_FULLSCREEN_WINDOWS));
        }
        if let Some(player) = &mut player
            && fullscreen_pressed
        {
            let is_fullscreen = player.is_fullscreen();
            player.set_fullscreen(!is_fullscreen);
        }
    }

    pub fn show(
        &mut self,
        locale: &LanguageIdentifier,
        egui_ui: &mut egui::Ui,
        dialogs: &mut Dialogs,
        mut player: Option<&mut Player>,
    ) {
        egui::Panel::top("menu_bar").show(egui_ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                self.file_menu(locale, ui, dialogs, player.is_some());
                self.view_menu(locale, ui, &mut player);
                self.controls_menu(locale, ui, dialogs);
                ui.menu_button(text(locale, "help-menu"), |ui| {
                    if ui.button(text(locale, "help-menu-report-a-bug")).clicked() {
                        dialogs.open_feedback();
                        ui.close();
                    }
                    if ui
                        .button(text(locale, "help-menu-sponsor-development"))
                        .clicked()
                    {
                        self.launch_website(ui, "https://opencollective.com/ruffle/");
                    }
                    ui.separator();
                    if ui.button(text(locale, "help-menu-about")).clicked() {
                        dialogs.open_about_screen();
                        ui.close();
                    }
                });
            });
        });
    }

    fn file_menu(
        &mut self,
        locale: &LanguageIdentifier,
        ui: &mut egui::Ui,
        dialogs: &mut Dialogs,
        player_exists: bool,
    ) {
        ui.menu_button("游戏", |ui| {
            if Button::new("返回游戏列表")
                .shortcut_text(ui.ctx().format_shortcut(&Self::SHORTCUT_OPEN))
                .ui(ui)
                .clicked()
            {
                self.close_movie(ui);
            }
            ui.separator();
            for &game in Game::available() {
                if ui.button(game.name()).clicked() {
                    ui.close();
                    let _ = self.event_loop.send_event(RuffleEvent::GameLaunch(game));
                }
            }
            ui.menu_button("切换账号 / 重新登录", |ui| {
                for &game in Game::available() {
                    if ui.button(game.name()).clicked() {
                        ui.close();
                        let _ = self.event_loop.send_event(RuffleEvent::GameRelogin(game));
                    }
                }
            });
            ui.menu_button("游戏设置", |ui| {
                for &game in Game::available() {
                    if ui.button(game.name()).clicked() {
                        dialogs.open_game_settings(game);
                        ui.close();
                    }
                }
            });
            ui.separator();
            if ui
                .add_enabled(player_exists, Button::new("重新进入游戏"))
                .clicked()
            {
                self.reload_movie(ui);
            }
            ui.separator();

            if Button::new(text(locale, "file-menu-preferences"))
                .ui(ui)
                .clicked()
            {
                ui.close();
                dialogs.open_preferences();
            }
            ui.separator();

            if Button::new(text(locale, "file-menu-exit"))
                .shortcut_text(ui.ctx().format_shortcut(&Self::SHORTCUT_QUIT))
                .ui(ui)
                .clicked()
            {
                ui.close();
                self.request_exit();
            }
        });
    }

    fn view_menu(
        &self,
        locale: &LanguageIdentifier,
        ui: &mut egui::Ui,
        player: &mut Option<&mut Player>,
    ) {
        ui.menu_button(text(locale, "view-menu"), |ui| {
            ui.add_enabled_ui(player.is_some(), |ui| {
                ui.menu_button(text(locale, "scale-mode"), |ui| {
                    let items = [
                        (
                            "scale-mode-noscale",
                            "scale-mode-noscale-tooltip",
                            StageScaleMode::NoScale,
                        ),
                        (
                            "scale-mode-showall",
                            "scale-mode-showall-tooltip",
                            StageScaleMode::ShowAll,
                        ),
                        (
                            "scale-mode-exactfit",
                            "scale-mode-exactfit-tooltip",
                            StageScaleMode::ExactFit,
                        ),
                        (
                            "scale-mode-noborder",
                            "scale-mode-noborder-tooltip",
                            StageScaleMode::NoBorder,
                        ),
                    ];
                    let current_scale_mode = player.as_mut().map(|player| player.scale_mode());
                    for (id, tooltip_id, scale_mode) in items {
                        let response = if Some(scale_mode) == current_scale_mode {
                            ui.checkbox(&mut true, text(locale, id))
                        } else {
                            ui.button(text(locale, id))
                        }
                        .on_hover_text_at_pointer(text(locale, tooltip_id));
                        if response.clicked() {
                            ui.close();
                            if let Some(player) = player {
                                player.set_scale_mode(scale_mode);
                            }
                        }
                    }
                    ui.separator();

                    let original_forced_scale_mode = player
                        .as_mut()
                        .map(|player| player.forced_scale_mode())
                        .unwrap_or_default();
                    let mut forced_scale_mode = original_forced_scale_mode;
                    ui.checkbox(&mut forced_scale_mode, text(locale, "scale-mode-force"))
                        .on_hover_text_at_pointer(text(locale, "scale-mode-force-tooltip"));
                    if let Some(player) = player
                        && forced_scale_mode != original_forced_scale_mode
                    {
                        player.set_forced_scale_mode(forced_scale_mode);
                    }
                });

                let original_letterbox = if let Some(player) = player {
                    player.letterbox() == Letterbox::On
                } else {
                    false
                };
                let mut letterbox = original_letterbox;
                ui.checkbox(&mut letterbox, text(locale, "letterbox"));
                if let Some(player) = player
                    && letterbox != original_letterbox
                {
                    player.set_letterbox(if letterbox {
                        Letterbox::On
                    } else {
                        Letterbox::Off
                    });
                }
                ui.separator();

                if Button::new(text(locale, "view-menu-fullscreen"))
                    .shortcut_text(ui.ctx().format_shortcut(&Self::SHORTCUT_FULLSCREEN))
                    .ui(ui)
                    .clicked()
                {
                    ui.close();
                    if let Some(player) = player {
                        player.set_fullscreen(true);
                    }
                }
                ui.separator();

                ui.menu_button(text(locale, "quality"), |ui| {
                    let items = [
                        ("quality-low", StageQuality::Low),
                        ("quality-medium", StageQuality::Medium),
                        ("quality-high", StageQuality::High),
                        ("quality-best", StageQuality::Best),
                        ("quality-high8x8", StageQuality::High8x8),
                        ("quality-high8x8linear", StageQuality::High8x8Linear),
                        ("quality-high16x16", StageQuality::High16x16),
                        ("quality-high16x16linear", StageQuality::High16x16Linear),
                    ];
                    let current_quality = player.as_mut().map(|player| player.quality());
                    for (id, quality) in items {
                        let clicked = if Some(quality) == current_quality {
                            ui.checkbox(&mut true, text(locale, id)).clicked()
                        } else {
                            ui.button(text(locale, id)).clicked()
                        };
                        if clicked {
                            ui.close();
                            if let Some(player) = player {
                                player.set_quality(quality);
                            }
                        }
                    }
                });
            });
        });
    }

    fn controls_menu(&self, locale: &LanguageIdentifier, ui: &mut egui::Ui, dialogs: &mut Dialogs) {
        ui.menu_button(text(locale, "controls-menu"), |ui| {
            if Button::new(text(locale, "controls-menu-volume"))
                .ui(ui)
                .clicked()
            {
                dialogs.open_volume_controls();
                ui.close();
            }
        });
    }

    fn close_movie(&mut self, ui: &egui::Ui) {
        let _ = self.event_loop.send_event(RuffleEvent::CloseFile);
        self.currently_opened = None;
        ui.close();
    }

    fn reload_movie(&mut self, ui: &egui::Ui) {
        if let Some(game) = self
            .currently_opened
            .as_ref()
            .and_then(|(content, _)| Game::from_url(&content.url))
        {
            let _ = self.event_loop.send_event(RuffleEvent::GameLaunch(game));
        }
        ui.close();
    }

    fn request_exit(&self) {
        let _ = self.event_loop.send_event(RuffleEvent::ExitRequested);
    }

    fn launch_website(&self, ui: &egui::Ui, url: &str) {
        let _ = webbrowser::open(url);
        ui.close();
    }
}
