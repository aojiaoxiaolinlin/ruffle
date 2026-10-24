//! 4399 Kabu login protocol, shared by the desktop login dialog and its tests.
//! Adapted from the user's kabu-login utility and Android KabuLoginClient.
//! Passwords, cookies and response bodies never go to disk or logs. The session
//! manager can encrypt and persist the final game ticket.

use aes::Aes256;
use base64::Engine;
use cbc::cipher::{BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
use md5::{Digest, Md5};
use reqwest::header::{CONTENT_TYPE, LOCATION, ORIGIN, REFERER};
use reqwest::{Client, StatusCode};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use url::Url;

use crate::player::LaunchOptions;
use ruffle_core::backend::navigator::SocketMode;

pub const GAME_URL: &str = "http://enter.wanwan4399.com/bin-debug/GreenGame.swf";
const BASE_URL: &str = "http://enter.wanwan4399.com/bin-debug/";
const GAME_PAGE: &str = "http://enter.wanwan4399.com/bin-debug/KBgameindex.html";
const APP_ID: &str = "kabu_xy";
const BIZ_ID: &str = "1202000651";
const HOSTS: [&str; 2] = ["http://ptlogin.4399.com", "http://ptlogin.3304399.net"];
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
const MAX_BODY: usize = 2 * 1024 * 1024;

// Error variants intentionally contain no transport errors or server content.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LoginError {
    EmptyCredentials,
    Unavailable,
    Network,
    LoginRejected,
    InvalidSession,
    UnexpectedResponse,
}

impl LoginError {
    pub fn message(self) -> &'static str {
        match self {
            Self::EmptyCredentials => "请输入4399账号和密码",
            Self::Unavailable => "暂时无法获取4399登录页面，请检查网络后重试",
            Self::Network => "登录请求失败或超时，请检查网络后重试",
            Self::LoginRejected => {
                "登录失败，请检查账号密码；若触发验证码，当前快捷登录无法完成验证"
            }
            Self::InvalidSession => {
                "未取得有效游戏凭据，请检查账号密码或稍后重试；当前快捷登录不支持验证码"
            }
            Self::UnexpectedResponse => "登录服务器返回了异常响应，请稍后重试",
        }
    }
}

/// The query is opaque: decoding and re-encoding it changes the game's signature.
/// No Debug implementation, since this holds a live ticket.
pub struct Session(String);

impl Session {
    pub fn from_auth(raw: &str) -> Result<Self, LoginError> {
        if raw.is_empty() || raw.contains('#') || raw.chars().any(char::is_control) {
            return Err(LoginError::InvalidSession);
        }
        let session = Self::from_redirect(&format!("{GAME_PAGE}?{raw}"))?;
        if session.0 != raw {
            return Err(LoginError::InvalidSession);
        }
        Ok(session)
    }

    pub fn raw(&self) -> &str {
        &self.0
    }

    pub fn from_redirect(raw: &str) -> Result<Self, LoginError> {
        if raw.len() > 128 * 1024 || raw.chars().any(char::is_control) {
            return Err(LoginError::InvalidSession);
        }
        let url = Url::parse(raw).map_err(|_| LoginError::InvalidSession)?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str() != Some("enter.wanwan4399.com")
            || url.port().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/bin-debug/KBgameindex.html"
        {
            return Err(LoginError::InvalidSession);
        }
        let query = url.query().ok_or(LoginError::InvalidSession)?;
        if query.len() > 64 * 1024
            || query.chars().any(char::is_control)
            || query
                .split('&')
                .filter(|entry| entry.starts_with("sig="))
                .count()
                != 1
            || !query.split('&').any(|entry| {
                entry
                    .strip_prefix("sig=")
                    .is_some_and(|sig| !sig.is_empty())
            })
        {
            return Err(LoginError::InvalidSession);
        }
        Ok(Self(query.into()))
    }

