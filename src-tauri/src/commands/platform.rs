//! The vault API surface the desktop client did not previously reach.
//!
//! Everything here is a thin, typed wrapper over an orchestrator route. The
//! rules that apply to the rest of the command layer apply here too:
//!
//! * The webview never holds a bearer or a CSRF token — `VaultClient::request`
//!   attaches both.
//! * Every mutating call goes through the CSRF header path.
//! * No plaintext secret ever crosses the IPC boundary except on the two
//!   explicit reveal/mint paths the user just asked for.
//!
//! The routes are grouped by the orchestrator handler that serves them:
//! `audit_chain.go`, `dynamic.go`, `security.go`, `capabilities.go`,
//! `federation_keys.go` and `shares.go`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::commands::{require_auth, AppState};
use crate::error::{AppError, AppErrorKind};

/// Asks the orchestrator to re-hash the tenant's audit hash chain and report
/// the first entry that does not verify.
///
/// `window` bounds how many of the most recent entries are re-hashed. `0`
/// verifies the whole chain — unbounded work, so it is the caller's explicit
/// choice. The server rejects a negative window or one above its cap.
///
/// The server answers **200 whether or not the chain verified**: a break is
/// a security finding, not a client error. So the caller must inspect
/// `intact` rather than treating a successful call as a successful audit.
#[tauri::command]
pub async fn verify_audit_chain(
    state: State<'_, AppState>,
    window: Option<u32>,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    let query: Vec<(&str, String)> = match window {
        Some(w) => vec![("window", w.to_string())],
        // No window at all lets the server apply its own default, which is
        // the bounded one. Sending `window=0` would mean "verify
        // everything", which is not what the caller asked for.
        None => vec![],
    };
    state
        .client
        .request(
            reqwest::Method::POST,
            "/v1/vault/audit/chain/verify",
            &query,
            None,
        )
        .await
        .map_err(Into::into)
}

/// Reads the audit chain's posture, its segments, and whether each anchor
/// still verifies. It does **not** re-hash any entry — use
/// [`verify_audit_chain`] for that.
#[tauri::command]
pub async fn get_audit_chain(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    state
        .client
        .request(reqwest::Method::GET, "/v1/vault/audit/chain", &[], None)
        .await
        .map_err(Into::into)
}

/// Lists the capability requests awaiting a decision.
///
/// A capability request is someone asking to be granted scoped access to a
/// secret. The secret must be **platform-held** for the grant to be
/// serviceable: a zero-knowledge secret has nothing the broker can deliver.
#[tauri::command]
pub async fn list_capabilities(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    state
        .client
        .request(reqwest::Method::GET, "/v1/vault/capabilities", &[], None)
        .await
        .map_err(Into::into)
}

/// Approves a pending capability request.
#[tauri::command]
pub async fn approve_capability(
    state: State<'_, AppState>,
    id: String,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    let path = format!("/v1/vault/capabilities/{id}/approve");
    state
        .client
        .request(reqwest::Method::PATCH, &path, &[], None)
        .await
        .map_err(Into::into)
}

/// Denies a pending capability request.
#[tauri::command]
pub async fn deny_capability(
    state: State<'_, AppState>,
    id: String,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    let path = format!("/v1/vault/capabilities/{id}/deny");
    state
        .client
        .request(reqwest::Method::PATCH, &path, &[], None)
        .await
        .map_err(Into::into)
}

/// Revokes a capability that was already granted.
#[tauri::command]
pub async fn revoke_capability(
    state: State<'_, AppState>,
    id: String,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    let path = format!("/v1/vault/capabilities/{id}/revoke");
    state
        .client
        .request(reqwest::Method::POST, &path, &[], None)
        .await
        .map_err(Into::into)
}

/// Reports whether the platform can resolve a secret's plaintext.
///
/// This is the cheap way for the UI to tell a user "this secret is
/// platform-held, so it can back a capability grant" without attempting a
/// decrypt that would fail.
#[tauri::command]
pub async fn get_secret_presence(
    state: State<'_, AppState>,
    id: String,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    let path = format!("/v1/vault/secrets/{id}/presence");
    state
        .client
        .request(reqwest::Method::GET, &path, &[], None)
        .await
        .map_err(Into::into)
}

/// Reads the tenant's vault MFA policy.
#[tauri::command]
pub async fn get_mfa_config(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    state
        .client
        .request(reqwest::Method::GET, "/v1/vault/mfa/config", &[], None)
        .await
        .map_err(Into::into)
}

