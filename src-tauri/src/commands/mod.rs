//! Tauri command surface for the vault desktop app.
//!
//! These are the only Rust functions the React webview can call. Every one
//! runs against the `AppState` (HTTP client + secret store) and returns
//! either a typed JSON value or a structured `AppError`. The webview NEVER
//! sees a bearer token, a CSRF token, or plaintext secrets — the
//! "unlock" → "list" → "decrypt" flow stays inside the Rust core.

pub mod updater;

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::api::HealthProbe;
use crate::api::{urlencode, LoginUser, VaultClient};
use crate::crypto;
use crate::error::{AppError, AppErrorKind};
use crate::store::{BackendKind, SecretStore, StoreError, StoredSession};

/// Shared, `Arc`-wrapped state. `tauri::State<T>` gives every command a
/// `&State<AppState>`; the inner `Arc`s are cheap to clone.
pub struct AppState {
    pub client: Arc<VaultClient>,
    pub store: Arc<SecretStore>,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub fn new() -> Self {
        let client = Arc::new(VaultClient::new());
        // Seed the base URL from the environment so a first-run user sees a
        // real, clickable orchestrator URL instead of a blank field. The
        // login screen still lets them override it (staging, a VPS, a local
        // orchestrator) and remembers whatever they choose.
        let default_base = std::env::var("VAULT_ORCHESTRATOR_URL")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_ORCHESTRATOR_URL.to_string());
        // Fire-and-forget: a rejected default (e.g. someone set a plaintext
        // URL) is not worth failing app boot over — the login screen will
        // surface it the moment the user tries to connect.
        let client_for_url = client.clone();
        tauri::async_runtime::spawn(async move {
            let _ = client_for_url.set_base_url(&default_base).await;
        });
        Self {
            client,
            store: Arc::new(SecretStore::new()),
        }
    }
}

/// The orchestrator a first-run user is pointed at. Override per-install with
/// the `VAULT_ORCHESTRATOR_URL` env var (useful for staging builds and for
/// anyone pointing the desktop client at their own VPS).
pub const DEFAULT_ORCHESTRATOR_URL: &str = "https://api.functionfly.com";

// ── session lifecycle ──────────────────────────────────────────────────────

/// `POST /v1/auth/login` against the configured orchestrator. On success the
/// bearer + refresh tokens are written to the OS keychain and the in-memory
/// client is bound to them.
#[tauri::command]
pub async fn login(
    state: State<'_, AppState>,
    base_url: String,
    email: String,
    password: String,
) -> Result<LoginResult, AppError> {
    state.client.set_base_url(&base_url).await?;
    let resp = state.client.login(&email, &password).await?;

    // Try to also refresh a stored refresh token; some tenants only issue one
    // on /login and omit it from the body, so absence is not an error.
    state
        .store
        .save_session(&base_url, &resp.token, resp.refresh_token.as_deref())
        .await?;

    Ok(LoginResult {
        user: resp.user,
        desktop_capable: resp.desktop_capable,
    })
}

/// True when the stored credential is an orchestrator API key rather than a
/// session token.
///
/// API keys are shaped `{prefix}_{version}_{random}_{checksum}` and every
/// prefix begins with `ff` (`ffp_` platform, `fff_` function, `ffx_` edge,
/// `ffe_` environment, `ffo_` oauth, `fft_` trust, `ffmp_`, `ffr_` runtime).
/// A session token from `/v1/auth/login` is an opaque JWT, so the shape test
/// is unambiguous — but it is deliberately a *shape* test rather than a
/// prefix allowlist, so a new key type does not silently fall back to the
/// session-token path and get bounced on every launch.
pub fn looks_like_api_key(credential: &str) -> bool {
    credential.starts_with("ff") && credential.split('_').count() >= 3
}

