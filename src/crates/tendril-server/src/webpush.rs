//! Browser push (the Web Push protocol) so the installed web app on a phone gets the same
//! notifications ntfy does, with nothing but the daemon and the browser's own push service involved.
//!
//! * [RFC 8292] (VAPID): the daemon holds a P-256 key pair in `webpush.json`; its public half is what a
//!   browser subscribes against, and a short-lived JWT signed with its private half tells the push
//!   service who is sending.
//! * [RFC 8291] (`aes128gcm`): each message is encrypted for one subscription with an ephemeral key
//!   agreement against the browser's public key and its auth secret, so the push service only ever
//!   carries ciphertext.
//!
//! Subscriptions the push service reports gone (404/410) are dropped on the spot.
//!
//! [RFC 8292]: https://www.rfc-editor.org/rfc/rfc8292
//! [RFC 8291]: https://www.rfc-editor.org/rfc/rfc8291

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use ring::aead::{self, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::agreement::{self, EphemeralPrivateKey, UnparsedPublicKey};
use ring::hkdf;
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// One browser's subscription, as `PushSubscription.toJSON()` reports it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Subscription {
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub created: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Store {
    /// The VAPID private key, PKCS#8, base64url.
    #[serde(default)]
    vapid_pkcs8: String,
    #[serde(default)]
    subscriptions: Vec<Subscription>,
}

/// Serialises read-modify-write of the store across request handlers and the sender.
static LOCK: Mutex<()> = Mutex::new(());

fn path(home: &Path) -> PathBuf {
    home.join("webpush.json")
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

/// The store with a VAPID key in it, minting one the first time.
fn load_with_key(home: &Path) -> Result<Store, String> {
    let mut store = load(home);
    if store.vapid_pkcs8.is_empty() {
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &SystemRandom::new())
            .map_err(|_| "could not generate a VAPID key".to_string())?;
        store.vapid_pkcs8 = B64.encode(pkcs8.as_ref());
        save(home, &store)?;
    }
    Ok(store)
}

fn key_pair(store: &Store) -> Result<EcdsaKeyPair, String> {
    let der = B64.decode(&store.vapid_pkcs8).map_err(|e| e.to_string())?;
    EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &der, &SystemRandom::new())
        .map_err(|_| "the stored VAPID key is unreadable".to_string())
}

/// What the browser subscribes against (`applicationServerKey`): the uncompressed public point.
pub fn public_key(home: &Path) -> Result<String, String> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let store = load_with_key(home)?;
    Ok(B64.encode(key_pair(&store)?.public_key().as_ref()))
}

pub fn subscription_count(home: &Path) -> usize {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    load(home).subscriptions.len()
}

/// Adds or refreshes a subscription (matched by endpoint).
pub fn subscribe(home: &Path, sub: Subscription) -> Result<(), String> {
    // Reject what could not possibly decrypt before it is stored.
    let p256dh = B64.decode(sub.p256dh.trim_end_matches('=')).map_err(|_| "p256dh is not base64url".to_string())?;
    let auth = B64.decode(sub.auth.trim_end_matches('=')).map_err(|_| "auth is not base64url".to_string())?;
    if p256dh.len() != 65 || p256dh[0] != 4 || auth.len() != 16 {
        return Err("that is not a valid push subscription".into());
    }
    if !sub.endpoint.starts_with("https://") {
        return Err("a push endpoint must be https".into());
    }
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut store = load_with_key(home)?;
    store.subscriptions.retain(|s| s.endpoint != sub.endpoint);
    store.subscriptions.push(sub);
    save(home, &store)
}

pub fn unsubscribe(home: &Path, endpoint: &str) -> Result<(), String> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut store = load(home);
    let before = store.subscriptions.len();
    store.subscriptions.retain(|s| s.endpoint != endpoint);
    if store.subscriptions.len() == before {
        return Ok(());
    }
    save(home, &store)
}

// ---- encryption (RFC 8291) ---------------------------------------------------------------------

struct Len(usize);
impl hkdf::KeyType for Len {
    fn len(&self) -> usize {
        self.0
    }
}

fn expand(prk: &hkdf::Prk, info: &[&[u8]], out: &mut [u8]) -> Result<(), String> {
    prk.expand(info, Len(out.len()))
        .and_then(|okm| okm.fill(out))
        .map_err(|_| "key derivation failed".to_string())
}

