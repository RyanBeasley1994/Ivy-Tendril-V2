//! API keys for the public API (`/api/public/v1`).
//!
//! These are deliberately separate from the daemon's master secret: that credential can do anything,
//! while a key here can only read project status, and (if it was created with the `write` scope)
//! message a project's manager. Keys are named, revocable, and stored only as a SHA-256 hash in
//! `api-keys.json` in the Tendril home, so the file leaking does not leak a usable key. The key itself
//! is shown once, when it is created. A key is 256 random bits, which is why an unsalted fast hash is
//! the right tool here (there is nothing to brute-force).

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use ring::digest::{digest, SHA256};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const KEY_PREFIX: &str = "fk_";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// Project list, progress and the manager's conversation.
    Read,
    /// Everything `Read` does, and sending the manager a message.
    Write,
}

impl Scope {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "read" => Some(Self::Read),
            "write" => Some(Self::Write),
            _ => None,
        }
    }

    pub fn allows_write(self) -> bool {
        self == Self::Write
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KeyRecord {
    pub id: String,
    pub name: String,
    pub scope: Scope,
    /// SHA-256 of the full key, hex.
    pub hash: String,
    /// The first characters of the key, so a list can say which one is which.
    pub prefix: String,
    pub created: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    keys: Vec<KeyRecord>,
}

fn path(home: &Path) -> PathBuf {
    home.join("api-keys.json")
}

fn load(home: &Path) -> Store {
    std::fs::read_to_string(path(home))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save(home: &Path, store: &Store) -> Result<(), String> {
    let raw = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
    let target = path(home);
    let tmp = target.with_extension("json.tmp");
    std::fs::write(&tmp, raw).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, &target).map_err(|e| e.to_string())
}

fn hash_hex(key: &str) -> String {
    digest(&SHA256, key.as_bytes())
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Creates a key. The returned string is the only time it is available in the clear.
pub fn create(home: &Path, name: &str, scope: Scope) -> Result<(KeyRecord, String), String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 60 {
        return Err("a key needs a name of up to 60 characters".into());
    }
    let mut store = load(home);
    if store.keys.iter().any(|k| k.name.eq_ignore_ascii_case(name)) {
        return Err(format!("a key named '{name}' already exists"));
    }
    let mut bytes = [0u8; 32];
    SystemRandom::new().fill(&mut bytes).map_err(|_| "could not generate a key".to_string())?;
    let key = format!("{KEY_PREFIX}{}", B64.encode(bytes));
    let mut id_bytes = [0u8; 6];
    SystemRandom::new().fill(&mut id_bytes).map_err(|_| "could not generate an id".to_string())?;
    let record = KeyRecord {
        id: id_bytes.iter().map(|b| format!("{b:02x}")).collect(),
        name: name.to_string(),
        scope,
        hash: hash_hex(&key),
        prefix: key.chars().take(KEY_PREFIX.len() + 4).collect(),
        created: chrono::Utc::now().to_rfc3339(),
    };
    store.keys.push(record.clone());
    save(home, &store)?;
    Ok((record, key))
}

pub fn list(home: &Path) -> Vec<KeyRecord> {
    load(home).keys
}

/// Revokes by id or name. Returns whether anything was removed.
pub fn revoke(home: &Path, id_or_name: &str) -> Result<bool, String> {
    let mut store = load(home);
    let before = store.keys.len();
    store.keys.retain(|k| k.id != id_or_name && !k.name.eq_ignore_ascii_case(id_or_name));
    if store.keys.len() == before {
        return Ok(false);
    }
    save(home, &store)?;
    Ok(true)
}

/// The record a presented key belongs to, if any. Compares in constant time.
pub fn verify(home: &Path, presented: &str) -> Option<KeyRecord> {
    if !presented.starts_with(KEY_PREFIX) || presented.len() > 128 {
        return None;
    }
    let wanted = hash_hex(presented);
    load(home)
        .keys
        .into_iter()
        .find(|k| crate::auth::secrets_match(&k.hash, &wanted))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("forge-keys-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_created_key_verifies_and_only_its_hash_is_stored() {
        let home = home();
        let (record, key) = create(&home, "ci", Scope::Read).unwrap();
        assert!(key.starts_with("fk_") && key.len() > 40);
        assert_eq!(verify(&home, &key).unwrap().id, record.id);
        let on_disk = std::fs::read_to_string(path(&home)).unwrap();
        assert!(!on_disk.contains(&key), "the key itself is never written");
        assert!(on_disk.contains(&record.hash));
    }

    #[test]
    fn wrong_malformed_and_revoked_keys_do_not_verify() {
        let home = home();
        let (record, key) = create(&home, "bot", Scope::Write).unwrap();
        assert!(verify(&home, "fk_nope").is_none());
        assert!(verify(&home, "").is_none());
        assert!(verify(&home, &key.replace("fk_", "xx_")).is_none());
        assert!(revoke(&home, &record.id).unwrap());
        assert!(verify(&home, &key).is_none());
        assert!(!revoke(&home, &record.id).unwrap());
    }

    #[test]
    fn names_are_unique_and_revoke_works_by_name() {
        let home = home();
        create(&home, "Phone", Scope::Read).unwrap();
        assert!(create(&home, "phone", Scope::Write).is_err());
        assert!(create(&home, "  ", Scope::Read).is_err());
        assert!(revoke(&home, "PHONE").unwrap());
        assert!(list(&home).is_empty());
    }

    #[test]
    fn scopes_parse() {
        assert_eq!(Scope::parse("Read"), Some(Scope::Read));
        assert_eq!(Scope::parse("write"), Some(Scope::Write));
        assert_eq!(Scope::parse("admin"), None);
        assert!(Scope::Write.allows_write() && !Scope::Read.allows_write());
    }
}
