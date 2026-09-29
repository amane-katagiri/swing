use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use hmac::{Hmac, Mac};
use sha2::Sha256;

pub const TOKEN_FILE: &str = "dashboard.token";
pub const SESSION_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);
pub const LOGIN_CODE_TTL: Duration = Duration::from_secs(5 * 60);
pub const DASHBOARD_SESSION: &str = "dashboard-session";

const TOKEN_BYTES: usize = 32;
const LOGIN_CODE_BYTES: usize = 16;
const MAX_CLOCK_SKEW: u64 = 5 * 60;

pub fn token_path(state_dir: &Path) -> PathBuf {
    state_dir.join(TOKEN_FILE)
}

pub fn read_token(state_dir: &Path) -> Result<Option<String>> {
    let path = token_path(state_dir);
    match std::fs::read_to_string(&path) {
        Ok(s) => {
            #[cfg(unix)]
            warn_if_readable_by_others(state_dir, &path);
            Ok(Some(s.trim().to_string()).filter(|t| !t.is_empty()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

#[cfg(unix)]
fn broader_than(path: &Path, allowed: u32) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path).ok()?.permissions().mode() & 0o777;
    (mode & !allowed != 0).then_some(mode)
}

#[cfg(unix)]
fn warn_if_readable_by_others(state_dir: &Path, token_file: &Path) {
    if let Some(mode) = broader_than(token_file, 0o600) {
        tracing::warn!(
            path = %token_file.display(),
            mode = format_args!("{mode:o}"),
            "the dashboard token file is accessible to other users; restrict it to 0600"
        );
    }
    if let Some(mode) = broader_than(state_dir, 0o700) {
        tracing::warn!(
            path = %state_dir.display(),
            mode = format_args!("{mode:o}"),
            "the state directory holding the dashboard token is accessible to other users; restrict it to 0700"
        );
    }
}

pub fn load_or_create_token(state_dir: &Path) -> Result<String> {
    match read_token(state_dir)? {
        Some(token) => Ok(token),
        None => write_new_token(state_dir),
    }
}

pub fn write_new_token(state_dir: &Path) -> Result<String> {
    create_private_dir_all(state_dir)?;
    let token = random_hex(TOKEN_BYTES);
    write_private_file(&token_path(state_dir), &format!("{token}\n"))?;
    Ok(token)
}

pub(crate) fn create_private_dir_all(path: &Path) -> Result<()> {
    create_private_dir_all_io(path).with_context(|| format!("creating {}", path.display()))
}

pub(crate) fn create_private_dir_all_io(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(path)
    }
}

pub(crate) fn private_file_options() -> std::fs::OpenOptions {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

pub(crate) fn write_private_file(path: &Path, contents: &str) -> Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".{}.tmp", random_hex(8)));
    let tmp = PathBuf::from(tmp);
    let mut file = private_file_options()
        .open(&tmp)
        .with_context(|| format!("creating {}", tmp.display()))?;
    let written = file
        .write_all(contents.as_bytes())
        .and_then(|()| file.sync_all())
        .with_context(|| format!("writing {}", tmp.display()));
    drop(file);
    let result = written.and_then(|()| {
        std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

pub(crate) fn random_hex(len: usize) -> String {
    let mut bytes = vec![0u8; len];
    getrandom::fill(&mut bytes).expect("reading OS randomness");
    hex(&bytes)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn session_mac(token: &str, purpose: &str, issued_at: u64) -> Hmac<Sha256> {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(token.as_bytes()).expect("HMAC accepts any key length");
    mac.update(purpose.as_bytes());
    mac.update(b"\0");
    mac.update(issued_at.to_string().as_bytes());
    mac
}

pub fn sign_session(token: &str, purpose: &str, issued_at: u64) -> String {
    let tag = session_mac(token, purpose, issued_at)
        .finalize()
        .into_bytes();
    format!("{issued_at}.{}", hex(&tag))
}

pub fn new_session(token: &str, purpose: &str) -> String {
    sign_session(token, purpose, unix_now())
}

pub fn verify_session_at(token: &str, purpose: &str, value: &str, now: u64) -> bool {
    let Some((issued, tag_hex)) = value.split_once('.') else {
        return false;
    };
    if issued.is_empty() || !issued.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let Ok(issued_at) = issued.parse::<u64>() else {
        return false;
    };
    if issued_at > now.saturating_add(MAX_CLOCK_SKEW)
        || now.saturating_sub(issued_at) >= SESSION_TTL.as_secs()
    {
        return false;
    }
    let Some(tag) = parse_hex(tag_hex) else {
        return false;
    };
    session_mac(token, purpose, issued_at)
        .verify_slice(&tag)
        .is_ok()
}

pub fn verify_session(token: &str, purpose: &str, value: &str) -> bool {
    verify_session_at(token, purpose, value, unix_now())
}

fn parse_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

// Compares MACs so timing doesn't depend on how much of the presented token matches.
pub fn token_matches(expected: &str, presented: &str) -> bool {
    let mac = session_mac(expected, "bearer", 0);
    let presented_tag = session_mac(presented, "bearer", 0).finalize().into_bytes();
    mac.verify_slice(&presented_tag).is_ok()
}

pub const IDENTITY_NONCE_BYTES: usize = 32;

pub fn new_identity_nonce() -> String {
    random_hex(IDENTITY_NONCE_BYTES)
}

pub fn is_identity_nonce(nonce: &str) -> bool {
    nonce.len() == IDENTITY_NONCE_BYTES * 2 && nonce.bytes().all(|b| b.is_ascii_hexdigit())
}

fn identity_mac(token: &str, nonce: &str) -> Hmac<Sha256> {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(token.as_bytes()).expect("HMAC accepts any key length");
    mac.update(b"swing-identity:");
    mac.update(nonce.to_ascii_lowercase().as_bytes());
    mac
}

pub fn identity_proof(token: &str, nonce: &str) -> String {
    hex(&identity_mac(token, nonce).finalize().into_bytes())
}

pub fn verify_identity_proof(token: &str, nonce: &str, proof: &str) -> bool {
    parse_hex(proof).is_some_and(|tag| identity_mac(token, nonce).verify_slice(&tag).is_ok())
}

#[derive(Default)]
pub struct LoginCodes {
    codes: Mutex<HashMap<String, Instant>>,
}

pub fn is_login_code(code: &str) -> bool {
    code.len() == LOGIN_CODE_BYTES * 2
        && code.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl LoginCodes {
    pub fn issue(&self) -> String {
        let code = random_hex(LOGIN_CODE_BYTES);
        let now = Instant::now();
        let mut codes = self.codes.lock().expect("login code lock");
        codes.retain(|_, expires| *expires > now);
        codes.insert(code.clone(), now + LOGIN_CODE_TTL);
        code
    }

    pub fn redeem(&self, code: &str) -> bool {
        let code = code.trim().to_ascii_lowercase();
        let mut codes = self.codes.lock().expect("login code lock");
        matches!(codes.remove(&code), Some(expires) if expires > Instant::now())
    }

    pub fn clear(&self) {
        self.codes.lock().expect("login code lock").clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 24 * 60 * 60;

    #[test]
    fn session_round_trips_within_ttl() {
        let v = sign_session("tok", DASHBOARD_SESSION, 1_000_000);
        assert!(verify_session_at("tok", DASHBOARD_SESSION, &v, 1_000_000));
        assert!(verify_session_at(
            "tok",
            DASHBOARD_SESSION,
            &v,
            1_000_000 + 30 * DAY - 1
        ));
    }

    #[test]
    fn session_expires_after_ttl() {
        let v = sign_session("tok", DASHBOARD_SESSION, 1_000_000);
        assert!(!verify_session_at(
            "tok",
            DASHBOARD_SESSION,
            &v,
            1_000_000 + 30 * DAY
        ));
    }

    #[test]
    fn session_from_the_future_is_rejected() {
        let v = sign_session("tok", DASHBOARD_SESSION, 1_000_000 + 3600);
        assert!(!verify_session_at("tok", DASHBOARD_SESSION, &v, 1_000_000));
    }

    #[test]
    fn session_is_bound_to_token_and_purpose() {
        let v = sign_session("tok", DASHBOARD_SESSION, 1_000_000);
        assert!(!verify_session_at(
            "other",
            DASHBOARD_SESSION,
            &v,
            1_000_000
        ));
        assert!(!verify_session_at("tok", "gateway-session", &v, 1_000_000));
    }

    #[test]
    fn tampered_session_is_rejected() {
        let v = sign_session("tok", DASHBOARD_SESSION, 1_000_000);
        let moved = v.replacen("1000000", "1000001", 1);
        assert!(!verify_session_at(
            "tok",
            DASHBOARD_SESSION,
            &moved,
            1_000_001
        ));
        assert!(!verify_session_at(
            "tok",
            DASHBOARD_SESSION,
            "garbage",
            1_000_000
        ));
        assert!(!verify_session_at(
            "tok",
            DASHBOARD_SESSION,
            "1000000.zz",
            1_000_000
        ));
    }

    #[test]
    fn session_issued_at_must_be_plain_digits() {
        let v = sign_session("tok", DASHBOARD_SESSION, 1_000_000);
        let plus = format!("+{v}");
        assert!(!verify_session_at(
            "tok",
            DASHBOARD_SESSION,
            &plus,
            1_000_000
        ));
    }

    #[test]
    fn identity_proof_is_bound_to_token_and_nonce() {
        let nonce = new_identity_nonce();
        assert!(is_identity_nonce(&nonce));
        let proof = identity_proof("tok", &nonce);
        assert!(verify_identity_proof("tok", &nonce, &proof));
        assert!(!verify_identity_proof("other", &nonce, &proof));
        assert!(!verify_identity_proof("tok", &new_identity_nonce(), &proof));
        assert!(!verify_identity_proof("tok", &nonce, "zz"));
        assert!(!is_identity_nonce("abc"));
        assert!(!is_identity_nonce(&"g".repeat(IDENTITY_NONCE_BYTES * 2)));
    }

    #[test]
    fn concurrent_private_writes_do_not_collide() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file");
        std::thread::scope(|scope| {
            for i in 0..8 {
                let path = &path;
                scope.spawn(move || write_private_file(path, &format!("{i}\n")).unwrap());
            }
        });
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.trim().parse::<u32>().unwrap() < 8);
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name() != "file")
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn token_matches_only_the_same_token() {
        assert!(token_matches("abc", "abc"));
        assert!(!token_matches("abc", "abd"));
        assert!(!token_matches("abc", ""));
    }

    #[test]
    fn login_code_is_single_use() {
        let codes = LoginCodes::default();
        let code = codes.issue();
        assert!(codes.redeem(&code.to_ascii_uppercase()));
        assert!(!codes.redeem(&code));
        assert!(!codes.redeem("unknown"));
    }

    #[test]
    fn token_file_is_created_once_and_reused() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_token(dir.path()).unwrap(), None);
        let first = load_or_create_token(dir.path()).unwrap();
        assert_eq!(first.len(), TOKEN_BYTES * 2);
        assert_eq!(load_or_create_token(dir.path()).unwrap(), first);
        let rotated = write_new_token(dir.path()).unwrap();
        assert_ne!(rotated, first);
        assert_eq!(read_token(dir.path()).unwrap(), Some(rotated));
    }

    #[test]
    fn issued_login_codes_are_recognized() {
        let code = LoginCodes::default().issue();
        assert!(is_login_code(&code));
        assert!(!is_login_code(&code.to_ascii_uppercase()));
        assert!(!is_login_code(&code[1..]));
        assert!(!is_login_code(&format!("{}\x1b", &code[1..])));
        assert!(!is_login_code(""));
    }

    #[cfg(unix)]
    #[test]
    fn broader_than_reports_modes_beyond_the_allowed_bits() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        std::fs::write(&file, "x").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(broader_than(&file, 0o600), None);
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(broader_than(&file, 0o600), Some(0o644));
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o400)).unwrap();
        assert_eq!(broader_than(&file, 0o600), None);
        assert_eq!(broader_than(&dir.path().join("missing"), 0o600), None);
    }

    #[cfg(unix)]
    #[test]
    fn token_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        load_or_create_token(dir.path()).unwrap();
        let mode = std::fs::metadata(token_path(dir.path()))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn create_private_dir_all_creates_new_dirs_as_0700() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b");
        create_private_dir_all(&nested).unwrap();
        for p in [dir.path().join("a"), nested] {
            let mode = std::fs::metadata(&p).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700, "{}", p.display());
        }
    }

    #[cfg(unix)]
    #[test]
    fn create_private_dir_all_leaves_a_preexisting_dir_untouched() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("state");
        std::fs::create_dir(&existing).unwrap();
        std::fs::set_permissions(&existing, std::fs::Permissions::from_mode(0o755)).unwrap();
        create_private_dir_all(&existing).unwrap();
        let mode = std::fs::metadata(&existing).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }
}
