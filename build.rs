// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Patrick Weppelmann

//! Embeds the icon and version info into the Windows executable.
fn main() {
    println!("cargo:rerun-if-changed=assets/icons/app.ico");
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
