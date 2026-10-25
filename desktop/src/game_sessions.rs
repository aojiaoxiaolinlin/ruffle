//! Persist game tickets and optional credentials for supported game logins.

use crate::{kabu, roco};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use url::Url;

const MAGIC: &[u8] = b"RUFFLE-SESSION-1\0";
const ACCOUNT_MAGIC: &[u8] = b"RUFFLE-ACCOUNT-1\0";
const MAX_STORED_SIZE: u64 = 512 * 1024;
const MAX_CREDENTIAL_SIZE: u64 = 16 * 1024;
const MAX_CREDENTIAL_FILE_SIZE: u64 = MAX_CREDENTIAL_SIZE + 4 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Game {
    Kabu,
    Roco,
}

impl Game {
    pub fn available() -> &'static [Self] {
        #[cfg(windows)]
        {
            &[Self::Kabu, Self::Roco]
        }
        #[cfg(not(windows))]
        {
            &[Self::Kabu]
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Kabu => "卡布西游",
            Self::Roco => "洛克王国",
        }
    }

    pub fn login_description(self) -> &'static str {
        match self {
            Self::Kabu => "4399 账号登录",
            Self::Roco => "QQ 账号登录",
        }
    }

    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::Kabu => "kabu",
            Self::Roco => "roco",
        }
    }

    pub fn from_url(url: &Url) -> Option<Self> {
        if kabu::is_game_url(url) {
            Some(Self::Kabu)
        } else if roco::is_game_url(url) {
            Some(Self::Roco)
        } else {
            None
        }
    }

    pub fn movie_url(self) -> Url {
        Url::parse(match self {
            Self::Kabu => kabu::GAME_URL,
            Self::Roco => roco::GAME_URL,
        })
        .expect("fixed game URL")
    }

    pub fn has_credentials(self, parameters: &[(String, String)]) -> bool {
        match self {
            Self::Kabu => parameters.iter().any(|(key, value)| {
                key == "auth_string" && kabu::Session::from_auth(value).is_ok()
            }),
            Self::Roco => {
                let raw = url::form_urlencoded::Serializer::new(String::new())
                    .extend_pairs(parameters.iter().map(|(key, value)| (key, value)))
                    .finish();
                roco::Session::parse(&raw).is_some()
            }
        }
    }

    /// An explicit navigation back to a known login page is a re-login request.
    /// Connection errors, timeouts and ordinary page links are not expiry signals.
    pub fn is_login_url(self, url: &Url) -> bool {
        if !matches!(url.scheme(), "http" | "https")
            || url.port().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return false;
        }
        match self {
            Self::Kabu => matches!(
                (url.host_str(), url.path()),
                (Some("news.4399.com"), "/login/kbxy.html")
                    | (
                        Some("ptlogin.4399.com" | "ptlogin.3304399.net"),
                        "/ptlogin/loginFrame.do"
                    )
            ),
            Self::Roco => matches!(
                (url.host_str(), url.path()),
                (Some("17roco.qq.com"), "/login.html")
            ),
        }
    }
}

// Deliberately no Debug implementation: both variants contain live tickets.
pub enum Session {
    Kabu(kabu::Session),
    Roco(roco::Session),
}

impl Session {
    pub fn game(&self) -> Game {
        match self {
            Self::Kabu(_) => Game::Kabu,
            Self::Roco(_) => Game::Roco,
        }
    }

    pub fn apply(self, options: &mut crate::player::LaunchOptions) {
        match self {
            Self::Kabu(session) => session.apply(options),
            Self::Roco(session) => session.apply(options),
        }
    }

    fn encode(&self) -> String {
        match self {
            Self::Kabu(session) => session.raw().to_owned(),
            Self::Roco(session) => session.encode(),
        }
    }

    fn decode(game: Game, raw: &str) -> io::Result<Self> {
        match game {
            Game::Kabu => kabu::Session::from_auth(raw).ok().map(Self::Kabu),
            Game::Roco => roco::Session::parse(raw).map(Self::Roco),
        }
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidData))
    }
}

pub struct SessionStore {
    directory: PathBuf,
}

/// Credentials are kept separate from reusable game tickets and encrypted
/// with a purpose-specific DPAPI entropy on Windows.
pub struct KabuCredentialStore {
    directory: PathBuf,
}

/// Deliberately no Debug implementation: contains an account password.
pub struct KabuCredentials {
    pub username: String,
    pub password: String,
}

impl KabuCredentialStore {
    pub fn new(config: &Path) -> Self {
        Self {
            directory: config.join("game-accounts"),
        }
    }

    fn path(&self) -> PathBuf {
        self.directory.join("kabu.bin")
    }

