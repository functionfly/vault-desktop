//! Zero-knowledge vault crypto.
//!
//! This is a byte-exact port of the canonical client contract shared by the
//! dashboard (`web/dashboard/src/utils/vault-crypto.ts`) and the Go SDK
//! (`sdk/go-vault-sdk/vault/crypto.go`). Any value encrypted here MUST decrypt
//! in the browser dashboard, and vice versa — `tests::interop` pins this.
//!
//! Wire format:
//!   - KDF         : PBKDF2-HMAC-SHA256, per-secret random 16-byte salt
//!   - Cipher      : AES-256-GCM, random 12-byte IV per message
//!   - key_version : 1 = 100_000 iterations (legacy), 2 = 600_000 (current)
//!   - Ciphertext  : base64(ct || tag) — the GCM tag is APPENDED to the
//!                   ciphertext bytes and the `tag` field is left EMPTY.
//!
//! The inline-tag layout is not an accident: WebCrypto's
//! `subtle.encrypt` returns `ct||tag`, so the dashboard writes it that way and
//! the server round-trips the field verbatim. A client that emits a detached
//! tag still interoperates because the server stores it, so we also accept and
//! normalize the detached form on read (see [`EncryptedData::normalized`]).

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde::{Deserialize, Serialize};

use aes_gcm::aead::{Aead, KeyInit};
use sha2::Sha256;

/// PBKDF2 iteration counts. MUST match vault-crypto.ts exactly.
pub const PBKDF2_ITERATIONS_V1: u32 = 100_000; // legacy; existing secrets stay readable
pub const PBKDF2_ITERATIONS_V2: u32 = 600_000; // current; all new secrets

pub const SALT_LENGTH: usize = 16; // 128-bit
pub const IV_LENGTH: usize = 12; // 96-bit nonce, the AES-GCM standard
pub const KEY_LENGTH: usize = 32; // AES-256
pub const AUTH_TAG_LENGTH: usize = 16; // GCM tag size

pub const CURRENT_KEY_VERSION: i64 = 2;
pub const LEGACY_KEY_VERSION: i64 = 1;

/// Errors surfaced to the UI. Deliberately coarse: decryption failures must
/// not tell an attacker whether the salt was wrong or the tag was forged.
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("unsupported key_version {0} (this client supports 1 and 2)")]
    UnsupportedKeyVersion(i64),
    #[error("missing salt: cannot derive key")]
    MissingSalt,
    #[error("payload is malformed: {0}")]
    Malformed(String),
    #[error("decryption failed: wrong passphrase or corrupted payload")]
    DecryptFailed,
    #[error("encryption failed: {0}")]
    EncryptFailed(String),
    #[error("random source unavailable: {0}")]
    Random(String),
}

/// Resolves the PBKDF2 iteration count for a payload, rejecting unknown
/// versions rather than silently defaulting to current.
fn iterations_for_key_version(key_version: i64) -> Result<u32, CryptoError> {
    match key_version {
        LEGACY_KEY_VERSION => Ok(PBKDF2_ITERATIONS_V1),
        CURRENT_KEY_VERSION => Ok(PBKDF2_ITERATIONS_V2),
        other => Err(CryptoError::UnsupportedKeyVersion(other)),
    }
}

/// The universal encrypted blob exchanged with the API.
///
/// Mirrors `EncryptedDataPayload` in
/// `internal/api/handlers/vault/types.go`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct EncryptedData {
    /// base64 of ciphertext ‖ auth_tag.
    pub ciphertext: String,
    /// base64 of the 12-byte nonce.
    pub iv: String,
    /// base64 of the 16-byte PBKDF2 salt.
    pub salt: String,
    /// Always empty for values written by this client. Populated only by
    /// Go-native writers; see [`EncryptedData::normalized`].
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tag: String,
    pub key_version: i64,
}

/// Same as [`EncryptedData`] plus the plaintext type, for the reveal path.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DecryptedSecret {
    #[serde(flatten)]
    pub encrypted: EncryptedData,
    /// Never persisted, never logged, never sent back to the server.
    pub plaintext: String,
}

impl EncryptedData {
    /// Normalizes a detached-tag payload into the inline-tag layout the rest
    /// of this module assumes.
    ///
    /// Payloads already inline pass through untouched. This is the single
    /// place the two layouts are reconciled; everything downstream can then
    /// assume `ciphertext = base64(ct||tag)`.
    pub fn normalized(&self) -> Result<EncryptedData, CryptoError> {
        if self.tag.is_empty() {
            return Ok(self.clone());
        }
        let ct = B64
            .decode(&self.ciphertext)
            .map_err(|e| CryptoError::Malformed(format!("ciphertext: {e}")))?;
        let tag = B64
            .decode(&self.tag)
            .map_err(|e| CryptoError::Malformed(format!("tag: {e}")))?;
        if tag.len() != AUTH_TAG_LENGTH {
            return Err(CryptoError::Malformed(format!(
                "tag length {} (want {AUTH_TAG_LENGTH})",
                tag.len()
            )));
        }
        let mut combined = Vec::with_capacity(ct.len() + tag.len());
        combined.extend_from_slice(&ct);
        combined.extend_from_slice(&tag);
        Ok(EncryptedData {
            ciphertext: B64.encode(combined),
            iv: self.iv.clone(),
            salt: self.salt.clone(),
            tag: String::new(),
            key_version: self.key_version,
        })
    }