/// Verifies a vault MFA code and stamps a vault-scoped MFA assertion on the
/// session.
///
/// The code travels in the `X-MFA-Code` header — never in the query string,
/// because proxies log query strings.
#[tauri::command]
pub async fn verify_mfa(
    state: State<'_, AppState>,
    code: String,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    let trimmed = code.trim();
    if trimmed.is_empty() {
        return Err(AppError::new(
            crate::error::AppErrorKind::InvalidInput,
            "MFA code is required",
        ));
    }
    state
        .client
        .request_with_header(
            reqwest::Method::POST,
            "/v1/vault/mfa/verify",
            &[],
            None,
            "X-MFA-Code",
            trimmed,
        )
        .await
        .map_err(Into::into)
}

// ── dynamic credentials ───────────────────────────────────────────────────
//
// A dynamic credential target is a database the platform can mint short
// lived credentials for. The credential itself is minted by the database, not
// typed in, so it never appears in the secret list.

#[derive(Deserialize)]
pub struct CreateDynamicTargetInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// `postgres` or `mysql`.
    pub db_type: String,
    pub host: String,
    pub port: u16,
    pub database_name: String,
    pub admin_username: String,
    /// The admin password. The platform encrypts it server-side and never
    /// returns it.
    pub admin_password: String,
    #[serde(default)]
    pub ssl_mode: String,
    #[serde(default)]
    pub allowed_roles: Vec<String>,
    #[serde(default)]
    pub default_ttl_seconds: Option<u32>,
    #[serde(default)]
    pub max_ttl_seconds: Option<u32>,
}

/// Creates a dynamic secret target.
#[tauri::command]
pub async fn create_dynamic_target(
    state: State<'_, AppState>,
    input: CreateDynamicTargetInput,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;

    let db_type = input.db_type.trim().to_ascii_lowercase();
    if db_type != "postgres" && db_type != "mysql" {
        return Err(AppError::new(
            crate::error::AppErrorKind::InvalidInput,
            format!(
                "invalid db_type {:?}: expected postgres or mysql",
                input.db_type
            ),
        ));
    }
    if input.name.trim().is_empty() {
        return Err(AppError::new(
            crate::error::AppErrorKind::InvalidInput,
            "name is required",
        ));
    }
    if input.host.trim().is_empty() || input.database_name.trim().is_empty() {
        return Err(AppError::new(
            crate::error::AppErrorKind::InvalidInput,
            "host and database_name are required",
        ));
    }
    if input.admin_password.is_empty() {
        return Err(AppError::new(
            crate::error::AppErrorKind::InvalidInput,
            "admin_password is required",
        ));
    }

    let body = serde_json::json!({
        "name": input.name.trim(),
        "description": input.description,
        "db_type": db_type,
        "host": input.host.trim(),
        "port": input.port,
        "database_name": input.database_name.trim(),
        "admin_username": input.admin_username,
        "admin_password": input.admin_password,
        "ssl_mode": input.ssl_mode,
        "allowed_roles": input.allowed_roles,
        "default_ttl_seconds": input.default_ttl_seconds,
        "max_ttl_seconds": input.max_ttl_seconds,
    });
    state
        .client
        .request(
            reqwest::Method::POST,
            "/v1/vault/dynamic-secret-targets",
            &[],
            Some(body),
        )
        .await
        .map_err(Into::into)
}

/// Lists dynamic secret targets.
#[tauri::command]
pub async fn list_dynamic_targets(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    state
        .client
        .request(
            reqwest::Method::GET,
            "/v1/vault/dynamic-secret-targets",
            &[],
            None,
        )
        .await
        .map_err(Into::into)
}

/// Deletes a dynamic secret target.
#[tauri::command]
pub async fn delete_dynamic_target(
    state: State<'_, AppState>,
    id: String,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    let path = format!("/v1/vault/dynamic-secret-targets/{id}");
    state
        .client
        .request(reqwest::Method::DELETE, &path, &[], None)
        .await
        .map_err(Into::into)
}

/// Tests a target's connectivity and credentials.
#[tauri::command]
pub async fn test_dynamic_target(
    state: State<'_, AppState>,
    id: String,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    let path = format!("/v1/vault/dynamic-secret-targets/{id}/test");
    state
        .client
        .request(reqwest::Method::POST, &path, &[], None)
        .await
        .map_err(Into::into)
}

#[derive(Deserialize)]
pub struct CreateDynamicCredentialInput {
    pub target_id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub role_template: String,
    #[serde(default)]
    pub ttl_seconds: Option<u32>,
    #[serde(default)]
    pub max_ttl_seconds: Option<u32>,
}

