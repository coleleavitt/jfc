use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const REFRESH_BUFFER_SECS: u64 = 300;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthMethod {
    ApiKey {
        key: String,
    },
    OAuth {
        access_token: String,
        refresh_token: String,
        expires_at: u64,
        account_id: Option<String>,
    },
    None,
}

impl AuthMethod {
    pub fn is_expired_or_expiring(&self, now_secs: u64) -> bool {
        match self {
            Self::OAuth { expires_at, .. } => {
                *expires_at <= now_secs.saturating_add(REFRESH_BUFFER_SECS)
            }
            _ => false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PkcePair {
    pub code_verifier: String,
    pub code_challenge: String,
}

pub fn generate_code_verifier() -> String {
    let mut bytes = [0u8; 64];
    for byte in &mut bytes {
        *byte = rand::random::<u8>();
    }
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub fn generate_code_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

pub fn generate_pkce() -> PkcePair {
    let code_verifier = generate_code_verifier();
    let code_challenge = generate_code_challenge(&code_verifier);
    PkcePair {
        code_verifier,
        code_challenge,
    }
}

pub fn generate_state() -> String {
    let mut bytes = [0u8; 32];
    for byte in &mut bytes {
        *byte = rand::random::<u8>();
    }
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct TokenStoreFile {
    #[serde(flatten)]
    pub providers: BTreeMap<String, AuthMethod>,
}

#[derive(Debug, Clone)]
pub struct TokenStore {
    path: PathBuf,
}

impl TokenStore {
    pub fn default_path() -> PathBuf {
        dirs::data_dir()
            .or_else(dirs::config_dir)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("jfc")
            .join("auth.json")
    }

    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> std::io::Result<TokenStoreFile> {
        match std::fs::File::open(&self.path) {
            Ok(mut file) => {
                let mut raw = String::new();
                file.read_to_string(&mut raw)?;
                serde_json::from_str(&raw).map_err(std::io::Error::other)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(TokenStoreFile::default()),
            Err(e) => Err(e),
        }
    }

    pub fn get(&self, provider: &str) -> std::io::Result<Option<AuthMethod>> {
        Ok(self.load()?.providers.get(provider).cloned())
    }

    pub fn set(&self, provider: &str, auth: AuthMethod) -> std::io::Result<()> {
        let mut store = self.load()?;
        store.providers.insert(provider.to_owned(), auth);
        self.save(&store)
    }

    pub fn remove(&self, provider: &str) -> std::io::Result<bool> {
        let mut store = self.load()?;
        let removed = store.providers.remove(provider).is_some();
        self.save(&store)?;
        Ok(removed)
    }

    pub fn save(&self, store: &TokenStoreFile) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_vec_pretty(store).map_err(std::io::Error::other)?;
        let tmp = self.path.with_extension("json.tmp");
        {
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            let mut file = opts.open(&tmp)?;
            file.write_all(&json)?;
            file.flush()?;
            file.sync_all().ok();
        }
        std::fs::rename(&tmp, &self.path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

pub fn bearer_headers(token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Ok(value) = HeaderValue::from_str(&format!("Bearer {token}")) {
        headers.insert(reqwest::header::AUTHORIZATION, value);
    }
    headers
}

pub fn jwt_claim(jwt: &str, names: &[&str]) -> Option<String> {
    let payload = jwt.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.as_bytes())
        .ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    for name in names {
        if let Some(s) = value.get(*name).and_then(|v| v.as_str()) {
            return Some(s.to_owned());
        }
    }
    if let Some(org) = value
        .get("organizations")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|v| v.get("id"))
        .and_then(|v| v.as_str())
    {
        return Some(org.to_owned());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_matches_rfc_vector() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            generate_code_challenge(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn token_store_round_trips_provider_keyed_auth() {
        let dir = tempfile::tempdir().unwrap();
        let store = TokenStore::new(dir.path().join("auth.json"));
        store
            .set(
                "codex",
                AuthMethod::OAuth {
                    access_token: "at".into(),
                    refresh_token: "rt".into(),
                    expires_at: 42,
                    account_id: Some("acct".into()),
                },
            )
            .unwrap();
        let loaded = store.get("codex").unwrap().unwrap();
        assert_eq!(
            loaded,
            AuthMethod::OAuth {
                access_token: "at".into(),
                refresh_token: "rt".into(),
                expires_at: 42,
                account_id: Some("acct".into()),
            }
        );
    }

    #[test]
    fn oauth_expiry_uses_five_minute_buffer() {
        let auth = AuthMethod::OAuth {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            expires_at: 1_300,
            account_id: None,
        };
        assert!(auth.is_expired_or_expiring(1_000));
        assert!(!auth.is_expired_or_expiring(900));
    }
}
