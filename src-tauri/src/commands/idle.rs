//! Idle auto-lock.
//!
//! The vault locks when the window closes. That is not enough: a laptop left
//! open on a desk keeps the unlock passphrase in memory for as long as the
//! process lives, and every password manager that does not have an idle
//! timeout is a security review finding.
//!
//! The timer lives in the Rust core, not the webview, for the same reason
//! the passphrase does: the webview is the untrusted half of the app. If the
//! idle countdown lived in JavaScript, a compromised renderer could simply
//! not run it — and it would be the renderer that had just been compromised.
//!
//! The webview reports *activity* (a mouse move, a keystroke, a command
//! being issued); the core decides when the idle period has elapsed.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;
use tauri::State;

use crate::commands::AppState;
use crate::error::{AppError, AppErrorKind};

/// The idle timeout options the UI offers, in seconds.
///
/// `0` means "never lock automatically". The default is 5 minutes, which is
/// short enough to matter and long enough not to interrupt a real editing
/// session.
pub const IDLE_TIMEOUT_CHOICES: [u32; 5] = [0, 60, 300, 900, 1800];

/// The default idle timeout, in seconds.
pub const DEFAULT_IDLE_TIMEOUT_SECONDS: u32 = 300;

/// Tracks the last time the user did something.
///
/// The elapsed time is measured with [`Instant`], which is monotonic — a
/// system clock change (NTP correction, a manual jump, a DST boundary)
/// cannot make the vault look idle or busy when it is neither.
#[derive(Debug)]
pub struct IdleTracker {
    /// The instant of the last recorded activity.
    last_activity: std::sync::Mutex<Instant>,
    /// The configured timeout, in seconds. `0` disables auto-lock.
    timeout_seconds: AtomicU64,
}

impl Default for IdleTracker {
    fn default() -> Self {
        Self::new(DEFAULT_IDLE_TIMEOUT_SECONDS)
    }
}

impl IdleTracker {
    pub fn new(timeout_seconds: u32) -> Self {
        Self {
            last_activity: std::sync::Mutex::new(Instant::now()),
            timeout_seconds: AtomicU64::new(timeout_seconds as u64),
        }
    }

    /// Records that the user did something.
    pub fn touch(&self) {
        if let Ok(mut guard) = self.last_activity.lock() {
            *guard = Instant::now();
        }
    }

    /// The configured timeout in seconds. `0` disables auto-lock.
    pub fn timeout_seconds(&self) -> u32 {
        self.timeout_seconds.load(Ordering::Relaxed) as u32
    }

    /// Sets the timeout. `0` disables auto-lock.
    pub fn set_timeout_seconds(&self, seconds: u32) {
        self.timeout_seconds
            .store(seconds as u64, Ordering::Relaxed);
    }

    /// How long the user has been idle.
    pub fn idle_for(&self) -> std::time::Duration {
        match self.last_activity.lock() {
            Ok(guard) => guard.elapsed(),
            // A poisoned mutex means another thread panicked while holding
            // it. Treat the vault as idle so it locks rather than panicking.
            Err(poisoned) => poisoned.into_inner().elapsed(),
        }
    }

    /// Whether the idle period has elapsed.
    pub fn should_lock(&self) -> bool {
        let timeout = self.timeout_seconds();
        if timeout == 0 {
            return false;
        }
        self.idle_for() >= std::time::Duration::from_secs(timeout as u64)
    }
}

/// The idle state shared between the command layer and the UI.
pub type SharedIdleTracker = Arc<IdleTracker>;

/// Reports that the user did something.
///
/// The webview calls this on user activity. It never re-arms a timer for a
/// locked vault, because the caller only reports activity while the vault is
/// open.
#[tauri::command]
pub async fn report_activity(state: State<'_, AppState>) -> Result<(), AppError> {
    state.idle.touch();
    Ok(())
}

/// Reads the current idle-lock configuration.
#[tauri::command]
pub async fn get_idle_settings(state: State<'_, AppState>) -> Result<IdleSettings, AppError> {
    Ok(IdleSettings {
        timeout_seconds: state.idle.timeout_seconds(),
        choices: IDLE_TIMEOUT_CHOICES.to_vec(),
    })
}

/// Sets the idle-lock timeout. `0` disables auto-lock.
#[tauri::command]
pub async fn set_idle_settings(
    state: State<'_, AppState>,
    timeout_seconds: u32,
) -> Result<IdleSettings, AppError> {
    if timeout_seconds != 0 && !IDLE_TIMEOUT_CHOICES.contains(&timeout_seconds) {
        return Err(AppError::new(
            AppErrorKind::InvalidInput,
            format!("idle timeout must be one of {IDLE_TIMEOUT_CHOICES:?} (or 0 to disable)"),
        ));
    }
    state.idle.set_timeout_seconds(timeout_seconds);
    // Changing the setting is itself activity.
    state.idle.touch();
    Ok(IdleSettings {
        timeout_seconds: state.idle.timeout_seconds(),
        choices: IDLE_TIMEOUT_CHOICES.to_vec(),
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct IdleSettings {
    /// The configured timeout in seconds. `0` means auto-lock is off.
    pub timeout_seconds: u32,
    /// The selectable values, for the UI to render without hardcoding them.
    pub choices: Vec<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_timeout_disables_auto_lock() {
        let tracker = IdleTracker::new(0);
        assert!(!tracker.should_lock());
    }

    #[test]
    fn a_fresh_tracker_is_not_idle() {
        let tracker = IdleTracker::new(300);
        assert!(!tracker.should_lock());
    }

    #[test]
    fn touch_resets_the_idle_window() {
        let tracker = IdleTracker::new(300);
        // Zero elapsed time cannot be forced without sleeping, so assert the
        // observable contract: the tracker reports a non-negative idle time
        // and touching it does not arm the lock.
        tracker.touch();
        assert!(tracker.idle_for() < std::time::Duration::from_secs(1));
        assert!(!tracker.should_lock());
    }

    #[test]
    fn setting_the_timeout_is_accepted_and_read_back() {
        let tracker = IdleTracker::new(300);
        tracker.set_timeout_seconds(60);
        assert_eq!(tracker.timeout_seconds(), 60);
    }

    #[test]
    fn the_default_is_five_minutes() {
        assert_eq!(DEFAULT_IDLE_TIMEOUT_SECONDS, 300);
        assert!(IDLE_TIMEOUT_CHOICES.contains(&DEFAULT_IDLE_TIMEOUT_SECONDS));
    }
}
