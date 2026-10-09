// Prevents a separate console window from popping up on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    functionfly_vault_desktop_lib::run();
}
