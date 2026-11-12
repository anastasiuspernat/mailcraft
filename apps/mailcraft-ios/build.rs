// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Link arguments of the iPad app that a library's build script can't give it: where the system
//! keeps the Swift runtime that `mailcraft-apple-bridge`'s Swift part uses, and Foundation Models
//! linked weakly when the SDK has it (Xcode 26 and later), so that an iPad without it (before
//! iPadOS 26) still starts.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let target = std::env::var("TARGET").unwrap_or_default();
    if !target.contains("apple-ios") {
        return;
    }
    println!("cargo:rustc-link-arg-bins=-Wl,-rpath,/usr/lib/swift");
    let sim = target.ends_with("-sim") || target.starts_with("x86_64");
    let sdk = if sim { "iphonesimulator" } else { "iphoneos" };
    let sdk_path = Command::new("xcrun")
        .args(["--sdk", sdk, "--show-sdk-path"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    if std::path::Path::new(&sdk_path).join("System/Library/Frameworks/FoundationModels.framework").exists() {
        println!("cargo:rustc-link-arg-bins=-Wl,-weak_framework,FoundationModels");
    }
}
