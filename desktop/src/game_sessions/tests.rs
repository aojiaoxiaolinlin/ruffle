use super::*;

#[test]
fn encoded_sessions_roundtrip_without_changing_tickets() {
    let kabu = "user=test&sig=abc%2Bdef%3D&other=a+b%26c";
    assert_eq!(
        Session::decode(Game::Kabu, kabu).expect("fixture").encode(),
        kabu
    );
    let roco = "angel_uin=123&angel_key=a%2Bb%26c%3D&skey=%252B&pskey=p";
    let session = Session::decode(Game::Roco, roco).expect("fixture");
    let encoded = session.encode();
    assert_eq!(
        Session::decode(Game::Roco, &encoded)
            .expect("roundtrip")
            .encode(),
        encoded
    );
    assert!(encoded.contains("angel_key=a%2Bb%26c%3D"));
    assert!(encoded.contains("skey=%252B"));
    assert!(Session::decode(Game::Kabu, "sig=fixture#truncated").is_err());
    assert!(Session::decode(Game::Roco, "angel_uin=123").is_err());
}

#[test]
fn game_entries_require_their_own_complete_credentials() {
    let kabu = vec![(
        "auth_string".to_owned(),
        "user=fixture&sig=private%2Bticket".to_owned(),
    )];
    let roco: Vec<_> =
        url::form_urlencoded::parse(b"angel_uin=123&angel_key=private&skey=s&pskey=p")
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
    assert!(Game::Kabu.has_credentials(&kabu));
    assert!(!Game::Roco.has_credentials(&kabu));
    assert!(Game::Roco.has_credentials(&roco));
    assert!(!Game::Kabu.has_credentials(&roco));
    assert!(!Game::Roco.has_credentials(&roco[..3]));
    assert!(!Game::Kabu.has_credentials(&[("auth_string".into(), "sig=".into())]));
}

#[test]
fn only_known_login_pages_request_reauthorization() {
    assert!(
        Game::Roco.is_login_url(&Url::parse("https://17roco.qq.com/login.html").expect("fixture"))
    );
    assert!(
        Game::Kabu.is_login_url(
            &Url::parse("http://news.4399.com/login/kbxy.html?reg=0").expect("fixture")
        )
    );
    for raw in [
        "https://17roco.qq.com/activity/maintenance.html",
        "https://17roco.qq.com.evil.test/login.html",
        "https://17roco.qq.com:444/login.html",
        "https://example.com/login.html",
    ] {
        assert!(!Game::Roco.is_login_url(&Url::parse(raw).expect("fixture")));
    }
    assert!(!Game::Kabu.is_login_url(&Game::Kabu.movie_url()));
    assert!(!Game::Roco.is_login_url(&Game::Roco.movie_url()));
}

#[cfg(windows)]
struct TestStore {
    config: PathBuf,
    store: SessionStore,
}

#[cfg(windows)]
impl TestStore {
    fn new() -> Self {
        let config = std::env::temp_dir().join(format!(
            "ruffle-game-session-test-{}",
            rand::random::<u64>()
        ));
        fs::create_dir(&config).expect("test configuration directory");
        Self {
            store: SessionStore::new(&config),
            config,
        }
    }

    fn kabu() -> Session {
        Session::decode(
            Game::Kabu,
            "user=fixture&sig=private%2Bticket%3D&extra=a+b%26c",
        )
        .expect("fixture")
    }

    fn roco() -> Session {
        Session::decode(
            Game::Roco,
            "angel_uin=123&angel_key=private_roco_ticket&skey=s&pskey=p",
        )
        .expect("fixture")
    }
}

#[cfg(windows)]
impl Drop for TestStore {
    fn drop(&mut self) {
        // Delete only synthetic records and their empty test directories.
        let credentials = KabuCredentialStore::new(&self.config);
        let _ = credentials.clear();
        let _ = fs::remove_dir(&credentials.directory);
        let _ = self.store.clear(Game::Kabu);
        let _ = self.store.clear(Game::Roco);
        let _ = fs::remove_dir(&self.store.directory);
        let _ = fs::remove_dir(&self.config);
    }
}

#[cfg(windows)]
#[test]
fn remembered_passwords_are_encrypted_replaceable_and_separate_from_tickets() {
    let test = TestStore::new();
    let store = KabuCredentialStore::new(&test.config);
    assert!(store.load().expect("missing credential file").is_none());
    test.store.save(&TestStore::kabu()).expect("save ticket");
    let credentials = KabuCredentials {
        username: "测试账号&=fixture".to_owned(),
        password: "synthetic-password-密码&=".to_owned(),
    };
    store.save(&credentials).expect("encrypt credentials");
    let bytes = fs::read(store.path()).expect("encrypted file");
    assert!(bytes.starts_with(ACCOUNT_MAGIC));
    for secret in [&credentials.username, &credentials.password] {
        assert!(
            !bytes
                .windows(secret.len())
                .any(|part| part == secret.as_bytes())
        );
    }
    let restored = KabuCredentialStore::new(&test.config)
        .load()
        .expect("decrypt after restart")
        .expect("saved credentials");
    assert_eq!(restored.username, credentials.username);
    assert_eq!(restored.password, credentials.password);

    // Account storage and ticket storage must not be interchangeable.
    let cross_purpose = protect(Game::Kabu, b"synthetic", true).expect("encrypt ticket data");
    assert!(protect_with_entropy("ruffle.game-account.v1.kabu", &cross_purpose, false).is_err());

    let replacement = KabuCredentials {
        username: "another-account".to_owned(),
        password: "another-synthetic-password".to_owned(),
    };
    store.save(&replacement).expect("replace account");
    let restored = store.load().expect("load replacement").expect("account");
    assert_eq!(restored.username, replacement.username);
    assert_eq!(restored.password, replacement.password);
    store.clear().expect("forget credentials");
    store.clear().expect("forget is idempotent");
    assert!(store.load().expect("forgotten account").is_none());
    assert_eq!(
        test.store
            .load(Game::Kabu)
            .expect("ticket unaffected")
            .expect("ticket")
            .encode(),
        TestStore::kabu().encode()
    );
}