/// `plaintext` as a single `aes128gcm` record for the subscription's keys.
pub fn encrypt(plaintext: &[u8], ua_public: &[u8], auth_secret: &[u8]) -> Result<Vec<u8>, String> {
    let rng = SystemRandom::new();
    let as_private = EphemeralPrivateKey::generate(&agreement::ECDH_P256, &rng).map_err(|_| "rng failed")?;
    let as_public = as_private.compute_public_key().map_err(|_| "key agreement failed")?;
    let as_public = as_public.as_ref().to_vec();

    let secret = agreement::agree_ephemeral(as_private, &UnparsedPublicKey::new(&agreement::ECDH_P256, ua_public), |s| {
        s.to_vec()
    })
    .map_err(|_| "the subscription's key is not a valid P-256 point".to_string())?;

    // IKM = HKDF(auth_secret, ecdh_secret, "WebPush: info\0" || ua_public || as_public, 32)
    let prk_key = hkdf::Salt::new(hkdf::HKDF_SHA256, auth_secret).extract(&secret);
    let mut key_info = b"WebPush: info\0".to_vec();
    key_info.extend_from_slice(ua_public);
    key_info.extend_from_slice(&as_public);
    let mut ikm = [0u8; 32];
    expand(&prk_key, &[&key_info], &mut ikm)?;

    let mut salt = [0u8; 16];
    rng.fill(&mut salt).map_err(|_| "rng failed")?;
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, &salt).extract(&ikm);
    let mut cek = [0u8; 16];
    expand(&prk, &[b"Content-Encoding: aes128gcm\0"], &mut cek)?;
    let mut nonce = [0u8; 12];
    expand(&prk, &[b"Content-Encoding: nonce\0"], &mut nonce)?;

    // One record, so the padding delimiter is 0x02 and the plaintext must fit what push services allow.
    if plaintext.len() > 3800 {
        return Err("notification payload is too large".into());
    }
    let mut record = plaintext.to_vec();
    record.push(2);
    let key = LessSafeKey::new(UnboundKey::new(&aead::AES_128_GCM, &cek).map_err(|_| "cipher setup failed")?);
    key.seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::empty(), &mut record)
        .map_err(|_| "encryption failed")?;

    let mut body = Vec::with_capacity(16 + 4 + 1 + 65 + record.len());
    body.extend_from_slice(&salt);
    body.extend_from_slice(&4096u32.to_be_bytes());
    body.push(as_public.len() as u8);
    body.extend_from_slice(&as_public);
    body.extend_from_slice(&record);
    Ok(body)
}

/// Reverses `encrypt` for a message addressed to `ua_private`. Used by the tests, standing in for the
/// browser.
#[cfg(test)]
fn decrypt(body: &[u8], ua_private: EphemeralPrivateKey, ua_public: &[u8], auth_secret: &[u8]) -> Vec<u8> {
    let salt = &body[..16];
    let id_len = body[20] as usize;
    let as_public = &body[21..21 + id_len];
    let ciphertext = &body[21 + id_len..];
    let secret = agreement::agree_ephemeral(ua_private, &UnparsedPublicKey::new(&agreement::ECDH_P256, as_public), |s| {
        s.to_vec()
    })
    .unwrap();
    let prk_key = hkdf::Salt::new(hkdf::HKDF_SHA256, auth_secret).extract(&secret);
    let mut key_info = b"WebPush: info\0".to_vec();
    key_info.extend_from_slice(ua_public);
    key_info.extend_from_slice(as_public);
    let mut ikm = [0u8; 32];
    expand(&prk_key, &[&key_info], &mut ikm).unwrap();
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, salt).extract(&ikm);
    let mut cek = [0u8; 16];
    expand(&prk, &[b"Content-Encoding: aes128gcm\0"], &mut cek).unwrap();
    let mut nonce = [0u8; 12];
    expand(&prk, &[b"Content-Encoding: nonce\0"], &mut nonce).unwrap();
    let key = LessSafeKey::new(UnboundKey::new(&aead::AES_128_GCM, &cek).unwrap());
    let mut buf = ciphertext.to_vec();
    let plain = key.open_in_place(Nonce::assume_unique_for_key(nonce), Aad::empty(), &mut buf).unwrap();
    let end = plain.iter().rposition(|b| *b != 0).unwrap();
    assert_eq!(plain[end], 2, "padding delimiter");
    plain[..end].to_vec()
}

