//! Orchestrator HTTP client for the vault API.
//!
//! Runs entirely in the Rust core: the webview never holds a bearer token and
//! never speaks HTTP to the orchestrator. That mirrors the S6 posture already
//! documented in `studio-desktop/src-tauri/capabilities/default.toml`.
//!
//! Two things here are easy to get wrong and are handled explicitly:
//!
//! 1. **CSRF is mandatory on mutating `/v1/vault/*` calls.** The dashboard
//!    dodges this by using relative URLs; an absolute-URL client still gets
//!    validated by `middleware.RequireCSRF`, which only accepts the
//!    `X-CSRF-Token` *header* (a cookie alone is rejected as XSS-readable).
//!    We therefore always fetch `GET /v1/csrf` and send the header.
//!
//! 2. **One CSRF token per user per hour.** `GET /v1/csrf` issues one token
//!    keyed on the user ID, so concurrent fetches are wasteful but harmless.
//!    We cache it and refresh on the documented `csrf_token_invalid` 403.

use std::time::Duration;

use serde::Deserialize;
use tokio::sync::RwLock;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("not configured: {0}")]
    NotConfigured(String),
    #[error("authentication required")]
    Unauthenticated,
    #[error("request was rejected (CSRF); retrying")]
    CsrfRetry,
    #[error("rate limited by the vault; retry later")]
    RateLimited,
    #[error("vault plan does not allow this operation")]
    PlanDenied,
    #[error("not found")]
    NotFound,
    #[error("{status} {code}: {message}")]
    Api {
        status: u16,
        code: String,
        message: String,
    },
    #[error("could not reach the orchestrator: {0}")]
    Transport(String),
}

/// Parsed error envelope. Two shapes exist in the vault handlers:
/// newer ones use `apierror` (`{code, message}`), older ones emit
/// `{error, code, message}`. We accept both plus a bare `{"error": "..."}`.
#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CsrfResponse {
    token: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginResponse {
    pub token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    pub user: LoginUser,
    /// Signals that this deployment allows desktop downloads
    /// (`STUDIO_DESKTOP_DOWNLOAD_ENABLED`). Surfaced so the UI can show a
    /// helpful hint, but not used as a gate.
    #[serde(default)]
    pub desktop_capable: bool,
}

#[derive(Debug, Clone, Default, Deserialize, serde::Serialize)]
pub struct LoginUser {
    pub id: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    pub tenant_id: String,
    #[serde(default)]
    pub role: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RefreshResponse {
    pub token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
}

// ── Device Authorization Grant (RFC 8628) ──────────────────────────────────
//
// The desktop app holds no password. It asks for a grant, shows the human a
// short code, and polls until the human approves that code in a browser where
// they are already signed in. Same shape as `gh auth login`.

/// What the app needs to render the "enter this code" screen.
///
/// `device_code` never leaves the Rust side except to poll the token
/// endpoint; the UI only ever sees `user_code`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DeviceAuthorization {
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in: u64,
    pub interval: u64,
    /// Held privately by the client. Never serialized to the webview.
    #[serde(skip)]
    device_code: String,
}

impl DeviceAuthorization {
    /// Rebuilds a pending flow.
    ///
    /// Needed because `device_code` is deliberately private: nothing outside
    /// this module may mint one, so the only way to obtain the struct is from
    /// `begin_device_flow`. This constructor exists for the two legitimate
    /// cases — rehydrating a flow the app persisted across a restart, and
    /// tests that need a deterministic `device_code` without standing up a
    /// mock authorize endpoint.
    pub fn new(
        device_code: impl Into<String>,
        user_code: impl Into<String>,
        verification_uri: impl Into<String>,
        verification_uri_complete: impl Into<String>,
        expires_in: u64,
        interval: u64,
    ) -> Self {
        Self {
            device_code: device_code.into(),
            user_code: user_code.into(),
            verification_uri: verification_uri.into(),
            verification_uri_complete: verification_uri_complete.into(),
            expires_in,
            interval,
        }
    }
}

/// Why a poll did not yet produce a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DevicePollPending {
    /// The human has not acted yet — keep waiting.
    Pending,
    /// The grant expired — stop waiting.
    Expired,
    /// The human denied the request — stop waiting.
    Denied,
    /// The grant is unknown to the server — stop waiting.
    Invalid,
}

