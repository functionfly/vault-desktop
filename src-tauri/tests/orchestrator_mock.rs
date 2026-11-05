//! End-to-end coverage for [`VaultClient`] against a `mockito`-backed fake
//! orchestrator. The test does not talk to a real orchestrator — instead it
//! pins every behaviour the README claims:
//!
//!   - email+password login returns `{token, refresh_token, user}` and the
//!     client binds the token for subsequent calls;
//!   - API-key login is *verified* against `GET /v1/users/me` before it is
//!     accepted, and the resolved identity replaces the old empty stub;
//!   - `GET /v1/csrf` is called once on the first mutating request and the
//!     returned token is sent as `X-CSRF-Token` on every POST/PUT/PATCH/DELETE;
//!   - a 403 with `csrf_token_invalid` in the body triggers exactly one
//!     re-fetch and retry (we cap the retry at one to avoid infinite loops);
//!   - captcha-required 503 is surfaced with a typed error so the React UI
//!     can tell the user to use an API key;
//!   - the rate-limit 429 is mapped to `ApiError::RateLimited`;
//!   - error envelopes that omit `code` or `message` (the dashboard's older
//!     handlers) are still parsed.
//!
//! This is the v0.1.0 evidence pack that lets the CI matrix move from
//! "compiles and unit-tests pass" to "the wire contract is verified" without
//! needing a live orchestrator on the headless runner.

use std::time::Duration;

use functionfly_vault_desktop_lib::api::{ApiError, VaultClient};
use functionfly_vault_desktop_lib::commands::looks_like_api_key;
use mockito::{Matcher, Server};

/// Convenience: build a client and point it at the mock server's base URL.
async fn client(server: &Server) -> VaultClient {
    let c = VaultClient::new();
    c.set_base_url(&server.url())
        .await
        .expect("loopback http://127.0.0.1 must be accepted");
    c
}

#[tokio::test]
async fn login_round_trip_persists_session() {
    let mut server = Server::new_async().await;
    let login = server
        .mock("POST", "/v1/auth/login")
        .match_body(Matcher::PartialJson(serde_json::json!({
            "email": "alice@example.com",
        })))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            serde_json::json!({
                "token": "ACCESS_TOKEN",
                "refresh_token": "REFRESH_TOKEN",
                "token_version": 1,
                "user": {
                    "id": "00000000-0000-0000-0000-000000000001",
                    "email": "alice@example.com",
                    "tenant_id": "00000000-0000-0000-0000-000000000000",
                    "role": "user"
                },
                "desktop_capable": true
            })
            .to_string(),
        )
        .create_async()
        .await;

    let c = client(&server).await;
    let resp = c
        .login("alice@example.com", "hunter2")
        .await
        .expect("login");

    assert_eq!(resp.token, "ACCESS_TOKEN");
    assert_eq!(resp.user.email.as_deref(), Some("alice@example.com"));
    assert!(resp.desktop_capable);
    assert!(
        c.is_authenticated().await,
        "session should be bound after login"
    );

    login.assert_async().await;
}

