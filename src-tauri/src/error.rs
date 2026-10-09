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
    /// CSRF validation failed even after a refresh. The UI should retry.
    CsrfFailed,
    /// Input failed client-side validation before it ever hit the network.
    InvalidInput,
    /// Keychain/credential store error. Almost always unrecoverable until
    /// the user signs in again.
    Storage,
    /// Network or orchestrator reachability problem.
    Transport,
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
