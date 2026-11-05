//! Contract test for the updater's artefact naming and signature format.
//!
//! WHY THIS EXISTS
//! ---------------
//! `scripts/release/updater-targets.py` maps a build artefact filename to
//! the manifest target keys the plugin looks up. Its table was written
//! against the *Tauri v1* layout, where every updater artefact was
//! wrapped in a `.tar.gz`.
//!
//! Tauri v2 does not wrap them. From the bundler source
//! (`tauri-bundler/src/bundle.rs`):
//!
//! ```text
//! // Targets the v2 updater plugin installs directly, no wrapping needed.
//! let has_self_contained_target = package_types.iter().any(|package_type| {
//!   matches!(package_type,
//!     PackageType::AppImage | PackageType::Nsis | PackageType::WindowsMsi
//!     | PackageType::Deb | PackageType::Rpm)
//! });
//!
//! // the macOS app bundle is always archived, the other v1 targets only
//! // for the legacy updater
//! if package_types.contains(&PackageType::MacOsBundle) || has_v1_target {
//! ```
//!
//! So a Tauri v2 build produces:
//!
//! | Platform | Artefact        | Wrapped? |
//! | -------- | --------------- | -------- |
//! | Linux    | `X.AppImage`    | no       |
//! | Linux    | `X.deb`         | no       |
//! | Linux    | `X.rpm`         | no       |
//! | macOS    | `X.app.tar.gz`  | yes      |
//! | Windows  | `X.nsis.zip`    | yes      |
//! | Windows  | `X.msi.zip`     | yes      |
//!
//! The old table expected `.appimage.tar.gz` / `.deb.tar.gz` /
//! `.rpm.tar.gz` on Linux, so `updater-targets.py` REJECTED every real
//! Linux artefact with "not a recognised updater artefact". The updater
//! manifest for Linux could never have been published.
//!
//! This test pins the real names, and verifies a signature with the same
//! `minisign-verify` crate the updater plugin itself uses — so "the
//! signature is valid" means "the plugin will accept it", not "some
//! tool said so".

use base64::Engine;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The `.sig` and `.pub` files are base64 of a two-line minisign box:
///
/// ```text
/// untrusted comment: ...
/// <base64 of alg[2] | keynum[8] | payload>
/// ```
///
/// `minisign-verify` wants the two-line form, so the outer base64 has
/// to be unwrapped first.
fn decode_box(file: &Path) -> String {
    use base64::engine::general_purpose::STANDARD;

    let raw = std::fs::read_to_string(file)
        .unwrap_or_else(|e| panic!("{} must be readable: {e}", file.display()));
    let bytes = STANDARD
        .decode(raw.trim())
        .unwrap_or_else(|e| panic!("{} must be base64: {e}", file.display()));
    String::from_utf8(bytes)
        .unwrap_or_else(|e| panic!("{} must decode to UTF-8: {e}", file.display()))
}