    pub fn load(&self) -> io::Result<Option<KabuCredentials>> {
        let file = match fs::File::open(self.path()) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let mut bytes = Vec::new();
        file.take(MAX_CREDENTIAL_FILE_SIZE + ACCOUNT_MAGIC.len() as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_CREDENTIAL_FILE_SIZE + ACCOUNT_MAGIC.len() as u64
            || !bytes.starts_with(ACCOUNT_MAGIC)
        {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let plain = protect_with_entropy(
            "ruffle.game-account.v1.kabu",
            &bytes[ACCOUNT_MAGIC.len()..],
            false,
        )?;
        if plain.len() < 8 {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let username_len = u32::from_le_bytes(plain[0..4].try_into().unwrap()) as usize;
        let password_len = u32::from_le_bytes(plain[4..8].try_into().unwrap()) as usize;
        if 8usize
            .checked_add(username_len)
            .and_then(|n| n.checked_add(password_len))
            != Some(plain.len())
        {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let username = String::from_utf8(plain[8..8 + username_len].to_vec())
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
        let password = String::from_utf8(plain[8 + username_len..].to_vec())
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
        Ok(Some(KabuCredentials { username, password }))
    }

    pub fn save(&self, credentials: &KabuCredentials) -> io::Result<()> {
        let username = credentials.username.as_bytes();
        let password = credentials.password.as_bytes();
        let username_len = u32::try_from(username.len())
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        let password_len = u32::try_from(password.len())
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        let plain_size = 8usize
            .checked_add(username.len())
            .and_then(|n| n.checked_add(password.len()))
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        if plain_size as u64 > MAX_CREDENTIAL_SIZE {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let mut plain = Vec::with_capacity(plain_size);
        plain.extend_from_slice(&username_len.to_le_bytes());
        plain.extend_from_slice(&password_len.to_le_bytes());
        plain.extend_from_slice(username);
        plain.extend_from_slice(password);
        let encrypted = protect_with_entropy("ruffle.game-account.v1.kabu", &plain, true)?;
        fs::create_dir_all(&self.directory)?;
        let temporary = self
            .directory
            .join(format!("kabu.{}.tmp", rand::random::<u64>()));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(ACCOUNT_MAGIC)?;
            file.write_all(&encrypted)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, self.path())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    pub fn clear(&self) -> io::Result<()> {
        match fs::remove_file(self.path()) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }
}

impl SessionStore {
    pub fn new(config: &Path) -> Self {
        Self {
            directory: config.join("game-sessions"),
        }
    }

    fn path(&self, game: Game) -> PathBuf {
        self.directory.join(format!("{}.bin", game.id()))
    }

    pub fn load(&self, game: Game) -> io::Result<Option<Session>> {
        let file = match fs::File::open(self.path(game)) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let mut bytes = Vec::new();
        file.take(MAX_STORED_SIZE + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_STORED_SIZE || !bytes.starts_with(MAGIC) {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let plain = protect(game, &bytes[MAGIC.len()..], false)?;
        let raw =
            String::from_utf8(plain).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
        Session::decode(game, &raw).map(Some)
    }

    pub fn save(&self, session: &Session) -> io::Result<()> {
        let game = session.game();
        let encrypted = protect(game, session.encode().as_bytes(), true)?;
        fs::create_dir_all(&self.directory)?;
        let temporary = self
            .directory
            .join(format!("{}.{}.tmp", game.id(), rand::random::<u64>()));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(MAGIC)?;
            file.write_all(&encrypted)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, self.path(game))
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    pub fn clear(&self, game: Game) -> io::Result<()> {
        match fs::remove_file(self.path(game)) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }
}

#[cfg(windows)]
fn protect(game: Game, data: &[u8], encrypt: bool) -> io::Result<Vec<u8>> {
    protect_with_entropy(
        &format!("ruffle.game-session.v1.{}", game.id()),
        data,
        encrypt,
    )
}

#[cfg(windows)]
fn protect_with_entropy(entropy: &str, data: &[u8], encrypt: bool) -> io::Result<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };

    let entropy_bytes = entropy.as_bytes();
    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(data.len())
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?,
        pbData: data.as_ptr().cast_mut(),
    };
    let entropy = CRYPT_INTEGER_BLOB {
        cbData: entropy_bytes.len() as u32,
        pbData: entropy_bytes.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: DPAPI only reads the borrowed input/entropy buffers. They outlive
    // the call. Windows allocates output with LocalAlloc; copy then LocalFree it.
    // No LOCAL_MACHINE flag: data is bound to the current Windows user.
    let succeeded = unsafe {
        if encrypt {
            CryptProtectData(
                &input,
                std::ptr::null(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if succeeded == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful DPAPI calls return a readable allocation of cbData
    // bytes. Null with zero length is permitted; LocalFree accepts null too.
    let bytes = unsafe {
        let bytes = if output.cbData == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec()
        };
        LocalFree(output.pbData.cast());
        bytes
    };
    Ok(bytes)
}

#[cfg(not(windows))]
fn protect(_: Game, _: &[u8], _: bool) -> io::Result<Vec<u8>> {
    // Do not silently fall back to plaintext on unsupported platforms.
    Err(io::ErrorKind::Unsupported.into())
}

#[cfg(not(windows))]
fn protect_with_entropy(_: &str, _: &[u8], _: bool) -> io::Result<Vec<u8>> {
    // Do not silently fall back to plaintext on unsupported platforms.
    Err(io::ErrorKind::Unsupported.into())
}

#[cfg(test)]
mod tests;
