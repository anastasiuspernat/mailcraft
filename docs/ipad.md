# MailCraft on iPad

> **Status:** early: runs in the iPad simulator and builds for iPad; TestFlight through GitHub · **Written:** 2026-11-11

MailCraft for iPad (`apps/mailcraft-ios`) is the same window as the desktop app, running on
iPadOS: the same engine, the same mailbox file and the same interface, drawn with Metal.

## What it does

- **Your mailbox stays on the iPad**, in the app's own storage, with the window's settings beside
  it. It is saved as you go: iPadOS may stop an app without warning, so nothing waits for a quit.
- **Accounts** as on the desktop: IMAP and SMTP with a password, kept in the iPad's keychain;
  Microsoft and Google sign in on their own page, shown in Apple's sign-in sheet over MailCraft.
- **Touch**: a swipe scrolls the lists, a tap selects, a long press opens a message's menu (Move
  to, Flag, Delete and the rest). Nothing reacts to a finger passing by.
- **Reminders and new mail** come as iPadOS notifications. Reminders still to come are handed to
  iPadOS in advance (up to 60, two weeks ahead), so they arrive on time with MailCraft closed.
  While MailCraft is in front, iPadOS keeps an app's own banners to itself; the status bar then
  says "🔔 n reminders due". The reminders window never opens by itself.
- **The smart Inbox** (Primary, Transactions, Updates, Promotions, Priority, summaries) runs on
  Apple's on-device model (Foundation Models), on an iPad with Apple Intelligence (iPadOS 26, an
  M1 or later, or the A17 Pro iPad mini) with Apple Intelligence turned on. Turn it on in File ›
  Options › Smart Inbox and summaries › "Sort the Inbox and summarise mail": there is no model
  file to choose, and the page says whether Apple's model is ready. Nothing leaves the iPad.
  Search by meaning needs an embedding model, which Apple's hasn't: it is off on iPad.
- **A trackpad or mouse** (a Magic Keyboard's): two-finger scrolling scrolls what the pointer is
  over, and the pointer shows what it is over, as on a Mac.
- **Interface size** starts at 110 % on a new iPad install (File › Options › General).

## Building

`packaging/ios/build.sh` builds `MailCraft.app`:

- `packaging/ios/build.sh --run`: for the iPad simulator of this Mac, installed and started.
- `packaging/ios/build.sh --device`: for a real iPad (arm64), unsigned.

Apple's on-device model is only in a build made with Xcode 26 or later; with an older Xcode the
app builds and runs without it. Rust targets: `x86_64-apple-ios` (simulator, Intel Mac),
`aarch64-apple-ios-sim` (simulator, Apple silicon), `aarch64-apple-ios` (iPad).

## TestFlight from GitHub

`.github/workflows/ios.yml` builds with Xcode 26 on GitHub's macOS runners and uploads to
TestFlight. Building and signing are separate jobs: the one that compiles holds no secret; the one
that signs checks the app against the build's digest and never compiles (`cargo xtask workflows`).

The signing job needs these repository secrets (Settings › Secrets and variables › Actions):

| Secret | What it is |
|---|---|
| `IOS_DIST_CERT_P12` | Your **Apple Distribution** certificate with its private key, exported as .p12, then base64 (`base64 -i cert.p12 \| pbcopy`) |
| `IOS_DIST_CERT_PASSWORD` | The password you gave that .p12 |
| `IOS_PROVISIONING_PROFILE` | An **App Store** provisioning profile for the app's identifier, base64 |
| `ASC_KEY_ID` | An App Store Connect API key's Key ID (Users and Access › Integrations › App Store Connect API) |
| `ASC_ISSUER_ID` | That page's Issuer ID |
| `ASC_KEY_P8` | The key's .p8 file, base64 |

The app's identifier is set in `packaging/ios/build.sh` (`MAILCRAFT_IOS_BUNDLE_ID`); register it
in your Apple Developer account and create the app in App Store Connect with it before the first
upload. Each run's number is the build number, so each upload is newer than the last. Without the
secrets the signing job says which are missing and stops without failing.

## The one exception to "no unsafe code"

`crates/apple-bridge` is the only crate allowed `unsafe` code: Apple's sign-in sheet and the Swift
functions that reach Apple's on-device model have no safe Rust bindings. Each `unsafe` block says
why it is sound (`clippy::undocumented_unsafe_blocks` is denied there); everything else in the
workspace still forbids unsafe code.