    /// Approximate plaintext-independent byte cost, used to enforce the
    /// server's 64 KiB payload cap client-side before we waste a round trip.
    pub fn approx_encoded_len(&self) -> usize {
        self.ciphertext.len() + self.iv.len() + self.salt.len() + self.tag.len()
    }
}

/// Encrypts `plaintext` under a fresh random salt and IV at the current key
/// version. The result is wire-ready for `POST /v1/vault/secrets`.
pub fn encrypt(plaintext: &str, passphrase: &str) -> Result<EncryptedData, CryptoError> {
    let salt = random_bytes(SALT_LENGTH)?;
    let iv = random_bytes(IV_LENGTH)?;
    let key = derive_key(passphrase, &salt, PBKDF2_ITERATIONS_V2);

    let sealed = aes_gcm_seal(&key, &iv, plaintext.as_bytes())?;

    Ok(EncryptedData {
        // IMPORTANT: tag appended to ciphertext, `tag` left empty — this is
        // what the dashboard writes and therefore what the server stores.
        ciphertext: B64.encode(&sealed),
        iv: B64.encode(&iv),
        salt: B64.encode(&salt),
        tag: String::new(),
        key_version: CURRENT_KEY_VERSION,
    })
}

/// Decrypts a payload. Iterations are resolved from `key_version` so legacy
/// v1 secrets remain readable.
pub fn decrypt(data: &EncryptedData, passphrase: &str) -> Result<String, CryptoError> {
    let data = data.normalized()?;
    if data.salt.is_empty() {
        return Err(CryptoError::MissingSalt);
    }
    let iterations = iterations_for_key_version(data.key_version)?;
    let salt = B64
        .decode(&data.salt)
        .map_err(|e| CryptoError::Malformed(format!("salt: {e}")))?;
    let key = derive_key(passphrase, &salt, iterations);
    decrypt_inner(&data, &key)
}

fn decrypt_inner(data: &EncryptedData, key: &[u8]) -> Result<String, CryptoError> {
    let sealed = B64
        .decode(&data.ciphertext)
        .map_err(|e| CryptoError::Malformed(format!("ciphertext: {e}")))?;
    let iv = B64
        .decode(&data.iv)
        .map_err(|e| CryptoError::Malformed(format!("iv: {e}")))?;
    if iv.len() != IV_LENGTH {
        return Err(CryptoError::Malformed(format!(
            "iv length {} (want {IV_LENGTH})",
            iv.len()
        )));
    }
    // A ciphertext shorter than the tag cannot possibly be authentic.
    if sealed.len() < AUTH_TAG_LENGTH {
        return Err(CryptoError::Malformed(format!(
            "sealed blob {} bytes is shorter than the {AUTH_TAG_LENGTH}-byte tag",
            sealed.len()
        )));
    }
    let plain = aes_gcm_open(key, &iv, &sealed)?;
    String::from_utf8(plain).map_err(|_| CryptoError::DecryptFailed)
}

/// Derives a 32-byte AES key via PBKDF2-HMAC-SHA256.
///
/// Note this is intentionally CPU-expensive (600k iterations ≈ 0.3–1 s on
/// commodity hardware). Callers on the UI thread must offload to
/// `spawn_blocking` — see `commands::unlock_vault`.
pub fn derive_key(passphrase: &str, salt: &[u8], iterations: u32) -> [u8; KEY_LENGTH] {
    let mut key = [0u8; KEY_LENGTH];
    pbkdf2::pbkdf2_hmac::<Sha256>(passphrase.as_bytes(), salt, iterations, &mut key);
    key
}

