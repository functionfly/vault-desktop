//! Credential storage.
//!
//! Secret material in this app has two very different lifetimes:
//!
//!   * **Long-lived** — the orchestrator bearer + refresh token. Stored in
//!     the OS keychain (macOS Keychain / Windows Credential Manager / libsecret
//!     via D-Bus) through the `keyring` crate when one is reachable. When none
//!     is — headless Linux, WSL without `gnome-keyring-daemon`, distros with no
//!     Secret Service provider — it falls back to a per-installation
//!     AES-256-GCM file store under the platform data dir. Neither path ever
//!     lets plaintext cross into the webview.
//!
//!   * **Session-only** — the vault passphrase that unlocks per-secret
//!     decryption. Deliberately kept in process memory only, zeroized on lock
//!     and on app exit. The passphrase is not the tenant DEK; it is the user's
//!     unlock phrase, and persisting it would defeat the zero-knowledge
//!     premise.
//!
//! The passphrase is held as a `Zeroizing` guard so an accidental clone or a
//! dropped error path does not leave copies lying around.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use aes_gcm::aead::{Aead, KeyInit};
use tokio::sync::Mutex;
use zeroize::{Zeroize, Zeroizing};

use crate::crypto::{IV_LENGTH, KEY_LENGTH};

/// Keychain namespace. Distinct from Studio's `functionfly-studio` so the two
/// apps never collide in the same login keychain.
pub const KEYRING_SERVICE: &str = "functionfly-vault-desktop";

/// Keychain account names. One entry per field so we can rotate a single
/// value without rewriting the others.
pub const KEY_USER_ACCESS: &str = "orchestrator-access-token";
pub const KEY_USER_REFRESH: &str = "orchestrator-refresh-token";
pub const KEY_USER_BASE_URL: &str = "orchestrator-base-url";

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("keychain unavailable: {0}")]
    Keychain(String),
    #[error("no credential stored")]
    NotFound,
    #[error("keychain backend does not support this operation: {0}")]
    Unsupported(String),
    /// The encrypted-file fallback could not read or write. Surfaced to the
    /// UI so the user knows their session will not survive a restart rather
    /// than silently losing it on the next launch.
    #[error("encrypted file store unavailable: {0}")]
    FileFallback(String),
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct SessionMaterial {
    pub base_url: String,
    pub has_access_token: bool,
    pub has_refresh_token: bool,
    /// Where the long-lived credential actually lives. Surfaced to the UI so
    /// the user can see whether their tokens are protected by the OS keyring
    /// or by the AES-GCM file fallback.
    pub backend: BackendKind,
}

/// A credential triple restored from whatever backend is online.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StoredSession {
    pub base_url: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub backend: BackendKind,
}

/// Where a session is persisted. The UI shows this as a small badge so the
/// user can tell at a glance whether their tokens live behind the OS
/// keyring or in the encrypted file fallback (e.g. on a headless Linux box).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    OsKeychain,
    FileFallback,
    /// Nothing persisted yet.
    #[default]
    Empty,
}

pub struct SecretStore {
    /// Serialises keychain mutations so concurrent `save`/`clear` calls cannot
    /// interleave a partial write.
    inner: Mutex<()>,
    /// In-memory unlock phrase. Held as `Zeroizing` so it is wiped on drop
    /// rather than left for the allocator to recycle.
    passphrase: Mutex<Option<Zeroizing<String>>>,
    /// Lazily-initialised file fallback. `Arc<Mutex<Option<...>>>` because the
    /// very first `set/get` decides whether to instantiate it, and we don't
    /// want to fail app boot just because the data dir is read-only or full.
    file: Arc<Mutex<Option<FileFallback>>>,
    /// Most recent `BackendKind` we wrote. Cached so `describe()` can report
    /// the right thing without re-touching the keychain on every render.
    last_backend: Arc<Mutex<BackendKind>>,
}

