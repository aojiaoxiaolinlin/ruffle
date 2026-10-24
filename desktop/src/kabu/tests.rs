use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

#[test]
fn password_encryption_matches_android_openssl_vector() {
    assert_eq!(
        encrypt_password("test-password", [0, 1, 2, 3, 4, 5, 6, 7]),
        "U2FsdGVkX18AAQIDBAUGBx3fzKjTUFY6GXZTEb7Qsro="
    );
}

#[test]
fn auth_string_preserves_full_opaque_query() {
    let query = "user=test&sig=abc%2Bdef%3D&other=a+b%26c";
    let session = Session::from_redirect(&format!("{GAME_PAGE}?{query}")).expect("fixture");
    assert_eq!(session.0, query);
    // Returning a sig alone, decoding %2B, or turning '+' into a space is incorrect.
    assert!(session.0.contains("&other=a+b%26c"));
}

#[test]
fn launching_kabu_replaces_previous_game_tickets_and_network_settings() {
    let mut options = LaunchOptions {
        player: ruffle_frontend_utils::player_options::PlayerOptions {
            parameters: vec![("angel_key".into(), "old-ticket".into())],
            upgrade_to_https: Some(true),
            cookie: Some("old=cookie".into()),
            ..Default::default()
        },
        proxy: None,
        socket_allowed: Default::default(),
        tcp_connections: None,
        fullscreen: false,
        save_directory: Default::default(),
        cache_directory: Default::default(),
        filesystem_access_mode: crate::cli::FilesystemAccessMode::Ask,
        gamepad_button_mapping: Default::default(),
        avm2_optimizer_enabled: true,
        soft_filter_scale: None,
    };
    Session::from_redirect(&format!("{GAME_PAGE}?sig=fixture%2Bkey&user=test"))
        .expect("fixture session")
        .apply(&mut options);
    assert_eq!(
        options.player.parameters,
        vec![("auth_string".into(), "sig=fixture%2Bkey&user=test".into())]
    );
    assert_eq!(
        options.player.base.as_ref().map(Url::as_str),
        Some(BASE_URL)
    );
    assert_eq!(options.player.upgrade_to_https, Some(false));
    assert!(options.player.cookie.is_none());
    assert!(matches!(options.tcp_connections, Some(SocketMode::Allow)));
}

#[test]
fn invalid_or_untrusted_sessions_are_rejected() {
    for raw in [
        "not a URL",
        "http://example.com/bin-debug/KBgameindex.html?sig=s",
        "http://enter.wanwan4399.com.evil.test/bin-debug/KBgameindex.html?sig=s",
        "http://enter.wanwan4399.com:1234/bin-debug/KBgameindex.html?sig=s",
        "http://user@enter.wanwan4399.com/bin-debug/KBgameindex.html?sig=s",
        "http://enter.wanwan4399.com/bin-debug/other.html?sig=s",
        "http://enter.wanwan4399.com/bin-debug/KBgameindex.html?sig=",
        "http://enter.wanwan4399.com/bin-debug/KBgameindex.html?user=test",
        "http://enter.wanwan4399.com/bin-debug/KBgameindex.html?sig=s&sig=other",
        "http://enter.wanwan4399.com/bin-debug/KBgameindex.html?sig=fi\nxture",
    ] {
        assert!(Session::from_redirect(raw).is_err());
    }
}

#[test]
fn password_redirects_cannot_change_origin() {
    let current = Url::parse("http://ptlogin.4399.com/ptlogin/login.do").expect("fixture");
    let other = Url::parse("http://ptlogin.3304399.net/login").expect("fixture");
    assert!(!allowed_redirect(
        &current,
        &other,
        true,
        StatusCode::TEMPORARY_REDIRECT
    ));
    let local = current.join("/ptlogin/other.do").expect("fixture");
    assert!(allowed_redirect(
        &current,
        &local,
        true,
        StatusCode::TEMPORARY_REDIRECT
    ));
    let evil = Url::parse("http://evil.test/login").expect("fixture");
    assert!(!allowed_redirect(&current, &evil, false, StatusCode::FOUND));
    let secure = Url::parse("https://ptlogin.4399.com/login").expect("fixture");
    assert!(!allowed_redirect(
        &secure,
        &current,
        false,
        StatusCode::FOUND
    ));
}

struct FakeServer {
    origin: String,
    worker: thread::JoinHandle<Vec<String>>,
}