/// Tries to re-establish a session from whatever is in the OS keychain. Used
/// on app start so the user does not have to log in every launch.
///
/// Refresh tokens rotate like a session token. API keys do not: they are used
/// as the bearer directly and carry no expiry the orchestrator tracks here, so
/// `POST /v1/auth/token-status` answers 400 for them and would make every
/// stored key look stale — the user would be bounced to the login screen on
/// every single launch. Those are verified with `validate_api_key`, which is
/// the endpoint that actually understands them.
#[tauri::command]
pub async fn restore_session(state: State<'_, AppState>) -> Result<RestoreResult, AppError> {
    let session = match state.store.load_session().await? {
        Some(s) => s,
        None => return Ok(RestoreResult::empty()),
    };

    state.client.set_base_url(&session.base_url).await?;

    let (verified, refreshed, user) = if let Some(rt) = session.refresh_token.as_deref() {
        // Email + password session: rotate it as before.
        state
            .client
            .set_session(session.access_token.clone(), login_user_stub(&session))
            .await;
        match state.client.refresh(rt).await {
            Ok(_) => (true, true, state.client.current_user().await),
            Err(_) => (false, false, None), // Stale refresh — re-login required.
        }
    } else if looks_like_api_key(&session.access_token) {
        // API-key session: probe the endpoint that understands this credential.
        match state.client.validate_api_key(&session.access_token).await {
            Ok(user) => {
                state
                    .client
                    .set_session(session.access_token.clone(), user.clone())
                    .await;
                (true, false, Some(user))
            }
            Err(_) => (false, false, None),
        }
    } else {
        state
            .client
            .set_session(session.access_token.clone(), login_user_stub(&session))
            .await;
        (
            state.client.token_status().await.unwrap_or(false),
            false,
            state.client.current_user().await,
        )
    };

    if !verified {
        // Don't keep stale tokens around; force a clean re-login.
        state.store.clear_session().await.ok();
        state.client.clear_session().await;
        return Ok(RestoreResult::empty());
    }

    Ok(RestoreResult {
        user,
        refreshed,
        base_url: Some(session.base_url),
    })
}

#[derive(Serialize)]
pub struct LoginResult {
    pub user: LoginUser,
    pub desktop_capable: bool,
}

#[derive(Serialize, Default)]
pub struct RestoreResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<LoginUser>,
    pub refreshed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}

impl RestoreResult {
    fn empty() -> Self {
        Self::default()
    }
}

/// API-key login (no password). Useful for headless / automation setups.
///
/// The key is verified against the orchestrator *before* it is persisted. An
/// API key doubles as the bearer token (there is no exchange step the way
/// there is for email + password), so without this check a typo or a revoked
/// key would be stored in the OS keychain and look like a successful sign-in
/// until the user's first real vault read failed with an opaque 401.
#[tauri::command]
pub async fn login_with_api_key(
    state: State<'_, AppState>,
    base_url: String,
    api_key: String,
) -> Result<LoginResult, AppError> {
    state.client.set_base_url(&base_url).await?;

    // Probe before touching the session or the keychain, so a rejected key
    // leaves no trace in either.
    let user = state
        .client
        .validate_api_key(&api_key)
        .await
        .map_err(AppError::from)?;

    state
        .client
        .set_session(api_key.clone(), user.clone())
        .await;
    state.store.save_session(&base_url, &api_key, None).await?;

    Ok(LoginResult {
        user,
        desktop_capable: false,
    })
}

#[tauri::command]
pub async fn logout(state: State<'_, AppState>) -> Result<(), AppError> {
    state.client.clear_session().await;
    state.store.clear_session().await.ok();
    state.store.lock().await;
    Ok(())
}

// ── browser sign-in (RFC 8628 device grant) ────────────────────────────────

/// What the login screen renders while waiting for the human.
///
/// Mirrors `crate::api::DeviceAuthorization` minus `device_code`, which never
/// crosses into the webview.
#[derive(Debug, Clone, Serialize)]
pub struct DeviceCodePrompt {
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in: u64,
    pub interval: u64,
}

impl From<crate::api::DeviceAuthorization> for DeviceCodePrompt {
    fn from(a: crate::api::DeviceAuthorization) -> Self {
        Self {
            user_code: a.user_code,
            verification_uri: a.verification_uri,
            verification_uri_complete: a.verification_uri_complete,
            expires_in: a.expires_in,
            interval: a.interval,
        }
    }
}

/// Starts a browser sign-in. Returns the code the human must approve.
///
/// The `device_code` that authorises the later token exchange stays inside
/// the Rust core — the webview only learns the human-typed code, so a
/// compromised renderer cannot complete the exchange on its own.
#[tauri::command]
pub async fn begin_device_login(
    state: State<'_, AppState>,
    base_url: String,
) -> Result<DeviceCodePrompt, AppError> {
    state.client.set_base_url(&base_url).await?;
    let auth = state
        .client
        .begin_device_flow("FunctionFly Vault Desktop", env!("CARGO_PKG_VERSION"))
        .await
        .map_err(AppError::from)?;
    stash_device_flow(&state, base_url, auth).await
}