impl Default for SecretStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(()),
            passphrase: Mutex::new(None),
            file: Arc::new(Mutex::new(None)),
            last_backend: Arc::new(Mutex::new(BackendKind::Empty)),
        }
    }

    // ── long-lived session: keychain first, encrypted file as fallback ────

    pub async fn save_session(
        &self,
        base_url: &str,
        access_token: &str,
        refresh_token: Option<&str>,
    ) -> Result<(), StoreError> {
        let _guard = self.inner.lock().await;

        // Prefer the OS keychain. Only when it is genuinely unreachable
        // (headless Linux, WSL without gnome-keyring-daemon, some distros
        // with no Secret Service provider) do we fall through to the
        // AES-GCM file store — otherwise a user with a working keyring
        // would silently get the weaker storage.
        let via_keychain = (|| -> Result<(), StoreError> {
            set_secret(KEY_USER_BASE_URL, base_url)?;
            set_secret(KEY_USER_ACCESS, access_token)?;
            match refresh_token {
                Some(rt) if !rt.is_empty() => set_secret(KEY_USER_REFRESH, rt)?,
                _ => {
                    // No refresh token offered (e.g. API-key mode). Clear any
                    // stale one so a later refresh attempt fails loudly rather
                    // than silently using someone else's token.
                    let _ = delete_secret(KEY_USER_REFRESH);
                }
            }
            Ok(())
        })();

        if via_keychain.is_ok() {
            self.set_backend(BackendKind::OsKeychain).await;
            // Wipe any stale file-fallback copy so a later sign-out on a
            // keyring-capable host does not resurrect an old session from
            // the weaker store.
            if let Some(fb) = self.file.lock().await.clone() {
                let _ = fb.clear_all();
            }
            return Ok(());
        }

        let fb = self.get_or_init_file_fallback().await?;
        fb.write(KEY_USER_BASE_URL, base_url)?;
        fb.write(KEY_USER_ACCESS, access_token)?;
        match refresh_token {
            Some(rt) if !rt.is_empty() => fb.write(KEY_USER_REFRESH, rt)?,
            _ => {
                let _ = fb.delete(KEY_USER_REFRESH);
            }
        }
        self.set_backend(BackendKind::FileFallback).await;
        Ok(())
    }

    /// Loads the stored session. Returns `None` when nothing is persisted,
    /// which is the normal state on first run.
    ///
    /// Tries the keychain first, then the file fallback. A partially-populated
    /// backend (base URL but no token, or vice versa) is treated as "no
    /// session" rather than an error, so a half-written credential can never
    /// produce an app that boots into a broken state.
    pub async fn load_session(&self) -> Result<Option<StoredSession>, StoreError> {
        let _guard = self.inner.lock().await;

        if let Ok(Some(s)) = try_keychain_load() {
            self.set_backend(BackendKind::OsKeychain).await;
            return Ok(Some(s));
        }

        let fb = self.get_or_init_file_fallback().await?;
        let base_url = match fb.read(KEY_USER_BASE_URL) {
            Ok(v) => v,
            Err(StoreError::NotFound) => return Ok(None),
            Err(e) => return Err(e),
        };
        let access = match fb.read(KEY_USER_ACCESS) {
            Ok(v) => v,
            Err(StoreError::NotFound) => return Ok(None),
            Err(e) => return Err(e),
        };
        let refresh = fb.read(KEY_USER_REFRESH).ok();
        self.set_backend(BackendKind::FileFallback).await;
        Ok(Some(StoredSession {
            base_url,
            access_token: access,
            refresh_token: refresh,
            backend: BackendKind::FileFallback,
        }))
    }

    pub async fn clear_session(&self) -> Result<(), StoreError> {
        let _guard = self.inner.lock().await;
        for account in [KEY_USER_ACCESS, KEY_USER_REFRESH, KEY_USER_BASE_URL] {
            let _ = delete_secret(account);
        }
        if let Some(fb) = self.file.lock().await.clone() {
            for account in [KEY_USER_ACCESS, KEY_USER_REFRESH, KEY_USER_BASE_URL] {
                let _ = fb.delete(account);
            }
        }
        self.set_backend(BackendKind::Empty).await;
        Ok(())
    }

    pub async fn describe(&self) -> Result<SessionMaterial, StoreError> {
        let _guard = self.inner.lock().await;

        // Keychain wins when it has anything at all.
        let base_url = try_keychain_get(KEY_USER_BASE_URL).unwrap_or_default();
        let has_access = try_keychain_get(KEY_USER_ACCESS).is_ok();
        if has_access {
            return Ok(SessionMaterial {
                base_url,
                has_access_token: true,
                has_refresh_token: try_keychain_get(KEY_USER_REFRESH).is_ok(),
                backend: BackendKind::OsKeychain,
            });
        }

        let fb = self.get_or_init_file_fallback().await?;
        let base_url = fb.read(KEY_USER_BASE_URL).unwrap_or_default();
        let has_access = fb.read(KEY_USER_ACCESS).is_ok();
        let has_refresh = fb.read(KEY_USER_REFRESH).is_ok();
        Ok(SessionMaterial {
            base_url,
            has_access_token: has_access,
            has_refresh_token: has_refresh,
            backend: if has_access {
                BackendKind::FileFallback
            } else {
                BackendKind::Empty
            },
        })
    }

    /// Which backend the most recent `save_session` / `load_session` actually
    /// used. The UI renders this as a badge so the user knows whether their
    /// tokens are protected by the OS keyring or the AES-GCM file fallback.
    pub async fn last_backend(&self) -> BackendKind {
        *self.last_backend.lock().await
    }

    async fn set_backend(&self, b: BackendKind) {
        *self.last_backend.lock().await = b;
    }

    async fn get_or_init_file_fallback(&self) -> Result<FileFallback, StoreError> {
        if let Some(fb) = self.file.lock().await.clone() {
            return Ok(fb);
        }
        // Opening the fallback creates the data dir. That can legitimately
        // fail on a read-only home; surface it as a storage error rather
        // than silently pretending the session was saved.
        let fb = FileFallback::open().map_err(StoreError::FileFallback)?;
        *self.file.lock().await = Some(fb.clone());
        Ok(fb)
    }

    // ── memory-only, session-scoped ───────────────────────────────────────

    /// Holds the unlock passphrase for the life of the session.
    ///
    /// `Zeroizing`'s Drop impl wipes the buffer, so replacing an existing
    /// passphrase is sufficient to clear the old one — deliberately no
    /// additional copy of the secret is made anywhere in this path.
    pub async fn lock_passphrase(&self, passphrase: &str) {
        let mut guard = self.passphrase.lock().await;
        *guard = Some(Zeroizing::new(passphrase.to_string()));
    }

    /// Returns the unlock passphrase to the caller for a single derivation.
    ///
    /// The caller is responsible for zeroizing the returned value once the
    /// derived key material exists.
    pub async fn with_passphrase<T, F>(&self, f: F) -> Result<T, StoreError>
    where
        F: FnOnce(&str) -> T,
    {
        let guard = self.passphrase.lock().await;
        match guard.as_ref() {
            // The `Zeroizing<&str>` deref produces `&str`; the caller gets a
            // plain `&str` for ergonomic use while the underlying buffer
            // stays pinned in the guard and is wiped on drop.
            Some(p) => Ok(f(p.as_str())),
            None => Err(StoreError::NotFound),
        }
    }

    pub async fn is_unlocked(&self) -> bool {
        self.passphrase.lock().await.is_some()
    }

    /// Drops the unlock passphrase from memory. Called on explicit lock and on
    /// window close.
    pub async fn lock(&self) {
        let mut guard = self.passphrase.lock().await;
        if let Some(mut p) = guard.take() {
            // `Zeroizing::Zeroize` wipes the inner buffer; the Drop impl on
            // `Zeroizing` is the safety net for paths that skip this call.
            p.zeroize();
        }
    }
}

