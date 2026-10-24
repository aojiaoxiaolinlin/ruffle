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
use ruffle_core::{PlayerBuilder, StageAlign};
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

/// Keep the fixed game stage centered in a wider desktop window. Let the
/// player's view/inverse-view matrices handle both display and mouse input.
pub fn center_stage(builder: PlayerBuilder) -> PlayerBuilder {
    builder.with_align(StageAlign::empty(), true)
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

    #[test]
    fn centered_stage_survives_script_alignment_and_preserves_input_coordinates() {
        use ruffle_core::StageScaleMode;
        use ruffle_core::swf::{self, Point, Twips};

        let header = swf::Header {
            compression: swf::Compression::None,
            version: 10,
            stage_size: swf::Rectangle {
                x_min: Twips::ZERO,
                x_max: Twips::from_pixels(960.0),
                y_min: Twips::ZERO,
                y_max: Twips::from_pixels(560.0),
            },
            frame_rate: swf::Fixed8::from_f32(24.0),
            num_frames: 1,
        };
        let mut swf_bytes = Vec::new();
        swf::write_swf(&header, &[swf::Tag::ShowFrame], &mut swf_bytes).expect("test SWF");
        let movie =
            ruffle_core::tag_utils::SwfMovie::from_data(&swf_bytes, GAME_URL.into(), None, None)
                .expect("test movie");

        let player = center_stage(
            PlayerBuilder::new()
                .with_movie(movie)
                .with_viewport_dimensions(2560, 1380, 1.25)
                .with_align(StageAlign::LEFT | StageAlign::TOP, true),
        )
        .build();
        let mut player = player.lock().expect("test player");
        for (width, height, dpi) in [(2560, 1380, 1.25), (1200, 700, 1.25), (1080, 1920, 1.0)] {
            player.set_viewport_dimensions(ruffle_render::backend::ViewportDimensions {
                width,
                height,
                scale_factor: dpi,
            });
            player.mutate_with_update_context(|context| {
                let stage = context.stage;
                stage.set_movie_size(960, 560);
                stage.set_scale_mode(context, StageScaleMode::ShowAll, false);
                // Reproduce a movie requesting top-left alignment after load.
                stage.set_align(context, StageAlign::LEFT | StageAlign::TOP);
                stage.build_matrices(context);
                assert!(stage.align().is_empty());

                let view = stage.view_matrix();
                assert!((view.a - view.d).abs() < 0.0001, "uniform scale");
                let first = view * Point::from_pixels(0.0, 0.0);
                let last = view * Point::from_pixels(960.0, 560.0);
                assert!(
                    (first.x.to_pixels() - (width as f64 - last.x.to_pixels())).abs() < 0.15,
                    "viewport={width}x{height} first={first:?} last={last:?} view={view:?}"
                );
                assert!((first.y.to_pixels() - (height as f64 - last.y.to_pixels())).abs() < 0.15);

                // The same inverse matrix is used by Player::handle_event.
                // Check edges as well as center to catch any added input offset.
                for (x, y) in [(0.0, 0.0), (480.0, 280.0), (950.0, 550.0)] {
                    let screen = view * Point::from_pixels(x, y);
                    let movie = stage.inverse_view_matrix() * screen;
                    assert!((movie.x.to_pixels() - x).abs() < 0.15);
                    assert!((movie.y.to_pixels() - y).abs() < 0.15);
                }
            });
        }
    }
}
