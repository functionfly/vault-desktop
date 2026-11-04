//! Signed auto-updater configuration.
//!
//! Mirrors `studio-desktop/src-tauri/src/updater.rs` (Phase 5 of the
//! studio plan) but with the FunctionFly Vault env-var contract:
//!
//! - `VAULT_UPDATER_PUBKEY` — **required**, fail-closed when missing.
//!   The Ed25519 public key (PEM) printed by `tauri signer generate`.
//! - `VAULT_UPDATER_{STABLE,BETA}_URL` — optional per-channel endpoint.
//!   When unset, defaults to
//!   `https://updates.functionfly.app/vault/{channel}/manifest.json`
//!   so a vanilla vault binary still works out of the box once the R2
//!   bucket `functionfly-vault-updates` is provisioned.
//!
//!   The endpoint is the channel's *static* manifest. `tauri-plugin-updater`
//!   substitutes only `{{current_version}}`, `{{target}}`, `{{arch}}` and
//!   `{{bundle_type}}`; there is no `{{target_version}}` placeholder, and
//!   the previous default embedded one, so the plugin issued a request
//!   for a URL containing the literal text `{{target_version}}` and the
//!   channel was permanently unreachable.
//!
//! Two channels (stable + beta) cover the launch surface; insider-style
//! rapid-iteration channels are an explicit follow-up because the vault
//! client never sends untrusted install requests — every code path
//! passes through the same Tauri-update pipeline, and the cost of an
//! extra channel (more matrix, more signing keys) outweighs the
//! benefit of "same-day insider builds" for a security-critical
//! client.
//!
//! Failure modes:
//! - **Missing pubkey** → fail closed. Every `vault_check_update` /
//!   `vault_apply_update` returns `missing_pubkey`. We never trust an
//!   unsigned bundle.
//! - **Blank URL** → fail closed. A blank `VAULT_UPDATER_*_URL` raises
//!   `InvalidUrl` so the operator catches the misconfig at boot, not
//!   in the field when an end user tries to check for updates.
//! - **Unknown channel** → `UpdaterChannel::parse` returns `None`; the
//!   Tauri command maps it to `unknown_channel` for the webview.

use std::collections::HashMap;
use std::fmt;

use thiserror::Error;

/// Release channels. The `as_str` value is the path segment in the
/// canonical R2 bucket layout
/// (`vault/{channel}/manifest.json` + `vault/{channel}/{version}/…`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UpdaterChannel {
    Stable,
    Beta,
}

impl UpdaterChannel {
    pub fn as_str(self) -> &'static str {
        match self {
            UpdaterChannel::Stable => "stable",
            UpdaterChannel::Beta => "beta",
        }
    }

    pub fn env_suffix(self) -> &'static str {
        // Title-cased for the env-var convention; channel strings
        // themselves stay lowercase per the R2 path layout.
        match self {
            UpdaterChannel::Stable => "STABLE",
            UpdaterChannel::Beta => "BETA",
        }
    }

    /// Parse a channel string. Returns `None` for empty / unknown
    /// values; the caller maps `None` to `UpdaterError::UnknownChannel`.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "stable" => Some(UpdaterChannel::Stable),
            "beta" => Some(UpdaterChannel::Beta),
            _ => None,
        }
    }

    pub const ALL: [UpdaterChannel; 2] = [UpdaterChannel::Stable, UpdaterChannel::Beta];
}

impl fmt::Display for UpdaterChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct UpdaterConfig {
    /// Ed25519 public key (PEM) printed by `tauri signer generate`.
    /// Stored in the build environment as `VAULT_UPDATER_PUBKEY`; never
    /// committed.
    pub pubkey: String,

    /// Per-channel update-manifest endpoint. The manifest is a static
    /// document published once per release, so the URL carries no
    /// version placeholder. `tauri-plugin-updater` substitutes
    /// `{{current_version}}`, `{{target}}`, `{{arch}}` and
    /// `{{bundle_type}}` — nothing else.
    pub channel_urls: HashMap<UpdaterChannel, String>,
}

