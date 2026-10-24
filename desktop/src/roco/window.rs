use super::Session;
use crate::custom_event::RuffleEvent;
use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};
use url::Url;
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};
use wry::{NewWindowResponse, PermissionResponse, WebView, WebViewBuilder};

const LOGIN_URL: &str = "https://17roco.qq.com/login.html";

// Keep the WebView alive only for login. All OAuth redirects and Set-Cookie
// processing stay inside its browser session, including cross-origin frames.
pub struct LoginWindow {
    webview: WebView,
    window: Window,
    started: Instant,
    timed_out: bool,
}

impl LoginWindow {
    pub fn new(
        event_loop: &ActiveEventLoop,
        proxy: EventLoopProxy<RuffleEvent>,
    ) -> anyhow::Result<Self> {
        let window = event_loop.create_window(
            Window::default_attributes()
                .with_title("洛克王国 — QQ 官方登录（关闭可取消）")
                .with_inner_size(winit::dpi::LogicalSize::new(1000, 680))
                .with_min_inner_size(winit::dpi::LogicalSize::new(640, 480)),
        )?;
        let window_id = window.id();
        let delivered = Rc::new(Cell::new(false));
        let webview = WebViewBuilder::new()
            .with_incognito(true)
            .with_devtools(false)
            .with_url(LOGIN_URL)
            .with_initialization_script(include_str!("login.js"))
            .with_navigation_handler(|raw| allowed_navigation(&raw))
            .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
            .with_permission_handler(|_| PermissionResponse::Deny)
            .with_download_started_handler(|_, _| false)
            .with_ipc_handler(move |request| {
                if delivered.get() || request.body().len() > 128 * 1024 {
                    return;
                }
                // The URI comes from WebView2's WebMessageReceived.Source, not JS.
                if !trusted_source(&request.uri().to_string()) {
                    return;
                }
                let Ok(message) = serde_json::from_str::<serde_json::Value>(request.body()) else {
                    return;
                };
                match message {
                    serde_json::Value::String(raw) => {
                        if let Some(session) = Session::parse(&raw) {
                            delivered.set(true);
                            let _ = proxy.send_event(RuffleEvent::RocoLoggedIn(window_id, session));
                        }
                    }
                    message
                        if message.get("type").and_then(|value| value.as_str())
                            == Some("ruffle-roco-night") =>
                    {
                        delivered.set(true);
                        tracing::info!("roco: night protection page detected");
                        let _ = proxy.send_event(RuffleEvent::RocoNight);
                    }
                    _ => {}
                }
            })
            .build(&window)?;
        Ok(Self {
            webview,
            window,
            started: Instant::now(),
            timed_out: false,
        })
    }

    pub fn id(&self) -> WindowId {
        self.window.id()
    }

    pub fn focus(&self) {
        self.window.focus_window();
    }

    pub fn resize(&self) {
        if let Err(_error) = self.webview.set_bounds(wry::Rect {
            position: wry::dpi::PhysicalPosition::new(0, 0).into(),
            size: self.window.inner_size().into(),
        }) {
            tracing::warn!("Could not resize Roco login WebView");
        }
    }

    pub fn check_timeout(&mut self) {
        if !self.timed_out && self.started.elapsed() >= Duration::from_secs(120) {
            self.timed_out = true;
            self.window
                .set_title("洛克王国 — 若登录未完成，请关闭窗口后重新点击登录");
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        (!self.timed_out).then_some(self.started + Duration::from_secs(120))
    }
}

fn allowed_navigation(raw: &str) -> bool {
    let Ok(url) = Url::parse(raw) else {
        return false;
    };
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && url
            .host_str()
            .is_some_and(|host| host == "qq.com" || host.ends_with(".qq.com"))
        && url.username().is_empty()
        && url.password().is_none()
}

fn trusted_source(raw: &str) -> bool {
    let Ok(url) = Url::parse(raw) else {
        return false;
    };
    allowed_navigation(raw)
        && matches!(
            (url.host_str(), url.path()),
            (
                Some("17roco.qq.com"),
                "/login.html" | "/main.html" | "/logintarget.html"
            ) | (
                Some("web2.17roco.qq.com"),
                "/fcgi-bin/login2" | "/fcgi-bin/login3"
            )
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_official_game_pages_can_deliver_tickets() {
        assert!(trusted_source(LOGIN_URL));
        assert!(trusted_source(
            "https://web2.17roco.qq.com/fcgi-bin/login3?code=test"
        ));
        assert!(trusted_source(
            "https://web2.17roco.qq.com/fcgi-bin/login2?platfrom_src=2"
        ));
        for raw in [
            "http://17roco.qq.com/main.html",
            "https://17roco.qq.com.evil.test/main.html",
            "https://graph.qq.com/main.html",
            "https://17roco.qq.com:444/main.html",
            "https://17roco.qq.com/other.html",
            "https://user@17roco.qq.com/main.html",
        ] {
            assert!(!trusted_source(raw));
        }
        assert!(!allowed_navigation("file:///C:/secret"));
        assert!(!allowed_navigation("javascript:alert(1)"));
    }
}