/// Creates a dynamic credential definition against a target.
#[tauri::command]
pub async fn create_dynamic_credential(
    state: State<'_, AppState>,
    input: CreateDynamicCredentialInput,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    if input.target_id.trim().is_empty() {
        return Err(AppError::new(
            crate::error::AppErrorKind::InvalidInput,
            "target_id is required",
        ));
    }
    if input.name.trim().is_empty() {
        return Err(AppError::new(
            crate::error::AppErrorKind::InvalidInput,
            "name is required",
        ));
    }
    let body = serde_json::json!({
        "target_id": input.target_id.trim(),
        "name": input.name.trim(),
        "description": input.description,
        "role_template": input.role_template,
        "ttl_seconds": input.ttl_seconds,
        "max_ttl_seconds": input.max_ttl_seconds,
    });
    state
        .client
        .request(
            reqwest::Method::POST,
            "/v1/vault/dynamic-credentials",
            &[],
            Some(body),
        )
        .await
        .map_err(Into::into)
}

/// Lists dynamic credential definitions.
#[tauri::command]
pub async fn list_dynamic_credentials(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    state
        .client
        .request(
            reqwest::Method::GET,
            "/v1/vault/dynamic-credentials",
            &[],
            None,
        )
        .await
        .map_err(Into::into)
}

/// Mints a live credential and a lease for it.
///
/// The response carries the plaintext username and password. They exist only
/// in this response — the platform does not store them, so this value is
/// shown once and never persisted by the client.
#[tauri::command]
pub async fn generate_dynamic_credential(
    state: State<'_, AppState>,
    id: String,
    ttl_seconds: Option<u32>,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    let path = format!("/v1/vault/dynamic-credentials/{id}/generate");
    let body = match ttl_seconds {
        Some(ttl) => serde_json::json!({ "ttl_seconds": ttl }),
        None => serde_json::json!({}),
    };
    state
        .client
        .request(reqwest::Method::POST, &path, &[], Some(body))
        .await
        .map_err(Into::into)
}

/// Revokes every live lease for a dynamic credential.
#[tauri::command]
pub async fn revoke_dynamic_credential(
    state: State<'_, AppState>,
    id: String,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    let path = format!("/v1/vault/dynamic-credentials/{id}/revoke");
    state
        .client
        .request(reqwest::Method::POST, &path, &[], None)
        .await
        .map_err(Into::into)
}

/// Renews a lease, extending the credential's life.
#[tauri::command]
pub async fn renew_lease(
    state: State<'_, AppState>,
    credential_id: String,
    lease_id: String,
    ttl_seconds: Option<u32>,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    let path = format!("/v1/vault/dynamic-credentials/{credential_id}/leases/{lease_id}/renew");
    let body = match ttl_seconds {
        Some(ttl) => serde_json::json!({ "ttl_seconds": ttl }),
        None => serde_json::json!({}),
    };
    state
        .client
        .request(reqwest::Method::POST, &path, &[], Some(body))
        .await
        .map_err(Into::into)
}

/// Revokes a single lease.
#[tauri::command]
pub async fn revoke_lease(
    state: State<'_, AppState>,
    credential_id: String,
    lease_id: String,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    let path = format!("/v1/vault/dynamic-credentials/{credential_id}/leases/{lease_id}/revoke");
    state
        .client
        .request(reqwest::Method::POST, &path, &[], None)
        .await
        .map_err(Into::into)
}

// ── shares and federation keys ────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ShareSecretInput {
    pub secret_id: String,
    /// The tenant member to share with.
    pub user_id: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub expires_at: String,
}

/// Shares a secret with another tenant member.
///
/// A shared secret must be **platform-held**. A zero-knowledge secret has no
/// ciphertext the platform could hand the recipient, so the server rejects
/// the share.
#[tauri::command]
pub async fn share_secret(
    state: State<'_, AppState>,
    input: ShareSecretInput,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    if input.secret_id.trim().is_empty() || input.user_id.trim().is_empty() {
        return Err(AppError::new(
            crate::error::AppErrorKind::InvalidInput,
            "secret_id and user_id are required",
        ));
    }
    let body = serde_json::json!({
        "user_id": input.user_id.trim(),
        "scopes": input.scopes,
        "expires_at": input.expires_at,
    });
    let path = format!("/v1/vault/secrets/{}/share", input.secret_id.trim());
    state
        .client
        .request(reqwest::Method::POST, &path, &[], Some(body))
        .await
        .map_err(Into::into)
}

/// Lists the secrets other members have shared with this user.
#[tauri::command]
pub async fn list_shared_secrets(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    state
        .client
        .request(reqwest::Method::GET, "/v1/vault/shared", &[], None)
        .await
        .map_err(Into::into)
}