#[cfg(windows)]
#[test]
fn remembered_password_limits_allow_encryption_overhead_and_reject_corruption() {
    let test = TestStore::new();
    let store = KabuCredentialStore::new(&test.config);
    let mut credentials = KabuCredentials {
        username: "fixture".to_owned(),
        password: "x".repeat(MAX_CREDENTIAL_SIZE as usize - 8 - "fixture".len()),
    };
    store
        .save(&credentials)
        .expect("save maximum plaintext size");
    let restored = store
        .load()
        .expect("allow DPAPI overhead")
        .expect("account");
    assert_eq!(restored.password, credentials.password);
    credentials.password.push('x');
    assert!(store.save(&credentials).is_err());
    assert_eq!(
        store
            .load()
            .expect("previous account retained")
            .expect("account")
            .password,
        restored.password
    );

    let mut bytes = fs::read(store.path()).expect("encrypted record");
    let last = bytes.len() - 1;
    bytes[last] ^= 0x80;
    fs::write(store.path(), bytes).expect("tamper synthetic record");
    assert!(store.load().is_err());
    fs::write(store.path(), b"broken account record").expect("corrupt fixture");
    assert!(store.load().is_err());
}

#[cfg(windows)]
#[test]
fn restart_restores_both_games_without_plaintext_on_disk() {
    let test = TestStore::new();
    test.store.save(&TestStore::kabu()).expect("save Kabu");
    test.store.save(&TestStore::roco()).expect("save Roco");
    for (game, private_value) in [
        (Game::Kabu, "private%2Bticket"),
        (Game::Roco, "private_roco_ticket"),
    ] {
        let bytes = fs::read(test.store.path(game)).expect("encrypted record");
        assert!(bytes.starts_with(MAGIC));
        assert!(
            !bytes
                .windows(private_value.len())
                .any(|part| part == private_value.as_bytes())
        );
    }
    let restarted = SessionStore::new(&test.config);
    assert_eq!(
        restarted
            .load(Game::Kabu)
            .expect("load")
            .expect("Kabu")
            .encode(),
        TestStore::kabu().encode()
    );
    assert_eq!(
        restarted
            .load(Game::Roco)
            .expect("load")
            .expect("Roco")
            .encode(),
        TestStore::roco().encode()
    );
}

#[cfg(windows)]
#[test]
fn replacing_and_clearing_one_account_preserves_the_other_game() {
    let test = TestStore::new();
    test.store.save(&TestStore::kabu()).expect("save Kabu");
    test.store.save(&TestStore::roco()).expect("save Roco");
    let replacement = Session::decode(Game::Kabu, "user=another&sig=replacement").expect("fixture");
    test.store.save(&replacement).expect("atomic replacement");
    assert_eq!(
        test.store
            .load(Game::Kabu)
            .expect("load")
            .expect("Kabu")
            .encode(),
        replacement.encode()
    );
    test.store.clear(Game::Kabu).expect("switch Kabu account");
    assert!(
        test.store
            .load(Game::Kabu)
            .expect("missing account")
            .is_none()
    );
    assert_eq!(
        test.store
            .load(Game::Roco)
            .expect("load")
            .expect("Roco")
            .encode(),
        TestStore::roco().encode()
    );
    test.store.clear(Game::Kabu).expect("clear is idempotent");
}

#[cfg(windows)]
#[test]
fn tampered_or_cross_game_records_cannot_be_reused() {
    let test = TestStore::new();
    test.store.save(&TestStore::roco()).expect("save Roco");
    fs::copy(test.store.path(Game::Roco), test.store.path(Game::Kabu))
        .expect("copy synthetic record");
    assert!(test.store.load(Game::Kabu).is_err());
    let mut bytes = fs::read(test.store.path(Game::Roco)).expect("encrypted record");
    let last = bytes.len() - 1;
    bytes[last] ^= 0x80;
    fs::write(test.store.path(Game::Roco), bytes).expect("tamper fixture");
    assert!(test.store.load(Game::Roco).is_err());
}

#[cfg(windows)]
#[test]
fn missing_corrupt_and_oversized_records_do_not_produce_sessions() {
    let test = TestStore::new();
    assert!(test.store.load(Game::Kabu).expect("missing file").is_none());
    fs::create_dir(&test.store.directory).expect("test session directory");
    fs::write(test.store.path(Game::Kabu), b"broken record").expect("corrupt fixture");
    assert!(test.store.load(Game::Kabu).is_err());
    fs::write(
        test.store.path(Game::Kabu),
        vec![0; MAX_STORED_SIZE as usize + 1],
    )
    .expect("oversized fixture");
    assert!(test.store.load(Game::Kabu).is_err());
}
