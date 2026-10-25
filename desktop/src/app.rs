use crate::custom_event::{OpenType, RuffleEvent};
use crate::game_sessions::{Game, Session, SessionStore};
use crate::gui::{GuiController, MENU_HEIGHT};
use crate::player::{LaunchOptions, PlayerController};
use crate::preferences::GlobalPreferences;
use crate::util::{
    get_screen_size, gilrs_button_to_gamepad_button, winit_input_to_ruffle_key_descriptor,
    winit_to_ruffle_text_control,
};
use anyhow::Error;
use gilrs::{Event, EventType, Gilrs};
use ruffle_core::FloatDuration;
use ruffle_core::PlayerEvent;
use ruffle_core::events::{ImeEvent, ImeNotification, PlayerNotification};
use ruffle_core::swf::HeaderExt;
use ruffle_frontend_utils::content::ContentDescriptor;
use ruffle_render::backend::ViewportDimensions;
use std::sync::Arc;
#[cfg(windows)]
use std::time::Duration;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize, Size};
use winit::event::{ElementState, Ime, KeyEvent, Modifiers, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Fullscreen, Icon, WindowAttributes, WindowId};

struct MainWindow {
    preferences: GlobalPreferences,
    gui: GuiController,
    player: PlayerController,
    minimized: bool,
    /// Set from `WindowEvent::Occluded`, which winit only reports on macOS and X11
    /// (and on X11 only without a compositor). Always false elsewhere.
    occluded: bool,
    mouse_pos: PhysicalPosition<f64>,
    modifiers: Modifiers,
    min_window_size: LogicalSize<u32>,
    max_window_size: PhysicalSize<u32>,
    no_gui: bool,
    preferred_width: Option<f64>,
    preferred_height: Option<f64>,
    start_fullscreen: bool,
    loaded: LoadingState,
    time: Instant,
    next_frame_time: Option<Instant>,
    #[cfg(windows)]
    fair_frame_deadline: crate::frame_deadline::FrameDeadline,
    performance: crate::performance::DesktopPerformance,
    event_loop_proxy: EventLoopProxy<RuffleEvent>,
}

impl MainWindow {
    fn render_frame(&mut self) {
        if self.minimized || self.occluded {
            return;
        }
        #[cfg(feature = "performance_diagnostics")]
        ruffle_core::performance_diagnostics::gap("desktop.redraw-gap");
        let mut player = self.player.get();
        if let Some(ref mut player) = player {
            // Debug tools may change the render output even while paused.
            let render_start = Instant::now();
            {
                let _scope = crate::performance::phase("desktop.movie-render");
                player.render();
            }
            self.performance.render(render_start.elapsed());
        }
        let gui_start = Instant::now();
        self.gui.render(player);
        self.performance.gui(gui_start.elapsed());
        self.performance.report_if_due();
    }

