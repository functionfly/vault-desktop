//! Contract test for the updater's build-time configuration.
//!
//! WHY THIS EXISTS
//! ---------------
//! The public key that verifies update signatures has to be compiled
//! INTO the shipped binary, not read from the operator's environment.
//! An end user who installs the app has no `VAULT_UPDATER_PUBKEY` set,
//! so a build that only works when that variable is present ships an
//! updater that can never verify anything.
//!
//! There is a second, sharper failure mode. The Tauri CLI reads
//! `plugins.updater.pubkey` out of `tauri.conf.json` when it signs the
//! updater artefacts, and `decode_key` on an empty string fails to
//! produce a minisign key — so a release build with an empty pubkey
//! fails at the signing step, not at runtime. Neither the CI
//! `--no-bundle` compile gate nor `cargo test` exercises that path,
//! because `--no-bundle` produces no artefacts to sign.
//!
//! This test reads `tauri.conf.json` the same way the CLI does and
//! asserts the key is present and parseable, so a regression to an
//! empty pubkey fails in normal CI rather than in a release.

use std::fs;
use std::path::PathBuf;

/// The `tauri.conf.json` that the CLI and the compiler both read.
fn tauri_conf() -> serde_json::Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json");
    let raw =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parsing {}: {e}", path.display()))
}

fn updater_plugin_section() -> serde_json::Value {
    tauri_conf()
        .get("plugins")
        .and_then(|p| p.get("updater"))
        .cloned()
        .expect("tauri.conf.json must declare a `plugins.updater` section")
}

#[test]
fn updater_pubkey_is_present_in_the_shipped_config() {
    let section = updater_plugin_section();
    let pubkey = section
        .get("pubkey")
        .and_then(|k| k.as_str())
        .expect("plugins.updater.pubkey must be a string");

    assert!(
        !pubkey.trim().is_empty(),
        "plugins.updater.pubkey is empty. The updater would ship without a \
         verification key: the Tauri CLI cannot sign the updater artefacts \
         and an installed client cannot verify them."
    );
}

#[test]
fn updater_pubkey_parses_with_the_plugins_own_config_type() {
    // Deserialize with the plugin's own `Config`, so this fails if the
    // shape of the section drifts from what `tauri-plugin-updater`
    // accepts at startup.
    let section = updater_plugin_section();
    let config: tauri_plugin_updater::Config = serde_json::from_value(section)
        .expect("plugins.updater must deserialize into tauri_plugin_updater::Config");

    assert!(
        !config.pubkey.trim().is_empty(),
        "the parsed updater config has an empty pubkey"
    );
}

#[test]
fn updater_pubkey_is_a_decodable_minisign_public_key() {
    let section = updater_plugin_section();
    let pubkey = section
        .get("pubkey")
        .and_then(|k| k.as_str())
        .expect("plugins.updater.pubkey must be a string");

    // The plugin base64-decodes the configured value and then parses it
    // as a minisign public key box. Reproducing that here means a
    // truncated or mis-encoded key fails here rather than at the first
    // update check on a user's machine.
    let decoded =
        base64_decode(pubkey).unwrap_or_else(|e| panic!("pubkey is not valid base64: {e}"));
    let text = String::from_utf8(decoded)
        .unwrap_or_else(|e| panic!("pubkey does not decode to UTF-8: {e}"));

    assert!(
        text.contains("minisign public key"),
        "the decoded pubkey does not look like a minisign public key box"
    );
}

#[test]
fn updater_requires_a_signed_version() {
    // `require_signed_version` stops a downgrade: the endpoint response
    // is not itself signed, so without this an attacker who can serve
    // a crafted response could pair a newer `version` field with the
    // signature of an older, genuine release.
    let section = updater_plugin_section();
    assert_eq!(
        section
            .get("requireSignedVersion")
            .and_then(|v| v.as_bool()),
        Some(true),
        "plugins.updater.requireSignedVersion must stay true"
    );
}

/// Minimal base64 decoder.
///
/// The crate graph already depends on `base64` through the Tauri
/// dependencies, but it is not a direct dependency of this crate, so
/// this test does not add one just to decode 152 bytes.
fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    const INVALID: &str = "invalid base64";
    let mut out = Vec::new();
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;

    for ch in input.chars() {
        if ch == '=' {
            break;
        }
        if ch.is_whitespace() {
            continue;
        }
        let value = match ch {
            'A'..='Z' => ch as u32 - 'A' as u32,
            'a'..='z' => ch as u32 - 'a' as u32 + 26,
            '0'..='9' => ch as u32 - '0' as u32 + 52,
            '+' => 62,
            '/' => 63,
            _ => return Err(format!("unexpected character {ch:?}")),
        };
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buffer >> bits) & 0xFF) as u8);
        }
    }

    if out.is_empty() {
        return Err(INVALID.to_string());
    }
    Ok(out)
}
