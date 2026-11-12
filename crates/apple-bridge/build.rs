// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! On iPadOS: compile `swift/MailCraftModel.swift` (Apple's on-device model) into a static
//! library and link it with the Swift runtime of the SDK. Elsewhere nothing is built.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=swift/MailCraftModel.swift");
    println!("cargo:rerun-if-changed=build.rs");
    let target = std::env::var("TARGET").unwrap_or_default();
    if !target.contains("apple-ios") {
        return;
    }
    let sim = target.ends_with("-sim") || target.starts_with("x86_64");
    let (sdk, platform) = if sim { ("iphonesimulator", "iphonesimulator") } else { ("iphoneos", "iphoneos") };
    let arch = if target.starts_with("x86_64") { "x86_64" } else { "arm64" };
    let triple = if sim { format!("{arch}-apple-ios16.0-simulator") } else { format!("{arch}-apple-ios16.0") };
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default());
    let lib = out.join("libmailcraftmodel.a");
    let run = |args: &[&str]| -> Option<String> {
        let o = Command::new("xcrun").args(args).output().ok()?;
        o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let sdk_path = run(&["--sdk", sdk, "--show-sdk-path"]).unwrap_or_default();
    let status = Command::new("xcrun")
        .args(["--sdk", sdk, "swiftc", "-parse-as-library", "-emit-library", "-static", "-O", "-module-name", "MailCraftModel", "-target"])
        .arg(&triple)
        .arg("-sdk")
        .arg(&sdk_path)
        .arg("swift/MailCraftModel.swift")
        .arg("-o")
        .arg(&lib)
        .status();
    if !status.is_ok_and(|s| s.success()) {
        println!("cargo:warning=the Swift part of mailcraft-apple-bridge didn't build; Apple's model won't be there");
        println!("cargo:rustc-cfg=mailcraft_no_swift");
        return;
    }
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=mailcraftmodel");
    // The Swift runtime is part of iPadOS; its stubs are in the SDK, its back-deployment
    // libraries in the toolchain.
    println!("cargo:rustc-link-search=native={sdk_path}/usr/lib/swift");
    if let Some(swiftc) = run(&["--sdk", sdk, "--find", "swiftc"]) {
        let toolchain = PathBuf::from(swiftc).parent().and_then(|p| p.parent()).map(|p| p.join("lib").join("swift").join(platform));
        if let Some(dir) = toolchain {
            println!("cargo:rustc-link-search=native={}", dir.display());
        }
    }
    // The Swift runtime the system has (`@rpath/libswiftCore.dylib` and the rest).
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    // The compiler's runtime helpers the Swift part uses (`#available` needs
    // `__isPlatformVersionAtLeast`, large frames `__chkstk_darwin`), which a link by rustc
    // doesn't bring in by itself.
    if let Some(dir) = run(&["--sdk", sdk, "clang", "-print-resource-dir"]).map(|d| PathBuf::from(d).join("lib").join("darwin")) {
        let lib = if sim { "clang_rt.iossim" } else { "clang_rt.ios" };
        if dir.join(format!("lib{lib}.a")).exists() {
            println!("cargo:rustc-link-search=native={}", dir.display());
            // (Not bundled into the rlib, which rustc can't read it for: handed to the final link.)
            println!("cargo:rustc-link-lib=static:-bundle={lib}");
        } else {
            println!("cargo:warning=no lib{lib}.a in {}", dir.display());
        }
    }
    // (The app that links this, `apps/mailcraft-ios`, adds the Swift runtime's place and links
    // Foundation Models weakly in its own build.rs: link arguments don't pass on from a library.)
}