    pub fn window_event(&mut self, event_loop: &ActiveEventLoop, event: WindowEvent) {
        if matches!(event, WindowEvent::RedrawRequested) {
            // Don't render when nobody can see the result: while minimized, the
            // surface also keeps the size it had before, as `GuiController::resize`
            // refuses to reconfigure it to a zero size.
            self.render_frame();

            // Important that we return here, or we'll get a feedback loop with egui
            // (winit says redraw, egui hears redraw and says redraw, we hear redraw and tell winit to redraw...)
            return;
        }

        let _event_scope = crate::performance::phase("desktop.window-event");
        #[cfg(feature = "performance_diagnostics")]
        let _button_scope = match &event {
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                ..
            } => Some(crate::performance::phase("desktop.mouse-down-received")),
            WindowEvent::MouseInput {
                state: ElementState::Released,
                ..
            } => Some(crate::performance::phase("desktop.mouse-up-received")),
            _ => None,
        };
        if self.gui.handle_event(&event) {
            #[cfg(feature = "performance_diagnostics")]
            if matches!(&event, WindowEvent::MouseInput { .. }) {
                ruffle_core::performance_diagnostics::record(
                    "desktop.mouse-button-ui-consumed",
                    std::time::Duration::ZERO,
                );
            }
            // Event consumed by GUI.
            return;
        }
        match event {
            WindowEvent::CloseRequested => {
                self.performance.report();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                // How minimizing shows up on Windows; winit still has no
                // `WindowEvent::Minimize`. (`Window::is_minimized` does exist by
                // now, but it has to be polled, and it wouldn't tell us anything
                // new: macOS reports minimized windows as occluded instead, and
                // on Wayland it always returns `None`.)
                self.minimized = size.width == 0 && size.height == 0;

                if let Some(mut player) = self.player.get() {
                    let viewport_scale_factor = self.gui.window().scale_factor();
                    player.set_viewport_dimensions(ViewportDimensions {
                        width: size.width,
                        height: size.height.saturating_sub(self.gui.height_offset() as u32),
                        scale_factor: viewport_scale_factor,
                    });
                }
                self.gui.window().request_redraw();
                if matches!(self.loaded, LoadingState::WaitingForResize) {
                    self.loaded = LoadingState::Loaded;
                }
            }
            WindowEvent::Occluded(occluded) => {
                self.occluded = occluded;
                if !occluded {
                    // Nothing repaints us on its own if the movie is paused.
                    self.gui.window().request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if self.gui.is_context_menu_visible() {
                    return;
                }

                // This is needed because some platforms (like macOS)
                // will fire this event at the same time as WindowEvent::MouseInput,
                // which may cause inaccurate behavior in the movie
                if position == self.mouse_pos {
                    return;
                }

                self.mouse_pos = position;
                let _scope = crate::performance::phase("desktop.mouse-move-game");
                let (x, y) = self.gui.window_to_movie_position(position);
                let event = PlayerEvent::MouseMove { x, y };
                self.player.handle_event(event);
                self.check_redraw();
            }
            // The desktop UI launches only games from the supported catalog.
            WindowEvent::DroppedFile(_) => {}
            WindowEvent::Focused(true) => {
                self.player.handle_event(PlayerEvent::FocusGained);
            }
            WindowEvent::Focused(false) => {
                self.player.handle_event(PlayerEvent::FocusLost);
            }
            WindowEvent::MouseInput { button, state, .. } => {
                if self.gui.is_context_menu_visible() {
                    return;
                }

                use ruffle_core::events::MouseButton as RuffleMouseButton;
                use winit::event::MouseButton;
                let (x, y) = self.gui.window_to_movie_position(self.mouse_pos);
                let button = match button {
                    MouseButton::Left => RuffleMouseButton::Left,
                    MouseButton::Right => RuffleMouseButton::Right,
                    MouseButton::Middle => RuffleMouseButton::Middle,
                    _ => RuffleMouseButton::Unknown,
                };
                let event = match state {
                    // TODO We should get information about click index from the OS,
                    //   but winit does not support that yet.
                    ElementState::Pressed => PlayerEvent::MouseDown {
                        x,
                        y,
                        button,
                        index: None,
                    },
                    ElementState::Released => PlayerEvent::MouseUp { x, y, button },
                };
                let _scope = crate::performance::phase(match state {
                    ElementState::Pressed => "desktop.mouse-down-game",
                    ElementState::Released => "desktop.mouse-up-game",
                });
                let handled = self.player.handle_event(event);
                if !handled && state == ElementState::Pressed && button == RuffleMouseButton::Right
                {
                    // Show context menu.
                    if let Some(mut player) = self.player.get() {
                        let context_menu = player.prepare_context_menu();

                        // MouseUp event will be ignored when the context menu is shown,
                        // but it has to be dispatched when the menu closes.
                        let close_event = PlayerEvent::MouseUp {
                            x,
                            y,
                            button: RuffleMouseButton::Right,
                        };
                        self.gui.show_context_menu(context_menu, close_event);
                    }
                }
                self.check_redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if self.gui.is_context_menu_visible() {
                    return;
                }

                use ruffle_core::events::MouseWheelDelta;
                use winit::event::MouseScrollDelta;
                let delta = match delta {
                    MouseScrollDelta::LineDelta(_, dy) => MouseWheelDelta::Lines(dy.into()),
                    MouseScrollDelta::PixelDelta(pos) => MouseWheelDelta::Pixels(pos.y),
                };
                let event = PlayerEvent::MouseWheel { delta };
                self.player.handle_event(event);
                self.check_redraw();
            }
            WindowEvent::CursorEntered { .. } => {
                if let Some(mut player) = self.player.get() {
                    player.set_mouse_in_stage(true);
                    if player.needs_render() {
                        self.gui.window().request_redraw();
                    }
                }
            }
            WindowEvent::CursorLeft { .. } => {
                if let Some(mut player) = self.player.get() {
                    player.set_mouse_in_stage(false);
                }
                self.player.handle_event(PlayerEvent::MouseLeave);
                self.check_redraw();
            }
            WindowEvent::ModifiersChanged(new_modifiers) => {
                self.modifiers = new_modifiers;
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if self.gui.is_context_menu_visible() {
                    return;
                }

                // Handle escaping from fullscreen.
                if let KeyEvent {
                    state: ElementState::Pressed,
                    logical_key: Key::Named(NamedKey::Escape),
                    ..
                } = event
                {
                    let _ = self
                        .event_loop_proxy
                        .send_event(RuffleEvent::ExitFullScreen);
                }

                let key = winit_input_to_ruffle_key_descriptor(&event);
                match event.state {
                    ElementState::Pressed => {
                        self.player.handle_event(PlayerEvent::KeyDown { key });
                        if let Some(control_code) =
                            winit_to_ruffle_text_control(&event, self.modifiers)
                        {
                            self.player
                                .handle_event(PlayerEvent::TextControl { code: control_code });
                        } else if let Some(text) = event.text {
                            for codepoint in text.chars() {
                                self.player
                                    .handle_event(PlayerEvent::TextInput { codepoint });
                            }
                        }
                    }
                    ElementState::Released => {
                        self.player.handle_event(PlayerEvent::KeyUp { key });
                    }
                };
                self.check_redraw();
            }
            WindowEvent::Ime(ime) => match ime {
                Ime::Enabled => {}
                Ime::Preedit(text, cursor) => {
                    self.player
                        .handle_event(PlayerEvent::Ime(ImeEvent::Preedit(text, cursor)));
                }
                Ime::Commit(text) => {
                    self.player
                        .handle_event(PlayerEvent::Ime(ImeEvent::Commit(text)));
                }
                Ime::Disabled => {}
            },
            _ => (),
        }
    }

