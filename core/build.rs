// If `scripts/dev-env.sh` has unpacked the ALSA dev package into `.sysroot`
// (machines where the user cannot install packages), point the linker at it.
// On a normal Linux box with alsa-lib-devel installed, or on Windows, this
// does nothing at all.
//
// The same script also bakes the optional AcoustID application key into the
// binary. Release builds receive `ACCOUSTID_APIKEY` from the GitHub
// `production` environment; local builds fall back to an uncommitted `.env`
// file at the workspace root. The key is a public application identifier, not
// a user credential, but it still must never be printed or committed.

use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    link_sysroot_alsa();
    bake_acoustid_key();
}

fn link_sysroot_alsa() {
    let Some(repo) = Path::new(env!("CARGO_MANIFEST_DIR")).parent() else {
        return;
    };

    let candidates = [
        repo.join(".sysroot/usr/lib/x86_64-linux-gnu"),
        repo.join(".sysroot/usr/lib64"),
        repo.join(".sysroot/usr/lib/aarch64-linux-gnu"),
    ];

    for dir in candidates {
        let lib = dir.join("libasound.so");
        if lib.exists() {
            println!("cargo:rustc-link-search=native={}", dir.display());
            return;
        }
    }
}

fn bake_acoustid_key() {
    println!("cargo:rerun-if-env-changed=ACCOUSTID_APIKEY");
    let env_file = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default())
        .parent()
        .map(|root| root.join(".env"))
        .unwrap_or_else(|| PathBuf::from(".env"));
    println!("cargo:rerun-if-changed={}", env_file.display());

    let key = std::env::var("ACCOUSTID_APIKEY")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| read_env(&env_file, "ACCOUSTID_APIKEY"))
        .unwrap_or_default();
    println!("cargo:rustc-env=MUSICWASP_ACOUSTID_KEY={key}");
}

fn read_env(path: &Path, name: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != name {
            continue;
        }
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .or_else(|| {
                value
                    .strip_prefix('\'')
                    .and_then(|value| value.strip_suffix('\''))
            })
            .unwrap_or(value);
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}