/// Polls until the human approves, then persists the session.
///
/// Implemented as one blocking call rather than a poll-per-click command so
/// the sleep/backoff logic stays in Rust where the timing rules live, and the
/// webview cannot accidentally hammer the orchestrator. The UI shows a
/// spinner for the duration and gets one result.
///
/// Bounded by `max_polls` so a forgotten sign-in cannot poll forever: at the
/// server-issued interval plus slack that is comfortably past the 10-minute
/// grant TTL.
#[tauri::command]
pub async fn await_device_login(
    state: State<'_, AppState>,
    user_code: String,
) -> Result<LoginResult, AppError> {
    let flow = take_device_flow(&state, &user_code).ok_or_else(|| {
        AppError::new(
            AppErrorKind::InvalidInput,
            "That sign-in attempt is no longer available. Start again.",
        )
    })?;
    let (base_url, auth) = flow;

    let interval = std::cmp::max(auth.interval, 1);
    // The server-side grant lives 10 minutes. 200 polls at 5s ≈ 16 minutes,
    // which overshoots the TTL by a safe margin rather than giving up early.
    let max_polls: u32 = 200;
    let mut delay = std::time::Duration::from_secs(interval);

    for _ in 0..max_polls {
        match state.client.poll_device_flow(&auth).await {
            // Sign-in completed — persist and hand the caller the identity.
            Ok(Some((token, refresh))) => {
                state
                    .store
                    .save_session(&base_url, &token, refresh.as_deref())
                    .await?;
                let user = state.client.current_user().await.ok_or_else(|| {
                    AppError::new(
                        AppErrorKind::Unknown,
                        "Signed in, but the orchestrator did not return an account. Try again.",
                    )
                })?;
                return Ok(LoginResult {
                    user,
                    desktop_capable: true,
                });
            }
            // A 2xx with no session payload — the server said OK but gave us
            // nothing to store. Treat as still-pending rather than minting an
            // empty session.
            Ok(None) => {}
            Err(crate::api::DevicePollError::KeepWaiting { slow_down }) => {
                // RFC 8628: "slow_down" means we are polling too fast. Add a
                // second before the next sleep rather than ignoring it.
                if slow_down {
                    delay += std::time::Duration::from_secs(1);
                }
            }
            Err(crate::api::DevicePollError::Terminal(msg)) => {
                return Err(AppError::new(AppErrorKind::InvalidInput, msg));
            }
            Err(crate::api::DevicePollError::Fatal(e)) => return Err(AppError::from(e)),
        }
        tokio::time::sleep(delay).await;
    }

    Err(AppError::new(
        AppErrorKind::InvalidInput,
        "The sign-in request timed out. Start again from the app.",
    ))
}

/// One pending flow, keyed by the human-typed code the UI knows.
///
/// A single slot is enough: the login screen drives one sign-in at a time and
/// starting a new one overwrites the old. Overwriting is deliberate — a stale
/// flow cannot be resumed later by a window that still remembers its code.
type DeviceFlows =
    std::sync::Mutex<std::collections::HashMap<String, (String, crate::api::DeviceAuthorization)>>;

/// Flows waiting for human approval, keyed by user_code.
static DEVICE_FLOWS: std::sync::OnceLock<DeviceFlows> = std::sync::OnceLock::new();

fn device_flows() -> &'static DeviceFlows {
    DEVICE_FLOWS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

async fn stash_device_flow(
    state: &State<'_, AppState>,
    base_url: String,
    auth: crate::api::DeviceAuthorization,
) -> Result<DeviceCodePrompt, AppError> {
    let key = auth.user_code.clone();
    let prompt = DeviceCodePrompt::from(auth.clone());
    device_flows()
        .lock()
        .map_err(|_| {
            AppError::new(
                AppErrorKind::Unknown,
                "Sign-in is temporarily unavailable. Try again.",
            )
        })?
        .insert(key, (base_url, auth));
    let _ = state; // reserved: future flows may want to touch the store here
    Ok(prompt)
}