/// Revokes a share.
#[tauri::command]
pub async fn revoke_share(
    state: State<'_, AppState>,
    share_id: String,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    let path = format!("/v1/vault/shares/{share_id}");
    state
        .client
        .request(reqwest::Method::DELETE, &path, &[], None)
        .await
        .map_err(Into::into)
}

/// Lists the tenant's federation keys.
#[tauri::command]
pub async fn list_federation_keys(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    state
        .client
        .request(reqwest::Method::GET, "/v1/vault/federation-keys", &[], None)
        .await
        .map_err(Into::into)
}

#[derive(Deserialize)]
pub struct CreateFederationKeyInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// The peer tenant to federate with.
    #[serde(default)]
    pub peer_tenant_id: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub expires_at: String,
}

/// Creates a federation key.
#[tauri::command]
pub async fn create_federation_key(
    state: State<'_, AppState>,
    input: CreateFederationKeyInput,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    if input.name.trim().is_empty() {
        return Err(AppError::new(
            crate::error::AppErrorKind::InvalidInput,
            "name is required",
        ));
    }
    let body = serde_json::json!({
        "name": input.name.trim(),
        "description": input.description,
        "peer_tenant_id": input.peer_tenant_id,
        "scopes": input.scopes,
        "expires_at": input.expires_at,
    });
    state
        .client
        .request(
            reqwest::Method::POST,
            "/v1/vault/federation-keys",
            &[],
            Some(body),
        )
        .await
        .map_err(Into::into)
}

// ── import / export ───────────────────────────────────────────────────────
//
// Export writes the tenant's secret METADATA (never a ciphertext, never a
// plaintext) to a JSON file the user chooses. Import reads such a file back
// and re-creates the secrets locally.
//
// The file is written by the Rust core, not the webview, so the webview
// never gets a filesystem handle. It is deliberately metadata-only: an
// encrypted export would need the passphrase on the other machine, and a
// plaintext export would defeat the point of a vault.
//
// THE PATH IS CHOSEN BY THE USER, IN A NATIVE DIALOG THE CORE OWNS.
//
// The earlier shape of this took the path as an IPC argument. That was a
// hole: the webview could name any path, so a renderer compromised by an
// XSS could ask the core to read any file the process can read, and to
// write a file over any path the process can write. The webview has no
// `fs:*` scope, but the core did the IO on its behalf — which made the
// missing scope irrelevant.
//
// The fix is not to validate the path (any allowlist can be probed) but to
// remove the webview's ability to choose one. The core opens the OS file
// dialog itself and the webview never learns or supplies a path. Because
// the dialog is invoked from Rust, the capability manifest does not need
// `dialog:*` either: Tauri permissions gate the webview→core direction,
// and the webview never calls this plugin.

#[derive(Serialize, Deserialize)]
pub struct SecretExport {
    /// The export format version, so a future format change is detectable.
    pub format_version: u32,
    /// When the export was produced (RFC 3339).
    pub exported_at: String,
    /// The orchestrator the export came from.
    pub base_url: String,
    /// The tenant the export came from.
    pub tenant_id: String,
    pub secrets: Vec<serde_json::Value>,
}

/// Opens the native "save as" dialog and returns the path the user chose.
///
/// Returns `None` when the user dismissed the dialog.
///
/// The callback form is used rather than `blocking_save_file` because the
/// blocking variants must not run on the main thread, and a Tauri command
/// body is not a safe place to block a UI thread.
async fn pick_save_path(app: &AppHandle) -> Option<PathBuf> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Export vault metadata")
        .set_file_name("vault-export.json")
        .add_filter("Vault export", &["json"])
        .save_file(move |picked| {
            let _ = tx.send(picked);
        });
    rx.await.ok().flatten().and_then(|p| p.into_path().ok())
}

/// Opens the native "open" dialog and returns the path the user chose.
///
/// Returns `None` when the user dismissed the dialog.
async fn pick_open_path(app: &AppHandle) -> Option<PathBuf> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Open a vault metadata export")
        .add_filter("Vault export", &["json"])
        .pick_file(move |picked| {
            let _ = tx.send(picked);
        });
    rx.await.ok().flatten().and_then(|p| p.into_path().ok())
}