fn entry(account: &str) -> Result<keyring::Entry, StoreError> {
    keyring::Entry::new(KEYRING_SERVICE, account).map_err(|e| StoreError::Keychain(e.to_string()))
}

fn set_secret(account: &str, value: &str) -> Result<(), StoreError> {
    entry(account)?.set_password(value).map_err(|e| match e {
        keyring::Error::NoEntry => StoreError::NotFound,
        other => StoreError::Keychain(other.to_string()),
    })
}

fn delete_secret(account: &str) -> Result<(), StoreError> {
    match entry(account)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(other) => Err(StoreError::Keychain(other.to_string())),
    }
}

/// Single-field keychain read. `Err(NotFound)` means "this backend has no
/// such entry", which is how `load_session` distinguishes an empty store
/// from an unreachable one.
fn try_keychain_get(account: &str) -> Result<String, StoreError> {
    match entry(account) {
        Ok(e) => match e.get_password() {
            Ok(v) => Ok(v),
            Err(keyring::Error::NoEntry) => Err(StoreError::NotFound),
            Err(other) => Err(StoreError::Keychain(other.to_string())),
        },
        Err(e) => Err(e),
    }
}

/// Loads all three keychain fields together. Used by `load_session` so we
/// either get everything or nothing — a half-written keychain could otherwise
/// pair today's base URL with yesterday's access token.
fn try_keychain_load() -> Result<Option<StoredSession>, StoreError> {
    let base_url = match try_keychain_get(KEY_USER_BASE_URL) {
        Ok(v) => v,
        Err(StoreError::NotFound) => return Ok(None),
        Err(e) => return Err(e),
    };
    let access = match try_keychain_get(KEY_USER_ACCESS) {
        Ok(v) => v,
        Err(StoreError::NotFound) => return Ok(None),
        Err(e) => return Err(e),
    };
    let refresh = try_keychain_get(KEY_USER_REFRESH).ok();
    Ok(Some(StoredSession {
        base_url,
        access_token: access,
        refresh_token: refresh,
        backend: BackendKind::OsKeychain,
    }))
}