// ---- VAPID (RFC 8292) --------------------------------------------------------------------------

fn origin_of(endpoint: &str) -> Option<String> {
    let rest = endpoint.strip_prefix("https://")?;
    let host = rest.split('/').next()?;
    (!host.is_empty()).then(|| format!("https://{host}"))
}

/// The `Authorization` header value for a push to `endpoint`.
fn vapid_authorization(pair: &EcdsaKeyPair, endpoint: &str, now: i64) -> Result<String, String> {
    let aud = origin_of(endpoint).ok_or("push endpoint is not an https URL")?;
    let header = B64.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
    let claims = B64.encode(json!({ "aud": aud, "exp": now + 12 * 3600, "sub": "mailto:forge@localhost" }).to_string());
    let signing_input = format!("{header}.{claims}");
    let signature = pair
        .sign(&SystemRandom::new(), signing_input.as_bytes())
        .map_err(|_| "signing failed".to_string())?;
    Ok(format!(
        "vapid t={signing_input}.{}, k={}",
        B64.encode(signature.as_ref()),
        B64.encode(pair.public_key().as_ref())
    ))
}

// ---- sending -----------------------------------------------------------------------------------

/// What the service worker shows. `tag` makes a newer notification replace an older one for the same
/// project rather than stacking.
#[derive(Debug, Clone, Serialize)]
pub struct Notice<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub urgent: bool,
    pub tag: &'a str,
    pub url: &'a str,
}

/// Delivers `notice` to every subscription. Returns how many accepted it. Failures never propagate:
/// a notification that cannot be delivered must not fail the manager turn that raised it.
pub async fn send_all(home: &Path, notice: &Notice<'_>) -> usize {
    let (pair, subs) = {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Ok(store) = load_with_key(home) else { return 0 };
        let Ok(pair) = key_pair(&store) else { return 0 };
        (pair, store.subscriptions)
    };
    if subs.is_empty() {
        return 0;
    }
    let Ok(payload) = serde_json::to_vec(notice) else { return 0 };
    let Ok(client) = reqwest::Client::builder().timeout(Duration::from_secs(15)).build() else { return 0 };

    let mut delivered = 0;
    let mut gone = Vec::new();
    for sub in &subs {
        let (Ok(p256dh), Ok(auth)) = (B64.decode(sub.p256dh.trim_end_matches('=')), B64.decode(sub.auth.trim_end_matches('=')))
        else {
            gone.push(sub.endpoint.clone());
            continue;
        };
        let Ok(body) = encrypt(&payload, &p256dh, &auth) else { continue };
        let Ok(authorization) = vapid_authorization(&pair, &sub.endpoint, chrono::Utc::now().timestamp()) else {
            continue;
        };
        let response = client
            .post(&sub.endpoint)
            .header("Authorization", authorization)
            .header("Content-Encoding", "aes128gcm")
            .header("Content-Type", "application/octet-stream")
            .header("TTL", "86400")
            .header("Urgency", if notice.urgent { "high" } else { "normal" })
            .body(body)
            .send()
            .await;
        match response {
            Ok(r) if r.status().is_success() => delivered += 1,
            Ok(r) if matches!(r.status().as_u16(), 404 | 410) => gone.push(sub.endpoint.clone()),
            Ok(r) => tracing::warn!("web push to {} answered {}", origin_of(&sub.endpoint).unwrap_or_default(), r.status()),
            Err(e) => tracing::warn!("web push failed: {e}"),
        }
    }
    for endpoint in gone {
        let _ = unsubscribe(home, &endpoint);
    }
    delivered
}

#[cfg(test)]
mod tests {
    use super::*;

    fn browser_keys() -> (EphemeralPrivateKey, Vec<u8>, [u8; 16]) {
        let rng = SystemRandom::new();
        let private = EphemeralPrivateKey::generate(&agreement::ECDH_P256, &rng).unwrap();
        let public = private.compute_public_key().unwrap().as_ref().to_vec();
        let mut auth = [0u8; 16];
        rng.fill(&mut auth).unwrap();
        (private, public, auth)
    }