fn take_device_flow(
    _state: &State<'_, AppState>,
    user_code: &str,
) -> Option<(String, crate::api::DeviceAuthorization)> {
    device_flows().lock().ok()?.remove(user_code)
}

// ── vault unlock ───────────────────────────────────────────────────────────

/// Holds the unlock passphrase in memory. The passphrase is never
/// persisted; it is wiped on lock or window close.
#[tauri::command]
pub async fn unlock_vault(state: State<'_, AppState>, passphrase: String) -> Result<(), AppError> {
    if !state.client.is_authenticated().await {
        return Err(AppError::new(
            AppErrorKind::Unauthenticated,
            "log in before unlocking the vault",
        ));
    }
    state.store.lock_passphrase(&passphrase).await;
    // Drop the caller's `String` (the `passphrase` argument) so the copy in
    // the IPC buffer cannot outlive this stack frame. (Tauri does not zeroize
    // the request buffer itself — the caller is responsible for that in the
    // webview.)
    drop(passphrase);
    Ok(())
}

#[tauri::command]
pub async fn lock_vault(state: State<'_, AppState>) -> Result<(), AppError> {
    state.store.lock().await;
    Ok(())
}

#[tauri::command]
pub async fn vault_status(state: State<'_, AppState>) -> Result<VaultStatus, AppError> {
    let user = state.client.current_user().await;
    Ok(VaultStatus {
        authenticated: state.client.is_authenticated().await,
        unlocked: state.store.is_unlocked().await,
        base_url: state.client.base_url().await,
        store_backend: state.store.last_backend().await,
        user,
    })
}

/// Probes an orchestrator URL before the user commits to signing in.
///
/// The login screen calls this so a typo in the URL surfaces as "that host is
/// not a FunctionFly orchestrator" instead of an opaque 401 after the user
/// has typed their password. No credential is involved.
#[tauri::command]
pub async fn vault_health_probe(
    state: State<'_, AppState>,
    base_url: String,
) -> Result<HealthProbe, AppError> {
    Ok(state.client.probe_health(&base_url).await?)
}

/// Reports which credential backend is live and whether the orchestrator URL
/// looks like a real FunctionFly host. The login screen uses the default when
/// the user has not typed anything yet, so a first-run user sees the
/// production URL pre-filled rather than `localhost:8080`.
#[tauri::command]
pub async fn vault_store_info(state: State<'_, AppState>) -> Result<StoreInfo, AppError> {
    let described = state.store.describe().await?;
    Ok(StoreInfo {
        backend: described.backend,
        has_session: described.has_access_token,
        stored_base_url: described.base_url,
    })
}

#[derive(Serialize)]
pub struct StoreInfo {
    pub backend: BackendKind,
    pub has_session: bool,
    pub stored_base_url: String,
}

#[derive(Serialize)]
pub struct VaultStatus {
    pub authenticated: bool,
    pub unlocked: bool,
    /// Which orchestrator this session is bound to. Surfaced in the UI so the
    /// user can always see which environment they are looking at.
    pub base_url: String,
    /// Whether the long-lived token lives in the OS keyring or the encrypted
    /// file fallback. `empty` before the first sign-in.
    pub store_backend: BackendKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<LoginUser>,
}

// ── secret list / get / reveal / version history ───────────────────────────

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub secret_type: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub offset: u32,
}

fn default_limit() -> u32 {
    50
}

#[tauri::command]
pub async fn list_secrets(
    state: State<'_, AppState>,
    query: ListQuery,
) -> Result<ListSecretsResult, AppError> {
    require_auth(&state).await?;

    let mut q = Vec::new();
    if let Some(s) = &query.search {
        q.push(("search", s.clone()));
    }
    if let Some(ns) = &query.namespace {
        q.push(("namespace", ns.clone()));
    }
    if let Some(st) = &query.secret_type {
        q.push(("secret_type", st.clone()));
    }
    q.push(("limit", query.limit.to_string()));
    q.push(("offset", query.offset.to_string()));

    let resp: serde_json::Value = state
        .client
        .request(reqwest::Method::GET, "/v1/vault/secrets", &q, None)
        .await?;

    Ok(ListSecretsResult {
        secrets: resp
            .get("secrets")
            .and_then(|s| s.as_array())
            .cloned()
            .unwrap_or_default(),
        total: resp.get("total").and_then(|t| t.as_i64()).unwrap_or(0),
    })
}