#[derive(Debug, Error)]
pub enum UpdaterError {
    #[error(
        "VAULT_UPDATER_PUBKEY env var is required (set to the Ed25519 public key printed by `tauri signer generate`)"
    )]
    MissingPubkey,

    #[error("invalid channel URL for {channel}: {url:?}")]
    InvalidUrl {
        channel: UpdaterChannel,
        url: String,
    },

    #[error("missing endpoint for channel: {0}")]
    MissingEndpoint(UpdaterChannel),
}

impl UpdaterConfig {
    /// Build the config from the process environment. Returns
    /// `Err(UpdaterError::MissingPubkey)` when `VAULT_UPDATER_PUBKEY`
    /// is unset or empty — fail-closed per the launch extras checklist
    /// (downstream commands refuse to check for updates without a
    /// valid signing key).
    pub fn from_env() -> Result<Self, UpdaterError> {
        let pubkey = std::env::var("VAULT_UPDATER_PUBKEY")
            .map_err(|_| UpdaterError::MissingPubkey)?
            .trim()
            .to_string();
        if pubkey.is_empty() {
            return Err(UpdaterError::MissingPubkey);
        }

        let mut channel_urls = HashMap::with_capacity(UpdaterChannel::ALL.len());
        for channel in UpdaterChannel::ALL {
            let env_name = format!("VAULT_UPDATER_{}_URL", channel.env_suffix());
            let url = std::env::var(&env_name).unwrap_or_else(|_| {
                format!(
                    "https://updates.functionfly.app/vault/{}/manifest.json",
                    channel
                )
            });
            if url.trim().is_empty() {
                return Err(UpdaterError::InvalidUrl {
                    channel,
                    url: url.clone(),
                });
            }
            channel_urls.insert(channel, url);
        }

        Ok(UpdaterConfig {
            pubkey,
            channel_urls,
        })
    }

    pub fn channel_url(&self, channel: UpdaterChannel) -> Result<&str, UpdaterError> {
        self.channel_urls
            .get(&channel)
            .map(String::as_str)
            .ok_or(UpdaterError::MissingEndpoint(channel))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Tests that touch the process environment must run serially —
    // `std::env::set_var`/`remove_var` are documented as not
    // thread-safe, and Rust's default test runner runs tests on
    // multiple threads. The mutex gates every env-mutating case so
    // they observe a consistent snapshot regardless of scheduling.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn clear_env() {
        for ch in UpdaterChannel::ALL {
            let name = format!("VAULT_UPDATER_{}_URL", ch.env_suffix());
            // SAFETY: callers hold `ENV_LOCK`, so no other test is
            // mutating the env table concurrently. The standard
            // library's thread-safety caveat is the only reason this
            // block needs `unsafe`.
            unsafe {
                std::env::remove_var(&name);
            }
        }
        // SAFETY: see above.
        unsafe {
            std::env::remove_var("VAULT_UPDATER_PUBKEY");
        }
    }

    fn set_pubkey(pubkey: &str) {
        // SAFETY: see `clear_env`.
        unsafe {
            std::env::set_var("VAULT_UPDATER_PUBKEY", pubkey);
        }
    }

    fn set_channel_url(channel: UpdaterChannel, url: &str) {
        let name = format!("VAULT_UPDATER_{}_URL", channel.env_suffix());
        // SAFETY: see `clear_env`.
        unsafe {
            std::env::set_var(&name, url);
        }
    }

    #[test]
    fn parse_roundtrip() {
        for ch in UpdaterChannel::ALL {
            assert_eq!(UpdaterChannel::parse(ch.as_str()), Some(ch));
            assert_eq!(UpdaterChannel::parse(&ch.to_string()), Some(ch));
            // Mixed-case tolerant
            assert_eq!(
                UpdaterChannel::parse(&ch.as_str().to_ascii_uppercase()),
                Some(ch)
            );
        }
    }