#[derive(Debug, Deserialize)]
struct DeviceAuthorizeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default)]
    verification_uri_complete: String,
    expires_in: u64,
    #[serde(default = "default_poll_interval")]
    interval: u64,
}

fn default_poll_interval() -> u64 {
    5
}

#[derive(Debug, Deserialize)]
struct DeviceTokenOk {
    token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    user: Option<DeviceTokenUser>,
}

#[derive(Debug, Clone, Deserialize)]
struct DeviceTokenUser {
    #[serde(default)]
    id: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    tenant_id: String,
    #[serde(default)]
    role: Option<String>,
}

/// The `{"error": "..."}` body every non-OK poll returns.
#[derive(Debug, Deserialize)]
struct OAuthErrorBody {
    #[serde(default)]
    error: String,
}

/// The subset of `GET /v1/users/me` that `LoginUser` needs. The handler emits
/// camelCase, so `tenantId` maps to `tenant_id` and an absent `username`
/// falls back to the display `name`.
#[derive(Debug, Deserialize)]
struct MeResponse {
    #[serde(default)]
    id: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(rename = "tenantId", default)]
    tenant_id: String,
    #[serde(default)]
    role: Option<String>,
}

/// Shared client state. `Arc`'d into Tauri state.
pub struct VaultClient {
    http: reqwest::Client,
    base_url: RwLock<String>,
    /// Bearer for the orchestrator, sourced from the OS keyring on startup.
    token: RwLock<Option<String>>,
    csrf: RwLock<Option<String>>,
    user: RwLock<Option<LoginUser>>,
}

impl Default for VaultClient {
    fn default() -> Self {
        Self::new()
    }
}

impl VaultClient {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            // rustls only — no OpenSSL dependency, consistent with
            // studio-desktop.
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .user_agent(concat!(
                "FunctionFlyVaultDesktop/",
                env!("CARGO_PKG_VERSION")
            ))
            .build()
            .expect("reqwest client construction is infallible with valid defaults");