/// Seals a 32-byte DEK under a key derived from `kek_passphrase` + a tenant
/// salt. Used to keep the tenant DEK out of the webview while still allowing
/// per-secret decryption to happen in the Rust core.
pub fn wrap_dek(
    dek: &[u8],
    kek_passphrase: &str,
    tenant_salt_b64: &str,
) -> Result<(String, String), CryptoError> {
    if dek.len() != KEY_LENGTH {
        return Err(CryptoError::Malformed(format!(
            "DEK must be {KEY_LENGTH} bytes, got {}",
            dek.len()
        )));
    }
    let salt = B64
        .decode(tenant_salt_b64)
        .map_err(|e| CryptoError::Malformed(format!("tenant salt: {e}")))?;
    let key = derive_key(kek_passphrase, &salt, PBKDF2_ITERATIONS_V2);
    let iv = random_bytes(IV_LENGTH)?;
    let sealed =
        aes_gcm_seal(&key, &iv, dek).map_err(|e| CryptoError::EncryptFailed(e.to_string()))?;
    Ok((B64.encode(&sealed), B64.encode(&iv)))
}

/// Unwraps a DEK previously sealed by [`wrap_dek`].
pub fn unwrap_dek(
    wrapped_b64: &str,
    wrap_iv_b64: &str,
    kek_passphrase: &str,
    tenant_salt_b64: &str,
) -> Result<Vec<u8>, CryptoError> {
    let salt = B64
        .decode(tenant_salt_b64)
        .map_err(|e| CryptoError::Malformed(format!("tenant salt: {e}")))?;
    let key = derive_key(kek_passphrase, &salt, PBKDF2_ITERATIONS_V2);
    let sealed = B64
        .decode(wrapped_b64)
        .map_err(|e| CryptoError::Malformed(format!("wrapped: {e}")))?;
    let iv = B64
        .decode(wrap_iv_b64)
        .map_err(|e| CryptoError::Malformed(format!("iv: {e}")))?;
    aes_gcm_open(&key, &iv, &sealed)
}

/// Zeroizes a buffer in place. Best-effort by nature — Rust's optimizer may
/// have already copied the bytes — but it removes the obvious exposure window
/// and documents intent.
pub fn zeroize(buf: &mut [u8]) {
    for b in buf.iter_mut() {
        // SAFETY: single-byte volatile write; no aliasing or lifetime issues.
        unsafe { core::ptr::write_volatile(b, 0u8) };
    }
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
}

fn random_bytes(n: usize) -> Result<Vec<u8>, CryptoError> {
    let mut v = vec![0u8; n];
    getrandom::fill(&mut v).map_err(|e| CryptoError::Random(e.to_string()))?;
    Ok(v)
}

fn aes_gcm_seal(key: &[u8], iv: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let cipher = aes_gcm::Aes256Gcm::new_from_slice(key)
        .map_err(|e| CryptoError::EncryptFailed(format!("new_from_slice: {e}")))?;
    // `Nonce::try_from` (not the deprecated `from_slice`) validates the IV
    // length on construction, so we never hand the cipher a 13-byte nonce
    // and rely on it to misbehave. The 12-byte length is enforced by the
    // `IV_LENGTH` constant in the caller; this is the second line of
    // defence that surfaces a programmer error loudly.
    let nonce =
        aes_gcm::Nonce::try_from(iv).map_err(|e| CryptoError::Malformed(format!("iv: {e}")))?;
    cipher
        .encrypt(&nonce, plaintext)
        .map_err(|e| CryptoError::EncryptFailed(format!("seal: {e}")))
}

