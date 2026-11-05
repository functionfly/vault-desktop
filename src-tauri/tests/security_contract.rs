//! Contract guards for the security properties the desktop client's IPC
//! surface is supposed to hold.
//!
//! These are SOURCE-level assertions, not behavioural ones, because the
//! property is about the *shape* of the API rather than what any one call
//! happens to do. A behavioural test can only prove that the paths the
//! author thought to try are rejected; these prove that the dangerous
//! parameter is not in the signature at all.
//!
//! Two properties are pinned here:
//!
//! 1. The file-touching commands do not accept a path from the webview.
//!    A path parameter turns the Rust core into a confused deputy: the
//!    webview has no `fs:*` scope, but it can still name a path and have
//!    the core act on it, which is the same capability by another route.
//!
//! 2. The webview is never granted `dialog:*`. The native file dialog is
//!    used by the CORE to let the user choose a path. If the webview could
//!    open the dialog itself it would learn the chosen path and could hand
//!    it straight back to a path-taking command — closing the loop that
//!    property (1) opens.

use std::fs;
use std::path::{Path, PathBuf};

/// The repository root, from the crate manifest's directory (`src-tauri`).
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri must live inside the repository")
        .to_path_buf()
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {relative}: {e}"))
}

/// Extracts the parameter list of a `#[tauri::command]` function.
///
/// Deliberately simple: it finds the `pub async fn <name>(` and walks to
/// the matching close paren. The command bodies in this crate are small
/// enough that no nested function declarations confuse it.
fn command_params(source: &str, name: &str) -> String {
    let needle = format!("pub async fn {name}(");
    let start = source
        .find(&needle)
        .unwrap_or_else(|| panic!("command `{name}` not found"));
    let after = start + needle.len() - 1; // at the '('
    let bytes: Vec<char> = source[after..].chars().collect();
    let mut depth = 0usize;
    for (i, c) in bytes.iter().enumerate() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return bytes[1..i].iter().collect();
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced parentheses in command `{name}`");
}

#[test]
fn export_command_takes_no_path_from_the_webview() {
    let source = read("src-tauri/src/commands/platform.rs");
    let params = command_params(&source, "export_secrets");
    assert!(
        !params.contains("path"),
        "export_secrets must not accept a path from the webview; it opens the native \
         save dialog instead. Found parameters: {params}"
    );
}

#[test]
fn import_command_takes_no_path_from_the_webview() {
    let source = read("src-tauri/src/commands/platform.rs");
    let params = command_params(&source, "read_secret_export");
    assert!(
        !params.contains("path"),
        "read_secret_export must not accept a path from the webview; it opens the native \
         open dialog instead. Found parameters: {params}"
    );
}

#[test]
fn the_webview_is_never_granted_the_dialog_plugin() {
    let manifest = read("src-tauri/capabilities/default.toml");
    for line in manifest.lines() {
        let line = line.trim();
        // Skip comments — the manifest explains WHY dialog:* is absent,
        // and that explanation must not be mistaken for a grant.
        if line.starts_with('#') {
            continue;
        }
        assert!(
            !line.contains("dialog:"),
            "the webview must not be granted a dialog permission: the file dialog belongs \
             to the Rust core. Offending line: {line}"
        );
    }
}

#[test]
fn the_webview_is_never_granted_filesystem_or_shell_permissions() {
    let manifest = read("src-tauri/capabilities/default.toml");
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        for forbidden in ["fs:", "shell:allow-execute", "shell:allow-spawn", "http:"] {
            assert!(
                !line.contains(forbidden),
                "the webview must not be granted `{forbidden}`. Offending line: {line}"
            );
        }
    }
}

#[test]
fn a_content_security_policy_is_configured() {
    let config = read("src-tauri/tauri.conf.json");
    assert!(
        !config.contains("\"csp\": null"),
        "the webview must have a Content Security Policy; `csp: null` disables it entirely"
    );
    assert!(
        config.contains("\"script-src\""),
        "the CSP must constrain script-src"
    );
    assert!(
        config.contains("\"connect-src\""),
        "the CSP must constrain connect-src"
    );
}

#[test]
fn plaintext_bearing_structs_do_not_derive_debug() {
    // `Debug` prints a struct's fields verbatim. On a type that holds
    // plaintext, that is a one-character-away leak into any log line.
    let source = read("src-tauri/src/crypto.rs");
    for (struct_name, derive_line) in [
        ("DecryptedSecret", "DecryptedSecret"),
        ("EncryptedData", "EncryptedData"),
    ] {
        if let Some(idx) = source.find(&format!("pub struct {struct_name}")) {
            // Look at the derive attributes immediately above the struct.
            let head = &source[..idx];
            let tail = head
                .rfind("#[derive(")
                .map(|i| head[i..].split(')').next().unwrap_or(""))
                .unwrap_or("");
            assert!(
                !tail.contains("Debug"),
                "{struct_name} must not derive Debug: it carries secret material that \
                 would be printed verbatim. Derive line: {tail}"
            );
        }
    }
}