    pub fn apply(self, options: &mut LaunchOptions) {
        options.player.parameters = vec![("auth_string".into(), self.0)];
        options.player.base = Url::parse(BASE_URL).ok();
        options.player.referer = Url::parse(GAME_PAGE).ok();
        options.player.upgrade_to_https = Some(false);
        options.player.cookie = None;
        options.player.spoof_url = None;
        options.tcp_connections = Some(SocketMode::Allow);
    }
}

pub fn is_game_url(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url.host_str() == Some("enter.wanwan4399.com")
        && url.port().is_none()
        && url.path() == "/bin-debug/GreenGame.swf"
}

/// CryptoJS passphrase mode: EVP_BytesToKey(MD5), AES-256-CBC, Salted__.
fn encrypt_password(password: &str, salt: [u8; 8]) -> String {
    let mut material = Vec::with_capacity(48);
    let mut previous = Vec::new();
    while material.len() < 48 {
        let mut digest = Md5::new();
        digest.update(&previous);
        digest.update(b"lzYW5qaXVqa");
        digest.update(salt);
        previous = digest.finalize().to_vec();
        material.extend_from_slice(&previous);
    }
    let ciphertext = cbc::Encryptor::<Aes256>::new_from_slices(&material[..32], &material[32..48])
        .expect("fixed AES key and IV lengths")
        .encrypt_padded_vec_mut::<Pkcs7>(password.as_bytes());
    let mut output = b"Salted__".to_vec();
    output.extend_from_slice(&salt);
    output.extend_from_slice(&ciphertext);
    base64::engine::general_purpose::STANDARD.encode(output)
}

pub async fn login(username: &str, password: &str) -> Result<Session, LoginError> {
    tokio::time::timeout(
        Duration::from_secs(90),
        login_with_hosts(username, password, &HOSTS),
    )
    .await
    .map_err(|_| LoginError::Network)?
}

async fn login_with_hosts(
    username: &str,
    password: &str,
    hosts: &[&str],
) -> Result<Session, LoginError> {
    if username.trim().is_empty() || password.is_empty() {
        return Err(LoginError::EmptyCredentials);
    }
    let client = Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(25))
        .user_agent(USER_AGENT)
        .build()
        .map_err(|_| LoginError::Network)?;

    let mut selected = None;
    for host in hosts {
        let frame =
            format!("{host}/ptlogin/loginFrame.do?displayMode=popup&bizId={BIZ_ID}&appId={APP_ID}");
        if let Ok(response) = request(&client, &frame, None, &[], true).await
            && response.status == StatusCode::OK
            && response.body.contains("login_form")
        {
            selected = Some(*host);
            break;
        }
    }
    let host = selected.ok_or(LoginError::Unavailable)?;
    let frame = format!(
        "{host}/ptlogin/loginFrame.do?postLoginHandler=default&displayMode=popup&bizId={BIZ_ID}&appId={APP_ID}"
    );
    if request(
        &client,
        &format!("{frame}&autoLogin=true&qrLogin=true"),
        None,
        &[],
        true,
    )
    .await?
    .status
        != StatusCode::OK
    {
        return Err(LoginError::UnexpectedResponse);
    }

    let encrypted = encrypt_password(password, rand::random());
    let fields = [
        ("loginFrom", "uframe"),
        ("postLoginHandler", "default"),
        ("layoutSelfAdapting", "true"),
        ("externalLogin", ""),
        ("displayMode", "popup"),
        ("layout", "vertical"),
        ("bizId", BIZ_ID),
        ("appId", APP_ID),
        ("gameId", ""),
        ("css", ""),
        ("redirectUrl", ""),
        ("sessionId", ""),
        ("mainDivId", "embed_login_div"),
        ("includeFcmInfo", "false"),
        ("level", "0"),
        ("regLevel", "0"),
        ("userNameLabel", "4399用户名"),
        ("userNameTip", "请输入4399用户名"),
        ("welcomeTip", "欢迎回到4399"),
        ("sec", "1"),
        ("password", &encrypted),
        ("iframeId", ""),
        ("username", username.trim()),
    ];
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields)
        .finish();
    let response = request(
        &client,
        &format!("{host}/ptlogin/login.do?v=1"),
        Some(body),
        &[(REFERER, &frame), (ORIGIN, host)],
        true,
    )
    .await?;
    if response.status != StatusCode::OK {
        return Err(LoginError::LoginRejected);
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let ret_url = format!("http://news.4399.com/login/kbxy.html?reg=0&pass=1&v={now}");
    let mut check = Url::parse(&format!("{host}/ptlogin/checkKidLoginUserCookie.do"))
        .map_err(|_| LoginError::UnexpectedResponse)?;
    check.query_pairs_mut().extend_pairs([
        ("appId", APP_ID),
        ("level", "8"),
        ("loginLevel", "8"),
        ("regLevel", "8"),
        ("bizId", BIZ_ID),
        ("canBack", "true"),
        ("gameUrl", GAME_PAGE),
        ("retUrl", &ret_url),
    ]);
    let response = request(
        &client,
        check.as_str(),
        None,
        &[(REFERER, "http://www.4399.com/flash/48399.htm")],
        false,
    )
    .await?;
    if !response.status.is_redirection() {
        return Err(LoginError::InvalidSession);
    }
    Session::from_redirect(&response.location)
}

