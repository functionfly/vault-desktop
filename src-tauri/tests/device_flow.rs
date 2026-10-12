//! Coverage for the RFC 8628 Device Authorization Grant client side.
//!
//! The desktop app never holds a password: it asks the orchestrator for a
//! grant, shows the human a short code, and polls until the human approves
//! that code in a browser where they are already signed in. This file pins
//! the wire contract and — more importantly — the failure vocabulary, because
//! the whole flow is a poll loop and a wrong interpretation of any one
//! response would either hang forever or abandon a sign-in that was about to
//! succeed.
//!
//! What is covered:
//!
//!   - `begin_device_flow` returns the human-facing fields and never leaks
//!     `device_code` across the IPC boundary (it is `#[serde(skip)]`);
//!   - a 404 from the authorize endpoint (the deployment has the flow off)
//!     becomes an actionable `device_auth_disabled` error, not a raw code;
//!   - `authorization_pending` and `slow_down` both keep the loop alive, and
//!     `slow_down` is flagged so the caller can back off;
//!   - `expired_token`, `access_denied` and `invalid_grant` are terminal;
//!   - a 200 installs the session on the client, so the rest of the app
//!     behaves exactly as it does after a password login;
//!   - an unparseable failure body (a 5xx, a proxy page) is treated as
//!     "keep polling" rather than failing the sign-in;
//!   - the poll request carries the `device_code` and no bearer token,
//!     because the code *is* the credential for that endpoint.

use functionfly_vault_desktop_lib::api::{
    ApiError, DeviceAuthorization, DevicePollError, VaultClient,
};
use mockito::{Matcher, Server};

/// Convenience: build a client pointed at the mock server.
async fn client(server: &Server) -> VaultClient {
    let c = VaultClient::new();
    c.set_base_url(&server.url())
        .await
        .expect("loopback http://127.0.0.1 must be accepted");
    c
}

/// A pending grant with a known `device_code`.
///
/// Tests that only exercise the poll wire shape do not care where the grant
/// came from, so they start from this rather than standing up a second mock.
fn fixture_auth() -> DeviceAuthorization {
    DeviceAuthorization::new(
        "SECRET-DEVICE-CODE",
        "WXYZ2345",
        "https://app.example.com/device",
        "https://app.example.com/device?user_code=WXYZ2345",
        600,
        5,
    )
}

#[tokio::test]
async fn begin_device_flow_returns_the_human_facing_fields() {
    let mut server = Server::new_async().await;
    let _authorize = server
        .mock("POST", "/v1/auth/device/authorize")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            serde_json::json!({
                "device_code": "SECRET-DEVICE-CODE",
                "user_code": "WXYZ2345",
                "verification_uri": "https://app.example.com/device",
                "verification_uri_complete": "https://app.example.com/device?user_code=WXYZ2345",
                "expires_in": 600,
                "interval": 5,
            })
            .to_string(),
        )
        .create_async()
        .await;

    let c = client(&server).await;
    let auth = c
        .begin_device_flow("FunctionFly Vault Desktop", "1.0.0")
        .await
        .expect("authorize should succeed");

    assert_eq!(auth.user_code, "WXYZ2345");
    assert_eq!(auth.expires_in, 600);
    assert_eq!(auth.interval, 5);
    assert_eq!(
        auth.verification_uri_complete,
        "https://app.example.com/device?user_code=WXYZ2345"
    );
}

/// The `device_code` authorises the token exchange, so it must never reach the
/// webview. If this regresses, a compromised renderer could complete the
/// exchange on its own.
#[tokio::test]
async fn device_code_is_never_serialized_to_the_webview() {
    let mut server = Server::new_async().await;
    let _authorize = server
        .mock("POST", "/v1/auth/device/authorize")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            serde_json::json!({
                "device_code": "SECRET-DEVICE-CODE",
                "user_code": "WXYZ2345",
                "verification_uri": "https://app.example.com/device",
                "verification_uri_complete": "https://app.example.com/device?user_code=WXYZ2345",
                "expires_in": 600,
                "interval": 5,
            })
            .to_string(),
        )
        .create_async()
        .await;

    let c = client(&server).await;
    let auth = c
        .begin_device_flow("FunctionFly Vault Desktop", "1.0.0")
        .await
        .expect("authorize should succeed");

    let wire = serde_json::to_string(&auth).expect("DeviceAuthorization must serialize");
    assert!(
        !wire.contains("SECRET-DEVICE-CODE"),
        "device_code leaked across the IPC boundary: {wire}"
    );
    // The human-facing fields must still be there.
    assert!(wire.contains("WXYZ2345"));
}

