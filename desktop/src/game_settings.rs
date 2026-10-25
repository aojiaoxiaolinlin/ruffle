//! Per-game display settings. Persist an explicit allowlist, never launch credentials.
use crate::{game_sessions::Game, player::LaunchOptions};
use ruffle_frontend_utils::{
    parse::{DocumentHolder, ParseContext},
    player_options::{
        PlayerOptions, PlayerOptionsWriter, read_player_options, write_player_options,
    },
};
use std::{
    fs, io,
    path::{Path, PathBuf},
};
use toml_edit::{DocumentMut, value};

#[derive(Default, Clone, Debug, PartialEq)]
pub struct DisplaySettings {
    pub player: PlayerOptions,
    pub soft_filter_scale: Option<u32>,
}

impl DisplaySettings {
    pub fn from_options(options: &LaunchOptions) -> Self {
        Self {
            player: Self::display_only(&options.player),
            soft_filter_scale: options.soft_filter_scale,
        }
    }

    fn display_only(player: &PlayerOptions) -> PlayerOptions {
        PlayerOptions {
            quality: player.quality,
            letterbox: player.letterbox,
            align: player.align,
            force_align: player.force_align,
            scale: player.scale,
            force_scale: player.force_scale,
            frame_rate: player
                .frame_rate
                .filter(|v| v.is_finite() && (1.0..=240.0).contains(v)),
            ..Default::default()
        }
    }

    pub fn apply(&self, options: &mut LaunchOptions) {
        let display = Self::display_only(&self.player);
        options.player.quality = display.quality;
        options.player.letterbox = display.letterbox;
        options.player.align = display.align;
        options.player.force_align = display.force_align;
        options.player.scale = display.scale;
        options.player.force_scale = display.force_scale;
        options.player.frame_rate = display.frame_rate;
        options.soft_filter_scale = self
            .soft_filter_scale
            .filter(|v| matches!(v, 50 | 75 | 100));
    }

    fn path(config: &Path, game: Game) -> PathBuf {
        config.join(format!("{}-display.toml", game.id()))
    }

    fn decode(raw: &str) -> io::Result<Self> {
        let source = raw.parse::<DocumentMut>().map_err(io::Error::other)?;
        let mut display = DocumentMut::new();
        for key in [
            "quality",
            "letterbox",
            "align",
            "force_align",
            "scale_mode",
            "force_scale_mode",
            "frame_rate",
        ] {
            if let Some(item) = source.get(key) {
                display[key] = item.clone();
            }
        }
        let player = read_player_options(&mut ParseContext::default(), display.as_table());
        let soft_filter_scale = source
            .get("soft_filter_scale")
            .and_then(|v| v.as_integer())
            .filter(|v| matches!(v, 50 | 75 | 100))
            .map(|v| v as u32);
        Ok(Self {
            player: Self::display_only(&player),
            soft_filter_scale,
        })
    }

    fn encode(&self) -> String {
        let mut document = DocumentHolder::default();
        write_player_options(
            &mut PlayerOptionsWriter::new(&mut document),
            &Self::display_only(&self.player),
        );
        if let Some(scale) = self
            .soft_filter_scale
            .filter(|v| matches!(v, 50 | 75 | 100))
        {
            document.edit(|_, doc| doc["soft_filter_scale"] = value(i64::from(scale)));
        }
        document.serialize()
    }

    pub fn load(config: &Path, game: Game) -> io::Result<Option<Self>> {
        match fs::read_to_string(Self::path(config, game)) {
            Ok(raw) => Self::decode(&raw).map(Some),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn save(&self, config: &Path, game: Game) -> io::Result<()> {
        fs::create_dir_all(config)?;
        let path = Self::path(config, game);
        let temporary = path.with_extension("toml.tmp");
        fs::write(&temporary, self.encode())?;
        fs::rename(temporary, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruffle_core::{StageAlign, StageScaleMode};
    use ruffle_render::quality::StageQuality;

    #[test]
    fn display_roundtrip_excludes_credentials_and_network_fields() {
        let mut settings = DisplaySettings::default();
        settings.player.quality = Some(StageQuality::Medium);
        settings.player.align = Some(StageAlign::LEFT);
        settings.player.force_align = Some(true);
        settings.player.scale = Some(StageScaleMode::ShowAll);
        settings.player.force_scale = Some(true);
        settings.player.frame_rate = Some(48.0);
        settings
            .player
            .parameters
            .push(("auth_string".into(), "private-ticket".into()));
        settings.player.cookie = Some("private-cookie".into());
        settings.soft_filter_scale = Some(75);
        let raw = settings.encode();
        assert!(!raw.contains("private"));
        assert!(!raw.contains("parameters"));
        let decoded = DisplaySettings::decode(&raw).expect("roundtrip");
        assert_eq!(
            decoded.player,
            DisplaySettings::display_only(&settings.player)
        );
        assert_eq!(decoded.soft_filter_scale, Some(75));
        let injected = DisplaySettings::decode("frame_rate = 0.0\nsoft_filter_scale = 999\nscript_timeout = -1.0\n[parameters]\nauth_string = 'ticket'").expect("ignore unrelated fields");
        assert_eq!(injected, DisplaySettings::default());
    }
    #[test]
    fn each_game_keeps_its_settings_and_existing_files_can_be_replaced() {
        let path = std::env::temp_dir().join(format!(
            "ruffle-display-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let mut kabu = DisplaySettings::default();
        kabu.player.frame_rate = Some(48.0);
        kabu.soft_filter_scale = Some(50);
        kabu.save(&path, Game::Kabu).expect("save Kabu");
        assert!(
            DisplaySettings::load(&path, Game::Roco)
                .expect("missing Roco")
                .is_none()
        );
        let mut roco = DisplaySettings::default();
        roco.player.quality = Some(StageQuality::Medium);
        roco.save(&path, Game::Roco).expect("save Roco");
        assert_eq!(
            DisplaySettings::load(&path, Game::Kabu)
                .expect("load")
                .expect("Kabu"),
            kabu
        );
        assert_eq!(
            DisplaySettings::load(&path, Game::Roco)
                .expect("load")
                .expect("Roco"),
            roco
        );
        DisplaySettings::default()
            .save(&path, Game::Kabu)
            .expect("replace existing settings");
        assert_eq!(
            DisplaySettings::load(&path, Game::Kabu)
                .expect("load")
                .expect("reset"),
            DisplaySettings::default()
        );
        fs::remove_file(DisplaySettings::path(&path, Game::Kabu)).expect("cleanup Kabu");
        fs::remove_file(DisplaySettings::path(&path, Game::Roco)).expect("cleanup Roco");
        fs::remove_dir(path).expect("cleanup empty directory");
    }
}
