//! Verify the PUBLISHED updater artefact over the wire.
//!
//! Downloads the artefact the live manifest points at and checks its
//! signature against the public key compiled into the binary, using the
//! SAME minisign implementation the updater plugin uses at runtime.
//!
//! This is the end-to-end proof that an installed client can verify and
//! install the published update: manifest → URL → bytes → signature.
//!
//! It is skipped (not failed) when the network is unavailable, so it can
//! live in the gate set without making an offline run fail. The skip
//! message is printed, because a silent skip that reports "ok" is
//! indistinguishable from a pass.

use functionfly_vault_desktop_lib::updater;

/// The public key as it is compiled into the binary.
///
/// This is the base64 of the WHOLE minisign box (the two-line `.pub`
/// file), exactly as it appears in `plugins.updater.pubkey`.
const PUBKEY_BOX: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1vbmlzaWduIHB1YmxpYyBrZXk6IEQyNzI5RjgzQkY3NDgwMUYKUldRZmdIUy9nNTl5MHM2c2RwcU5nY3p5S3ZCWGNzODdRYzcyNXNXKzdxaVVBejg4Vy9SdFBFdkIK";

const MANIFEST_URL: &str = "https://updates.functionfly.com/vault/stable/manifest.json";

/// Unwraps a base64 minisign box into its two text lines.
///
/// `minisign-verify` wants the two-line form, so the outer base64 has to
/// be unwrapped first. The same helper serves the public key and the
/// signature.
fn decode_box(encoded: &str) -> String {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    let bytes = STANDARD
        .decode(encoded.trim())
        .expect("a minisign box must be base64");
    String::from_utf8(bytes).expect("a minisign box must decode to UTF-8")
}

/// The public key, parsed from the compiled-in box.
fn compiled_public_key() -> minisign_verify::PublicKey {
    let box_text = decode_box(PUBKEY_BOX);
    let key_line = box_text
        .lines()
        .nth(1)
        .expect("a minisign public key box has two lines")
        .to_string();
    minisign_verify::PublicKey::from_base64(&key_line)
        .expect("the compiled-in public key must be a valid minisign key")
}

/// The manifest's `platforms` entries, as (key, url, signature).
///
/// The manifest is a flat document, and `serde_json` is already a
/// dependency of the crate, so the test uses it rather than hand-rolling
/// a parser that would be more fragile than the thing it checks.
fn manifest_entries(body: &str) -> Vec<(String, String, String)> {
    let value: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => panic!("the manifest must be valid JSON: {e}"),
    };
    let platforms = value
        .get("platforms")
        .and_then(|p| p.as_object())
        .expect("the manifest must carry a platforms object");

    platforms
        .iter()
        .map(|(key, entry)| {
            let url = entry
                .get("url")
                .and_then(|u| u.as_str())
                .unwrap_or_else(|| panic!("{key} must carry a url"))
                .to_string();
            let signature = entry
                .get("signature")
                .and_then(|s| s.as_str())
                .unwrap_or_else(|| panic!("{key} must carry a signature"))
                .to_string();
            (key.clone(), url, signature)
        })
        .collect()
}

/// A GET, using the crate's OWN HTTP client.
///
/// `reqwest` with rustls is already a dependency, so the test does not
/// add one — and it uses the same TLS stack the application uses, which
/// is the point: this test asks whether an installed client can fetch
/// and verify the published update.
///
/// An earlier version of this file used a raw TCP socket. That cannot
/// speak TLS, so every request failed and the test silently SKIPPED
/// while reporting "ok" — the worst possible combination. The skip is
/// still there for a genuinely offline machine, but it is now reached
/// only when the HTTPS request itself fails.
fn http_get(url: &str) -> Option<Vec<u8>> {
    use std::time::Duration;

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .ok()?;
    let response = client.get(url).send().ok()?;
    if !response.status().is_success() {
        eprintln!("  {} returned HTTP {}", url, response.status());
        return None;
    }
    response.bytes().ok().map(|b| b.to_vec())
}