/// The `updater-targets.py` mapping, transcribed.
///
/// Kept in sync with the Python script by the test that runs the script
/// and compares its output.
const EXPECTED: &[(&str, &[&str])] = &[
    // Linux artefacts are NOT wrapped by the v2 bundler.
    (
        "linux-x86_64_amd64.AppImage",
        &["linux-x86_64-appimage", "linux-x86_64"],
    ),
    ("linux-x86_64_amd64.deb", &["linux-x86_64-deb"]),
    ("linux-x86_64_amd64.rpm", &["linux-x86_64-rpm"]),
    // macOS and Windows ARE wrapped.
    (
        "darwin-universal_amd64.app.tar.gz",
        &[
            // Per architecture: the bundle-specific key, then the
            // bundle-type-less fallback.
            "darwin-aarch64-app",
            "darwin-aarch64",
            "darwin-x86_64-app",
            "darwin-x86_64",
        ],
    ),
    (
        "windows-x86_64-setup.nsis.zip",
        &["windows-x86_64-nsis", "windows-x86_64"],
    ),
    ("windows-x86_64_en-US.msi.zip", &["windows-x86_64-msi"]),
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn targets_script() -> PathBuf {
    repo_root().join("scripts/release/updater-targets.py")
}

/// Run `updater-targets.py` and return the targets it printed.
fn targets_for(filename: &str) -> Vec<String> {
    let output = Command::new("python3")
        .arg(targets_script())
        .arg(filename)
        .output()
        .expect("python3 must be available to run updater-targets.py");

    assert!(
        output.status.success(),
        "updater-targets.py rejected '{filename}':\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    String::from_utf8_lossy(&output.stdout)
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[test]
fn the_real_artefact_names_are_recognised() {
    // These are the EXACT filenames a Tauri v2 build produced in the
    // release build on 2026-11-04. If the mapping regresses to the v1
    // `.tar.gz` names, this fails.
    for (filename, expected) in EXPECTED {
        let got = targets_for(filename);
        assert_eq!(
            &got[..],
            *expected,
            "updater-targets.py mapped '{filename}' to {got:?}, expected {expected:?}"
        );
    }
}

#[test]
fn the_v1_tar_gz_names_are_still_rejected() {
    // The v1 names must NOT silently map to something, because they are
    // not what the v2 bundler produces. Publishing one would put a URL
    // in the manifest that no client can fetch.
    for filename in [
        "functionfly-vault-desktop_0.1.0_amd64.AppImage.tar.gz",
        "functionfly-vault-desktop_0.1.0_amd64.deb.tar.gz",
        "functionfly-vault-desktop_0.1.0_amd64.rpm.tar.gz",
    ] {
        let output = Command::new("python3")
            .arg(targets_script())
            .arg(filename)
            .output()
            .expect("python3 must be available to run updater-targets.py");

        assert!(
            !output.status.success(),
            "updater-targets.py accepted the v1 artefact name '{filename}'. \
             The v2 bundler never produces it, so a manifest entry built \
             from it would point at a URL that does not exist."
        );
    }
}

#[test]
fn a_built_artefact_and_its_signature_verify() {
    // If a release has been built on this host, verify it the way the
    // updater plugin will. Without this, "the .sig file exists" is the
    // only claim anyone can make about a release.
    let bundle = Path::new("/tmp/vault-target-linux/release/bundle");
    let appimage_dir = bundle.join("appimage");
    if !appimage_dir.exists() {
        eprintln!(
            "SKIP: no release build at {} — run `bunx tauri build --bundles \
             appimage,deb` with the signing env loaded to exercise this.",
            bundle.display()
        );
        return;
    }

    let entries: Vec<PathBuf> = std::fs::read_dir(&appimage_dir)
        .expect("the appimage bundle dir must be readable")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "AppImage"))
        .collect();

    assert!(
        !entries.is_empty(),
        "the bundle dir exists but holds no .AppImage"
    );

    // The operator's signing public key. The path is read from the
    // environment so the build host's home directory is not baked into
    // the source tree — this file ships in a public repository.
    let pubkey = match std::env::var("VAULT_SIGNING_PUBKEY") {
        Ok(p) if !p.is_empty() => PathBuf::from(p),
        _ => {
            eprintln!(
                "SKIP: VAULT_SIGNING_PUBKEY is not set — the signature cannot \
                 be verified without it."
            );
            return;
        }
    };
    if !pubkey.exists() {
        eprintln!(
            "SKIP: no signing public key at {} — the signature cannot be \
             verified without it.",
            pubkey.display()
        );
        return;
    }

    for artefact in entries {
        let sig = artefact.with_extension("AppImage.sig");
        assert!(
            sig.exists(),
            "{} has no side-car signature; an unsigned artefact cannot be \
             published to the updater channel.",
            artefact.display()
        );

        let sig_text = decode_box(&sig);

        // The plugin reads the manifest's `signature` field as the CONTENT
        // of the .sig file (base64 minisign), not a URL to it.
        let parsed = minisign_verify::Signature::decode(&sig_text)
            .unwrap_or_else(|e| panic!("{} is not a valid minisign signature: {e}", sig.display()));

        let pub_text = decode_box(&pubkey);
        let key_line = pub_text
            .lines()
            .nth(1)
            .expect("a minisign public key box has two lines")
            .to_string();
        let key = minisign_verify::PublicKey::from_base64(&key_line)
            .unwrap_or_else(|e| panic!("the public key must be a valid minisign key: {e}"));

        // `verify` compares the key ids itself and returns
        // `UnexpectedKeyId` when they differ, so a signature made with a
        // different key is rejected here too.
        let bytes = std::fs::read(&artefact).expect("the artefact must be readable");
        key.verify(&bytes, &parsed, false).unwrap_or_else(|e| {
            panic!(
                "the signature does not verify against {}: {e}",
                artefact.display()
            )
        });
    }
}

/// The Rust `Cargo.toml` lists `minisign-verify` only transitively (the
/// updater plugin depends on it). This test needs it directly, so it is
/// declared as a dev-dependency.
#[test]
fn the_dev_dependency_is_declared() {
    let cargo = std::fs::read_to_string(repo_root().join("src-tauri/Cargo.toml"))
        .expect("Cargo.toml must be readable");
    assert!(
        cargo.contains("minisign-verify"),
        "Cargo.toml must declare minisign-verify as a dev-dependency so \
         this test can verify signatures with the same crate the updater \
         plugin uses."
    );
}