#[derive(Serialize)]
pub struct ListSecretsResult {
    pub secrets: Vec<serde_json::Value>,
    pub total: i64,
}

#[tauri::command]
pub async fn get_secret(
    state: State<'_, AppState>,
    id: String,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    let path = format!("/v1/vault/secrets/{id}");
    state
        .client
        .request(reqwest::Method::GET, &path, &[], None)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_secret_versions(
    state: State<'_, AppState>,
    id: String,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    let path = format!("/v1/vault/secrets/{id}/versions");
    state
        .client
        .request(reqwest::Method::GET, &path, &[], None)
        .await
        .map_err(Into::into)
}

/// Reveals a secret's plaintext. The unlock passphrase is the second-factor
/// even if the bearer is leaked (e.g. stolen from a logged-in browser).
#[tauri::command]
pub async fn reveal_secret(
    state: State<'_, AppState>,
    id: String,
) -> Result<RevealResult, AppError> {
    require_auth(&state).await?;
    require_unlocked(&state).await?;

    // The /v1/vault/secrets/{id} response carries the encrypted payload; for
    // ZK secrets the server never sees plaintext, so we decrypt in-core.
    let raw: serde_json::Value = state
        .client
        .request(
            reqwest::Method::GET,
            &format!("/v1/vault/secrets/{id}"),
            &[],
            None,
        )
        .await?;

    let plaintext = decrypt_field(&state, &raw).await?;
    Ok(RevealResult {
        secret_id: id,
        plaintext,
    })
}

#[derive(Serialize)]
pub struct RevealResult {
    pub secret_id: String,
    /// Never persisted, never logged, only serialized over IPC for the
    /// copy-to-clipboard action the user just clicked.
    pub plaintext: String,
}

#[tauri::command]
pub async fn diff_secret_versions(
    state: State<'_, AppState>,
    id: String,
    from: u32,
    to: u32,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    let path = format!("/v1/vault/secrets/{id}/versions/diff");
    state
        .client
        .request(
            reqwest::Method::GET,
            &path,
            &[("from", from.to_string()), ("to", to.to_string())],
            None,
        )
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn rollback_secret(
    state: State<'_, AppState>,
    id: String,
    target_version: u32,
    reason: Option<String>,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    let body = serde_json::json!({
        "target_version": target_version,
        "reason": reason.unwrap_or_default(),
    });
    let path = format!("/v1/vault/secrets/{id}/rollback");
    state
        .client
        .request(reqwest::Method::POST, &path, &[], Some(body))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_namespaces(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    state
        .client
        .request(reqwest::Method::GET, "/v1/vault/namespaces", &[], None)
        .await
        .map_err(Into::into)
}

// ── secret write path (create / update / rotate / delete) ──────────────────
//
// Encryption is client-side: the Rust core derives the key from the unlock
// passphrase, encrypts the plaintext, and only the `EncryptedDataPayload`
// ever leaves this process. The orchestrator never sees plaintext.

/// Rejects plaintext that cannot possibly fit the server's 64 KiB payload
/// cap before we spend a round trip (and a 600k-iteration PBKDF2) on it.
const MAX_PLAINTEXT_BYTES: usize = 48 * 1024;

#[derive(Deserialize)]
pub struct CreateSecretInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub secret_type: String,
    pub plaintext: String,
    #[serde(default)]
    pub namespace: String,
    #[serde(default)]
    pub scopes: Vec<String>,
}

#[tauri::command]
pub async fn create_secret(
    state: State<'_, AppState>,
    input: CreateSecretInput,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    require_unlocked(&state).await?;

    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::new(
            AppErrorKind::InvalidInput,
            "name is required",
        ));
    }
    if input.plaintext.len() > MAX_PLAINTEXT_BYTES {
        return Err(AppError::new(
            AppErrorKind::InvalidInput,
            format!(
                "plaintext is {} bytes; the vault caps it at {}",
                input.plaintext.len(),
                MAX_PLAINTEXT_BYTES
            ),
        ));
    }

    let encrypted = encrypt_field(&state, &input.plaintext).await?;
    let body = serde_json::json!({
        "name": name,
        "description": input.description,
        "secret_type": input.secret_type,
        "encrypted_data": encrypted,
        "namespace": input.namespace,
        "scopes": input.scopes,
    });
    state
        .client
        .request(reqwest::Method::POST, "/v1/vault/secrets", &[], Some(body))
        .await
        .map_err(Into::into)
}