    fn on_metadata(&mut self, swf_header: HeaderExt) {
        let height_offset = if self.gui.window().fullscreen().is_some() || self.no_gui {
            0.0
        } else {
            MENU_HEIGHT as f64
        };

        // To prevent issues like waiting on resize indefinitely (#11364) or desyncing the window state on Windows,
        // do not resize while window is maximized.
        let should_resize = !self.gui.window().is_maximized();

        let (viewport_size, state) = if should_resize {
            let movie_width = swf_header.stage_size().width().to_pixels();
            let movie_height = swf_header.stage_size().height().to_pixels();

            let window_size: Size = match (self.preferred_width, self.preferred_height) {
                (None, None) => LogicalSize::new(movie_width, movie_height + height_offset).into(),
                (Some(width), None) => {
                    let scale = width / movie_width;
                    let height = movie_height * scale;
                    PhysicalSize::new(
                        width.max(1.0),
                        height.max(1.0) + height_offset * self.gui.window().scale_factor(),
                    )
                    .into()
                }
                (None, Some(height)) => {
                    let scale = height / movie_height;
                    let width = movie_width * scale;
                    PhysicalSize::new(
                        width.max(1.0),
                        height.max(1.0) + height_offset * self.gui.window().scale_factor(),
                    )
                    .into()
                }
                (Some(width), Some(height)) => PhysicalSize::new(
                    width.max(1.0),
                    height.max(1.0) + height_offset * self.gui.window().scale_factor(),
                )
                .into(),
            };

            let window_size = Size::clamp(
                window_size,
                self.min_window_size.into(),
                self.max_window_size.into(),
                self.gui.window().scale_factor(),
            );

            let viewport_size = self.gui.window().inner_size();
            let mut window_resize_denied = false;

            if let Some(new_viewport_size) = self.gui.window().request_inner_size(window_size) {
                if new_viewport_size != viewport_size {
                    self.gui.resize(new_viewport_size);
                } else {
                    tracing::warn!("Unable to resize window");
                    window_resize_denied = true;
                }
            }

            let viewport_size = self.gui.window().inner_size();

            // On X11 (and possibly other platforms), the window size is not updated immediately.
            // On a successful resize request, wait for the window to be resized to the requested size
            // before we start running the SWF (which can observe the viewport size in "noScale" mode)
            let state = if !window_resize_denied && window_size != viewport_size.into() {
                LoadingState::WaitingForResize
            } else {
                LoadingState::Loaded
            };

            (viewport_size, state)
        } else {
            (self.gui.window().inner_size(), LoadingState::Loaded)
        };

        self.loaded = state;

        self.gui.window().set_fullscreen(if self.start_fullscreen {
            Some(Fullscreen::Borderless(None))
        } else {
            None
        });
        self.gui.window().set_visible(true);

        let viewport_scale_factor = self.gui.window().scale_factor();
        if let Some(mut player) = self.player.get() {
            player.set_viewport_dimensions(ViewportDimensions {
                width: viewport_size.width,
                height: viewport_size.height - (height_offset * viewport_scale_factor) as u32,
                scale_factor: viewport_scale_factor,
            });
        }
    }