struct Response {
    status: StatusCode,
    body: String,
    location: String,
}

async fn request(
    client: &Client,
    raw: &str,
    mut body: Option<String>,
    headers: &[(reqwest::header::HeaderName, &str)],
    follow: bool,
) -> Result<Response, LoginError> {
    let mut url = Url::parse(raw).map_err(|_| LoginError::UnexpectedResponse)?;
    for _ in 0..6 {
        let mut request = if let Some(body) = &body {
            client
                .post(url.clone())
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(body.clone())
        } else {
            client.get(url.clone())
        };
        for (name, value) in headers {
            request = request.header(name, *value);
        }
        let mut response = request.send().await.map_err(|_| LoginError::Network)?;
        let status = response.status();
        let location = response
            .headers()
            .get(LOCATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        if follow && status.is_redirection() {
            let next = url
                .join(&location)
                .map_err(|_| LoginError::UnexpectedResponse)?;
            if !allowed_redirect(&url, &next, body.is_some(), status) {
                return Err(LoginError::UnexpectedResponse);
            }
            if !matches!(
                status,
                StatusCode::TEMPORARY_REDIRECT | StatusCode::PERMANENT_REDIRECT
            ) {
                body = None;
            }
            url = next;
            continue;
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| LoginError::Network)? {
            if bytes.len() + chunk.len() > MAX_BODY {
                return Err(LoginError::UnexpectedResponse);
            }
            bytes.extend_from_slice(&chunk);
        }
        return Ok(Response {
            status,
            body: String::from_utf8_lossy(&bytes).into_owned(),
            location,
        });
    }
    Err(LoginError::UnexpectedResponse)
}

fn allowed_redirect(current: &Url, next: &Url, has_password: bool, status: StatusCode) -> bool {
    if !matches!(next.scheme(), "http" | "https")
        || !next.username().is_empty()
        || next.password().is_some()
        || (current.scheme() == "https" && next.scheme() != "https")
    {
        return false;
    }
    // Never forward the encrypted password to a different origin.
    if has_password
        && matches!(
            status,
            StatusCode::TEMPORARY_REDIRECT | StatusCode::PERMANENT_REDIRECT
        )
    {
        return current.origin() == next.origin();
    }
    next.origin() == current.origin()
        || (next.port().is_none()
            && matches!(
                next.host_str(),
                Some("ptlogin.4399.com" | "ptlogin.3304399.net")
            ))
}

#[cfg(test)]
mod tests;
