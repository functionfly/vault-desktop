//! Library entry point. Splitting library and binary lets integration tests
//! import the crate and exercise individual commands without going through
//! the Tauri runtime.

pub mod api;
pub mod cache;
pub mod commands;
pub mod crypto;
pub mod error;
pub mod store;
pub mod updater;

use tauri::{Emitter, Manager};

use commands::AppState;

/// The event emitted when the vault auto-locks because the user went idle.
///
/// The webview listens for it and switches to the unlock screen. It is a
/// plain string event rather than a payload so the webview cannot read
/// anything sensitive out of it.
pub const EVENT_VAULT_LOCKED: &str = "vault://locked";

/// Builds a Tauri runtime and registers the full command surface. The
/// concrete `tauri::Builder` lives in `main.rs` so the binary can tweak it
/// (icon, tray, etc.) without dragging the library into binary-only deps.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        // The native file dialog, used ONLY from the Rust core: the core
        // opens it to let the user choose where an export lands, and to
        // choose which export to read. The webview never calls this
        // plugin, so the capability manifest does not grant `dialog:*`
        // — Tauri permissions gate the webview→core direction only.
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            commands::login,
            commands::login_with_api_key,
            commands::begin_device_login,
            commands::await_device_login,
            commands::restore_session,
            commands::logout,
            commands::unlock_vault,
            commands::lock_vault,
            commands::vault_status,
            commands::vault_health_probe,
            commands::vault_store_info,
            commands::list_secrets,
            commands::get_secret,
            commands::list_secret_versions,
            commands::reveal_secret,
            commands::diff_secret_versions,
            commands::rollback_secret,
            commands::create_secret,
            commands::update_secret,
            commands::rotate_secret,
            commands::delete_secret,
            commands::list_namespaces,
            commands::create_namespace,
            commands::delete_namespace,
            commands::list_tokens,
            commands::generate_token,
            commands::revoke_token,
            commands::bulk_delete_secrets,
            commands::list_audit,
            commands::list_audit_by_action,
            commands::list_audit_by_actor,
            // Audit hash chain (tamper-evidence).
            commands::platform::get_audit_chain,
            commands::platform::verify_audit_chain,
            // Capability grants.
            commands::platform::list_capabilities,
            commands::platform::approve_capability,
            commands::platform::deny_capability,
            commands::platform::revoke_capability,
            commands::platform::get_secret_presence,
            // Tenant vault MFA policy.
            commands::platform::get_mfa_config,
            commands::platform::verify_mfa,
            // Dynamic credentials + leases.
            commands::platform::create_dynamic_target,
            commands::platform::list_dynamic_targets,
            commands::platform::delete_dynamic_target,
            commands::platform::test_dynamic_target,
            commands::platform::create_dynamic_credential,
            commands::platform::list_dynamic_credentials,
            commands::platform::generate_dynamic_credential,
            commands::platform::revoke_dynamic_credential,
            commands::platform::renew_lease,
            commands::platform::revoke_lease,
            // Sharing and federation.
            commands::platform::share_secret,
            commands::platform::list_shared_secrets,
            commands::platform::revoke_share,
            commands::platform::list_federation_keys,
            commands::platform::create_federation_key,
            // Import / export (metadata only — never a ciphertext).
            commands::platform::export_secrets,
            commands::platform::read_secret_export,
            // Offline metadata cache (metadata only — never a ciphertext).
            commands::read_metadata_cache,
            commands::clear_metadata_cache,
            // Idle auto-lock.
            commands::idle::report_activity,
            commands::idle::get_idle_settings,
            commands::idle::set_idle_settings,
            // Updater (signed auto-update; fail-closed when
            // `VAULT_UPDATER_PUBKEY` is unset).
            commands::updater::vault_check_update,
            commands::updater::vault_apply_update,
        ])
        .setup(|app| {
            // The idle watcher. It polls the shared tracker and locks the
            // vault when the idle period elapses.
            //
            // Polling rather than a timer per activity event keeps the cost
            // to one cheap clock read per tick, and makes the timeout
            // adjustable at runtime without re-arming anything.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // Tick fast enough that the lock lands close to the
                // configured timeout, but slow enough not to matter.
                const TICK: std::time::Duration = std::time::Duration::from_secs(5);
                loop {
                    tokio::time::sleep(TICK).await;
                    let Some(state) = handle.try_state::<AppState>() else {
                        // The app is shutting down.
                        continue;
                    };
                    // Only consider locking when the vault is actually
                    // unlocked — otherwise this is a no-op loop.
                    if !state.store.is_unlocked().await {
                        continue;
                    }
                    if !state.idle.should_lock() {
                        continue;
                    }
                    state.store.lock().await;
                    let _ = handle.emit(EVENT_VAULT_LOCKED, ());
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                // Wipe the unlock passphrase as the window goes away. The
                // keychain-resident session persists; the next launch can
                // prompt for the passphrase again.
                if let Some(state) = window.try_state::<AppState>() {
                    let store = state.store.clone();
                    tauri::async_runtime::spawn(async move {
                        store.lock().await;
                    });
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