// ── encrypted file fallback ───────────────────────────────────────────────
//
// On a host with no reachable OS keychain, the app falls back to a
// per-installation AES-256-GCM store under the platform data dir:
//
//   {data_dir}/functionfly/vault-desktop/
//     machine.key              32-byte AES key, written once, mode 0600
//     accounts/{name}.bin      nonce(12) || ciphertext||tag
//
// This is strictly weaker than the OS keychain — someone with shell access
// can copy `machine.key` and the ciphertexts and decrypt them. But it is
// still encrypted at rest, and it is strictly better than the alternative of
// refusing to store anything, which is what produced the "keychain
// unavailable" dead end this replaces. The keychain is always preferred.

#[derive(Clone)]
struct FileFallback {
    base: PathBuf,
}

impl FileFallback {
    fn open() -> Result<Self, String> {
        let base = data_dir()?.join("functionfly").join("vault-desktop");
        std::fs::create_dir_all(base.join("accounts"))
            .map_err(|e| format!("create_dir_all({}) failed: {e}", base.display()))?;
        // The fallback holds the encrypted session and the machine key that
        // encrypts it. On a multi-user machine the default directory mode
        // (0755 on Unix, inherited ACLs on Windows) is enough for another
        // local account to read the ciphertext and the key material.
        restrict_to_owner(&base)?;
        restrict_to_owner(&base.join("accounts"))?;
        Ok(Self { base })
    }

    fn machine_key_path(&self) -> PathBuf {
        self.base.join("machine.key")
    }

    /// Resolves the on-disk path for an account name.
    ///
    /// Rejects anything containing a path separator or `..` rather than
    /// sanitising it. The only callers today are the constants at the top of
    /// this module, so an unexpected name is a bug or a probe — and refusing
    /// to build a path from it is the safe response. (Pinned by
    /// `file_fallback_rejects_traversal_account_names`.)
    fn account_path(&self, account: &str) -> Result<PathBuf, StoreError> {
        if account.is_empty()
            || account.contains('/')
            || account.contains('\\')
            || account.contains("..")
        {
            return Err(StoreError::FileFallback(format!(
                "refusing to build a path for unsafe account name {account:?}"
            )));
        }
        Ok(self.base.join("accounts").join(format!("{account}.bin")))
    }

    fn read_machine_key(&self) -> Result<[u8; KEY_LENGTH], String> {
        match std::fs::read(self.machine_key_path()) {
            Ok(bytes) if bytes.len() == KEY_LENGTH => {
                let mut k = [0u8; KEY_LENGTH];
                k.copy_from_slice(&bytes);
                Ok(k)
            }
            Ok(bytes) => Err(format!(
                "machine.key is {} bytes, expected {KEY_LENGTH}",
                bytes.len()
            )),
            Err(e) => Err(format!("read machine.key failed: {e}")),
        }
    }

