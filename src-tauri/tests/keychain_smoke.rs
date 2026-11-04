//! Round-trip test for the REAL OS keychain.
//!
//! WHY THIS EXISTS
//! ---------------
//! The launch checklist in `README.md` says, per platform:
//!
//!   - Linux:   `secret-tool search service functionfly-vault-desktop`
//!   - macOS:   `security find-generic-password -s functionfly-vault-desktop`
//!   - Windows: `cmdkey /list:functionfly-vault-desktop`
//!
//! and then "log in, confirm the keychain now contains those entries".
//! That was a manual step, which means it never ran in CI and the very
//! first person to ship the app would discover a broken keychain path
//! in front of users.
//!
//! This test automates the same check against the actual operating-system
//! credential store, using the application's own `SecretStore` code rather
//! than a parallel implementation.
//!
//! THE CRITICAL ASSERTION
//! ----------------------
//! `SecretStore::save_session` silently falls back to the encrypted file
//! store when the OS keychain is unreachable. That fallback is correct
//! behaviour for a headless Linux box, and it is also exactly what would
//! hide a broken keychain on a platform that *should* have one — the user
//! would get a weaker store and never be told.
//!
//! So the test asserts that the backend actually used was
//! `BackendKind::OsKeychain`. On Windows and macOS an OS keychain always
//! exists, so a file fallback there is a hard failure. On Linux the
//! keychain is optional (it needs a running Secret Service), so the test
//! skips with an explicit message instead of failing.
//!
//! PROBE MODE
//! ----------
//! Set `VAULT_KEYCHAIN_PROBE_KEEP=1` to leave the written credential in
//! place so a shell wrapper can confirm it through the platform's own
//! tooling (`cmdkey`, `security`, `secret-tool`) and then delete it.
//! The default is to clean up after itself.

use functionfly_vault_desktop_lib::store::{BackendKind, SecretStore};

/// Unique-ish account material so a probe never collides with a real
/// session that happens to be stored on the same machine.
const PROBE_BASE_URL: &str = "https://keychain-smoke.invalid";
const PROBE_TOKEN: &str = "keychain-smoke-access-token";
const PROBE_REFRESH: &str = "keychain-smoke-refresh-token";

/// Whether this platform is expected to have a working OS keychain.
///
/// Windows (Credential Manager) and macOS (Keychain) always do. Linux
/// depends on a running Secret Service, which a headless box (CI, WSL,
/// a container) does not have.
fn os_keychain_expected() -> bool {
    cfg!(any(target_os = "windows", target_os = "macos"))
}

fn keep_probe() -> bool {
    matches!(
        std::env::var("VAULT_KEYCHAIN_PROBE_KEEP").as_deref(),
        Ok("1") | Ok("true")
    )
}

#[tokio::test]
async fn session_round_trips_through_the_os_keychain() {
    let store = SecretStore::new();

    // Start from a clean slate. A previous crashed run could otherwise
    // leave a half-written credential that makes the assertions below
    // pass for the wrong reason.
    let _ = store.clear_session().await;

    store
        .save_session(PROBE_BASE_URL, PROBE_TOKEN, Some(PROBE_REFRESH))
        .await
        .expect("save_session should succeed");

    let backend = store.last_backend().await;

    if backend != BackendKind::OsKeychain {
        if !os_keychain_expected() {
            // Headless Linux: no Secret Service, so the encrypted file
            // fallback is the documented, intended behaviour. Say so
            // loudly rather than silently passing.
            eprintln!(
                "SKIP: no OS keychain on this host; the session went to the \
                 encrypted file fallback (backend = file_fallback). This is \
                 expected on headless Linux / WSL / containers."
            );
            let _ = store.clear_session().await;
            return;
        }

        panic!(
            "the OS keychain was NOT used on this platform \
             (backend = file_fallback). On {} a working credential store \
             always exists, so this is a real failure, not an environment \
             limitation.",
            std::env::consts::OS
        );
    }

    // The session must read back byte-for-byte.
    let loaded = store
        .load_session()
        .await
        .expect("load_session should succeed")
        .expect("a saved session must be readable");
    assert_eq!(loaded.base_url, PROBE_BASE_URL);
    assert_eq!(loaded.access_token, PROBE_TOKEN);
    assert_eq!(loaded.refresh_token.as_deref(), Some(PROBE_REFRESH));
    assert_eq!(
        loaded.backend,
        BackendKind::OsKeychain,
        "a session read back from the keychain must report the keychain backend"
    );

    // `describe()` is what the UI renders as its storage badge. It must
    // agree with the backend that was actually used.
    let described = store.describe().await.expect("describe should succeed");
    assert_eq!(described.backend, BackendKind::OsKeychain);
    assert!(described.has_access_token);
    assert!(described.has_refresh_token);
    assert_eq!(described.base_url, PROBE_BASE_URL);

    if !keep_probe() {
        store
            .clear_session()
            .await
            .expect("clear_session should succeed");

        // Nothing may survive the clear.
        let after = store
            .load_session()
            .await
            .expect("load_session after clear should succeed");
        assert!(
            after.is_none(),
            "the session survived clear_session; the credential is still in the keychain"
        );
    }
}

#[tokio::test]
async fn cleared_session_reports_empty() {
    let store = SecretStore::new();
    let _ = store.clear_session().await;

    let described = store.describe().await.expect("describe should succeed");
    assert_eq!(
        described.backend,
        BackendKind::Empty,
        "a cleared store must report the empty backend"
    );
    assert!(!described.has_access_token);
    assert!(!described.has_refresh_token);
}
