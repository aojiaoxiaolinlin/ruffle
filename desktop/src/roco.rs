//! Roco's official web login, separate from the Flash player.
//!
//! The WebView keeps ownership of OAuth and cookies. Only the final FlashVars
//! cross the bridge; there is no password form or authorization-code exchange here.

#[cfg(windows)]
mod window;
#[cfg(windows)]
pub use window::LoginWindow;

use crate::player::LaunchOptions;
use ruffle_core::backend::navigator::SocketMode;
use ruffle_core::context::UpdateContext;
use ruffle_core::external::{ExternalInterfaceProvider, Value};
use url::Url;
#[cfg(windows)]
use winit::event_loop::EventLoopProxy;

pub const GAME_URL: &str = "https://res.17roco.qq.com/main.swf";
pub const NIGHT_URL: &str = "https://res.17roco.qq.com/swf/night.swf";
const BASE_URL: &str = "https://res.17roco.qq.com/";
const TICKETS: [&str; 4] = ["angel_uin", "angel_key", "skey", "pskey"];

pub fn is_game_url(url: &Url) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some("res.17roco.qq.com")
        && url.port_or_known_default() == Some(443)
        && url.path() == "/main.swf"
}

/// Deliberately has no Debug implementation: this contains live credentials.
pub struct Session(Vec<(String, String)>);

impl Session {
    pub fn encode(&self) -> String {
        url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(self.0.iter().map(|(key, value)| (key, value)))
            .finish()
    }

    pub fn parse(raw: &str) -> Option<Self> {
        if raw.len() > 128 * 1024 {
            return None;
        }
        let mut parameters: Vec<(String, String)> = Vec::new();
        for (key, value) in url::form_urlencoded::parse(raw.as_bytes()) {
            if TICKETS.contains(&key.as_ref()) {
                if value.trim().is_empty()
                    || value.len() > 16384
                    || value.contains("{{")
                    || value.chars().any(char::is_control)
                {
                    return None;
                }
                if let Some((_, previous)) = parameters.iter().find(|(k, _)| k == key.as_ref()) {
                    // object/embed may repeat fields, but conflicting values are invalid.
                    if previous != value.as_ref() {
                        return None;
                    }
                } else {
                    parameters.push((key.into_owned(), value.into_owned()));
                }
            }
        }
        if parameters.len() != TICKETS.len() {
            return None;
        }
        parameters.push(("config".into(), format!("{BASE_URL}Global.xml")));
        Some(Self(parameters))
    }

    pub fn apply(self, options: &mut LaunchOptions) {
        options.player.parameters = self.0;
        options.player.base = Url::parse(BASE_URL).ok();
        options.player.referer = Url::parse("https://17roco.qq.com/main.html").ok();
        options.player.upgrade_to_https = Some(true);
        options.player.cookie = None;
        options.player.spoof_url = None;
        // Directory responses supply additional game server addresses.
        options.tcp_connections = Some(SocketMode::Allow);
    }
}

pub struct RocoExternalInterface {
    #[cfg(windows)]
    pub event_loop: EventLoopProxy<crate::custom_event::RuffleEvent>,
}

impl ExternalInterfaceProvider for RocoExternalInterface {
    fn call_method(&self, _: &mut UpdateContext<'_>, name: &str, _: &[Value]) -> Value {
        match name {
            "get_platfrom_src" => "2".into(),
            #[cfg(windows)]
            "flashreload" => {
                let _ = self
                    .event_loop
                    .send_event(crate::custom_event::RuffleEvent::GameLaunch(
                        crate::game_sessions::Game::Roco,
                    ));
                Value::Undefined
            }
            "flashInit" | "pgvSendClick" | "OnGameRS" | "PutRSInfo" | "AddFav" | "setHome"
            | "tt" | "tti" | "preload" => Value::Undefined,
            _ => {
                // Never log arguments: the game may pass tickets to JavaScript.
                tracing::debug!("Unhandled Roco ExternalInterface method: {name}");
                Value::Undefined
            }
        }
    }

    fn on_callback_available(&self, _: &str) {}

    fn get_id(&self) -> Option<String> {
        Some("angelMain".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = "angel_uin=123&angel_key=a%2Bb%26c%3D&skey=s&pskey=p";

    #[test]
    fn validates_and_decodes_session_once() {
        let session = Session::parse(VALID).expect("valid fixture");
        assert!(session.0.contains(&("angel_key".into(), "a+b&c=".into())));
        assert!(session.0.contains(&(
            "config".into(),
            "https://res.17roco.qq.com/Global.xml".into()
        )));
        assert!(Session::parse(&format!("{VALID}&angel_key=different")).is_none());
        assert!(Session::parse("angel_uin=123&angel_key=k&skey=s").is_none());
        assert!(Session::parse(&VALID.replace("pskey=p", "pskey=%7B%7Bpskey%7D%7D")).is_none());
        assert!(Session::parse(&VALID.replace("pskey=p", "pskey=%0A")).is_none());
    }

    #[test]
    fn game_detection_does_not_match_other_sites() {
        for raw in [
            "https://res.17roco.qq.com.evil.test/main.swf",
            "https://res.17roco.qq.com:444/main.swf",
            "https://res.17roco.qq.com/other.swf",
        ] {
            assert!(!is_game_url(&Url::parse(raw).expect("fixture URL")));
        }
        assert!(is_game_url(&Url::parse(GAME_URL).expect("game URL")));
    }
}