        Self {
            http,
            base_url: RwLock::new(String::new()),
            token: RwLock::new(None),
            csrf: RwLock::new(None),
            user: RwLock::new(None),
        }
    }

    pub async fn set_base_url(&self, url: &str) -> Result<(), ApiError> {
        let trimmed = url.trim().trim_end_matches('/').to_string();
        // Validate eagerly so a typo surfaces at connect time, not mid-write.
        if !trimmed.starts_with("https://") && !is_loopback_http(&trimmed) {
            return Err(ApiError::NotConfigured(
                "orchestrator URL must be https:// (http:// allowed only for localhost)".into(),
            ));
        }
        *self.base_url.write().await = trimmed;
        Ok(())
    }

    pub async fn base_url(&self) -> String {
        self.base_url.read().await.clone()
    }

    pub async fn set_session(&self, token: String, user: LoginUser) {
        *self.token.write().await = Some(token);
        *self.user.write().await = Some(user);
        // A token change invalidates any cached CSRF token.
        *self.csrf.write().await = None;
    }

    pub async fn clear_session(&self) {
        *self.token.write().await = None;
        *self.user.write().await = None;
        *self.csrf.write().await = None;
    }

    pub async fn current_user(&self) -> Option<LoginUser> {
        self.user.read().await.clone()
    }

    pub async fn is_authenticated(&self) -> bool {
        self.token.read().await.is_some()
    }

    /// Exchanges credentials for a bearer token. `/v1/auth/login` is
    /// CSRF-exempt, so no token is needed here.
    pub async fn login(&self, email: &str, password: &str) -> Result<LoginResponse, ApiError> {
        let base = self.require_base().await?;
        #[derive(serde::Serialize)]
        struct Body<'a> {
            email: &'a str,
            password: &'a str,
        }
        let resp = self
            .http
            .post(format!("{base}/v1/auth/login"))
            .json(&Body { email, password })
            .send()
            .await
            .map_err(|e| ApiError::Transport(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            // In production without a configured captcha provider the backend
            // fails closed with 503; surface that verbatim rather than a
            // generic "bad credentials".
            let body = resp.text().await.unwrap_or_default();
            if status.as_u16() == 503 {
                return Err(ApiError::Api {
                    status: 503,
                    code: "captcha_not_configured".into(),
                    message: "This deployment requires a CAPTCHA on login, which the desktop \
                              client cannot complete. Use an API key instead."
                        .into(),
                });
            }
            return Err(classify(status.as_u16(), &body));
        }

        let parsed: LoginResponse = resp
            .json()
            .await
            .map_err(|e| ApiError::Transport(format!("decoding login response: {e}")))?;

        self.set_session(parsed.token.clone(), parsed.user.clone())
            .await;
        Ok(parsed)
    }

    /// Rotates the access token using the stored refresh token.
    ///
    /// NOTE: refresh is single-use — the old token is revoked before the new
    /// one is issued. If the write of the new refresh token fails the session
    /// is unrecoverable, so callers must persist atomically.
    pub async fn refresh(&self, refresh_token: &str) -> Result<String, ApiError> {
        let base = self.require_base().await?;
        #[derive(serde::Serialize)]
        struct Body<'a> {
            refresh_token: &'a str,
        }
        let resp = self
            .http
            .post(format!("{base}/v1/auth/refresh"))
            .json(&Body { refresh_token })
            .send()
            .await
            .map_err(|e| ApiError::Transport(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(classify(
                resp.status().as_u16(),
                &resp.text().await.unwrap_or_default(),
            ));
        }
        let parsed: RefreshResponse = resp
            .json()
            .await
            .map_err(|e| ApiError::Transport(format!("decoding refresh response: {e}")))?;
        // Keep the existing identity; only the bearer changes.
        if let Some(user) = self.current_user().await {
            self.set_session(parsed.token.clone(), user).await;
        }
        Ok(parsed.token)
    }

    /// Validates the stored credential without performing a login. Always
    /// yields Ok — the truth is in the `valid` flag, matching the backend's
    /// `POST /v1/auth/token-status` contract.
    pub async fn token_status(&self) -> Result<bool, ApiError> {
        let base = self.require_base().await?;
        let token = self.require_token().await?;
        #[derive(Deserialize)]
        struct Status {
            valid: bool,
        }
        let resp = self
            .http
            .post(format!("{base}/v1/auth/token-status"))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| ApiError::Transport(e.to_string()))?;
        if !resp.status().is_success() {
            return Ok(false);
        }
        Ok(resp
            .json::<Status>()
            .await
            .map(|s| s.valid)
            .unwrap_or(false))
    }

    /// Probes the orchestrator's unauthenticated `/health` endpoint.
    ///
    /// Used by the login screen's "Test connection" button so a user can tell
    /// "wrong password" apart from "wrong URL" without a round-trip through
    /// the credential form. No token is sent — this must work before the user
    /// has one.
    ///
    /// Returns `Ok(true)` for any success status. A 404 is treated as a
    /// failure with a specific message because the orchestrator always serves
    /// `/health`, so its absence means the URL points at something else.
    pub async fn probe_health(&self, base_url: &str) -> Result<HealthProbe, ApiError> {
        // Validate the URL the same way `set_base_url` does, so the probe
        // surfaces a bad scheme before burning a network round-trip.
        self.set_base_url(base_url).await?;

        let started = std::time::Instant::now();
        let probe_base = self.base_url.read().await.clone();
        let resp = self
            .http
            .get(format!("{}/health", probe_base))
            .timeout(std::time::Duration::from_secs(8))
            .send()
            .await
            .map_err(|e| transport_error(&probe_base, e))?;

        let latency_ms = started.elapsed().as_millis() as u64;
        let status = resp.status();
        if status.is_success() {
            return Ok(HealthProbe {
                reachable: true,
                latency_ms,
                version: None,
                detail: None,
            });
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(HealthProbe {
                reachable: false,
                latency_ms,
                version: None,
                detail: Some(format!(
                    "{} answered 404 on /health — that host is reachable but is not a FunctionFly orchestrator.",
                    self.base_url.read().await
                )),
            });
        }
        Ok(HealthProbe {
            reachable: false,
            latency_ms,
            version: None,
            detail: Some(format!(
                "{} answered {} on /health.",
                self.base_url.read().await,
                status.as_u16()
            )),
        })
    }

    /// Issues a fresh CSRF token. Cached for the process lifetime; the token
    /// is valid for 1 hour server-side.
    pub async fn csrf_token(&self, force: bool) -> Result<String, ApiError> {
        if !force {
            if let Some(t) = self.csrf.read().await.clone() {
                return Ok(t);
            }
        }
        let base = self.require_base().await?;
        let token = self.require_token().await?;
        let resp = self
            .http
            .get(format!("{base}/v1/csrf"))
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| ApiError::Transport(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(classify(
                resp.status().as_u16(),
                &resp.text().await.unwrap_or_default(),
            ));
        }
        let parsed: CsrfResponse = resp
            .json()
            .await
            .map_err(|e| ApiError::Transport(format!("decoding csrf response: {e}")))?;
        *self.csrf.write().await = Some(parsed.token.clone());
        Ok(parsed.token)
    }

    /// Probes an API key against `/v1/users/me` and returns the identity it
    /// resolves to.
    ///
    /// API keys are not exchangeable for a session token the way an email +
    /// password pair is, so the key itself becomes the bearer. That means a
    /// typo, a revoked key, or a key for the wrong orchestrator would
    /// otherwise sail straight through login and only surface as a 401 on the
    /// user's first real vault read. This call is what turns that into an
    /// immediate, actionable failure at sign-in time.
    ///
    /// `caller` is the supplied key: it is attached as the bearer without
    /// mutating the client's own session, so a failed probe leaves an
    /// already-authenticated client untouched.
    pub async fn validate_api_key(&self, caller: &str) -> Result<LoginUser, ApiError> {
        let base = self.require_base().await?;
        self.validate_api_key_at(&base, caller).await
    }

    /// The body of [`Self::validate_api_key`] with the target base URL passed
    /// explicitly, so a test can point at an arbitrary URL without mutating
    /// the client's configured session.
    pub async fn validate_api_key_at(
        &self,
        base: &str,
        caller: &str,
    ) -> Result<LoginUser, ApiError> {
        let key = caller.trim();
        if key.is_empty() {
            return Err(ApiError::Unauthenticated);
        }

        let resp = self
            .http
            .get(format!("{base}/v1/users/me"))
            .bearer_auth(key)
            .send()
            .await
            .map_err(|e| transport_error(base, e))?;

        // 401/403 mean "the orchestrator answered, and it does not accept this
        // key" — that is an auth problem, not a transport one.
        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(match status {
                401 | 403 => ApiError::Unauthenticated,
                _ => classify(status, &text),
            });
        }

        let me: MeResponse = resp
            .json()
            .await
            .map_err(|e| ApiError::Transport(format!("decoding /v1/users/me response: {e}")))?;

        Ok(LoginUser {
            id: me.id,
            email: me.email,
            username: me.username.or(me.name),
            tenant_id: me.tenant_id,
            role: me.role,
        })
    }

    /// Performs a request against the orchestrator, attaching the bearer and —
    /// for mutating methods — the CSRF header.
    pub async fn request<T: serde::de::DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<serde_json::Value>,
    ) -> Result<T, ApiError> {
        self.request_inner(method, path, query, body, false).await
    }

    async fn request_inner<T: serde::de::DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<serde_json::Value>,
        is_retry: bool,
    ) -> Result<T, ApiError> {
        let base = self.require_base().await?;
        let token = self.require_token().await?;
        let mut url = format!("{base}{path}");
        if !query.is_empty() {
            let qs: Vec<String> = query
                .iter()
                .map(|(k, v)| format!("{k}={}", urlencode(v)))
                .collect();
            url.push('?');
            url.push_str(&qs.join("&"));
        }

        let mut req = self.http.request(method.clone(), &url).bearer_auth(&token);

        // `middleware.RequireCSRF` validates POST/PUT/PATCH/DELETE only, and
        // only via the header.
        if matches!(
            method,
            reqwest::Method::POST
                | reqwest::Method::PUT
                | reqwest::Method::PATCH
                | reqwest::Method::DELETE
        ) {
            let csrf = self.csrf_token(false).await?;
            req = req.header("X-CSRF-Token", csrf);
        }

        if let Some(ref b) = body {
            req = req.json(b);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| ApiError::Transport(e.to_string()))?;
        let status = resp.status();

        if status.as_u16() == 403 && !is_retry {
            let text = resp.text().await.unwrap_or_default();
            // The documented CSRF failure signature; refetch once and retry.
            if text.contains("csrf_token_invalid") {
                self.csrf_token(true).await?;
                return Box::pin(self.request_inner::<T>(method, path, query, body, true)).await;
            }
            return Err(classify(403, &text));
        }

        if !status.is_success() {
            return Err(classify(
                status.as_u16(),
                &resp.text().await.unwrap_or_default(),
            ));
        }

        // 204 No Content has no body to decode.
        if status == reqwest::StatusCode::NO_CONTENT {
            return serde_json::from_value(serde_json::json!({}))
                .map_err(|e| ApiError::Transport(format!("empty response: {e}")));
        }

        resp.json::<T>()
            .await
            .map_err(|e| ApiError::Transport(format!("decoding {path} response: {e}")))
    }

    async fn require_base(&self) -> Result<String, ApiError> {
        let base = self.base_url().await;
        if base.is_empty() {
            return Err(ApiError::NotConfigured(
                "no orchestrator URL configured".into(),
            ));
        }
        Ok(base)
    }

    async fn require_token(&self) -> Result<String, ApiError> {
        self.token
            .read()
            .await
            .clone()
            .ok_or(ApiError::Unauthenticated)
    }

    // ── Device Authorization Grant ───────────────────────────────────────
    //
    // Both endpoints below are unauthenticated on the server: the client is
    // proving possession of the `device_code` it was issued, which is the
    // credential for this exchange. No bearer, no CSRF token.

    /// Starts a device sign-in and returns what the UI needs to prompt.
    ///
    /// The returned `device_code` is held in the Rust struct and marked
    /// `#[serde(skip)]`, so it never crosses into the webview — the frontend
    /// only ever learns the human-typed `user_code`.
    pub async fn begin_device_flow(
        &self,
        client_name: &str,
        client_version: &str,
    ) -> Result<DeviceAuthorization, ApiError> {
        let base = self.require_base().await?;
        #[derive(serde::Serialize)]
        struct Body<'a> {
            client_name: &'a str,
            client_version: &'a str,
        }
        let resp = self
            .http
            .post(format!("{base}/v1/auth/device/authorize"))
            .json(&Body {
                client_name,
                client_version,
            })
            .send()
            .await
            .map_err(|e| transport_error(&base, e))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            // The endpoint is gated by DEVICE_AUTH_ENABLED. A 404 here is the
            // normal signal that this deployment has the flow switched off, so
            // it gets a message the UI can act on rather than a raw code.
            if status == 404 {
                return Err(ApiError::Api {
                    status: 404,
                    code: "device_auth_disabled".into(),
                    message: "This orchestrator does not offer browser sign-in. Use an API key \
                              instead (Settings → API keys)."
                        .into(),
                });
            }
            return Err(classify(status, &body));
        }

        let parsed: DeviceAuthorizeResponse = resp
            .json()
            .await
            .map_err(|e| ApiError::Transport(format!("decoding device authorization: {e}")))?;

        Ok(DeviceAuthorization {
            device_code: parsed.device_code,
            user_code: parsed.user_code,
            verification_uri: parsed.verification_uri,
            verification_uri_complete: parsed.verification_uri_complete,
            expires_in: parsed.expires_in,
            interval: parsed.interval,
        })
    }

    /// Polls once. `Ok(None)` means "keep waiting"; `Err` means stop.
    ///
    /// On success the session is installed on the client so the rest of the
    /// app behaves exactly as it does after a password login.
    pub async fn poll_device_flow(
        &self,
        auth: &DeviceAuthorization,
    ) -> Result<Option<(String, Option<String>)>, DevicePollError> {
        let base = self.require_base().await.map_err(DevicePollError::Fatal)?;
        #[derive(serde::Serialize)]
        struct Body<'a> {
            device_code: &'a str,
        }
        let resp = self
            .http
            .post(format!("{base}/v1/auth/device/token"))
            .json(&Body {
                device_code: &auth.device_code,
            })
            .send()
            .await
            .map_err(|e| DevicePollError::Fatal(transport_error(&base, e)))?;

        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();

        if status == 200 || status == 201 {
            let parsed: DeviceTokenOk = serde_json::from_str(&body).map_err(|e| {
                DevicePollError::Fatal(ApiError::Transport(format!(
                    "decoding device token response: {e}"
                )))
            })?;

            let user = parsed.user.map(|u| LoginUser {
                id: u.id,
                email: u.email,
                username: u.username.or(u.name),
                tenant_id: u.tenant_id,
                role: u.role,
            });
            // Install the session so the caller does not have to.
            if let Some(u) = user {
                self.set_session(parsed.token.clone(), u).await;
            }
            return Ok(Some((parsed.token, parsed.refresh_token)));
        }

        // Every failure carries a machine-readable code. Anything unexpected
        // is treated as "keep polling" rather than failing the sign-in, so a
        // transient proxy error does not strand the user mid-flow.
        let code: String = serde_json::from_str::<OAuthErrorBody>(&body)
            .map(|e| e.error)
            .unwrap_or_default();

        match code.as_str() {
            "authorization_pending" | "slow_down" => Err(DevicePollError::KeepWaiting {
                slow_down: code == "slow_down",
            }),
            "expired_token" => Err(DevicePollError::Terminal(
                "The sign-in request expired. Start again from the app.".into(),
            )),
            "access_denied" => Err(DevicePollError::Terminal(
                "The sign-in request was denied. Start again from the app.".into(),
            )),
            "invalid_grant" => Err(DevicePollError::Terminal(
                "That sign-in request is no longer valid. Start again from the app.".into(),
            )),
            other if !other.is_empty() => Err(DevicePollError::Terminal(format!(
                "Sign-in failed: {other}"
            ))),
            // Unparseable body — a 5xx or a proxy page. Retry rather than fail.
            _ => Err(DevicePollError::KeepWaiting { slow_down: false }),
        }
    }
}

