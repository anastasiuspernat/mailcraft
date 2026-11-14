// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The relay's settings and what it has seen, in `relay.json` in its own folder (`%APPDATA%\MailCraft
//! Relay` on Windows, `~/Library/Application Support/MailCraft Relay` on a Mac, `~/.config/mailcraft-relay`
//! elsewhere), written whole to a file beside it and moved into place. No secret is ever in it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use mailcraft_model::{Account, OAuth, OAuthProvider, Security, Server};
use serde::{Deserialize, Serialize};

/// The most accounts watched.
pub const MAX_ACCOUNTS: usize = 20;
/// The most iPads paired.
pub const MAX_DEVICES: usize = 10;
/// The largest settings file read.
const MAX_FILE: u64 = 1 << 20;

/// An account the relay watches.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Watched {
    pub email: String,
    /// The user name the server knows, when it isn't the address.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub username: String,
    pub server: Server,
    /// Signs in on Microsoft's page (an Outlook.com or Microsoft 365 account) rather than with a password.
    #[serde(default)]
    pub microsoft: bool,
}

impl Watched {
    /// The account as the network crate takes it.
    pub fn account(&self, client_id: &str) -> Account {
        Account {
            email: self.email.clone(),
            username: self.username.clone(),
            incoming: Some(self.server.clone()),
            oauth: self.microsoft.then(|| OAuth { provider: OAuthProvider::Microsoft, client_id: client_id.to_string(), ..Default::default() }),
            ..Default::default()
        }
    }

    /// The credential store's name for this account's password or sign-in.
    pub fn secret_key(&self) -> String {
        format!("{}:{}", if self.microsoft { "signin" } else { "password" }, self.email.to_ascii_lowercase())
    }
}

/// A paired iPad.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    /// Its push address, from Apple (hexadecimal).
    pub token: String,
    /// What it calls itself, for the owner.
    #[serde(default)]
    pub name: String,
}

/// The push key's ids (the key itself is in the credential store).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppleKey {
    pub key_id: String,
    pub team_id: String,
}

/// Where an account's Inbox was looked up to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Seen {
    pub uid_validity: u32,
    pub next: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default)]
    pub accounts: Vec<Watched>,
    #[serde(default)]
    pub devices: Vec<Device>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apple: Option<AppleKey>,
    /// By account address (lower case).
    #[serde(default)]
    pub seen: BTreeMap<String, Seen>,
}

/// The relay's folder.
pub fn folder() -> Option<PathBuf> {
    let home = |var: &str| std::env::var_os(var).map(PathBuf::from);
    if cfg!(target_os = "windows") {
        return home("APPDATA").map(|p| p.join("MailCraft Relay"));
    }
    if cfg!(target_os = "macos") {
        return home("HOME").map(|p| p.join("Library").join("Application Support").join("MailCraft Relay"));
    }
    home("XDG_CONFIG_HOME").or_else(|| home("HOME").map(|p| p.join(".config"))).map(|p| p.join("mailcraft-relay"))
}

/// The settings file in `folder`.
pub fn file_in(folder: &Path) -> PathBuf {
    folder.join("relay.json")
}

impl Config {
    /// Read the settings from `path`; none yet is an empty set. A file that can't be read is an
    /// error and is never written over.
    pub fn load(path: &Path) -> Result<Config, String> {
        let size = match std::fs::metadata(path) {
            Ok(m) => m.len(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
            Err(e) => return Err(format!("{} couldn't be read ({e})", path.display())),
        };
        if size > MAX_FILE {
            return Err(format!("{} is too large to be the relay's settings", path.display()));
        }
        let text = std::fs::read_to_string(path).map_err(|e| format!("{} couldn't be read ({e})", path.display()))?;
        let mut config: Config = serde_json::from_str(&text).map_err(|e| format!("{} isn't the relay's settings ({e})", path.display()))?;
        config.accounts.truncate(MAX_ACCOUNTS);
        config.devices.truncate(MAX_DEVICES);
        config.devices.retain(|d| mailcraft_net::apns::is_device_token(&d.token));
        Ok(config)
    }

    /// Write the settings to `path`, whole.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{} couldn't be made ({e})", dir.display()))?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let fresh = path.with_extension("json.new");
        std::fs::write(&fresh, text).map_err(|e| format!("{} couldn't be written ({e})", fresh.display()))?;
        std::fs::rename(&fresh, path).map_err(|e| format!("{} couldn't be written ({e})", path.display()))
    }

    /// The account watched under `email`.
    pub fn account(&self, email: &str) -> Option<&Watched> {
        self.accounts.iter().find(|a| a.email.eq_ignore_ascii_case(email))
    }

    /// Add `device`, or rename it when it is paired already. False when there are too many.
    pub fn pair(&mut self, device: Device) -> bool {
        if let Some(known) = self.devices.iter_mut().find(|d| d.token.eq_ignore_ascii_case(&device.token)) {
            known.name = device.name;
            return true;
        }
        if self.devices.len() >= MAX_DEVICES {
            return false;
        }
        self.devices.push(device);
        true
    }
}

/// The incoming server for `email`: the provider's when it is known, else `imap.<domain>`, and
/// whether it signs in on Microsoft's page.
pub fn server_for(email: &str) -> Option<(Server, bool)> {
    let (_, domain) = email.rsplit_once('@')?;
    if domain.is_empty() || !email.contains('.') {
        return None;
    }
    let preset = mailcraft_model::provider::by_domain(domain).unwrap_or_else(|| mailcraft_model::provider::guess(domain));
    let microsoft = preset.oauth == Some(OAuthProvider::Microsoft);
    Some((preset.incoming, microsoft))
}

/// A server the owner typed: `host` or `host:port`, over TLS unless `starttls`.
pub fn typed_server(text: &str, starttls: bool) -> Option<Server> {
    let text = text.trim();
    let (host, port) = match text.rsplit_once(':') {
        Some((host, port)) => (host, port.parse().ok()?),
        None => (text, if starttls { 143 } else { 993 }),
    };
    let ok = !host.is_empty() && host.len() <= 253 && host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    ok.then(|| Server { host: host.to_ascii_lowercase(), port, security: if starttls { Security::StartTls } else { Security::Tls } })
}
