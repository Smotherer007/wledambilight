// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Patrick Weppelmann

//! Embeds the icon and version info into the Windows executable.
fn main() {
    println!("cargo:rerun-if-changed=assets/icons/app.ico");

    // macOS: ScreenCaptureKit is partly written in Swift. Its build script sets
    // the rpath to the system Swift runtime, but Cargo does not pass link args
    // of dependencies on to our binary - without this the app (and the tests)
    // abort at launch with "Library not loaded: @rpath/libswift_Concurrency.dylib".
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    }
    #[cfg(windows)]
    {
        if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
            let mut res = winresource::WindowsResource::new();
            res.set_icon("assets/icons/app.ico")
                .set("ProductName", "WLED Ambilight")
                .set("FileDescription", "WLED Ambilight")
                .set("CompanyName", "Patrick Weppelmann")
                .set(
                    "LegalCopyright",
                    "Copyright (C) 2026 Patrick Weppelmann, GPL-3.0",
                );
            if let Err(e) = res.compile() {
                println!("cargo:warning=could not embed the Windows icon: {e}");
            }
        }
    }
}