/// Outcome of a single device poll.
///
/// Split out so the Tauri command can drive the retry loop without the UI
/// having to understand the protocol vocabulary.
#[derive(Debug, thiserror::Error)]
pub enum DevicePollError {
    /// Not resolved yet — sleep `interval` and poll again.
    #[error("still pending")]
    KeepWaiting {
        /// RFC 8628 asks the client to back off when the server says so.
        slow_down: bool,
    },
    /// Resolved, but not in our favour. Stop polling and surface the message.
    #[error("{0}")]
    Terminal(String),
    /// Transport or decode failure that ended the flow.
    #[error(transparent)]
    Fatal(#[from] ApiError),
}

fn is_loopback_http(url: &str) -> bool {
    url.starts_with("http://localhost")
        || url.starts_with("http://127.0.0.1")
        || url.starts_with("http://[::1]")
}

/// True when the URL is `https://` pointed at a loopback host.
///
/// A local orchestrator almost never has a certificate that validates for
/// `localhost`, so `https://localhost:8080` fails the TLS handshake and
/// surfaces as a bare "error sending request" — technically correct, useless
/// to the person reading it. `set_base_url` accepts it (it only requires https
/// for *non*-loopback hosts), so this is the signal to explain the real cause.
fn is_https_loopback(url: &str) -> bool {
    url.starts_with("https://localhost")
        || url.starts_with("https://127.0.0.1")
        || url.starts_with("https://[::1]")
}

/// Wraps a transport failure, upgrading the message to name the likely cause
/// when the URL is loopback-over-https. Returns `ApiError::Transport`
/// otherwise, after the same `transport_hint` pass every other call site uses.
fn transport_error(base_url: &str, err: impl std::fmt::Display) -> ApiError {
    let raw = err.to_string();
    let hinted = if is_https_loopback(base_url) {
        let host = base_url
            .trim_start_matches("https://")
            .trim_end_matches('/');
        format!(
            "{} A local orchestrator normally speaks plain HTTP — retry with \
             http://{host} instead of https://",
            transport_hint(&raw)
        )
    } else {
        transport_hint(&raw)
    };
    ApiError::Transport(hinted)
}

/// Result of the login screen's "Test connection" probe.
///
/// `reachable` is the only field the UI keys off; `detail` carries a
/// human-readable explanation when it is `false` so the login screen can
/// tell the user *why* rather than showing a bare failure.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HealthProbe {
    pub reachable: bool,
    pub latency_ms: u64,
    pub version: Option<String>,
    pub detail: Option<String>,
}