    fn about_to_wait(&mut self, gilrs: Option<&mut Gilrs>) {
        let _scope = crate::performance::phase("desktop.frame-loop");
        let gamepad_event = {
            let _scope = crate::performance::phase("desktop.gamepad-poll");
            gilrs.and_then(|gilrs| gilrs.next_event())
        };
        if let Some(Event { event, .. }) = gamepad_event {
            match event {
                EventType::ButtonPressed(button, _) => {
                    if let Some(button) = gilrs_button_to_gamepad_button(button) {
                        self.player
                            .handle_event(PlayerEvent::GamepadButtonDown { button });
                        self.check_redraw();
                    }
                }
                EventType::ButtonReleased(button, _) => {
                    if let Some(button) = gilrs_button_to_gamepad_button(button) {
                        self.player
                            .handle_event(PlayerEvent::GamepadButtonUp { button });
                        self.check_redraw();
                    }
                }
                _ => {}
            }
        }

        // Core loop
        // [NA] This used to be called `MainEventsCleared`, but I think the behaviour is different now.
        // We should look at changing our tick to happen somewhere else if we see any behavioural problems.
        if self.advance_player() {
            self.check_redraw();
        }
    }

    /// Share the same clock for idle and busy-loop updates, so extra service
    /// opportunities never count elapsed time twice or accelerate the movie.
    fn advance_player(&mut self) -> bool {
        if matches!(self.loaded, LoadingState::Loaded) {
            let new_time = Instant::now();
            let dt = FloatDuration::from_std(new_time.duration_since(self.time));
            if dt.as_millis() > 0.0 {
                self.time = new_time;
                self.next_frame_time = self.player.get().map(|mut player| {
                    let tick_start = Instant::now();
                    player.tick(dt);
                    self.performance.tick(tick_start.elapsed());
                    let _scope = crate::performance::phase("desktop.frame-schedule");
                    new_time + player.time_til_next_frame()
                });
                #[cfg(feature = "performance_diagnostics")]
                self.player.report_memory(false);
                self.performance.report_if_due();
                return true;
            }
        }
        false
    }

    #[cfg(windows)]
    fn service_busy_frame(&mut self) {
        if self.minimized || self.occluded || !matches!(self.loaded, LoadingState::Loaded) {
            self.fair_frame_deadline.reset();
            return;
        }
        let rate = self.player.get().map(|player| player.frame_rate());
        let Some(rate) = rate.filter(|rate| rate.is_finite() && *rate > 0.0) else {
            self.fair_frame_deadline.reset();
            return;
        };
        let interval = Duration::from_secs_f64(1.0 / rate.clamp(1.0, 60.0));
        if !self
            .fair_frame_deadline
            .should_service(Instant::now(), interval)
        {
            return;
        }
        let _scope = crate::performance::phase("desktop.fair-frame-service");
        if self.advance_player() {
            let movie_needs_render = self
                .player
                .get()
                .is_some_and(|player| player.needs_render());
            if movie_needs_render || self.gui.needs_render() {
                // Windows WM_PAINT is low priority. Present here as well as
                // ticking, otherwise a busy input queue can still hide updates.
                self.render_frame();
            }
        }
    }

    fn check_redraw(&self) {
        let _scope = crate::performance::phase("desktop.redraw-check");
        let player = self.player.get();
        if player.map(|p| p.needs_render()).unwrap_or_default() || self.gui.needs_render() {
            self.gui.window().request_redraw();
        }
    }
}

pub struct App {
    kabu_login_attempt: Option<u64>,
    sessions: SessionStore,
    #[cfg(windows)]
    roco_login: Option<crate::roco::LoginWindow>,
    main_window: Option<MainWindow>,
    runtime: Option<tokio::runtime::Runtime>,
    gilrs: Option<Gilrs>,
    event_loop_proxy: EventLoopProxy<RuffleEvent>,
    preferences: GlobalPreferences,
    font_database: fontdb::Database,
}

/// Enters the tokio runtime context.
/// This cannot be a method, as the borrow-checker would complain.
macro_rules! enter_runtime {
    ($this:expr) => {
        let _guard = $this.runtime.as_ref().map(|runtime| runtime.enter());
    };
}