fn aes_gcm_open(key: &[u8], iv: &[u8], sealed: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let cipher = aes_gcm::Aes256Gcm::new_from_slice(key)
        .map_err(|e| CryptoError::EncryptFailed(format!("new_from_slice: {e}")))?;
    let nonce =
        aes_gcm::Nonce::try_from(iv).map_err(|e| CryptoError::Malformed(format!("iv: {e}")))?;
    cipher
        .decrypt(&nonce, sealed)
        .map_err(|_| CryptoError::DecryptFailed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_at_current_key_version() {
        let enc = encrypt("hunter2", "correct horse battery staple").unwrap();
        assert_eq!(enc.key_version, CURRENT_KEY_VERSION);
        assert!(enc.tag.is_empty(), "inline-tag layout leaves tag empty");
        assert_eq!(
            decrypt(&enc, "correct horse battery staple").unwrap(),
            "hunter2"
        );
    }

    #[test]
    fn wrong_passphrase_fails_closed() {
        let enc = encrypt("hunter2", "right").unwrap();
        let err = decrypt(&enc, "wrong").unwrap_err();
        assert!(matches!(err, CryptoError::DecryptFailed));
    }

    #[test]
    fn salt_and_iv_are_unique_per_message() {
        let a = encrypt("same", "pw").unwrap();
        let b = encrypt("same", "pw").unwrap();
        assert_ne!(a.salt, b.salt, "salt must not repeat");
        assert_ne!(a.iv, b.iv, "IV must not repeat under one key");
        assert_ne!(a.ciphertext, b.ciphertext);
    }

    #[test]
    fn legacy_v1_payloads_remain_readable() {
        // Hand-build a v1 payload: 100k iterations, inline tag.
        let salt = random_bytes(SALT_LENGTH).unwrap();
        let key = derive_key("legacy-pw", &salt, PBKDF2_ITERATIONS_V1);
        let iv = random_bytes(IV_LENGTH).unwrap();
        let sealed = aes_gcm_seal(&key, &iv, b"old secret").unwrap();
        let payload = EncryptedData {
            ciphertext: B64.encode(&sealed),
            iv: B64.encode(&iv),
            salt: B64.encode(&salt),
            tag: String::new(),
            key_version: LEGACY_KEY_VERSION,
        };
        assert_eq!(decrypt(&payload, "legacy-pw").unwrap(), "old secret");
    }

    #[test]
    fn unknown_key_version_is_rejected_not_guessed() {
        let payload = EncryptedData {
            ciphertext: B64.encode(b"x"),
            iv: B64.encode(&[0u8; 12]),
            salt: B64.encode(&[0u8; 16]),
            tag: String::new(),
            key_version: 99,
        };
        assert!(matches!(
            decrypt(&payload, "pw"),
            Err(CryptoError::UnsupportedKeyVersion(99))
        ));
    }

    #[test]
    fn detached_tag_is_normalized_to_inline() {
        let salt = random_bytes(SALT_LENGTH).unwrap();
        let key = derive_key("pw", &salt, PBKDF2_ITERATIONS_V2);
        let iv = random_bytes(IV_LENGTH).unwrap();
        let sealed = aes_gcm_seal(&key, &iv, b"split tag").unwrap();
        let (ct, tag) = sealed.split_at(sealed.len() - AUTH_TAG_LENGTH);

        // Go-native writer: ciphertext and tag in separate fields.
        let detached = EncryptedData {
            ciphertext: B64.encode(ct),
            iv: B64.encode(&iv),
            salt: B64.encode(&salt),
            tag: B64.encode(tag),
            key_version: CURRENT_KEY_VERSION,
        };
        assert_eq!(decrypt(&detached, "pw").unwrap(), "split tag");

        let normalized = detached.normalized().unwrap();
        assert!(normalized.tag.is_empty());
        assert_eq!(normalized.ciphertext, B64.encode(&sealed));
    }

    #[test]
    fn tampered_ciphertext_is_rejected_by_the_auth_tag() {
        let mut enc = encrypt("integrity", "pw").unwrap();
        let mut sealed = B64.decode(&enc.ciphertext).unwrap();
        sealed[0] ^= 0x01; // flip one bit of the plaintext portion
        enc.ciphertext = B64.encode(&sealed);
        assert!(matches!(
            decrypt(&enc, "pw"),
            Err(CryptoError::DecryptFailed)
        ));
    }

    #[test]
    fn wrong_length_tag_is_rejected() {
        let bad = EncryptedData {
            ciphertext: B64.encode(b"ct"),
            iv: B64.encode(&[0u8; 12]),
            salt: B64.encode(&[0u8; 16]),
            tag: B64.encode([0u8; 8]),
            key_version: CURRENT_KEY_VERSION,
        };
        assert!(matches!(bad.normalized(), Err(CryptoError::Malformed(_))));
    }

    #[test]
    fn missing_salt_is_reported_distinctly() {
        let enc = EncryptedData {
            ciphertext: B64.encode(b"ct"),
            iv: B64.encode(&[0u8; 12]),
            salt: String::new(),
            tag: String::new(),
            key_version: CURRENT_KEY_VERSION,
        };
        assert!(matches!(decrypt(&enc, "pw"), Err(CryptoError::MissingSalt)));
    }

    #[test]
    fn dek_wrap_roundtrip() {
        let dek = random_bytes(KEY_LENGTH).unwrap();
        let salt_b64 = B64.encode(random_bytes(SALT_LENGTH).unwrap());
        let (wrapped, iv) = wrap_dek(&dek, "kek-pw", &salt_b64).unwrap();
        let out = unwrap_dek(&wrapped, &iv, "kek-pw", &salt_b64).unwrap();
        assert_eq!(out, dek);
    }

    #[test]
    fn dek_wrap_rejects_wrong_length_dek() {
        assert!(wrap_dek(&[0u8; 16], "kek", &B64.encode([0u8; 16])).is_err());
    }

    #[test]
    fn zeroize_clears_buffer() {
        let mut buf = vec![0xABu8; 32];
        zeroize(&mut buf);
        assert!(buf.iter().all(|&b| b == 0));
    }

    #[test]
    fn empty_and_unicode_plaintext_roundtrip() {
        for case in ["", "pässwörd-🔐-日本語", &"x".repeat(50_000)] {
            let enc = encrypt(case, "pw").unwrap();
            assert_eq!(decrypt(&enc, "pw").unwrap(), case);
        }
    }
}