    fn read_or_init_machine_key(&self) -> Result<[u8; KEY_LENGTH], String> {
        if let Ok(k) = self.read_machine_key() {
            return Ok(k);
        }
        let mut k = [0u8; KEY_LENGTH];
        getrandom::fill(&mut k).map_err(|e| format!("getrandom: {e}"))?;
        atomic_write(&self.machine_key_path(), &k)?;
        Ok(k)
    }

    fn write(&self, account: &str, plaintext: &str) -> Result<(), StoreError> {
        let key = self
            .read_or_init_machine_key()
            .map_err(StoreError::FileFallback)?;
        let mut nonce = [0u8; IV_LENGTH];
        getrandom::fill(&mut nonce)
            .map_err(|e| StoreError::FileFallback(format!("getrandom: {e}")))?;
        let cipher = aes_gcm::Aes256Gcm::new_from_slice(&key)
            .map_err(|e| StoreError::FileFallback(format!("new_from_slice: {e}")))?;
        let nonce_bytes = aes_gcm::Nonce::try_from(&nonce[..])
            .map_err(|e| StoreError::FileFallback(format!("nonce: {e}")))?;
        let ct = cipher
            .encrypt(&nonce_bytes, plaintext.as_bytes())
            .map_err(|e| StoreError::FileFallback(format!("encrypt: {e}")))?;

        let mut payload = Vec::with_capacity(IV_LENGTH + ct.len());
        payload.extend_from_slice(&nonce);
        payload.extend_from_slice(&ct);
        atomic_write(&self.account_path(account)?, &payload).map_err(StoreError::FileFallback)
    }

    fn read(&self, account: &str) -> Result<String, StoreError> {
        let path = self.account_path(account)?;
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(StoreError::NotFound);
            }
            Err(e) => return Err(StoreError::FileFallback(format!("read failed: {e}"))),
        };
        if bytes.len() < IV_LENGTH + 16 {
            return Err(StoreError::FileFallback(format!(
                "entry at {} is too short to be a valid AES-GCM blob",
                path.display()
            )));
        }
        let (nonce, ct) = bytes.split_at(IV_LENGTH);
        let key = self
            .read_or_init_machine_key()
            .map_err(StoreError::FileFallback)?;
        let cipher = aes_gcm::Aes256Gcm::new_from_slice(&key)
            .map_err(|e| StoreError::FileFallback(format!("new_from_slice: {e}")))?;
        let nonce_bytes = aes_gcm::Nonce::try_from(nonce)
            .map_err(|e| StoreError::FileFallback(format!("nonce: {e}")))?;
        let plaintext = cipher
            .decrypt(&nonce_bytes, ct)
            .map_err(|e| StoreError::FileFallback(format!("decrypt: {e}")))?;
        String::from_utf8(plaintext).map_err(|e| StoreError::FileFallback(format!("utf8: {e}")))
    }

    fn delete(&self, account: &str) -> Result<(), StoreError> {
        let path = self.account_path(account)?;
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(StoreError::FileFallback(format!("remove_file: {e}"))),
        }
    }

    fn clear_all(&self) -> Result<(), StoreError> {
        let dir = self.base.join("accounts");
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(StoreError::FileFallback(format!("read_dir: {e}"))),
        };
        for entry in entries.flatten() {
            let _ = std::fs::remove_file(entry.path());
        }
        Ok(())
    }
}

/// Writes via a `.tmp` sibling then renames. Same-filesystem renames are
/// atomic on POSIX, so a crash mid-write cannot leave a truncated
/// `machine.key` or ciphertext behind.
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    // The file inherits the umask-derived mode on creation, which is
    // usually 0644 — world-readable. The bytes are encrypted, but the
    // machine key is written the same way, so tighten the mode before
    // the content is in place.
    restrict_to_owner_file(&tmp)?;
    std::fs::rename(&tmp, path)
        .map_err(|e| format!("rename {} -> {}: {e}", tmp.display(), path.display()))?;
    Ok(())
}

/// Restricts a path to the owning user on Unix. A no-op on Windows, where
/// the per-user data directory already inherits ACLs that exclude other
/// accounts, and where a POSIX mode bit has no equivalent.
#[cfg(unix)]
fn restrict_to_owner(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("chmod 700 {}: {e}", path.display()))
}

/// Restricts a file to the owning user on Unix. A no-op on Windows.
#[cfg(unix)]
fn restrict_to_owner_file(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("chmod 600 {}: {e}", path.display()))
}

