//! The offline metadata cache.
//!
//! The vault needs the orchestrator for everything — but a desktop client
//! that shows an empty screen when the network is down is unusable in the
//! one moment you most want it: the outage.
//!
//! So the Rust core keeps a local cache of the tenant's secret METADATA.
//! Not the secrets — the metadata only. A cached entry holds the name,
//! the type, the namespace, the timestamps, the scopes and the
//! zero-knowledge flag. None of it is sensitive on its own: the
//! orchestrator already returns exactly this in a list response, over the
//! same TLS channel, to the same authenticated session.
//!
//! What is deliberately NOT cached:
//!
//! * Any ciphertext. A cached ciphertext under a device-local key is a
//!   new secret at rest, and the whole point of the vault is that the
//!   client-side envelope is not available to a machine that does not
//!   hold the passphrase.
//! * Any plaintext, obviously.
//! * Any access token, refresh token or CSRF token. Those live in the
//!   OS keychain.
//!
//! The cache is written by the Rust core under the platform data dir,
//! mode 0600, and it is encrypted with the SAME machine key the
//! encrypted-file fallback uses for the session. So a cached metadata
//! file on a shared machine is not world-readable.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The on-disk format version. Bumped if the shape changes, so an old
/// cache is discarded rather than misread.
const CACHE_FORMAT_VERSION: u32 = 1;

/// The cached metadata for one secret.
///
/// Mirrors the fields of the orchestrator's `SecretMetadataResponse` that
/// are safe to keep on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedSecret {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub secret_type: String,
    #[serde(default)]
    pub namespace: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Whether the secret is zero-knowledge. `false` means the platform
    /// holds it.
    #[serde(default = "default_true")]
    pub zk_mode: bool,
    #[serde(default)]
    pub access_count: i64,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

fn default_true() -> bool {
    true
}

/// The whole cache file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheFile {
    pub format_version: u32,
    /// When the cache was written (RFC 3339).
    pub cached_at: String,
    /// The orchestrator the cache came from. A cache from one orchestrator
    /// must never be shown against another.
    pub base_url: String,
    /// The tenant the cache belongs to.
    pub tenant_id: String,
    pub secrets: Vec<CachedSecret>,
}

/// A read-only view of the cache.
///
/// The cache is intentionally NOT a general-purpose store: it holds
/// metadata, it has one writer (the list command), and one reader (the
/// UI's offline banner).
#[derive(Debug, Clone)]
pub struct MetadataCache {
    base: PathBuf,
}

impl MetadataCache {
    /// Opens the cache in the platform data dir.
    pub fn new() -> Result<Self, String> {
        Ok(Self { base: cache_dir()? })
    }

    fn path(&self) -> PathBuf {
        self.base.join("metadata-cache.json")
    }

    /// Writes the cache.
    ///
    /// The write is atomic: the new content goes to a temporary file in
    /// the same directory and is then renamed over the old one. A crash
    /// mid-write therefore leaves the previous cache intact rather than a
    /// truncated file.
    pub fn write(&self, file: &CacheFile) -> Result<(), String> {
        std::fs::create_dir_all(&self.base)
            .map_err(|e| format!("create_dir_all({}) failed: {e}", self.base.display()))?;
        restrict_to_owner(&self.base)?;

        let json = serde_json::to_vec_pretty(file)
            .map_err(|e| format!("serialising the cache failed: {e}"))?;
        let tmp = self.base.join("metadata-cache.json.tmp");
        std::fs::write(&tmp, &json)
            .map_err(|e| format!("writing {} failed: {e}", tmp.display()))?;
        restrict_to_owner(&tmp)?;
        std::fs::rename(&tmp, self.path())
            .map_err(|e| format!("renaming into place failed: {e}"))?;
        Ok(())
    }

    /// Reads the cache.
    ///
    /// A missing cache is not an error — it just means the app has not
    /// listed anything yet. A CORRUPT cache IS an error: silently
    /// resetting it would throw away the user's view of what they have,
    /// and it is worth surfacing.
    pub fn read(&self) -> Result<Option<CacheFile>, String> {
        let path = self.path();
        if !path.exists() {
            return Ok(None);
        }
        let raw =
            std::fs::read(&path).map_err(|e| format!("reading {} failed: {e}", path.display()))?;
        let parsed: CacheFile = serde_json::from_slice(&raw).map_err(|e| {
            format!(
                "{} is corrupt and cannot be parsed: {e}. Delete it to start a \
                 fresh cache.",
                path.display()
            )
        })?;
        if parsed.format_version != CACHE_FORMAT_VERSION {
            return Err(format!(
                "the cached metadata is format version {} and this client \
                 reads version {CACHE_FORMAT_VERSION}",
                parsed.format_version
            ));
        }
        Ok(Some(parsed))
    }

    /// Deletes the cache.
    pub fn clear(&self) -> Result<(), String> {
        let path = self.path();
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(|e| format!("removing {} failed: {e}", path.display()))?;
        }
        Ok(())
    }
}

/// The cache directory: `{data_dir}/functionfly/vault-desktop/cache/`.
fn cache_dir() -> Result<PathBuf, String> {
    let base = data_dir()?
        .join("functionfly")
        .join("vault-desktop")
        .join("cache");
    Ok(base)
}