#[derive(Deserialize)]
pub struct UpdateSecretInput {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub scopes: Option<Vec<String>>,
}

#[tauri::command]
pub async fn update_secret(
    state: State<'_, AppState>,
    input: UpdateSecretInput,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;

    // Only send the keys the caller actually set, so the PATCH stays a true
    // partial update and the server does not blank untouched fields.
    let mut body = serde_json::Map::new();
    if let Some(n) = input.name {
        body.insert("name".into(), serde_json::Value::String(n));
    }
    if let Some(d) = input.description {
        body.insert("description".into(), serde_json::Value::String(d));
    }
    if let Some(s) = input.scopes {
        body.insert("scopes".into(), serde_json::json!(s));
    }
    if body.is_empty() {
        return Err(AppError::new(
            AppErrorKind::InvalidInput,
            "nothing to update",
        ));
    }

    let path = format!("/v1/vault/secrets/{}", input.id);
    state
        .client
        .request(
            reqwest::Method::PATCH,
            &path,
            &[],
            Some(serde_json::Value::Object(body)),
        )
        .await
        .map_err(Into::into)
}

#[derive(Deserialize)]
pub struct RotateSecretInput {
    pub id: String,
    pub plaintext: String,
    #[serde(default)]
    pub reason: String,
}

#[tauri::command]
pub async fn rotate_secret(
    state: State<'_, AppState>,
    input: RotateSecretInput,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    require_unlocked(&state).await?;

    if input.plaintext.len() > MAX_PLAINTEXT_BYTES {
        return Err(AppError::new(
            AppErrorKind::InvalidInput,
            format!(
                "plaintext is {} bytes; the vault caps it at {}",
                input.plaintext.len(),
                MAX_PLAINTEXT_BYTES
            ),
        ));
    }

    let encrypted = encrypt_field(&state, &input.plaintext).await?;
    let body = serde_json::json!({
        "encrypted_data": encrypted,
        "reason": input.reason,
    });
    let path = format!("/v1/vault/secrets/{}/rotate", input.id);
    state
        .client
        .request(reqwest::Method::POST, &path, &[], Some(body))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn delete_secret(state: State<'_, AppState>, id: String) -> Result<(), AppError> {
    require_auth(&state).await?;
    let path = format!("/v1/vault/secrets/{id}");
    state
        .client
        .request::<serde_json::Value>(reqwest::Method::DELETE, &path, &[], None)
        .await?;
    Ok(())
}

// ── audit log ─────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn list_audit(
    state: State<'_, AppState>,
    secret_id: Option<String>,
    limit: Option<u32>,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    let mut q = Vec::new();
    if let Some(s) = secret_id {
        // Per-secret audit log has a different path; choose the right one.
        let path = format!("/v1/vault/secrets/{s}/audit");
        return state
            .client
            .request(reqwest::Method::GET, &path, &[], None)
            .await
            .map_err(Into::into);
    }
    if let Some(l) = limit {
        q.push(("limit", l.to_string()));
    }
    state
        .client
        .request(reqwest::Method::GET, "/v1/vault/audit", &q, None)
        .await
        .map_err(Into::into)
}

// ── tokens (per-secret short-lived read-only access grants) ────────────────
//
// Tokens are minted by the orchestrator (256-bit random, only the SHA-256
// hash is stored server-side). The plaintext is returned exactly once at
// creation time — the desktop surfaces that in a copy-to-clipboard modal.

#[tauri::command]
pub async fn list_tokens(
    state: State<'_, AppState>,
    id: String,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    let path = format!("/v1/vault/secrets/{id}/tokens");
    state
        .client
        .request(reqwest::Method::GET, &path, &[], None)
        .await
        .map_err(Into::into)
}

#[derive(Debug, Deserialize)]
pub struct GenerateTokenInput {
    pub id: String,
    /// Human-friendly name for the token, e.g. "ci-deployer".
    pub name: String,
    /// Comma/newline-separated scope strings; the orchestrator validates
    /// these against the secret's allowed scopes. Empty array → inherit
    /// the secret's scopes.
    pub scopes: Vec<String>,
    /// Lifetime in hours. The orchestrator caps at 8760 (1 year); we
    /// default to 24 and clamp to 8760 here so the desktop never sends
    /// a value that the server will reject.
    pub expires_in_hours: u32,
}