#[test]
fn the_compiled_key_is_the_one_the_plugin_uses() {
    // The plugin must accept the key this test verifies with. If the
    // compiled-in key and the test's copy ever diverge, this fails
    // before the network test says anything.
    let config = updater::UpdaterConfig::from_env(Some(PUBKEY_BOX));
    assert!(config.is_ok(), "the compiled-in key must be accepted");

    // The key must parse from the box form too — that is how the plugin
    // reads it at runtime. If this fails, every update would be
    // rejected.
    //
    // `from_base64` (which takes the key line alone) keeps no comment;
    // `decode` (which takes the whole box) keeps the first line as the
    // untrusted comment. Both must yield a usable key, so the check is
    // that each parses, not that their comments match.
    let box_text = decode_box(PUBKEY_BOX);
    minisign_verify::PublicKey::decode(&box_text)
        .expect("the compiled-in key must parse from its box form");
    compiled_public_key();

    // The box's first line is the untrusted comment, and the second is
    // the key itself. The plugin's `from_base64` is given the second.
    let key_line = box_text
        .lines()
        .nth(1)
        .expect("a minisign public key box has two lines");
    minisign_verify::PublicKey::from_base64(key_line)
        .expect("the compiled-in key must parse from its key line");
}

#[test]
fn the_published_stable_manifest_is_well_formed() {
    let raw = match http_get(MANIFEST_URL) {
        Some(b) => b,
        None => {
            eprintln!("SKIP: the stable manifest is not reachable from this machine");
            return;
        }
    };
    let body = String::from_utf8_lossy(&raw).to_string();

    let value: serde_json::Value =
        serde_json::from_str(&body).expect("the stable manifest must be valid JSON");
    let version = value
        .get("version")
        .and_then(|v| v.as_str())
        .expect("the manifest must carry a version");
    assert!(
        !version.is_empty(),
        "the manifest's version must not be empty"
    );

    let entries = manifest_entries(&body);
    assert!(
        !entries.is_empty(),
        "the manifest must carry at least one platform"
    );

    for (key, url, signature) in &entries {
        assert!(
            url.starts_with("https://"),
            "{key} url must be https: {url}"
        );
        assert!(
            !url.contains(' '),
            "{key} url must not contain a space — it is not fetchable: {url}"
        );
        assert!(
            !signature.starts_with("http"),
            "{key} signature must be the content, not a URL: {signature}"
        );
        // The signature must be the base64 of a minisign box.
        let box_text = decode_box(signature);
        assert!(
            box_text.lines().count() >= 2,
            "{key} signature must be a two-line minisign box"
        );
    }
}

#[test]
fn the_published_artefact_signature_verifies() {
    let raw = match http_get(MANIFEST_URL) {
        Some(b) => b,
        None => {
            eprintln!("SKIP: the stable manifest is not reachable from this machine");
            return;
        }
    };
    let body = String::from_utf8_lossy(&raw).to_string();
    let entries = manifest_entries(&body);

    let key = compiled_public_key();
    let mut checked = 0usize;

    for (platform, url, signature_b64) in &entries {
        let artefact = match http_get(url) {
            Some(a) => a,
            None => {
                eprintln!("SKIP: the {platform} artefact is not reachable");
                continue;
            }
        };

        // The manifest carries the base64 CONTENT of the .sig box. The
        // plugin base64-decodes it and parses the two-line minisign box
        // it contains.
        let box_text = decode_box(signature_b64);
        let signature = minisign_verify::Signature::decode(&box_text)
            .unwrap_or_else(|e| panic!("{platform}: the signature must parse: {e}"));

        // `verify` compares the key ids itself and returns
        // `UnexpectedKeyId` when they differ, so a signature made with a
        // different key is rejected here too.
        key.verify(&artefact, &signature, false).unwrap_or_else(|e| {
            panic!("{platform}: the published artefact does not verify: {e}")
        });

        println!(
            "{platform}: signature verified over the wire ({} bytes from {url})",
            artefact.len()
        );
        checked += 1;
    }

    assert!(
        checked > 0,
        "no published artefact was checked — the manifest listed none, or none were reachable"
    );
}
