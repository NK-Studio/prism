//! Windows only: embed the app icon and version info (VERSIONINFO) into `prism.exe`.
//!
//! On every other target this does nothing. A missing resource compiler is a warning, so a
//! cross-compile from macOS still links, unless `PRISM_REQUIRE_WINRES=1` (set by the
//! release workflow) turns it into an error.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/app-icon/prism.ico");
    println!("cargo:rerun-if-env-changed=PRISM_REQUIRE_WINRES");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/app-icon/prism.ico")
        .set("ProductName", "Prism")
        .set("FileDescription", "Prism image editor")
        .set("CompanyName", "NK Studio")
        .set("LegalCopyright", "Copyright (c) 2026 NK Studio.")
        .set("OriginalFilename", "prism.exe")
        .set("InternalName", "prism");
    if let Err(e) = res.compile() {
        if std::env::var_os("PRISM_REQUIRE_WINRES").is_some() {
            panic!("embedding Windows resources failed: {e}");
        }
        println!("cargo:warning=prism.exe built without icon/version resources: {e}");
    }
}
