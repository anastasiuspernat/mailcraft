// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! What `llama-cpp-sys-2` leaves out of the link on macOS:
//!
//! - Vulkan: the Vulkan loader, which it links only on Windows and Linux. Linked here from
//!   `$VULKAN_SDK/lib`, else Homebrew's.
//! - Metal: clang's runtime (`libclang_rt.osx.a`), for the `@available` checks in llama.cpp's
//!   Objective-C (`__isPlatformVersionAtLeast`), which `rustc` doesn't link by itself. Found
//!   through `xcrun clang -print-resource-dir`.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=VULKAN_SDK");
    let macos = std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "macos");
    if !macos {
        return;
    }
    if std::env::var_os("CARGO_FEATURE_VULKAN").is_some() {
        let dir = match std::env::var("VULKAN_SDK") {
            Ok(sdk) => format!("{sdk}/lib"),
            Err(_) if std::path::Path::new("/opt/homebrew/lib/libvulkan.dylib").exists() => "/opt/homebrew/lib".to_string(),
            Err(_) => "/usr/local/lib".to_string(),
        };
        println!("cargo:rustc-link-search=native={dir}");
        println!("cargo:rustc-link-lib=dylib=vulkan");
    }
    if std::env::var_os("CARGO_FEATURE_METAL").is_some() {
        let resource = Command::new("xcrun").args(["clang", "-print-resource-dir"]).output().ok().filter(|o| o.status.success());
        match resource.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()) {
            Some(dir) if !dir.is_empty() => {
                println!("cargo:rustc-link-search=native={dir}/lib/darwin");
                println!("cargo:rustc-link-lib=static=clang_rt.osx");
            }
            _ => println!("cargo:warning=clang's runtime wasn't found (xcrun clang -print-resource-dir): Metal may not link"),
        }
    }
}
