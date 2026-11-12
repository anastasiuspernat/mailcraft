// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Apple's on-device model (the Foundation Models framework, iPadOS 26 with Apple Intelligence),
// for MailCraft's smart Inbox on iPad. Two C functions, called from Rust through
// `crates/apple-bridge/src/model.rs`; built by that crate's build.rs. Where the SDK has no
// Foundation Models (Xcode before 26) both are stubs that say the model isn't there.
//
// The mail is untrusted: the instructions (the same system prompt as on the desktop) say so, and
// whatever comes back is read as JSON and checked by the Rust side before anything uses it.

import Foundation
#if canImport(FoundationModels)
import FoundationModels
#endif

/// Whether the model can answer now: 1 yes, 0 not on this device (or not turned on, or not
/// ready yet), -1 not in this build.
@_cdecl("mailcraft_model_available")
public func mailcraftModelAvailable() -> Int32 {
    #if canImport(FoundationModels)
    if #available(iOS 26.0, *) {
        switch SystemLanguageModel.default.availability {
        case .available:
            return 1
        default:
            return 0
        }
    }
    return 0
    #else
    return -1
    #endif
}

/// What the Rust side is called back with: its context, the text (UTF-8, valid only during the
/// call), and 0 for an answer or 1 for an error's description.
public typealias MailCraftModelDone = @convention(c) (UnsafeMutableRawPointer?, UnsafePointer<CChar>?, Int32) -> Void

/// Ask the model `prompt` under `instructions`; `done` is called exactly once, on some thread.
@_cdecl("mailcraft_model_respond")
public func mailcraftModelRespond(
    _ instructions: UnsafePointer<CChar>,
    _ prompt: UnsafePointer<CChar>,
    _ context: UnsafeMutableRawPointer?,
    _ done: MailCraftModelDone
) {
    let said = String(cString: instructions)
    let asked = String(cString: prompt)
    #if canImport(FoundationModels)
    if #available(iOS 26.0, *) {
        Task.detached {
            do {
                let session = LanguageModelSession(instructions: said)
                let answer = try await session.respond(to: asked)
                answer.content.withCString { done(context, $0, 0) }
            } catch {
                "\(error)".withCString { done(context, $0, 1) }
            }
        }
        return
    }
    #endif
    _ = said
    _ = asked
    "Apple's on-device model isn't in this build or on this device".withCString { done(context, $0, 1) }
}
