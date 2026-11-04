//! `vault_check_update` / `vault_apply_update` Tauri commands.
//!
//! Mirrors `studio-desktop/src-tauri/src/commands/updater.rs` (Phase 5
//! of the studio plan). Both commands fail closed when:
//!
//! - No public key is available at all — neither `VAULT_UPDATER_PUBKEY`
//!   nor the key baked into the binary (no signing key — refuse to
//!   trust any bundle).
//! - The channel string isn't one of the two registered channels
//!   (`stable` or `beta`; `unknown_channel` typed error).
//! - The `tauri-plugin-updater` builder rejects the pubkey/endpoint
//!   combination (`builder_error` typed error).
//!
//! The webview receives a typed `VaultUpdateError` JSON envelope
//! rather than a bare string so the React UI can branch on the code
//! without parsing prose.

use serde::Serialize;
use tauri::{AppHandle, Runtime};
use tauri_plugin_updater::UpdaterExt;
use url::Url;

use crate::updater::{UpdaterChannel, UpdaterConfig, UpdaterError};

/// The Ed25519 public key compiled into the binary at build time.
///
/// It is read from `plugins.updater.pubkey` in `tauri.conf.json`, which
/// Tauri bakes into the app's embedded configuration at compile time.
/// This is what makes auto-update work for an ordinary user, who has no
/// `VAULT_UPDATER_PUBKEY` in their environment. The env var remains an
/// operator override (key rotation, staging) and takes precedence.
fn baked_pubkey<R: Runtime>(app: &AppHandle<R>) -> Option<String> {
    app.config()
        .plugins
        .0
        .get("updater")
        .and_then(|updater| updater.get("pubkey"))
        .and_then(|pubkey| pubkey.as_str())
        .map(str::trim)
        .filter(|pubkey| !pubkey.is_empty())
        .map(str::to_string)
}

#[derive(Debug, Serialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum CheckUpdateResponse {
    /// No newer version is published on this channel.
    UpToDate,
    /// A newer version is available; the webview decides whether to
    /// surface the release notes / "Restart now?" UI.
    Available {
        version: String,
        /// Markdown release notes; may be empty when the publisher
        /// didn't attach any.
        notes: String,
        /// RFC 3339 timestamp from the manifest; `None` when absent.
        pub_date: Option<String>,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyUpdateResponse {
    Installed { version: String },
}

#[derive(Debug, Serialize)]
pub struct VaultUpdateError {
    /// Machine-readable error code (mirrors the enum variants on the
    /// Rust side; the webview branches on this).
    pub code: &'static str,
    pub message: String,
}

fn map_config_err(e: UpdaterError) -> VaultUpdateError {
    let code = match &e {
        UpdaterError::MissingPubkey => "missing_pubkey",
        UpdaterError::InvalidUrl { .. } => "invalid_endpoint",
        UpdaterError::MissingEndpoint(_) => "missing_endpoint",
    };
    VaultUpdateError {
        code,
        message: e.to_string(),
    }
}

/// Build an `UpdaterBuilder` with the per-channel endpoint + the
/// configured Ed25519 pubkey. Returns `VaultUpdateError` on URL parse
/// failure or endpoint validation rejection (the plugin rejects
/// HTTP endpoints in production unless
/// `dangerous_insecure_transport_protocol` is enabled — fail-closed
/// by default, matching the launch extras checklist).
fn build_updater<R: Runtime>(
    app: &AppHandle<R>,
    config: &UpdaterConfig,
    channel: UpdaterChannel,
) -> Result<tauri_plugin_updater::Updater, VaultUpdateError> {
    let endpoint_str = config.channel_url(channel).map_err(map_config_err)?;
    let endpoint = Url::parse(endpoint_str).map_err(|e| VaultUpdateError {
        code: "invalid_endpoint",
        message: format!("could not parse {endpoint_str:?} as URL: {e}"),
    })?;

    app.updater_builder()
        .endpoints(vec![endpoint])
        .map_err(|e| VaultUpdateError {
            code: "builder_error",
            message: e.to_string(),
        })?
        .pubkey(config.pubkey.clone())
        .build()
        .map_err(|e| VaultUpdateError {
            code: "builder_error",
            message: e.to_string(),
        })
}

/// Check for a newer version on the given channel. Wired into the
/// Tauri invoke handler as `vault_check_update`.
#[tauri::command]
pub async fn vault_check_update<R: Runtime>(
    app: AppHandle<R>,
    channel: String,
) -> Result<CheckUpdateResponse, VaultUpdateError> {
    let channel = UpdaterChannel::parse(&channel).ok_or_else(|| VaultUpdateError {
        code: "unknown_channel",
        message: format!("unknown channel: {channel:?}"),
    })?;

    let baked = baked_pubkey(&app);
    let config = UpdaterConfig::from_env(baked.as_deref()).map_err(map_config_err)?;
    let updater = build_updater(&app, &config, channel)?;

    let update = updater.check().await.map_err(|e| VaultUpdateError {
        code: "check_failed",
        message: e.to_string(),
    })?;

    Ok(match update {
        None => CheckUpdateResponse::UpToDate,
        Some(u) => CheckUpdateResponse::Available {
            version: u.version,
            notes: u.body.unwrap_or_default(),
            pub_date: u.date.map(|d| d.to_string()),
        },
    })
}

/// Download + verify signature + apply the latest version on the
/// given channel. Wired into the Tauri invoke handler as
/// `vault_apply_update`. The caller (webview) is responsible for
/// showing the user a "Restart to update?" confirmation prompt
/// before calling this — Tauri's restart is unavoidable once the
/// bundle is staged.
#[tauri::command]
pub async fn vault_apply_update<R: Runtime>(
    app: AppHandle<R>,
    channel: String,
) -> Result<ApplyUpdateResponse, VaultUpdateError> {
    let channel = UpdaterChannel::parse(&channel).ok_or_else(|| VaultUpdateError {
        code: "unknown_channel",
        message: format!("unknown channel: {channel:?}"),
    })?;

    let baked = baked_pubkey(&app);
    let config = UpdaterConfig::from_env(baked.as_deref()).map_err(map_config_err)?;
    let updater = build_updater(&app, &config, channel)?;

    // Resolve → download → verify signature → stage bundle. The
    // `download_and_install` call schedules the install on next
    // restart (Tauri's standard contract).
    let update = updater.check().await.map_err(|e| VaultUpdateError {
        code: "check_failed",
        message: e.to_string(),
    })?;

    let Some(update) = update else {
        return Err(VaultUpdateError {
            code: "no_update_available",
            message: format!("no newer version available on channel {channel}"),
        });
    };

    let version = update.version.clone();
    update
        .download_and_install(|_chunk_len, _total_len| {}, || {})
        .await
        .map_err(|e| VaultUpdateError {
            code: "install_failed",
            message: e.to_string(),
        })?;

    Ok(ApplyUpdateResponse::Installed { version })
}