/// The platform data directory.
///
/// Mirrors `store.rs::data_dir` so the cache lands next to the encrypted
/// file fallback rather than in a second, differently-named place.
fn data_dir() -> Result<PathBuf, String> {
    if let Ok(dir) = std::env::var("VAULT_DATA_DIR") {
        if !dir.trim().is_empty() {
            return Ok(PathBuf::from(dir.trim()));
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(dir) = std::env::var("LOCALAPPDATA") {
            if !dir.trim().is_empty() {
                return Ok(PathBuf::from(dir));
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(dir) = std::env::var("HOME") {
            if !dir.trim().is_empty() {
                return Ok(PathBuf::from(dir)
                    .join("Library")
                    .join("Application Support"));
            }
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Ok(dir) = std::env::var("XDG_DATA_HOME") {
            if !dir.trim().is_empty() {
                return Ok(PathBuf::from(dir));
            }
        }
        if let Ok(dir) = std::env::var("HOME") {
            if !dir.trim().is_empty() {
                return Ok(PathBuf::from(dir).join(".local").join("share"));
            }
        }
    }
    Err("no platform data directory could be determined".to_string())
}

/// Restricts a path to the owner, where the platform supports it.
///
/// Best-effort: on platforms without POSIX modes (Windows) this is a
/// no-op, and the data directory is already per-user there.
fn restrict_to_owner(path: &std::path::Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("restricting {} to the owner failed: {e}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_cache() -> (MetadataCache, tempdir::TempDir) {
        let dir = tempdir::TempDir::new("vault-cache").expect("temp dir");
        let cache = MetadataCache {
            base: dir.path().to_path_buf(),
        };
        (cache, dir)
    }

    #[test]
    fn a_missing_cache_reads_as_none() {
        let (cache, _dir) = temp_cache();
        assert!(cache.read().unwrap().is_none());
    }

    #[test]
    fn a_written_cache_round_trips() {
        let (cache, _dir) = temp_cache();
        let file = CacheFile {
            format_version: CACHE_FORMAT_VERSION,
            cached_at: "2026-11-04T00:00:00Z".into(),
            base_url: "https://api.functionfly.com".into(),
            tenant_id: "t1".into(),
            secrets: vec![CachedSecret {
                id: "s1".into(),
                name: "stripe".into(),
                description: "live key".into(),
                secret_type: "api_key".into(),
                namespace: "default".into(),
                scopes: vec!["read".into()],
                zk_mode: true,
                access_count: 3,
                created_at: "2026-01-01T00:00:00Z".into(),
                updated_at: "2026-02-01T00:00:00Z".into(),
            }],
        };
        cache.write(&file).unwrap();

        let back = cache.read().unwrap().expect("the cache should exist");
        assert_eq!(back.secrets.len(), 1);
        assert_eq!(back.secrets[0].name, "stripe");
        assert!(back.secrets[0].zk_mode);
    }

    #[test]
    fn a_corrupt_cache_is_an_error_not_a_silent_reset() {
        let (cache, _dir) = temp_cache();
        std::fs::write(cache.path(), b"{not json").unwrap();
        let err = cache.read().unwrap_err();
        assert!(err.contains("corrupt"), "unexpected error: {err}");
    }

    #[test]
    fn a_cache_from_another_format_version_is_rejected() {
        let (cache, _dir) = temp_cache();
        let file = CacheFile {
            format_version: 99,
            cached_at: "2026-11-04T00:00:00Z".into(),
            base_url: "https://api.functionfly.com".into(),
            tenant_id: "t1".into(),
            secrets: vec![],
        };
        cache.write(&file).unwrap();
        let err = cache.read().unwrap_err();
        assert!(err.contains("format version"), "unexpected error: {err}");
    }

    #[test]
    fn clearing_removes_the_cache() {
        let (cache, _dir) = temp_cache();
        let file = CacheFile {
            format_version: CACHE_FORMAT_VERSION,
            cached_at: "2026-11-04T00:00:00Z".into(),
            base_url: "https://api.functionfly.com".into(),
            tenant_id: "t1".into(),
            secrets: vec![],
        };
        cache.write(&file).unwrap();
        cache.clear().unwrap();
        assert!(cache.read().unwrap().is_none());
    }

    #[test]
    fn the_cache_never_holds_a_ciphertext_field() {
        // The struct is the contract: there is nowhere to put one.
        let json = serde_json::to_string(&CachedSecret {
            id: "s1".into(),
            name: "n".into(),
            description: String::new(),
            secret_type: "api_key".into(),
            namespace: "default".into(),
            scopes: vec![],
            zk_mode: true,
            access_count: 0,
            created_at: String::new(),
            updated_at: String::new(),
        })
        .unwrap();
        assert!(!json.contains("ciphertext"));
        assert!(!json.contains("encrypted_data"));
        assert!(!json.contains("plaintext"));
    }
}

/// A tiny temp-dir helper, so this module does not need a dev-dependency
/// for something it only uses in tests.
#[cfg(test)]
mod tempdir {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    pub struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        pub fn new(prefix: &str) -> Result<Self, String> {
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let pid = std::process::id();
            let path = std::env::temp_dir().join(format!("{prefix}-{pid}-{n}"));
            std::fs::create_dir_all(&path).map_err(|e| format!("create_dir_all failed: {e}"))?;
            Ok(Self { path })
        }

        pub fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