impl FakeServer {
    fn new(replies: Vec<(u16, String, String)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("local test listener");
        let origin = format!("http://{}", listener.local_addr().expect("local address"));
        listener.set_nonblocking(true).expect("nonblocking");
        let worker = thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, headers, body) in replies {
                let deadline = std::time::Instant::now() + Duration::from_secs(10);
                let mut socket = loop {
                    match listener.accept() {
                        Ok((socket, _)) => break socket,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(
                                std::time::Instant::now() < deadline,
                                "test request timed out"
                            );
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("local accept failed: {error}"),
                    }
                };
                // Accepted sockets can inherit the listener's nonblocking
                // mode on Windows. Wait for the client's request instead of
                // racing its first write and failing with WSAEWOULDBLOCK.
                socket.set_nonblocking(false).expect("blocking test socket");
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .expect("read timeout");
                let mut bytes = Vec::new();
                let mut buffer = [0; 4096];
                loop {
                    let count = socket.read(&mut buffer).expect("read local request");
                    assert_ne!(count, 0);
                    bytes.extend_from_slice(&buffer[..count]);
                    let request = String::from_utf8_lossy(&bytes);
                    if let Some((head, body)) = request.split_once("\r\n\r\n") {
                        let length: usize = head
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length: ")?
                                    .parse()
                                    .ok()
                            })
                            .unwrap_or_default();
                        if body.len() >= length {
                            break;
                        }
                    }
                }
                requests.push(String::from_utf8(bytes).expect("UTF-8 request"));
                write!(socket, "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}", body.len()).expect("local response");
            }
            requests
        });
        Self { origin, worker }
    }

    fn frame() -> (u16, String, String) {
        (
            200,
            "Set-Cookie: sid=fixture; Path=/ptlogin\r\n".into(),
            "<form id=\"login_form\"></form>".into(),
        )
    }
}

#[tokio::test]
async fn login_replays_cookie_form_and_ticket_exchange() {
    let server = FakeServer::new(vec![
        FakeServer::frame(),
        FakeServer::frame(),
        (200, String::new(), "ok".into()),
        (
            302,
            format!("Location: {GAME_PAGE}?user=test&sig=mock%2Bsignature%3D\r\n"),
            String::new(),
        ),
    ]);
    let session = login_with_hosts("test+账号", "test-password", &[&server.origin])
        .await
        .expect("local login");
    assert_eq!(session.0, "user=test&sig=mock%2Bsignature%3D");
    let requests = server.worker.join().expect("local worker");
    assert_eq!(requests.len(), 4);
    let post = &requests[2];
    assert!(post.starts_with("POST /ptlogin/login.do?v=1 "));
    assert!(post.to_ascii_lowercase().contains("cookie: sid=fixture"));
    assert!(
        post.to_ascii_lowercase()
            .contains("referer: http://127.0.0.1:")
    );
    assert!(
        post.to_ascii_lowercase()
            .contains("origin: http://127.0.0.1:")
    );
    let (_, body) = post.split_once("\r\n\r\n").expect("request body");
    let fields: std::collections::HashMap<_, _> =
        url::form_urlencoded::parse(body.as_bytes()).collect();
    assert_eq!(
        fields.get("username").map(|v| v.as_ref()),
        Some("test+账号")
    );
    assert!(
        fields
            .get("password")
            .expect("encrypted password")
            .starts_with("U2FsdGVkX1")
    );
    assert!(!post.contains("test-password"));
    assert!(requests[3].starts_with("GET /ptlogin/checkKidLoginUserCookie.do?"));
    assert!(
        requests[3]
            .to_ascii_lowercase()
            .contains("cookie: sid=fixture")
    );
}

#[tokio::test]
async fn rejected_credentials_stop_before_ticket_exchange() {
    let server = FakeServer::new(vec![
        FakeServer::frame(),
        FakeServer::frame(),
        (
            401,
            String::new(),
            "private server response must not appear in errors".into(),
        ),
    ]);
    let result = login_with_hosts("test", "private-password", &[&server.origin]).await;
    assert!(matches!(result, Err(LoginError::LoginRejected)));
    assert_eq!(server.worker.join().expect("worker").len(), 3);
    assert!(!LoginError::LoginRejected.message().contains("private"));
}

#[tokio::test]
async fn unavailable_primary_host_falls_back_before_submitting_password() {
    let primary = FakeServer::new(vec![(200, String::new(), "not a login form".into())]);
    let fallback = FakeServer::new(vec![
        FakeServer::frame(),
        FakeServer::frame(),
        (200, String::new(), "ok".into()),
        (
            302,
            format!("Location: {GAME_PAGE}?sig=fixture\r\n"),
            String::new(),
        ),
    ]);
    assert!(
        login_with_hosts("test", "password", &[&primary.origin, &fallback.origin])
            .await
            .is_ok()
    );
    let requests = primary.worker.join().expect("primary");
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with("GET "));
    assert_eq!(fallback.worker.join().expect("fallback").len(), 4);
}

#[tokio::test]
async fn empty_credentials_fail_before_networking() {
    assert!(matches!(
        login_with_hosts(" ", "password", &[]).await,
        Err(LoginError::EmptyCredentials)
    ));
    assert!(matches!(
        login_with_hosts("user", "", &[]).await,
        Err(LoginError::EmptyCredentials)
    ));
}
