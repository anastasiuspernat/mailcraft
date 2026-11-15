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
    /// Signs in on the provider's own page (Microsoft for Outlook.com and Microsoft 365, Google
    /// for Gmail and Google Workspace) rather than with a password.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sign_in: Option<OAuthProvider>,
    /// The application id its sign-in was made for, when an iPad gave it (else this build's).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub client_id: String,
    /// Added here with `add` (every paired device is told of its mail), rather than by an iPad
    /// (only the devices that asked for it are, and it goes when none wants it any more).
    #[serde(default = "yes")]
    pub by_hand: bool,
}

fn yes() -> bool {
    true
}

impl Watched {
    /// The account as the network crate takes it, signing in with this build's application id
    /// for its provider.
    pub fn account(&self) -> Account {
        Account {
            email: self.email.clone(),
            username: self.username.clone(),
            incoming: Some(self.server.clone()),
            oauth: self.sign_in.map(|provider| OAuth {
                provider,
                client_id: if self.client_id.is_empty() { client_id(provider) } else { self.client_id.clone() },
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// The credential store's name for this account's password or sign-in.
    pub fn secret_key(&self) -> String {
        format!("{}:{}", if self.sign_in.is_some() { "signin" } else { "password" }, self.email.to_ascii_lowercase())
    }
}

/// A paired iPad.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    /// Its own id, which stays when Apple gives it a new push address (empty: paired by the
    /// first version, which can't sync accounts or read sealed notifications).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// Its push address, from Apple (hexadecimal).
    pub token: String,
    /// What it calls itself, for the owner.
    #[serde(default)]
    pub name: String,
    /// The accounts it wants notifications for (lower case), besides those added by hand.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accounts: Vec<String>,
    /// When it was last heard from (seconds since 1970).
    #[serde(default)]
    pub seen: u64,
}

impl Device {
    /// The credential store's names for its link key and its note key.
    pub fn link_key_name(id: &str) -> String {
        format!("link:{id}")
    }
    pub fn note_key_name(id: &str) -> String {
        format!("note:{id}")
    }

    /// Whether it wants notifications for `account`.
    pub fn wants(&self, account: &Watched) -> bool {
        account.by_hand || self.accounts.iter().any(|a| a.eq_ignore_ascii_case(&account.email))
    }
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
    /// The six digits an iPad pairs with (`pair`): made once, kept until the owner asks for a
    /// new one (`code --new`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pairing_code: String,
}

/// The settings file, read and changed by one thread at a time (the watchers, the pairing).
pub struct Settings {
    path: PathBuf,
    lock: std::sync::Mutex<()>,
}

impl Settings {
    pub fn new(path: PathBuf) -> Settings {
        Settings { path, lock: std::sync::Mutex::new(()) }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The settings as they are now.
    pub fn read(&self) -> Result<Config, String> {
        let _held = self.lock.lock().map_err(|_| "the settings are unusable".to_string())?;
        Config::load(&self.path)
    }

    /// Change the settings with `f`, read fresh from the file, and write them.
    pub fn update<T>(&self, f: impl FnOnce(&mut Config) -> T) -> Result<T, String> {
        let _held = self.lock.lock().map_err(|_| "the settings are unusable".to_string())?;
        let mut config = Config::load(&self.path)?;
        let out = f(&mut config);
        config.save(&self.path)?;
        Ok(out)
    }
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

    /// The accounts no device wants and nobody added by hand: they go, with their secrets.
    pub fn unwanted(&self) -> Vec<Watched> {
        self.accounts.iter().filter(|a| !a.by_hand && !self.devices.iter().any(|d| d.wants(a))).cloned().collect()
    }

    /// The account watched under `email`.
    pub fn account(&self, email: &str) -> Option<&Watched> {
        self.accounts.iter().find(|a| a.email.eq_ignore_ascii_case(email))
    }

    /// Add `device`, or take its new name and push address when it is paired already (the same
    /// id, or for a device of the first version the same push address). False when there are
    /// too many.
    pub fn pair(&mut self, device: Device) -> bool {
        let same = |d: &Device| if device.id.is_empty() { d.token.eq_ignore_ascii_case(&device.token) } else { d.id == device.id };
        // (A push address belongs to one device: one that had it before has a new one by now.)
        self.devices.retain(|d| same(d) || !d.token.eq_ignore_ascii_case(&device.token));
        if let Some(known) = self.devices.iter_mut().find(|d| same(d)) {
            known.name = device.name;
            known.token = device.token;
            known.seen = device.seen.max(known.seen);
            return true;
        }
        if self.devices.len() >= MAX_DEVICES {
            return false;
        }
        self.devices.push(device);
        true
    }
}

/// The application id this build signs in to `provider` with (an identifier, not a secret):
/// `MAILCRAFT_MS_CLIENT_ID` or `MAILCRAFT_GOOGLE_CLIENT_ID` when the relay starts, else the one it
/// was built with; empty when there is none.
pub fn client_id(provider: OAuthProvider) -> String {
    let (name, built) = match provider {
        OAuthProvider::Microsoft => ("MAILCRAFT_MS_CLIENT_ID", option_env!("MAILCRAFT_MS_CLIENT_ID")),
        OAuthProvider::Google => ("MAILCRAFT_GOOGLE_CLIENT_ID", option_env!("MAILCRAFT_GOOGLE_CLIENT_ID")),
    };
    std::env::var(name).ok().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| built.unwrap_or_default().to_string())
}

/// The pairing code in the settings at `path`: made and kept the first time, or anew when `renew`.
pub fn pairing_code(path: &Path, renew: bool) -> Result<String, String> {
    let mut config = Config::load(path)?;
    let good = config.pairing_code.len() == 6 && config.pairing_code.bytes().all(|b| b.is_ascii_digit());
    if renew || !good {
        config.pairing_code = crate::pair::new_code();
        config.save(path)?;
    }
    Ok(config.pairing_code)
}

/// The incoming server for `email`: the provider's when it is known, else `imap.<domain>`, and
/// the provider whose page it signs in on, if any (Google only when this build can sign in there).
pub fn server_for(email: &str) -> Option<(Server, Option<OAuthProvider>)> {
    let (_, domain) = email.rsplit_once('@')?;
    if domain.is_empty() || !email.contains('.') {
        return None;
    }
    let preset = mailcraft_model::provider::by_domain(domain).unwrap_or_else(|| mailcraft_model::provider::guess(domain));
    let google = mailcraft_model::is_gmail_host(&preset.incoming.host) && !client_id(OAuthProvider::Google).is_empty();
    let sign_in = if google { Some(OAuthProvider::Google) } else { preset.oauth.filter(|p| *p == OAuthProvider::Microsoft) };
    Some((preset.incoming, sign_in))
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