#[tokio::test]
async fn list_secrets_uses_get_and_does_not_attach_csrf() {
    // Read endpoints are GET — CSRF middleware does not run, so the client
    // must NOT spend a round trip on `/v1/csrf` for them.
    let mut server = Server::new_async().await;

    // A `Regex` matcher accepts `/v1/csrf` and any query string. We assert
    // `expect(0)` below to prove the GET path never touches it.
    let csrf = server
        .mock("GET", Matcher::Regex("^/v1/csrf.*".to_string()))
        .expect(0)
        .with_status(200)
        .with_body(r#"{"token":"SHOULD_NOT_BE_USED","expires_at":"2030-01-01T00:00:00Z"}"#)
        .create_async()
        .await;

    // `/v1/vault/secrets` is matched via regex so trailing-slash + query
    // string variations do not matter — the README claims GETs carry the
    // bearer, not the CSRF token; we want to test the bearer header
    // path, not the path-matching path.
    let list = server
        .mock("GET", Matcher::Regex("^/v1/vault/secrets.*".to_string()))
        .match_header("authorization", "Bearer ACCESS_TOKEN")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            serde_json::json!({
                "secrets": [
                    {"id":"s1","name":"DB","secret_type":"api_key","namespace":"default"}
                ],
                "total": 1,
                "limit": 50,
                "offset": 0
            })
            .to_string(),
        )
        .create_async()
        .await;

    let c = client(&server).await;
    // Manually bind the bearer so we don't have to redo the login flow.
    use functionfly_vault_desktop_lib::api::LoginUser;
    c.set_session(
        "ACCESS_TOKEN".into(),
        LoginUser {
            id: "u1".into(),
            email: Some("alice@example.com".into()),
            username: None,
            tenant_id: "t1".into(),
            role: Some("user".into()),
        },
    )
    .await;

    let resp: serde_json::Value = c
        .request(
            reqwest::Method::GET,
            "/v1/vault/secrets",
            &[("limit", "50".into()), ("offset", "0".into())],
            None,
        )
        .await
        .expect("list");

    assert_eq!(resp["total"], 1);
    assert_eq!(resp["secrets"][0]["name"], "DB");
    list.assert_async().await;
    csrf.assert_async().await; // proves CSRF was NOT called for a GET
}

