//! One error type that crosses the Tauri FFI boundary.
//!
//! Tauri serialises the return value of every `#[tauri::command]` as JSON. We
//! expose a single `AppError` to keep the contract with the React UI small:
//! the UI only needs to know the *category* of failure to decide what to
//! show, not the exact chain. The `source` chain is preserved for logging.

use serde::Serialize;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AppErrorKind {
    /// User has not logged in, or the stored credential is invalid.
    Unauthenticated,
    /// Stored passphrase is missing or wrong; ask the user to re-unlock.
    VaultLocked,
    /// Login credentials were wrong, or a stored token was rejected.
    InvalidCredentials,
    /// Rate-limited by the orchestrator. UI should show a "slow down" hint.
    RateLimited,
    /// The tenant's plan does not include the requested feature.
    PlanDenied,
    /// Resource does not exist (404, etc.).
    NotFound,
    /// The secret is held by the PLATFORM rather than the client — its
    /// zero-knowledge flag is off, so there is no client-side envelope to
    /// open. Retrying or re-entering the passphrase will not help.
    PlatformHeld,
    /// CSRF validation failed even after a refresh. The UI should retry.
    CsrfFailed,
    /// Input failed client-side validation before it ever hit the network.
    InvalidInput,
    /// Keychain/credential store error. Almost always unrecoverable until
    /// the user signs in again.
    Storage,
    /// Network or orchestrator reachability problem.
    Transport,
    /// The user dismissed a native dialog. Not a failure — the UI should
    /// simply stand down rather than show an error.
    Cancelled,
    /// Anything else; details in `message`.
    Unknown,
}

#[derive(Debug)]
pub struct AppError {
    pub kind: AppErrorKind,
    pub message: String,
}

impl AppError {
    pub fn new(kind: AppErrorKind, message: impl fmt::Display) -> Self {
        AppError {
            kind,
            message: message.to_string(),
        }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for AppError {}

impl Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = ser.serialize_struct("AppError", 2)?;
        s.serialize_field("kind", &self.kind)?;
        s.serialize_field("message", &self.message)?;
        s.end()
    }
}

impl From<crate::api::ApiError> for AppError {
    fn from(e: crate::api::ApiError) -> Self {
        use crate::api::ApiError::*;
        let kind = match &e {
            Unauthenticated => AppErrorKind::Unauthenticated,
            RateLimited => AppErrorKind::RateLimited,
            PlanDenied => AppErrorKind::PlanDenied,
            NotFound => AppErrorKind::NotFound,
            CsrfRetry => AppErrorKind::CsrfFailed,
            NotConfigured(_) => AppErrorKind::InvalidInput,
            Api { .. } => AppErrorKind::Unknown,
            Transport(_) => AppErrorKind::Transport,
        };
        AppError::new(kind, e)
    }
}

impl From<crate::store::StoreError> for AppError {
    fn from(e: crate::store::StoreError) -> Self {
        use crate::store::StoreError::*;
        let kind = match e {
            NotFound => AppErrorKind::VaultLocked,
            Keychain(_) | Unsupported(_) | crate::store::StoreError::FileFallback(_) => {
                AppErrorKind::Storage
            }
        };
        AppError::new(kind, e)
    }
}

