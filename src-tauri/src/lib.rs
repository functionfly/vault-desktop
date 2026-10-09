//! Library entry point. Splitting library and binary lets integration tests
//! import the crate and exercise individual commands without going through
//! the Tauri runtime.

pub mod api;
pub mod commands;
pub mod crypto;
pub mod error;
pub mod store;
pub mod updater;

use tauri::Manager;

use commands::AppState;

/// Builds a Tauri runtime and registers the full command surface. The
/// concrete `tauri::Builder` lives in `main.rs` so the binary can tweak it
/// (icon, tray, etc.) without dragging the library into binary-only deps.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
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
            // Updater (signed auto-update; fail-closed when
            // `VAULT_UPDATER_PUBKEY` is unset).
            commands::updater::vault_check_update,
            commands::updater::vault_apply_update,
        ])
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