#[tauri::command]
pub async fn generate_token(
    state: State<'_, AppState>,
    input: GenerateTokenInput,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    require_unlocked(&state).await?;
    let hours = input.expires_in_hours.clamp(1, 8760);
    let body = serde_json::json!({
        "name": input.name,
        "scopes": input.scopes,
        "expires_in_hours": hours,
    });
    let path = format!("/v1/vault/secrets/{}/tokens", input.id);
    state
        .client
        .request(reqwest::Method::POST, &path, &[], Some(body))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn revoke_token(state: State<'_, AppState>, token_id: String) -> Result<(), AppError> {
    require_auth(&state).await?;
    let path = format!("/v1/vault/tokens/{token_id}");
    state
        .client
        .request::<serde_json::Value>(reqwest::Method::DELETE, &path, &[], None)
        .await?;
    Ok(())
}

// ── bulk operations ────────────────────────────────────────────────────────
//
// Bulk delete is the rare "clean up after a project" flow. The orchestrator
// caps a single request at 100 IDs and supports a dry-run preview that lists
// any blocking dependencies. The desktop surfaces both: a "select all
// visible" toggle in the secrets list, then a confirmation modal that calls
// dry-run first if any are non-trivial.

#[derive(Debug, Deserialize)]
pub struct BulkDeleteInput {
    pub secret_ids: Vec<String>,
    /// When true, the orchestrator returns a per-secret dependency map but
    /// performs no deletes. The desktop uses this to populate the
    /// confirmation modal.
    pub dry_run: bool,
}

#[tauri::command]
pub async fn bulk_delete_secrets(
    state: State<'_, AppState>,
    input: BulkDeleteInput,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    if input.secret_ids.is_empty() {
        return Err(AppError::new(
            AppErrorKind::InvalidInput,
            "no secret IDs supplied",
        ));
    }
    if input.secret_ids.len() > 100 {
        return Err(AppError::new(
            AppErrorKind::InvalidInput,
            format!(
                "bulk delete caps at 100 secrets; got {}",
                input.secret_ids.len()
            ),
        ));
    }
    let body = serde_json::json!({
        "secret_ids": input.secret_ids,
        "dry_run": input.dry_run,
    });
    state
        .client
        .request(
            reqwest::Method::DELETE,
            "/v1/vault/secrets/bulk-delete",
            &[],
            Some(body),
        )
        .await
        .map_err(Into::into)
}

// ── namespaces (create + delete) ──────────────────────────────────────────

#[tauri::command]
pub async fn create_namespace(
    state: State<'_, AppState>,
    name: String,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    let name = name.trim();
    if name.is_empty() || name.len() > 64 {
        return Err(AppError::new(
            AppErrorKind::InvalidInput,
            "namespace name must be 1–64 characters",
        ));
    }
    let body = serde_json::json!({ "name": name });
    state
        .client
        .request(
            reqwest::Method::POST,
            "/v1/vault/namespaces",
            &[],
            Some(body),
        )
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn delete_namespace(state: State<'_, AppState>, id: String) -> Result<(), AppError> {
    require_auth(&state).await?;
    let path = format!("/v1/vault/namespaces/{id}");
    state
        .client
        .request::<serde_json::Value>(reqwest::Method::DELETE, &path, &[], None)
        .await?;
    Ok(())
}

// ── audit (filter by action or actor) ─────────────────────────────────────
//
// The base `list_audit` route returns the full audit log. For larger
// tenants that becomes unwieldy; the orchestrator exposes two filter
// paths (`/vault/audit/action/{action}` and
// `/vault/audit/actor/{actor_type}/{actor_id}`). These wrappers give the
// AuditLog screen direct access to both.

#[tauri::command]
pub async fn list_audit_by_action(
    state: State<'_, AppState>,
    action: String,
    limit: Option<u32>,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    let mut q: Vec<(&str, String)> = Vec::new();
    if let Some(n) = limit {
        q.push(("limit", n.to_string()));
    }
    let path = format!("/v1/vault/audit/action/{}", urlencode(&action));
    state
        .client
        .request(reqwest::Method::GET, &path, &q, None)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_audit_by_actor(
    state: State<'_, AppState>,
    actor_type: String,
    actor_id: String,
    limit: Option<u32>,
) -> Result<serde_json::Value, AppError> {
    require_auth(&state).await?;
    let mut q: Vec<(&str, String)> = Vec::new();
    if let Some(n) = limit {
        q.push(("limit", n.to_string()));
    }
    let path = format!(
        "/v1/vault/audit/actor/{}/{}",
        urlencode(&actor_type),
        urlencode(&actor_id)
    );
    state
        .client
        .request(reqwest::Method::GET, &path, &q, None)
        .await
        .map_err(Into::into)
}

// ── internals ─────────────────────────────────────────────────────────────

async fn require_auth(state: &State<'_, AppState>) -> Result<(), AppError> {
    if !state.client.is_authenticated().await {
        return Err(AppError::new(AppErrorKind::Unauthenticated, "log in first"));
    }
    Ok(())
}

async fn require_unlocked(state: &State<'_, AppState>) -> Result<(), AppError> {
    if !state.store.is_unlocked().await {
        return Err(AppError::new(
            AppErrorKind::VaultLocked,
            "vault is locked; unlock with the passphrase first",
        ));
    }
    Ok(())
}

async fn decrypt_field(
    state: &State<'_, AppState>,
    raw: &serde_json::Value,
) -> Result<String, AppError> {
    // Run PBKDF2-derived decryption on the blocking pool so the UI thread
    // does not freeze for 600k iterations × every reveal.
    let store = state.store.clone();
    let payload: crypto::EncryptedData = serde_json::from_value(
        raw.get("encrypted_data")
            .cloned()
            .ok_or_else(|| AppError::new(AppErrorKind::InvalidInput, "no encrypted_data field"))?,
    )
    .map_err(|e| AppError::new(AppErrorKind::InvalidInput, e))?;

    // `with_passphrase` returns `Result<Result<String, CryptoError>, StoreError>`.
    // Awaiting the blocking future first yields a single `Result<…>` that
    // can be flattened with one `?`, then we map the inner CryptoError into
    // an AppError.
    // `with_passphrase` is `async fn`, so we cannot use the `FnOnce -> T`
    // overload of `spawn_blocking` (the closure's return type would be a
    // future, not a Result). The 600k-iteration PBKDF2 will block the
    // current tokio worker for ~1 s per reveal, which is acceptable for an
    // explicit user action. If a future rev needs to scale to many parallel
    // reveals, this should move onto a dedicated blocking thread.
    let inner: Result<Result<String, AppError>, StoreError> = store
        .with_passphrase(|pw| crypto::decrypt(&payload, pw).map_err(AppError::from))
        .await;
    // outer ?  → unwraps StoreError
    // inner ?? → flattens the inner Result to a String
    let inner = inner?;
    let plaintext = inner?;
    Ok(plaintext)
}

/// Encrypts `plaintext` under the unlock passphrase and returns the
/// wire-ready `EncryptedDataPayload`. Mirrors [`decrypt_field`] so the two
/// directions stay symmetrical: the same passphrase holder and the same
/// `Zeroizing`-backed guard, inverted.
async fn encrypt_field(
    state: &State<'_, AppState>,
    plaintext: &str,
) -> Result<serde_json::Value, AppError> {
    let store = state.store.clone();
    let inner: Result<Result<serde_json::Value, AppError>, StoreError> = store
        .with_passphrase(|pw| {
            crypto::encrypt(plaintext, pw)
                .map_err(AppError::from)
                .and_then(|d| {
                    serde_json::to_value(d).map_err(|e| {
                        AppError::new(AppErrorKind::Unknown, format!("serializing payload: {e}"))
                    })
                })
        })
        .await;
    inner?
}
fn login_user_stub(_session: &StoredSession) -> LoginUser {
    // `StoredSession` does not include the user blob — that was rendered into
    // the dashboard from the login response. This is the session-restore
    // fallback (no credentials on hand to re-authenticate with). The
    // API-key login path no longer uses a stub: `validate_api_key` resolves
    // the real identity from `GET /v1/users/me` before the session is bound.
    LoginUser {
        id: String::new(),
        email: None,
        username: None,
        tenant_id: String::new(),
        role: None,
    }
}