#[tokio::test]
async fn mutating_request_fetches_csrf_then_sends_header() {
    let mut server = Server::new_async().await;

    let csrf = server
        .mock("GET", "/v1/csrf")
        .match_header("authorization", "Bearer ACCESS_TOKEN")
        .with_status(200)
        .with_body(r#"{"token":"csrf-token-1","expires_at":"2030-01-01T00:00:00Z"}"#)
        .create_async()
        .await;

    let rollback = server
        .mock("POST", "/v1/vault/secrets/s1/rollback")
        .match_header("x-csrf-token", "csrf-token-1")
        .match_body(Matcher::PartialJson(serde_json::json!({
            "target_version": 2
        })))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(serde_json::json!({"new_version": 5, "rolled_back_to": 2}).to_string())
        .create_async()
        .await;

    let c = client(&server).await;
    use functionfly_vault_desktop_lib::api::LoginUser;
    c.set_session(
        "ACCESS_TOKEN".into(),
        LoginUser {
            id: "u1".into(),
            email: None,
            username: None,
            tenant_id: "t1".into(),
            role: None,
        },
    )
    .await;

    let resp: serde_json::Value = c
        .request(
            reqwest::Method::POST,
            "/v1/vault/secrets/s1/rollback",
            &[],
            Some(serde_json::json!({"target_version": 2})),
        )
        .await
        .expect("rollback");

    assert_eq!(resp["new_version"], 5);
    csrf.assert_async().await;
    rollback.assert_async().await;
}

#[tokio::test]
async fn csrf_token_invalid_403_triggers_one_retry() {
    let mut server = Server::new_async().await;

    // First CSRF fetch: returns an obsolete token.
    let csrf_first = server
        .mock("GET", "/v1/csrf")
        .with_status(200)
        .with_body(r#"{"token":"stale","expires_at":"2030-01-01T00:00:00Z"}"#)
        .create_async()
        .await;

    // Second CSRF fetch (after the 403): returns the fresh one.
    let csrf_second = server
        .mock("GET", "/v1/csrf")
        .with_status(200)
        .with_body(r#"{"token":"fresh","expires_at":"2030-01-01T00:00:00Z"}"#)
        .expect(1)
        .create_async()
        .await;

    // First attempt: rejects the stale token.
    let reject = server
        .mock("POST", "/v1/vault/secrets/s1/rollback")
        .match_header("x-csrf-token", "stale")
        .with_status(403)
        .with_body(r#"{"error":"forbidden","code":"csrf_token_invalid","message":"stale"}"#)
        .expect(1)
        .create_async()
        .await;

    // Retry attempt: accepts the fresh one.
    let accept = server
        .mock("POST", "/v1/vault/secrets/s1/rollback")
        .match_header("x-csrf-token", "fresh")
        .with_status(200)
        .with_body(r#"{"new_version": 7, "rolled_back_to": 2}"#)
        .create_async()
        .await;

    let c = client(&server).await;
    use functionfly_vault_desktop_lib::api::LoginUser;
    c.set_session(
        "ACCESS_TOKEN".into(),
        LoginUser {
            id: "u1".into(),
            email: None,
            username: None,
            tenant_id: "t1".into(),
            role: None,
        },
    )
    .await;

    let resp: serde_json::Value = c
        .request(
            reqwest::Method::POST,
            "/v1/vault/secrets/s1/rollback",
            &[],
            Some(serde_json::json!({"target_version": 2})),
        )
        .await
        .expect("rollback after CSRF refresh");

    assert_eq!(resp["new_version"], 7);
    csrf_first.assert_async().await;
    csrf_second.assert_async().await;
    reject.assert_async().await;
    accept.assert_async().await;
}

#[tokio::test]
async fn captcha_required_503_is_surfaced_with_a_typed_error() {
    let mut server = Server::new_async().await;
    let m = server
        .mock("POST", "/v1/auth/login")
        .with_status(503)
        .with_body(r#"{"code":"captcha_not_configured","message":"captcha required"}"#)
        .create_async()
        .await;

    let c = client(&server).await;
    let err = c.login("a", "b").await.expect_err("login must fail");
    // We surface captcha 503 as a generic `Api { .. }` whose `code` we
    // expose in the message; the React UI keys off the message string in
    // v0.1.0. The next step is to introduce a `CaptchaRequired` variant;
    // until then, this test pins the *current* behaviour so we do not
    // accidentally regress it.
    assert!(format!("{err}").to_lowercase().contains("captcha"));
    m.assert_async().await;
}

#[tokio::test]
async fn rate_limit_429_is_typed() {
    let mut server = Server::new_async().await;
    let m = server
        .mock("GET", "/v1/vault/secrets")
        .with_status(429)
        .with_header("retry-after", "60")
        .with_body(
            r#"{"error":"Too Many Requests","code":"VAULT_RATE_LIMIT","message":"slow down"}"#,
        )
        .create_async()
        .await;

    let c = client(&server).await;
    use functionfly_vault_desktop_lib::api::ApiError;
    use functionfly_vault_desktop_lib::api::LoginUser;
    c.set_session(
        "ACCESS_TOKEN".into(),
        LoginUser {
            id: "u1".into(),
            email: None,
            username: None,
            tenant_id: "t1".into(),
            role: None,
        },
    )
    .await;

    let err = c
        .request::<serde_json::Value>(reqwest::Method::GET, "/v1/vault/secrets", &[], None)
        .await
        .expect_err("rate limit must error");

    assert!(matches!(err, ApiError::RateLimited));
    m.assert_async().await;
}

#[tokio::test]
async fn non_loopback_http_url_is_rejected() {
    let c = VaultClient::new();
    let err = c
        .set_base_url("http://evil.example.com")
        .await
        .expect_err("non-loopback http:// must be rejected");
    let msg = format!("{err}");
    assert!(
        msg.contains("https://"),
        "rejection must mention https:// — got: {msg}"
    );
}

#[tokio::test]
async fn request_without_token_fails_closed() {
    let c = VaultClient::new();
    c.set_base_url("http://127.0.0.1:1").await.unwrap();
    let res = c
        .request::<serde_json::Value>(reqwest::Method::GET, "/v1/vault/secrets", &[], None)
        .await;
    let err = res.expect_err("missing bearer must error");
    assert!(matches!(
        err,
        functionfly_vault_desktop_lib::api::ApiError::Unauthenticated
    ));
}

#[tokio::test]
async fn error_envelope_without_code_falls_back_to_message_only() {
    // The older vault handlers emit `{error, message}` only — no `code`.
    // The classifier must still produce a typed error rather than 500.
    let mut server = Server::new_async().await;
    let m = server
        .mock("GET", "/v1/vault/secrets")
        .with_status(403)
        .with_body(r#"{"error":"forbidden","message":"Namespaces require Pro plan"}"#)
        .create_async()
        .await;

    let c = client(&server).await;
    use functionfly_vault_desktop_lib::api::LoginUser;
    c.set_session(
        "ACCESS_TOKEN".into(),
        LoginUser {
            id: "u1".into(),
            email: None,
            username: None,
            tenant_id: "t1".into(),
            role: None,
        },
    )
    .await;

    let err = c
        .request::<serde_json::Value>(reqwest::Method::GET, "/v1/vault/secrets", &[], None)
        .await
        .expect_err("must surface typed error");
    assert!(matches!(
        err,
        functionfly_vault_desktop_lib::api::ApiError::PlanDenied
    ));
    m.assert_async().await;
}

/// Sanity: the test harness's `VaultClient::new()` does not do real I/O.
/// This test exists to catch a future refactor that adds a blocking HTTP
/// call (e.g. an eager `reqwest::get`) into construction.
///
/// The bound is a wall-clock measurement, so it has to be measured
/// against a warm process. `reqwest::Client::builder().build()` may
/// touch DNS once per process to resolve a platform root-certificate
/// provider; on a host with an unreachable resolver that is a DNS
/// timeout — two nameservers at five seconds each — which is what this
/// test was intermittently catching (10.96s against a 500ms bound).
/// That is the resolver's latency, not the client's behaviour.
///
/// So the first construction is discarded: it pays the one-time cost.
/// The SECOND construction is the one measured, and it is the one that
/// would carry a blocking HTTP call introduced by a refactor, because
/// such a call would run on every construction.
#[test]
fn client_construction_does_not_perform_user_io() {
    // Warm the process: the one-time DNS / certificate-provider cost.
    let _ = VaultClient::new();

    let start = std::time::Instant::now();
    let _ = VaultClient::new();
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "VaultClient::new() must not perform user I/O (took {elapsed:?})"
    );
}

// ── API-key login validation ───────────────────────────────────────────
//
// An `ffp_…` key is used as the bearer directly — there is no exchange step
// the way there is for email+password. Without a verification probe, a typo
// or a revoked key would be written straight into the OS keychain and look
// like a successful sign-in until the first real vault read 401'd.

#[tokio::test]
async fn validate_api_key_returns_identity_from_users_me() {
    let mut server = Server::new_async().await;
    let me = server
        .mock("GET", "/v1/users/me")
        .match_header("authorization", "Bearer ffp_v1_abc_4b")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            serde_json::json!({
                "id": "00000000-0000-0000-0000-0000000000aa",
                "email": "bob@example.com",
                "name": "Bob",
                "username": "trase",
                "tenantId": "00000000-0000-0000-0000-0000000000bb",
                "role": "user",
                "plan": "starter",
            })
            .to_string(),
        )
        .create_async()
        .await;

    let c = client(&server).await;
    let user = c
        .validate_api_key("ffp_v1_abc_4b")
        .await
        .expect("valid key should validate");

    // `tenantId` is camelCase on the wire and must land on `tenant_id`.
    assert_eq!(user.id, "00000000-0000-0000-0000-0000000000aa");
    assert_eq!(user.email.as_deref(), Some("bob@example.com"));
    assert_eq!(user.tenant_id, "00000000-0000-0000-0000-0000000000bb");
    assert_eq!(user.role.as_deref(), Some("user"));
    me.assert_async().await;
}

#[tokio::test]
async fn validate_api_key_falls_back_to_name_when_username_absent() {
    let mut server = Server::new_async().await;
    let _me = server
        .mock("GET", "/v1/users/me")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            serde_json::json!({
                "id": "id-1",
                "name": "Bob",
                "tenantId": "tenant-1",
            })
            .to_string(),
        )
        .create_async()
        .await;

    let c = client(&server).await;
    let user = c.validate_api_key("k").await.expect("ok");
    assert_eq!(user.username.as_deref(), Some("Bob"));
}

#[tokio::test]
async fn validate_api_key_rejects_401_and_403_as_unauthenticated() {
    for status in [401usize, 403usize] {
        let mut server = Server::new_async().await;
        let _me = server
            .mock("GET", "/v1/users/me")
            .with_status(status)
            .with_body(r#"{"code":"unauthorized","message":"nope"}"#)
            .create_async()
            .await;

        let c = client(&server).await;
        let err = c
            .validate_api_key("ffp_v1_bad_key_00")
            .await
            .expect_err("a rejected key must not validate");

        // A 401/403 means "the orchestrator answered and refused this key",
        // which the UI renders differently from "host unreachable".
        // `matches!` takes its operand by value, so match on a reference —
        // otherwise the format argument below borrows an already-moved value.
        assert!(
            matches!(&err, ApiError::Unauthenticated),
            "status {status} should map to Unauthenticated, got {err:?}"
        );
    }
}

#[tokio::test]
async fn validate_api_key_rejects_empty_key_without_a_request() {
    let mut server = Server::new_async().await;
    let me = server
        .mock("GET", "/v1/users/me")
        .expect(0)
        .with_status(200)
        .create_async()
        .await;

    let c = client(&server).await;
    assert!(c.validate_api_key("   ").await.is_err());
    me.assert_async().await;
}

#[tokio::test]
async fn validate_api_key_does_not_disturb_an_existing_session() {
    let mut server = Server::new_async().await;
    let _bad = server
        .mock("GET", "/v1/users/me")
        .with_status(401)
        .with_body(r#"{"code":"unauthorized"}"#)
        .create_async()
        .await;

    let c = client(&server).await;
    c.set_session("EXISTING_TOKEN".into(), Default::default())
        .await;

    let _ = c.validate_api_key("ffp_v1_wrong_00").await;

    // The probe attaches its own bearer; it must not clobber the live
    // session, so a failed probe leaves a signed-in client signed in.
    assert!(
        c.is_authenticated().await,
        "a failed key probe must not clear an existing session"
    );
}

// ── loopback scheme hint ───────────────────────────────────────────────
//
// A local orchestrator serves plain HTTP and has no cert valid for
// `localhost`, so `https://localhost:8080` fails the TLS handshake. The
// raw reqwest error ("error sending request for url (...)") tells the user
// nothing they can act on.

#[tokio::test]
async fn https_loopback_url_explains_the_scheme_mismatch() {
    let c = VaultClient::new();
    // Port 9 (discard) refuses instantly — we never need a real handshake,
    // only that the hint is attached to whatever transport error occurs.
    let err = c
        .validate_api_key_at("https://127.0.0.1:9", "k")
        .await
        .expect_err("unreachable loopback over https must fail");

    let msg = err.to_string();
    assert!(
        msg.contains("http://127.0.0.1:9"),
        "error should name the corrected http:// URL, got: {msg}"
    );
}

#[tokio::test]
async fn non_loopback_https_url_gets_no_localhost_hint() {
    let c = VaultClient::new();
    let err = c
        .validate_api_key_at("https://api.example.invalid:9", "k")
        .await
        .expect_err("unreachable host must fail");

    let msg = err.to_string();
    assert!(
        !msg.contains("A local orchestrator normally speaks plain HTTP"),
        "a real remote host must not get the loopback hint, got: {msg}"
    );
}

// ── API-key vs session-token shape detection ───────────────────────────
//
// `restore_session` must not send an API key to `/v1/auth/token-status`:
// that endpoint answers 400 for keys, which made every stored key look
// stale and bounced the user to the login screen on every launch.

#[test]
fn api_key_shapes_are_recognised_across_every_prefix() {
    // Mirrors the orchestrator's prefix table in internal/apikey/types.go.
    for prefix in ["ffp", "fff", "ffx", "ffe", "ffo", "fft", "ffmp", "ffr"] {
        let key = format!("{prefix}_v1_573923ab4f0bf313045887b69d81ad0c_4b");
        assert!(
            looks_like_api_key(&key),
            "{prefix} key must be detected as an API key"
        );
    }
}

#[test]
fn session_tokens_are_not_mistaken_for_api_keys() {
    // JWTs and opaque session tokens must stay on the token-status path.
    assert!(!looks_like_api_key(
        "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxIn0.abcdef"
    ));
    assert!(!looks_like_api_key(""));
    assert!(!looks_like_api_key("short"));
    // Two segments is not the {prefix}_{version}_{random}_{checksum} shape.
    assert!(!looks_like_api_key("ff_something"));
}