    #[test]
    fn parse_rejects_unknown() {
        assert_eq!(UpdaterChannel::parse(""), None);
        assert_eq!(UpdaterChannel::parse("   "), None);
        assert_eq!(UpdaterChannel::parse("release"), None);
        assert_eq!(UpdaterChannel::parse("alpha"), None);
        assert_eq!(UpdaterChannel::parse("insider"), None);
        assert_eq!(UpdaterChannel::parse("stäble"), None);
    }

    #[test]
    fn from_env_fails_closed_when_pubkey_missing() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        let err = UpdaterConfig::from_env().unwrap_err();
        assert!(matches!(err, UpdaterError::MissingPubkey));
    }

    #[test]
    fn from_env_fails_closed_when_pubkey_empty() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        set_pubkey("   ");
        let err = UpdaterConfig::from_env().unwrap_err();
        assert!(matches!(err, UpdaterError::MissingPubkey));
    }

    #[test]
    fn from_env_applies_default_urls() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        set_pubkey("dGVzdC1wdWJrZXk=");
        let cfg = UpdaterConfig::from_env().unwrap();
        for ch in UpdaterChannel::ALL {
            let url = cfg.channel_url(ch).unwrap();
            assert!(url.starts_with("https://updates.functionfly.app/vault/"));
            assert!(url.contains(ch.as_str()));
            assert!(url.ends_with("/manifest.json"));
            // `tauri-plugin-updater` substitutes only these four tokens.
            // A `{{target_version}}` in the endpoint is never expanded and
            // would make the channel permanently unreachable.
            assert!(!url.contains("{{target_version}}"));
            for placeholder in [
                "{{current_version}}",
                "{{target}}",
                "{{arch}}",
                "{{bundle_type}}",
            ] {
                assert!(
                    !url.contains(placeholder),
                    "unexpected placeholder {placeholder}"
                );
            }
        }
    }

    #[test]
    fn from_env_honors_per_channel_overrides() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        set_pubkey("dGVzdC1wdWJrZXk=");
        set_channel_url(
            UpdaterChannel::Beta,
            "https://staging.example.com/vault-beta.json",
        );
        let cfg = UpdaterConfig::from_env().unwrap();
        assert_eq!(
            cfg.channel_url(UpdaterChannel::Beta).unwrap(),
            "https://staging.example.com/vault-beta.json"
        );
        // Stable keeps the default R2 path.
        assert!(cfg
            .channel_url(UpdaterChannel::Stable)
            .unwrap()
            .contains("updates.functionfly.app/vault/stable"));
    }

    #[test]
    fn from_env_rejects_blank_url() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        set_pubkey("dGVzdC1wdWJrZXk=");
        set_channel_url(UpdaterChannel::Stable, "   ");
        let err = UpdaterConfig::from_env().unwrap_err();
        match err {
            UpdaterError::InvalidUrl { channel, url } => {
                assert_eq!(channel, UpdaterChannel::Stable);
                assert_eq!(url, "   ");
            }
            other => panic!("expected InvalidUrl, got {other:?}"),
        }
    }

    #[test]
    fn channel_url_returns_missing_for_unknown() {
        // Defensive: `from_env` always populates both channels, but
        // tests should pin the failure mode in case the env table
        // mutates in the future.
        let mut urls = HashMap::new();
        urls.insert(UpdaterChannel::Stable, "https://example.com".into());
        let cfg = UpdaterConfig {
            pubkey: "k".into(),
            channel_urls: urls,
        };
        let err = cfg.channel_url(UpdaterChannel::Beta).unwrap_err();
        assert!(matches!(
            err,
            UpdaterError::MissingEndpoint(UpdaterChannel::Beta)
        ));
    }
}
