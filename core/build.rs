// If `scripts/dev-env.sh` has unpacked the ALSA dev package into `.sysroot`
// (machines where the user cannot install packages), point the linker at it.
// On a normal Linux box with alse-lib-devel installed, or on Windows, this
// does nothing at all.

use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

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