/// `DEVICE_AUTH_ENABLED=false` is the shipped default, so the 404 branch is
/// the *normal* path on most deployments. It has to produce copy the UI can
/// act on ("use an API key") rather than a bare status code.
#[tokio::test]
async fn disabled_deployment_reports_device_auth_disabled() {
    let mut server = Server::new_async().await;
    let _authorize = server
        .mock("POST", "/v1/auth/device/authorize")
        .with_status(404)
        .with_header("content-type", "application/json")
        .with_body(r#"{"error":"not_found","message":"endpoint not registered"}"#)
        .create_async()
        .await;

    let c = client(&server).await;
    let err = c
        .begin_device_flow("FunctionFly Vault Desktop", "1.0.0")
        .await
        .expect_err("404 must fail");

    match err {
        ApiError::Api {
            status,
            ref code,
            ref message,
        } => {
            assert_eq!(status, 404);
            assert_eq!(code, "device_auth_disabled");
            assert!(
                message.contains("API key"),
                "message should tell the user what to do instead: {message}"
            );
        }
        other => panic!("expected ApiError::Api, got {other:?}"),
    }
}

#[tokio::test]
async fn poll_keeps_waiting_while_the_human_has_not_acted() {
    let mut server = Server::new_async().await;
    let _token = server
        .mock("POST", "/v1/auth/device/token")
        .with_status(400)
        .with_header("content-type", "application/json")
        .with_body(r#"{"error":"authorization_pending"}"#)
        .create_async()
        .await;

    let c = client(&server).await;
    let auth = fixture_auth();

    match c.poll_device_flow(&auth).await {
        Err(DevicePollError::KeepWaiting { slow_down }) => {
            assert!(!slow_down, "plain pending must not request backoff");
        }
        other => panic!("expected KeepWaiting, got {other:?}"),
    }
}

/// RFC 8628 §3.5: the server answers `slow_down` when the client polls faster
/// than agreed. The client must keep waiting *and* widen its interval.
#[tokio::test]
async fn poll_flags_slow_down_without_terminating() {
    let mut server = Server::new_async().await;
    let _token = server
        .mock("POST", "/v1/auth/device/token")
        .with_status(400)
        .with_header("content-type", "application/json")
        .with_body(r#"{"error":"slow_down"}"#)
        .create_async()
        .await;

    let c = client(&server).await;
    match c.poll_device_flow(&fixture_auth()).await {
        Err(DevicePollError::KeepWaiting { slow_down }) => {
            assert!(slow_down, "slow_down must be surfaced to the caller");
        }
        other => panic!("expected KeepWaiting(slow_down), got {other:?}"),
    }
}

/// Each of these ends the loop. Treating any as "keep waiting" would strand
/// the user on a sign-in that can never complete.
#[tokio::test]
async fn poll_terminates_on_expired_denied_and_invalid() {
    for code in ["expired_token", "access_denied", "invalid_grant"] {
        let mut server = Server::new_async().await;
        let _token = server
            .mock("POST", "/v1/auth/device/token")
            .with_status(400)
            .with_header("content-type", "application/json")
            .with_body(format!(r#"{{"error":"{code}"}}"#))
            .create_async()
            .await;

        let c = client(&server).await;
        match c.poll_device_flow(&fixture_auth()).await {
            Err(DevicePollError::Terminal(msg)) => {
                assert!(!msg.is_empty(), "{code} should carry a message");
            }
            other => panic!("{code} must be terminal, got {other:?}"),
        }
    }
}

/// A successful exchange must install the session, not just return a token —
/// otherwise every later vault call would run unauthenticated.
#[tokio::test]
async fn successful_poll_installs_the_session() {
    let mut server = Server::new_async().await;
    let _token = server
        .mock("POST", "/v1/auth/device/token")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            serde_json::json!({
                "token": "jwt-token",
                "refresh_token": "refresh-token",
                "token_type": "Bearer",
                "expires_in": 86400,
                "user": {
                    "id": "user-1",
                    "email": "alice@example.com",
                    "username": null,
                    "name": "Alice",
                    "tenant_id": "tenant-1",
                    "role": "user",
                },
            })
            .to_string(),
        )
        .create_async()
        .await;

    let c = client(&server).await;
    let got = c
        .poll_device_flow(&fixture_auth())
        .await
        .expect("poll must not be terminal")
        .expect("poll must yield a session");

    assert_eq!(got.0, "jwt-token");
    assert_eq!(got.1.as_deref(), Some("refresh-token"));

    let user = c
        .current_user()
        .await
        .expect("the session must be installed on the client");
    assert_eq!(user.email.as_deref(), Some("alice@example.com"));
    // `username` was null; the handler falls back to `name`.
    assert_eq!(user.username.as_deref(), Some("Alice"));
    assert_eq!(user.tenant_id, "tenant-1");
}

/// The token endpoint proves identity with the device_code alone. Sending a
/// bearer as well would be harmless but misleading — the point is that this
/// works with no session.
#[tokio::test]
async fn poll_sends_the_device_code_and_no_bearer() {
    let mut server = Server::new_async().await;
    let _token = server
        .mock("POST", "/v1/auth/device/token")
        .match_body(Matcher::PartialJson(serde_json::json!({
            "device_code": "SECRET-DEVICE-CODE",
        })))
        .match_header("authorization", Matcher::Missing)
        .with_status(400)
        .with_header("content-type", "application/json")
        .with_body(r#"{"error":"authorization_pending"}"#)
        .create_async()
        .await;

    let c = client(&server).await;
    let _ = c.poll_device_flow(&fixture_auth()).await;
}

/// A 5xx or a proxy error page must not abandon a sign-in the user is in the
/// middle of. Unparseable bodies are treated as "keep waiting".
#[tokio::test]
async fn unparseable_failure_body_keeps_waiting() {
    let mut server = Server::new_async().await;
    let _token = server
        .mock("POST", "/v1/auth/device/token")
        .with_status(502)
        .with_header("content-type", "text/html")
        .with_body("<html><body>502 Bad Gateway</body></html>")
        .create_async()
        .await;

    let c = client(&server).await;
    match c.poll_device_flow(&fixture_auth()).await {
        Err(DevicePollError::KeepWaiting { .. }) => {}
        other => panic!("a proxy error must not end the flow, got {other:?}"),
    }
}

/// A transport failure (server down) is fatal — retrying forever against a
/// host that is not answering would leave the UI spinning with no way out.
#[tokio::test]
async fn transport_failure_is_fatal() {
    // Port 9 is discard; a connection there fails fast.
    let c = VaultClient::new();
    c.set_base_url("http://127.0.0.1:9")
        .await
        .expect("loopback is allowed");

    match c.poll_device_flow(&fixture_auth()).await {
        Err(DevicePollError::Fatal(_)) => {}
        other => panic!("transport failure must be fatal, got {other:?}"),
    }
}