#[cfg(not(unix))]
fn restrict_to_owner(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(not(unix))]
fn restrict_to_owner_file(_path: &Path) -> Result<(), String> {
    Ok(())
}

/// Resolves the platform data dir the same way Tauri does, so a backup of the
/// app's data dir captures the fallback's ciphertext and machine key together.
fn data_dir() -> Result<PathBuf, String> {
    #[cfg(target_os = "linux")]
    {
        if let Ok(x) = std::env::var("XDG_DATA_HOME") {
            if !x.is_empty() {
                return Ok(PathBuf::from(x));
            }
        }
        if let Ok(home) = std::env::var("HOME") {
            if !home.is_empty() {
                return Ok(PathBuf::from(home).join(".local").join("share"));
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            if !home.is_empty() {
                return Ok(PathBuf::from(home)
                    .join("Library")
                    .join("Application Support"));
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(roam) = std::env::var("APPDATA") {
            if !roam.is_empty() {
                return Ok(PathBuf::from(roam));
            }
        }
    }
    Err("no platform data dir resolved (set XDG_DATA_HOME / HOME / APPDATA)".into())
}

// ── tests ──────────────────────────────────────────────────────────────────
//
// These exercise only the in-memory passphrase path plus error mapping. The
// keychain-backed functions need a real OS keychain (a headless CI runner has
// none) and are covered by the manual smoke checklist in README.md.

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn passphrase_lifecycle() {
        let s = SecretStore::new();
        assert!(!s.is_unlocked().await, "starts locked");
        assert!(
            s.with_passphrase(|_| ()).await.is_err(),
            "locked store yields nothing"
        );

        s.lock_passphrase("correct horse").await;
        assert!(s.is_unlocked().await);
        assert_eq!(
            s.with_passphrase(|p| p.to_string()).await.unwrap(),
            "correct horse"
        );

        s.lock().await;
        assert!(!s.is_unlocked().await);
        assert!(
            s.with_passphrase(|_| ()).await.is_err(),
            "lock revokes access"
        );
    }

    #[tokio::test]
    async fn relock_replaces_the_previous_passphrase() {
        let s = SecretStore::new();
        s.lock_passphrase("first").await;
        s.lock_passphrase("second").await;
        assert_eq!(
            s.with_passphrase(|p| p.to_string()).await.unwrap(),
            "second"
        );
    }

    #[tokio::test]
    async fn describe_tolerates_an_empty_keychain() {
        let s = SecretStore::new();
        // On a runner with no keychain this is an error, not a panic — either
        // outcome is acceptable, the point is that it must not panic.
        let _ = s.describe().await;
    }

    /// File-fallback round-trip: the AES-256-GCM file store must persist and
    /// return a session's three fields, and the bytes on disk must be
    /// ciphertext rather than plaintext.
    ///
    /// This exercises `FileFallback` directly rather than going through
    /// `SecretStore`. `SecretStore` prefers the OS keychain and only falls
    /// back when the keychain is unavailable, and whether a keychain exists
    /// is a property of the *host*: macOS and Windows CI runners have working
    /// keychains, so asserting the fallback was selected made this test fail
    /// there while passing on headless Linux. The fallback's own contract is
    /// platform-independent, so test it directly.
    #[test]
    fn file_fallback_roundtrips_a_session() {
        let tmp = std::env::temp_dir().join(format!(
            "vault-desktop-fallback-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let fb = FileFallback { base: tmp.clone() };
        std::fs::create_dir_all(tmp.join("accounts")).unwrap();

        // Write the same three fields `SecretStore::save_session` writes.
        fb.write(KEY_USER_BASE_URL, "https://api.example.com/v1")
            .expect("write base_url");
        fb.write(KEY_USER_ACCESS, "access-xyz")
            .expect("write access");
        fb.write(KEY_USER_REFRESH, "refresh-abc")
            .expect("write refresh");

        // Round-trip.
        assert_eq!(
            fb.read(KEY_USER_BASE_URL).expect("read base_url"),
            "https://api.example.com/v1"
        );
        assert_eq!(fb.read(KEY_USER_ACCESS).expect("read access"), "access-xyz");
        assert_eq!(
            fb.read(KEY_USER_REFRESH).expect("read refresh"),
            "refresh-abc"
        );

        // The stored blob must be ciphertext, not the plaintext token.
        let raw = std::fs::read(fb.account_path(KEY_USER_ACCESS).expect("path")).expect("read raw");
        assert!(
            raw.len() > IV_LENGTH + 16,
            "blob should carry a nonce and a GCM tag"
        );
        assert!(
            !raw.windows("access-xyz".len()).any(|w| w == b"access-xyz"),
            "access token must not appear in plaintext on disk"
        );

        // A missing entry is NotFound, not an opaque error.
        assert!(matches!(
            fb.read("orchestrator-nonexistent"),
            Err(StoreError::NotFound)
        ));

        // Delete removes one field; clear_all wipes the rest.
        fb.delete(KEY_USER_ACCESS).expect("delete");
        assert!(matches!(
            fb.read(KEY_USER_ACCESS),
            Err(StoreError::NotFound)
        ));
        fb.clear_all().expect("clear_all");
        assert!(matches!(
            fb.read(KEY_USER_BASE_URL),
            Err(StoreError::NotFound)
        ));
        assert!(matches!(
            fb.read(KEY_USER_REFRESH),
            Err(StoreError::NotFound)
        ));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Path-traversal guard on the account name. `account_path` refuses to
    /// build a path from anything containing a separator or `..`, so an
    /// attacker-influenced name can never escape `accounts/`.
    #[test]
    fn file_fallback_rejects_path_traversal_account_names() {
        let tmp = std::env::temp_dir().join("vault-desktop-traversal-test");
        let fb = FileFallback { base: tmp };
        for bad in ["../../etc/passwd", "..", "a/b", "a\\b", ""] {
            assert!(
                matches!(fb.account_path(bad), Err(StoreError::FileFallback(_))),
                "expected {bad:?} to be refused, got {:?}",
                fb.account_path(bad)
            );
        }
        // A legitimate name still resolves inside `accounts/`.
        let ok = fb.account_path(KEY_USER_ACCESS).unwrap();
        assert!(ok.to_string_lossy().contains("accounts"));
        assert!(!ok.to_string_lossy().contains(".."));
    }

    /// The fallback directory and the files in it must not be readable by
    /// other local accounts. The machine key encrypts the session, so a
    /// world-readable key file is the same as a world-readable session.
    #[cfg(unix)]
    #[test]
    fn file_fallback_is_owner_only_on_unix() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = std::env::temp_dir().join(format!(
            "vault-desktop-perms-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let base = tmp.join("functionfly").join("vault-desktop");
        std::fs::create_dir_all(base.join("accounts")).unwrap();

        restrict_to_owner(&base).expect("chmod base");
        restrict_to_owner(&base.join("accounts")).expect("chmod accounts");

        let dir_mode = std::fs::metadata(&base).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700, "base dir mode is {:o}", dir_mode);

        let accounts_mode = std::fs::metadata(base.join("accounts"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            accounts_mode, 0o700,
            "accounts dir mode is {:o}",
            accounts_mode
        );

        // A file written through atomic_write must be 0600 too.
        let key_path = base.join("machine.key");
        atomic_write(&key_path, &[7u8; KEY_LENGTH]).expect("write key");
        let file_mode = std::fs::metadata(&key_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(file_mode, 0o600, "machine key mode is {:o}", file_mode);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// A tampered ciphertext must fail the GCM auth tag rather than decrypt
    /// to garbage — otherwise a corrupted file would silently produce a
    /// wrong-but-plausible bearer token.
    #[test]
    fn file_fallback_rejects_tampered_ciphertext() {
        let tmp = std::env::temp_dir().join(format!(
            "vault-desktop-tamper-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let fb = FileFallback { base: tmp.clone() };
        std::fs::create_dir_all(tmp.join("accounts")).unwrap();
        std::fs::write(tmp.join("machine.key"), [7u8; KEY_LENGTH]).unwrap();

        fb.write("acct", "super-secret-token").unwrap();
        let path = fb.account_path("acct").unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        std::fs::write(&path, &bytes).unwrap();

        assert!(
            matches!(fb.read("acct"), Err(StoreError::FileFallback(_))),
            "tampered ciphertext must not decrypt"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