impl App {
    pub fn new(preferences: GlobalPreferences) -> Result<(Self, EventLoop<RuffleEvent>), Error> {
        #[cfg(windows)]
        tracing::info!(
            "Frame scheduling: scope=all-movies; input events remain immediate and ordered"
        );
        let event_loop = EventLoop::with_user_event().build()?;

        let mut font_database = fontdb::Database::default();
        font_database.load_system_fonts();

        let gilrs = Gilrs::new()
            .inspect_err(|err| {
                tracing::warn!("Gamepad support could not be initialized: {err}");
            })
            .ok();
        let event_loop_proxy = event_loop.create_proxy();
        let runtime = tokio::runtime::Runtime::new()?;

        Ok((
            Self {
                kabu_login_attempt: None,
                sessions: SessionStore::new(&preferences.cli.config),
                #[cfg(windows)]
                roco_login: None,
                main_window: None,
                runtime: Some(runtime),
                gilrs,
                event_loop_proxy,
                font_database,
                preferences,
            },
            event_loop,
        ))
    }
    fn launch_game_session(&mut self, session: Session, save: bool) {
        if self.main_window.is_none() {
            return;
        }
        let save_failed = save && self.sessions.save(&session).is_err();
        self.kabu_login_attempt = None;
        let game = session.game();
        let mut options = LaunchOptions::from(&self.preferences);
        session.apply(&mut options);
        match crate::game_settings::DisplaySettings::load(&self.preferences.cli.config, game) {
            Ok(Some(settings)) => settings.apply(&mut options),
            Ok(None) => {}
            Err(_) => {
                tracing::warn!("Could not load game display settings; using defaults");
            }
        }

        #[cfg(windows)]
        {
            self.roco_login = None;
        }
        if let Some(main_window) = &mut self.main_window {
            main_window.performance.report();
            main_window.gui.create_movie(
                &mut main_window.player,
                options,
                ContentDescriptor::new_remote(game.movie_url()),
            );
            main_window.gui.window().focus_window();
        }
        if save_failed {
            tracing::warn!("Could not persist the game session");
            rfd::MessageDialog::new()
                .set_title("保存登录状态失败")
                .set_description(
                    "本次登录已成功，但无法加密保存登录状态。下次进入游戏可能需要重新登录。",
                )
                .set_level(rfd::MessageLevel::Warning)
                .show();
        }
    }

    fn enter_game(&mut self, event_loop: &ActiveEventLoop, game: Game, reauthorize: bool) {
        if self.main_window.is_none() {
            return;
        }
        // A selection from the catalog supersedes another game's pending login.
        if game != Game::Kabu {
            self.kabu_login_attempt = None;
            if let Some(main_window) = &mut self.main_window {
                main_window.gui.cancel_kabu_login();
            }
        }
        #[cfg(windows)]
        if game != Game::Roco {
            self.roco_login = None;
        }
        if reauthorize {
            if self.sessions.clear(game).is_err() {
                rfd::MessageDialog::new()
                    .set_title("无法清除旧登录状态")
                    .set_description("请检查配置目录的写入权限后重试。")
                    .set_level(rfd::MessageLevel::Error)
                    .show();
                return;
            }
            if let Some(main_window) = &mut self.main_window {
                if main_window.gui.current_game() == Some(game) {
                    main_window.gui.forget_game_session(&mut main_window.player);
                }
                if game == Game::Kabu {
                    self.kabu_login_attempt = None;
                    main_window.gui.cancel_kabu_login();
                }
            }
            #[cfg(windows)]
            if game == Game::Roco {
                self.roco_login = None;
            }
        } else {
            match self.sessions.load(game) {
                Ok(Some(session)) => {
                    self.launch_game_session(session, false);
                    return;
                }
                Ok(None) => {}
                Err(_) => {
                    // Corrupt/unreadable local state is not a server/network failure.
                    tracing::warn!("Could not restore the saved game session");
                    let _ = self.sessions.clear(game);
                }
            }
        }
        match game {
            Game::Kabu => {
                if let Some(main_window) = &mut self.main_window {
                    self.kabu_login_attempt = Some(main_window.gui.show_kabu_login(rand::random()));
                }
            }
            #[cfg(windows)]
            Game::Roco => {
                if let Some(login) = &self.roco_login {
                    login.focus();
                } else {
                    match crate::roco::LoginWindow::new(event_loop, self.event_loop_proxy.clone()) {
                        Ok(login) => self.roco_login = Some(login),
                        Err(_) => {
                            tracing::warn!("Could not create Roco login WebView");
                            rfd::MessageDialog::new()
                                .set_title("洛克王国登录")
                                .set_description("无法打开登录窗口。请确认已安装 Microsoft Edge WebView2 Runtime，然后重试。")
                                .set_level(rfd::MessageLevel::Error)
                                .show();
                        }
                    }
                }
            }
            #[cfg(not(windows))]
            Game::Roco => {}
        }
    }
}