/// Turns a reqwest error string into something a non-network engineer can
/// act on. `reqwest`'s default messages ("error sending request for url")
/// are useless in a login form.
fn transport_hint(raw: &str) -> String {
    let lower = raw.to_lowercase();
    if lower.contains("dns error") || lower.contains("failed to lookup") {
        "Could not resolve that hostname. Check the spelling, or use the full \
         URL including https://."
            .into()
    } else if lower.contains("certificate") || lower.contains("tls") {
        "TLS handshake failed — the host may not be serving HTTPS, or its \
         certificate is not trusted on this machine."
            .into()
    } else if lower.contains("timed out") || lower.contains("timeout") {
        "The connection timed out. Check your network, or whether the \
         orchestrator is running."
            .into()
    } else if lower.contains("connection refused") {
        "Connection refused — nothing is listening on that host and port.".into()
    } else {
        raw.to_string()
    }
}

/// Minimal percent-encoding for query values. We only ever emit UUIDs,
/// enums and numbers, but encoding is still correct-by-default.
pub(crate) fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Maps HTTP status + body onto the coarse error taxonomy the UI renders.
fn classify(status: u16, body: &str) -> ApiError {
    // Be lenient about malformed bodies — the status alone is often enough.
    let env: ErrorEnvelope = serde_json::from_str(body).unwrap_or(ErrorEnvelope {
        error: None,
        code: None,
        message: None,
    });

    let code = env.code.clone().unwrap_or_default();
    let message = env
        .message
        .clone()
        .or(env.error.clone())
        .unwrap_or_else(|| format!("request failed with status {status}"));

    match status {
        401 => ApiError::Unauthenticated,
        403 if code.contains("PLAN") || message.contains("plan") => ApiError::PlanDenied,
        403 => ApiError::Api {
            status,
            code: if code.is_empty() {
                "forbidden".into()
            } else {
                code
            },
            message,
        },
        404 => ApiError::NotFound,
        429 => ApiError::RateLimited,
        _ => ApiError::Api {
            status,
            code: if code.is_empty() {
                "error".into()
            } else {
                code
            },
            message,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_401_as_unauthenticated() {
        assert!(matches!(
            classify(401, r#"{"error":"unauthorized"}"#),
            ApiError::Unauthenticated
        ));
    }

    #[test]
    fn classifies_429_as_rate_limited() {
        assert!(matches!(
            classify(
                429,
                r#"{"error":"Too Many Requests","code":"VAULT_RATE_LIMIT"}"#
            ),
            ApiError::RateLimited
        ));
    }

    #[test]
    fn classifies_plan_denial_from_message() {
        assert!(matches!(
            classify(
                403,
                r#"{"error":"Forbidden","message":"Namespaces require Professional plan or higher"}"#
            ),
            ApiError::PlanDenied
        ));
    }

    #[test]
    fn tolerates_non_json_error_body() {
        match classify(500, "<html>gateway exploded</html>") {
            ApiError::Api { status, .. } => assert_eq!(status, 500),
            other => panic!("expected Api variant, got {other:?}"),
        }
    }

    #[test]
    fn urlencodes_query_values() {
        assert_eq!(urlencode("a b&c=d"), "a%20b%26c%3Dd");
        assert_eq!(urlencode("plain-uuid_1.0~"), "plain-uuid_1.0~");
    }

    #[tokio::test]
    async fn rejects_plaintext_remote_base_url() {
        let c = VaultClient::new();
        assert!(matches!(
            c.set_base_url("http://evil.example.com").await,
            Err(ApiError::NotConfigured(_))
        ));
        assert!(c.set_base_url("http://localhost:8080").await.is_ok());
        assert!(c.set_base_url("https://api.functionfly.com/").await.is_ok());
        // Trailing slash is normalised away so paths don't double up.
        assert_eq!(c.base_url().await, "https://api.functionfly.com");
    }

    #[tokio::test]
    async fn requests_fail_closed_without_a_token() {
        let c = VaultClient::new();
        c.set_base_url("https://api.functionfly.com").await.unwrap();
        let res: Result<serde_json::Value, _> = c
            .request(reqwest::Method::GET, "/v1/vault/secrets", &[], None)
            .await;
        assert!(matches!(res, Err(ApiError::Unauthenticated)));
    }
}