/// Builds a metadata-only export of the tenant's secrets and writes it to a
/// location the user picks in the native save dialog.
///
/// The webview supplies no path. It cannot influence where this lands.
#[tauri::command]
pub async fn export_secrets(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;

    // Ask for the destination BEFORE spending a round trip on the API: a
    // cancelled dialog should cost nothing.
    let Some(path) = pick_save_path(&app).await else {
        return Err(AppError::new(
            AppErrorKind::Cancelled,
            "export cancelled — no file was written",
        ));
    };

    let mut all: Vec<serde_json::Value> = Vec::new();
    // The list endpoint is paged. Walk it so the export is complete rather
    // than truncated at the first page.
    let mut offset = 0u32;
    const PAGE: u32 = 100;
    loop {
        let page: serde_json::Value = state
            .client
            .request(
                reqwest::Method::GET,
                "/v1/vault/secrets",
                &[("limit", PAGE.to_string()), ("offset", offset.to_string())],
                None,
            )
            .await?;
        let batch = page
            .get("secrets")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let count = batch.len() as u32;
        all.extend(batch);
        offset += PAGE;
        if count < PAGE {
            break;
        }
        // Hard stop so a server that ignores `offset` cannot spin forever.
        if offset > 100_000 {
            break;
        }
    }

    let total = all.len();
    let user = state.client.current_user().await;
    let export = SecretExport {
        format_version: 1,
        exported_at: rfc3339_now(),
        base_url: state.client.base_url().await,
        tenant_id: user.map(|u| u.tenant_id).unwrap_or_default(),
        secrets: all,
    };

    let json = serde_json::to_string_pretty(&export)
        .map_err(|e| AppError::new(AppErrorKind::Unknown, e))?;
    tokio::fs::write(&path, json).await.map_err(|e| {
        AppError::new(
            AppErrorKind::Storage,
            format!("writing export to {}: {e}", path.display()),
        )
    })?;

    Ok(serde_json::json!({
        "path": path.to_string_lossy(),
        "count": total,
    }))
}

/// Reads a metadata export back and reports what it contains.
///
/// The file is chosen by the user in the native open dialog; the webview
/// supplies no path. The file is parsed and summarised; nothing is written
/// to the vault. That is a separate, explicit action — the user should see
/// what an import would create before it creates anything.
#[tauri::command]
pub async fn read_secret_export(app: AppHandle) -> Result<serde_json::Value, AppError> {
    let Some(path) = pick_open_path(&app).await else {
        return Err(AppError::new(
            AppErrorKind::Cancelled,
            "import cancelled — no file was read",
        ));
    };

    let raw = tokio::fs::read_to_string(&path).await.map_err(|e| {
        AppError::new(
            AppErrorKind::Storage,
            format!("reading export {}: {e}", path.display()),
        )
    })?;
    let parsed: SecretExport = serde_json::from_str(&raw).map_err(|e| {
        AppError::new(
            AppErrorKind::InvalidInput,
            format!("{} is not a valid vault export: {e}", path.display()),
        )
    })?;
    if parsed.format_version != 1 {
        return Err(AppError::new(
            AppErrorKind::InvalidInput,
            format!(
                "unsupported export format version {} (this client reads version 1)",
                parsed.format_version
            ),
        ));
    }
    Ok(serde_json::json!({
        "format_version": parsed.format_version,
        "exported_at": parsed.exported_at,
        "base_url": parsed.base_url,
        "tenant_id": parsed.tenant_id,
        "count": parsed.secrets.len(),
        "secrets": parsed.secrets,
    }))
}

/// RFC 3339 UTC timestamp, formatted without pulling in a date-time crate.
///
/// `chrono` is not a dependency of this crate and the format only needs
/// second precision, so this is done by hand.
pub fn rfc3339_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    format_epoch_seconds(secs)
}

/// Converts epoch seconds to an RFC 3339 UTC timestamp.
///
/// The civil-date conversion is Howard Hinnant's `civil_from_days`
/// algorithm, which is exact for every date the Gregorian calendar
/// covers.
fn format_epoch_seconds(secs: i64) -> String {
    // Days since the epoch → civil date.
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_epoch_formats_correctly() {
        assert_eq!(format_epoch_seconds(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn a_known_date_formats_correctly() {
        // 2026-11-04T19:26:13Z — the timestamp on the published beta
        // manifest, used here as an independent check on the conversion.
        assert_eq!(format_epoch_seconds(1_793_820_373), "2026-11-04T19:26:13Z");
    }

    #[test]
    fn a_leap_day_formats_correctly() {
        // 2024-02-29T12:00:00Z
        assert_eq!(format_epoch_seconds(1_709_208_000), "2024-02-29T12:00:00Z");
    }

    #[test]
    fn the_current_timestamp_is_rfc3339() {
        let now = rfc3339_now();
        assert_eq!(now.len(), 20, "expected YYYY-MM-DDTHH:MM:SSZ");
        assert!(now.ends_with('Z'));
        assert_eq!(now.chars().filter(|c| *c == '-').count(), 2);
    }
}