    #[test]
    fn a_message_decrypts_with_the_subscribers_keys() {
        let (private, public, auth) = browser_keys();
        let body = encrypt("{\"title\":\"coding-env · needs you\"}".as_bytes(), &public, &auth).unwrap();
        assert_eq!(&body[16..20], &4096u32.to_be_bytes());
        assert_eq!(body[20], 65);
        assert_eq!(decrypt(&body, private, &public, &auth), "{\"title\":\"coding-env · needs you\"}".as_bytes());
    }

    #[test]
    fn each_message_uses_a_fresh_salt_and_key() {
        let (_, public, auth) = browser_keys();
        let a = encrypt(b"same", &public, &auth).unwrap();
        let b = encrypt(b"same", &public, &auth).unwrap();
        assert_ne!(a[..16], b[..16]);
        assert_ne!(a, b);
    }

    #[test]
    fn a_bad_subscription_key_is_an_error_not_a_panic() {
        assert!(encrypt(b"x", &[4u8; 65], &[0u8; 16]).is_err());
        assert!(encrypt(b"x", b"short", &[0u8; 16]).is_err());
    }

    #[test]
    fn oversized_payloads_are_refused() {
        let (_, public, auth) = browser_keys();
        assert!(encrypt(&vec![b'x'; 5000], &public, &auth).is_err());
    }

    #[test]
    fn the_vapid_token_verifies_against_the_published_key() {
        let home = tempdir();
        let key = public_key(&home).unwrap();
        let store = load(&home);
        let pair = key_pair(&store).unwrap();
        let header = vapid_authorization(&pair, "https://push.example.net/send/abc", 1_700_000_000).unwrap();

        let rest = header.strip_prefix("vapid t=").unwrap();
        let (jwt, k) = rest.split_once(", k=").unwrap();
        assert_eq!(k, key, "the k parameter is the public key a browser subscribed against");
        let mut parts = jwt.split('.');
        let (h, c, s) = (parts.next().unwrap(), parts.next().unwrap(), parts.next().unwrap());
        let claims: serde_json::Value = serde_json::from_slice(&B64.decode(c).unwrap()).unwrap();
        assert_eq!(claims["aud"], "https://push.example.net");
        assert_eq!(claims["exp"], 1_700_000_000 + 12 * 3600);

        let public = B64.decode(k).unwrap();
        let verifier = ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_FIXED, public);
        verifier.verify(format!("{h}.{c}").as_bytes(), &B64.decode(s).unwrap()).expect("signature verifies");
    }

    #[test]
    fn the_key_is_minted_once_and_kept() {
        let home = tempdir();
        assert_eq!(public_key(&home).unwrap(), public_key(&home).unwrap());
    }

    #[test]
    fn subscriptions_are_stored_replaced_and_removed() {
        let home = tempdir();
        let (_, public, auth) = browser_keys();
        let sub = |label: &str| Subscription {
            endpoint: "https://push.example.net/send/abc".into(),
            p256dh: B64.encode(&public),
            auth: B64.encode(auth),
            label: label.into(),
            created: String::new(),
        };
        subscribe(&home, sub("phone")).unwrap();
        subscribe(&home, sub("phone again")).unwrap();
        assert_eq!(subscription_count(&home), 1, "the same endpoint is one subscription");
        unsubscribe(&home, "https://push.example.net/send/abc").unwrap();
        assert_eq!(subscription_count(&home), 0);
    }

    #[test]
    fn junk_subscriptions_are_rejected() {
        let home = tempdir();
        let bad = Subscription {
            endpoint: "http://insecure.example/x".into(),
            p256dh: B64.encode([4u8; 65]),
            auth: B64.encode([0u8; 16]),
            label: String::new(),
            created: String::new(),
        };
        assert!(subscribe(&home, bad.clone()).is_err());
        let short = Subscription { endpoint: "https://ok.example/x".into(), p256dh: "AAAA".into(), ..bad };
        assert!(subscribe(&home, short).is_err());
        assert_eq!(subscription_count(&home), 0);
    }

    fn tempdir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("forge-webpush-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
