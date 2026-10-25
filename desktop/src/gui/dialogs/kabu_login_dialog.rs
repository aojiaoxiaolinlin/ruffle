use crate::custom_event::RuffleEvent;
use crate::kabu::{self, LoginError, Session};
use egui::{Align2, TextEdit, Window};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use winit::event_loop::EventLoopProxy;

struct PendingLogin {
    receiver: oneshot::Receiver<Result<(Session, Option<(String, String)>), LoginError>>,
    task: JoinHandle<()>,
}

impl Drop for PendingLogin {
    fn drop(&mut self) {
        // Closing/replacing the dialog cancels network work and discards tickets.
        self.task.abort();
    }
}

pub struct KabuLoginDialog {
    pub attempt: u64,
    username: String,
    password: String,
    remember_password: bool,
    error: Option<LoginError>,
    pending: Option<PendingLogin>,
    event_loop: EventLoopProxy<RuffleEvent>,
}

impl KabuLoginDialog {
    pub fn new(
        attempt: u64,
        event_loop: EventLoopProxy<RuffleEvent>,
        credentials: Option<(String, String)>,
    ) -> Self {
        let (username, password, remember_password) = match credentials {
            Some((username, password)) => (username, password, true),
            None => (String::new(), String::new(), false),
        };
        Self {
            attempt,
            username,
            password,
            remember_password,
            error: None,
            pending: None,
            event_loop,
        }
    }

    pub fn show(&mut self, context: &egui::Context) -> bool {
        if let Some(pending) = &mut self.pending {
            match pending.receiver.try_recv() {
                Ok(Ok((session, credentials))) => {
                    self.pending = None;
                    self.password.clear();
                    let _ = self.event_loop.send_event(RuffleEvent::KabuLoggedIn(
                        self.attempt,
                        session,
                        credentials.map(|(username, password)| {
                            crate::game_sessions::KabuCredentials { username, password }
                        }),
                    ));
                    return false;
                }
                Ok(Err(error)) => {
                    self.pending = None;
                    self.error = Some(error);
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.pending = None;
                    self.error = Some(LoginError::Network);
                }
                Err(oneshot::error::TryRecvError::Empty) => {}
            }
        }

        let mut keep_open = true;
        let mut cancel = false;
        let mut submit = false;
        let busy = self.pending.is_some();
        Window::new("卡布西游 — 4399账号登录")
            .id(egui::Id::new("kabu_login"))
            .open(&mut keep_open)
            .anchor(Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .collapsible(false)
            .resizable(false)
            .default_width(360.0)
            .show(context, |ui| {
                ui.label("使用4399账号登录卡布西游");
                ui.add_space(8.0);
                ui.add_enabled_ui(!busy, |ui| {
                    ui.label("4399账号");
                    ui.add(TextEdit::singleline(&mut self.username).desired_width(f32::INFINITY));
                    ui.label("密码");
                    let password = ui.add(
                        TextEdit::singleline(&mut self.password)
                            .password(true)
                            .desired_width(f32::INFINITY),
                    );
                    ui.checkbox(&mut self.remember_password, "记住账号和密码");
                    submit = password.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter));
                });
                if let Some(error) = self.error {
                    ui.colored_label(ui.visuals().error_fg_color, error.message());
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if busy {
                        ui.spinner();
                        ui.label("正在登录…");
                    } else {
                        submit |= ui
                            .add_enabled(
                                !self.username.trim().is_empty() && !self.password.is_empty(),
                                egui::Button::new("登录并进入游戏"),
                            )
                            .clicked();
                    }
                    cancel = ui.button("取消").clicked();
                });
                ui.add_space(4.0);
                #[cfg(windows)]
                ui.small(
                    "勾选后，账号密码会由 Windows 加密保存在此用户下。当前快捷登录不支持验证码。",
                );
                #[cfg(not(windows))]
                ui.small("当前平台暂不支持安全保存账号密码。当前快捷登录不支持验证码。");
            });

        if keep_open
            && !cancel
            && submit
            && !busy
            && !self.username.trim().is_empty()
            && !self.password.is_empty()
        {
            self.error = None;
            let username = self.username.trim().to_owned();
            let password = self.password.clone();
            let remember_password = self.remember_password;
            let (sender, receiver) = oneshot::channel();
            let proxy = self.event_loop.clone();
            let task = tokio::spawn(async move {
                let result = kabu::login(&username, &password).await.map(|session| {
                    let credentials = if remember_password {
                        Some((username, password))
                    } else {
                        None
                    };
                    (session, credentials)
                });
                if sender.send(result).is_ok() {
                    // Wake an idle/paused desktop so the dialog consumes its result.
                    let _ = proxy.send_event(RuffleEvent::KabuLoginWake);
                }
            });
            self.pending = Some(PendingLogin { receiver, task });
        }
        keep_open && !cancel
    }
}
