//! Embed the Windows executable icon so Explorer, the taskbar and shortcuts
//! show the MusicWasp artwork even before the app runs.

fn main() {
    println!("cargo:rerun-if-changed=../assets/musicwasp.ico");

    // `cfg(target_os)` in a build script describes the host, so check the
    // compiler's target environment instead (also works when cross-compiling).
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("../assets/musicwasp.ico");
    resource.set("ProductName", "MusicWasp");
    resource.set("FileDescription", "MusicWasp music player");
    resource.set("LegalCopyright", "Copyright (C) 2026 giwi.fr");
    if let Err(e) = resource.compile() {
        panic!("failed to compile Windows resources: {e}");
    }
}