impl From<crate::crypto::CryptoError> for AppError {
    fn from(e: crate::crypto::CryptoError) -> Self {
        use crate::crypto::CryptoError::*;
        let kind = match e {
            DecryptFailed | MissingSalt => AppErrorKind::InvalidCredentials,
            UnsupportedKeyVersion(_) | Malformed(_) => AppErrorKind::InvalidInput,
            EncryptFailed(_) | Random(_) => AppErrorKind::Unknown,
        };
        AppError::new(kind, e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ApiError;
    use crate::crypto::CryptoError;
    use crate::store::StoreError;

    /// The webview branches on `kind`, so the wire shape is a contract.
    /// A change to the field names or the casing silently breaks the UI.
    #[test]
    fn error_serializes_to_kind_and_message() {
        let err = AppError::new(AppErrorKind::VaultLocked, "vault is locked");
        let json = serde_json::to_value(&err).unwrap();

        assert_eq!(json["kind"], "vault_locked");
        assert_eq!(json["message"], "vault is locked");
        // Exactly two fields: the UI's TypeScript type has no room for more.
        assert_eq!(
            json.as_object().unwrap().len(),
            2,
            "AppError must serialize to exactly {{kind, message}}"
        );
    }

    #[test]
    fn error_kinds_are_snake_case() {
        // Every variant the UI can receive, with the exact casing it
        // expects. `AppErrorKind` is `rename_all = "snake_case"`, and
        // this pins the result so a serde attribute change is caught.
        let cases = [
            (AppErrorKind::Unauthenticated, "unauthenticated"),
            (AppErrorKind::VaultLocked, "vault_locked"),
            (AppErrorKind::InvalidCredentials, "invalid_credentials"),
            (AppErrorKind::RateLimited, "rate_limited"),
            (AppErrorKind::PlanDenied, "plan_denied"),
            (AppErrorKind::NotFound, "not_found"),
            (AppErrorKind::CsrfFailed, "csrf_failed"),
            (AppErrorKind::InvalidInput, "invalid_input"),
            (AppErrorKind::Storage, "storage"),
            (AppErrorKind::Transport, "transport"),
            (AppErrorKind::Unknown, "unknown"),
        ];
        for (kind, expected) in cases {
            let json = serde_json::to_value(AppError::new(kind, "x")).unwrap();
            assert_eq!(json["kind"], expected);
        }
    }

    #[test]
    fn api_errors_map_to_the_expected_kinds() {
        let cases: Vec<(ApiError, AppErrorKind)> = vec![
            (ApiError::Unauthenticated, AppErrorKind::Unauthenticated),
            (ApiError::RateLimited, AppErrorKind::RateLimited),
            (ApiError::PlanDenied, AppErrorKind::PlanDenied),
            (ApiError::NotFound, AppErrorKind::NotFound),
            (ApiError::CsrfRetry, AppErrorKind::CsrfFailed),
            (
                ApiError::NotConfigured("no base url".into()),
                AppErrorKind::InvalidInput,
            ),
            (
                ApiError::Api {
                    status: 500,
                    code: "boom".into(),
                    message: "server error".into(),
                },
                AppErrorKind::Unknown,
            ),
            (
                ApiError::Transport("dns failure".into()),
                AppErrorKind::Transport,
            ),
        ];
        for (api_err, expected) in cases {
            let app_err: AppError = api_err.into();
            assert_eq!(app_err.kind, expected);
        }
    }

    #[test]
    fn crypto_errors_map_to_the_expected_kinds() {
        // A wrong passphrase must NOT be reported as invalid input: the
        // user needs to be told to re-enter the passphrase, and it must
        // not reveal whether the salt or the tag was wrong.
        let cases: Vec<(CryptoError, AppErrorKind)> = vec![
            (CryptoError::DecryptFailed, AppErrorKind::InvalidCredentials),
            (CryptoError::MissingSalt, AppErrorKind::InvalidCredentials),
            (
                CryptoError::UnsupportedKeyVersion(99),
                AppErrorKind::InvalidInput,
            ),
            (
                CryptoError::Malformed("bad".into()),
                AppErrorKind::InvalidInput,
            ),
            (
                CryptoError::EncryptFailed("x".into()),
                AppErrorKind::Unknown,
            ),
            (
                CryptoError::Random("no entropy".into()),
                AppErrorKind::Unknown,
            ),
        ];
        for (crypto_err, expected) in cases {
            let app_err: AppError = crypto_err.into();
            assert_eq!(app_err.kind, expected);
        }
    }

    #[test]
    fn store_errors_map_to_the_expected_kinds() {
        let cases: Vec<(StoreError, AppErrorKind)> = vec![
            // "no credential stored" means the vault is locked, not
            // that the keychain is broken — the UI re-prompts for the
            // passphrase in that case.
            (StoreError::NotFound, AppErrorKind::VaultLocked),
            (StoreError::Keychain("denied".into()), AppErrorKind::Storage),
            (
                StoreError::Unsupported("no backend".into()),
                AppErrorKind::Storage,
            ),
            (
                StoreError::FileFallback("disk".into()),
                AppErrorKind::Storage,
            ),
        ];
        for (store_err, expected) in cases {
            let app_err: AppError = store_err.into();
            assert_eq!(app_err.kind, expected);
        }
    }

    #[test]
    fn decrypt_failure_message_does_not_leak_which_part_failed() {
        // `CryptoError::DecryptFailed` deliberately collapses "wrong
        // salt" and "forged tag" into one message. If a future change
        // splits them, the distinction would become an oracle.
        let err: AppError = CryptoError::DecryptFailed.into();
        let message = err.message.to_lowercase();
        assert!(
            !message.contains("salt"),
            "message leaked 'salt': {message}"
        );
        assert!(!message.contains("tag"), "message leaked 'tag': {message}");
    }

    #[test]
    fn error_display_is_the_message() {
        let err = AppError::new(AppErrorKind::Storage, "keychain unavailable");
        assert_eq!(err.to_string(), "keychain unavailable");
    }
}