impl ApplicationHandler<RuffleEvent> for App {
    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        enter_runtime!(self);
        let _scope = crate::performance::phase("desktop.new-events");
        #[cfg(feature = "performance_diagnostics")]
        {
            ruffle_core::performance_diagnostics::gap("desktop.event-batch-gap");
            let now = Instant::now();
            let wait = match cause {
                StartCause::ResumeTimeReached {
                    start,
                    requested_resume,
                } => Some((start, Some(requested_resume), "desktop.wake-lateness")),
                StartCause::WaitCancelled {
                    start,
                    requested_resume,
                } => Some((start, requested_resume, "desktop.interrupted-wake-lateness")),
                _ => None,
            };
            if let Some((start, requested_resume, phase)) = wait {
                ruffle_core::performance_diagnostics::record(
                    "desktop.wait-elapsed",
                    now.saturating_duration_since(start),
                );
                if let Some(deadline) = requested_resume {
                    ruffle_core::performance_diagnostics::record(
                        "desktop.wait-requested",
                        deadline.saturating_duration_since(start),
                    );
                    ruffle_core::performance_diagnostics::record(
                        phase,
                        now.saturating_duration_since(deadline),
                    );
                }
            }
        }

        if cause == StartCause::Init {
            let movie_url = self.preferences.cli.movie_url.clone();
            let icon_bytes = include_bytes!("../assets/favicon-32.rgba");
            let icon =
                Icon::from_rgba(icon_bytes.to_vec(), 32, 32).expect("App icon should be correct");

            let no_gui = self.preferences.cli.no_gui;
            let min_window_size = if no_gui {
                (16, 16)
            } else {
                (350, MENU_HEIGHT + 16)
            }
            .into();
            let preferred_width = self.preferences.cli.width;
            let preferred_height = self.preferences.cli.height;
            let start_fullscreen = self.preferences.cli.fullscreen;

            #[cfg_attr(not(target_os = "linux"), allow(unused_mut))]
            let mut window_attributes = WindowAttributes::default()
                .with_visible(false)
                .with_title(crate::branding::APP_NAME)
                .with_window_icon(Some(icon))
                .with_min_inner_size(min_window_size);

            if movie_url.is_none() {
                window_attributes = window_attributes.with_inner_size(LogicalSize::new(800, 640));
            }

            #[cfg(target_os = "linux")]
            {
                use winit::platform::startup_notify::{
                    self, EventLoopExtStartupNotify, WindowAttributesExtStartupNotify,
                };
                use winit::platform::wayland::WindowAttributesExtWayland;
                window_attributes = window_attributes.with_name("rs.ruffle.Ruffle", "main");
                if let Some(token) = event_loop.read_token_from_env() {
                    startup_notify::reset_activation_token_env();
                    window_attributes = window_attributes.with_activation_token(token);
                }
            }

            let event_loop_proxy = self.event_loop_proxy.clone();
            let preferences = self.preferences.clone();
            let window = event_loop
                .create_window(window_attributes)
                .expect("Window should be created");
            let max_window_size = get_screen_size(&window);
            window.set_max_inner_size(Some(max_window_size));
            let window = Arc::new(window);
            let font_database = self.font_database.clone();

            let mut gui = GuiController::new(
                window.clone(),
                event_loop_proxy.clone(),
                preferences.clone(),
                &font_database,
                movie_url.clone(),
                no_gui,
            )
            .expect("GUI controller should be created");

            let mut player = PlayerController::new(
                event_loop_proxy.clone(),
                window.clone(),
                gui.descriptors().clone(),
                font_database,
                preferences.clone(),
                gui.file_picker(),
            );

            if let Some(movie_url) = &movie_url {
                gui.create_movie(
                    &mut player,
                    LaunchOptions::from(&preferences),
                    ContentDescriptor {
                        url: movie_url.clone(),
                        root_content_path: None,
                    },
                );
            }

            let mut loaded = LoadingState::Loading;

            if movie_url.is_none() {
                window.set_title(&crate::branding::library_title());
                window.set_visible(true);
                loaded = LoadingState::Loaded;
            }

            self.main_window = Some(MainWindow {
                preferences,
                gui,
                player,
                min_window_size,
                max_window_size,
                no_gui,
                preferred_width,
                preferred_height,
                start_fullscreen,
                loaded,
                minimized: false,
                occluded: false,
                mouse_pos: PhysicalPosition::new(0.0, 0.0),
                modifiers: Modifiers::default(),
                time: Instant::now(),
                next_frame_time: None,
                #[cfg(windows)]
                fair_frame_deadline: Default::default(),
                performance: Default::default(),
                event_loop_proxy,
            });
        }
    }

    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: RuffleEvent) {
        enter_runtime!(self);
        let _scope = crate::performance::phase("desktop.user-event");

        let event = match event {
            RuffleEvent::Open(desc, options)
                if Game::from_url(&desc.url)
                    .is_some_and(|game| !game.has_credentials(&options.player.parameters)) =>
            {
                // Recents/bookmarks contain public movie URLs. Restore their
                // tickets through the same path as the dedicated game entries.
                if let Some(game) = Game::from_url(&desc.url) {
                    self.enter_game(event_loop, game, false);
                }
                return;
            }
            RuffleEvent::CloseFile => {
                self.kabu_login_attempt = None;
                #[cfg(windows)]
                {
                    self.roco_login = None;
                }
                if let Some(main_window) = &mut self.main_window {
                    main_window.gui.cancel_kabu_login();
                    main_window.performance.report();
                    main_window.gui.forget_game_session(&mut main_window.player);
                    main_window.gui.window().set_fullscreen(None);
                    main_window
                        .gui
                        .window()
                        .set_title(&crate::branding::library_title());
                }
                return;
            }
            RuffleEvent::SaveGameSettings(game, settings) => {
                if settings.save(&self.preferences.cli.config, game).is_err() {
                    rfd::MessageDialog::new()
                        .set_title("保存游戏设置失败")
                        .set_description("请检查配置目录的写入权限后重试。本次设置未保存。")
                        .set_level(rfd::MessageLevel::Error)
                        .show();
                }
                return;
            }
            RuffleEvent::GameLaunch(game) => {
                self.enter_game(event_loop, game, false);
                return;
            }
            RuffleEvent::GameRelogin(game) => {
                self.enter_game(event_loop, game, true);
                return;
            }
            RuffleEvent::KabuLoggedIn(attempt, session) => {
                if self.kabu_login_attempt == Some(attempt) {
                    self.launch_game_session(Session::Kabu(session), true);
                }
                return;
            }
            #[cfg(windows)]
            RuffleEvent::RocoNight => {
                // Login succeeded but the game is in its night-time protection
                // window. Show the official notice instead of staying on the
                // Flash-less WebView page.
                tracing::info!("roco: showing night protection notice");
                self.roco_login = None;
                if let Some(main_window) = &mut self.main_window {
                    let mut options = LaunchOptions::from(&self.preferences);
                    options.player.base = url::Url::parse("https://res.17roco.qq.com/").ok();
                    options.player.referer =
                        url::Url::parse("https://17roco.qq.com/main.html").ok();
                    options.player.upgrade_to_https = Some(true);
                    main_window.performance.report();
                    main_window.gui.create_movie(
                        &mut main_window.player,
                        options,
                        ContentDescriptor::new_remote(
                            url::Url::parse(crate::roco::NIGHT_URL)
                                .expect("fixed night notice URL"),
                        ),
                    );
                    main_window.gui.window().focus_window();
                }
                return;
            }
            #[cfg(windows)]
            RuffleEvent::RocoLoggedIn(id, session) => {
                if self
                    .roco_login
                    .as_ref()
                    .is_some_and(|login| login.id() == id)
                {
                    self.launch_game_session(Session::Roco(session), true);
                }
                return;
            }
            event => event,
        };

        match (&mut self.main_window, event) {
            (Some(main_window), RuffleEvent::KabuLoginWake) => {
                main_window.gui.window().request_redraw();
            }

            (Some(main_window), RuffleEvent::TaskPoll(task)) => main_window.player.poll(task),

            (Some(main_window), RuffleEvent::OnMetadata(swf_header)) => {
                main_window.on_metadata(swf_header)
            }

            (Some(main_window), RuffleEvent::ContextMenuItemClicked(index)) => {
                if let Some(mut player) = main_window.player.get() {
                    player.run_context_menu_callback(index);
                }
            }

            (Some(main_window), RuffleEvent::BrowseAndOpen(options, open_type)) => {
                let event_loop = main_window.event_loop_proxy.clone();
                let picker = main_window.gui.file_picker();
                tokio::spawn(async move {
                    let picked = match open_type {
                        OpenType::File => {
                            picker.pick_ruffle_file(None).await.map(|file| (file, None))
                        }
                        OpenType::Directory => picker
                            .pick_ruffle_directory_and_content(None)
                            .await
                            .map(|(dir, file)| (file, Some(dir))),
                    };

                    if let Some(desc) =
                        picked.and_then(|(file, dir)| ContentDescriptor::new_local(&file, dir))
                    {
                        let _ = event_loop.send_event(RuffleEvent::Open(desc, options));
                    }
                });
            }

            (Some(main_window), RuffleEvent::Open(desc, options)) => {
                self.kabu_login_attempt = None;
                #[cfg(windows)]
                {
                    self.roco_login = None;
                }
                main_window.performance.report();
                main_window
                    .gui
                    .create_movie(&mut main_window.player, *options, desc);
            }

            (Some(main_window), RuffleEvent::OpenDialog(descriptor)) => {
                main_window.gui.open_dialog(descriptor);
            }

            (Some(main_window), RuffleEvent::ExportBundle) => {
                main_window.gui.export_bundle();
            }

            (Some(main_window), RuffleEvent::EnterFullScreen) => {
                if let Some(mut player) = main_window.player.get()
                    && player.is_playing()
                {
                    player.set_fullscreen(true);
                }
            }

            (Some(main_window), RuffleEvent::ExitFullScreen) => {
                if let Some(mut player) = main_window.player.get()
                    && player.is_playing()
                {
                    player.set_fullscreen(false);
                }
            }

            (Some(main_window), RuffleEvent::PlayerNotification(notification)) => {
                match notification {
                    PlayerNotification::ImeNotification(ImeNotification::ImeReady {
                        purpose,
                        cursor_area,
                    }) => {
                        let ime_enabled = main_window.preferences.ime_enabled().unwrap_or(false);
                        main_window.gui.set_ime_allowed(ime_enabled);
                        main_window.gui.set_ime_purpose(purpose);
                        main_window.gui.set_ime_cursor_area(cursor_area);
                    }
                    PlayerNotification::ImeNotification(ImeNotification::ImePurposeUpdated(
                        purpose,
                    )) => {
                        main_window.gui.set_ime_purpose(purpose);
                    }
                    PlayerNotification::ImeNotification(ImeNotification::ImeCursorAreaUpdated(
                        cursor_area,
                    )) => {
                        main_window.gui.set_ime_cursor_area(cursor_area);
                    }
                    PlayerNotification::ImeNotification(ImeNotification::ImeNotReady) => {
                        main_window.gui.set_ime_allowed(false);
                    }
                }
            }

            (_, RuffleEvent::ExitRequested) => {
                event_loop.exit();
            }

            _ => {}
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        enter_runtime!(self);
        let _scope = crate::performance::phase("desktop.window-callback");

        #[cfg(windows)]
        if let Some(login) = &self.roco_login
            && login.id() == window_id
        {
            match event {
                WindowEvent::CloseRequested => self.roco_login = None,
                WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => login.resize(),
                _ => {}
            }
            return;
        }

        if let Some(main_window) = &mut self.main_window {
            if main_window.gui.window().id() == window_id {
                #[cfg(windows)]
                let service_frame = matches!(
                    &event,
                    WindowEvent::CursorMoved { .. }
                        | WindowEvent::MouseInput { .. }
                        | WindowEvent::MouseWheel { .. }
                        | WindowEvent::KeyboardInput { .. }
                );
                main_window.window_event(event_loop, event);
                #[cfg(windows)]
                if service_frame {
                    // The original event has finished in full before servicing
                    // a frame. Do not buffer, merge, or reorder input events.
                    main_window.service_busy_frame();
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        enter_runtime!(self);
        let _scope = crate::performance::phase("desktop.about-to-wait");

        #[cfg(windows)]
        if let Some(login) = &mut self.roco_login {
            login.check_timeout();
        }

        if let Some(main_window) = &mut self.main_window {
            main_window.about_to_wait(self.gilrs.as_mut());

            // The event loop is finished; let's find out how long we need to wait for.
            // We don't need to worry about earlier update requests, as it's the
            // only place where we're setting control flow, and events cancel wait.
            // Note: the control flow might be set to `ControlFlow::WaitUntil` with a
            // timestamp in the past! Take that into consideration when changing this code.
            if let Some(next_frame_time) = main_window.next_frame_time {
                event_loop.set_control_flow(ControlFlow::WaitUntil(next_frame_time));
            }
        }

        #[cfg(windows)]
        if let Some(deadline) = self.roco_login.as_ref().and_then(|login| login.deadline()) {
            match event_loop.control_flow() {
                ControlFlow::Wait => event_loop.set_control_flow(ControlFlow::WaitUntil(deadline)),
                ControlFlow::WaitUntil(next) if deadline < next => {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
                }
                _ => {}
            }
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        #[cfg(feature = "performance_diagnostics")]
        if let Some(main_window) = &mut self.main_window {
            main_window.player.report_memory(true);
        }
        #[cfg(feature = "performance_diagnostics")]
        ruffle_core::performance_diagnostics::report(true);
        #[cfg(windows)]
        {
            self.roco_login = None;
        }
        // `MainWindow` needs a tokio context to properly drop.
        {
            enter_runtime!(self);
            let _ = self.main_window.take();
        }

        // Manually stop the tokio runtime: this makes sure that any pending Player-bound futures
        // are properly cancelled and put back on the winit event loop before it closes, preventing
        // them from being dropped on the wrong thread and causing a panic.
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(std::time::Duration::from_secs(1));
        }
    }
}

enum LoadingState {
    Loading,
    WaitingForResize,
    Loaded,
}
