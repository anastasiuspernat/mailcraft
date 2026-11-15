// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! What the owner types: `mailcraft-relay <command>`.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;

use mailcraft_model::OAuthProvider;

use crate::config::{self, AppleKey, Config, Watched};
use crate::{logfile, pair, store, watcher};

pub const HELP: &str = "\
MailCraft relay: tells MailCraft on your iPad when new mail arrives.

  mailcraft-relay add you@example.com     watch an account: opens Google's or Microsoft's sign-in
                                          page for Gmail, Outlook and Microsoft 365, and asks for
                                          the password of any other account
        [--google] (a Google Workspace address at a domain of its own)
        [--server imap.example.com[:993]] [--user name] [--starttls]
  mailcraft-relay remove you@example.com  stop watching it and forget its password
  mailcraft-relay apple-key AuthKey_XXXXXXXXXX.p8 --key-id XXXXXXXXXX --team-id XXXXXXXXXX
                                          the push key from your Apple developer account
  mailcraft-relay code [--new]            this computer's address and the pairing code to enter on
                                          the iPad (--new: make a new code)
  mailcraft-relay pair                    pair an iPad, and say when it has
  mailcraft-relay devices                 the paired iPads and the accounts each wants
  mailcraft-relay unpair <n>              forget the iPad numbered n in `devices`
  mailcraft-relay test                    send a test notification to the paired iPads
  mailcraft-relay run                     watch and notify, until stopped
  mailcraft-relay install                 start with Windows, without a window
  mailcraft-relay uninstall               stop starting with Windows
  mailcraft-relay status                  what is set up, and the end of the log

An iPad paired with the relay brings its own accounts (File › Options › Push from your computer):
they need no `add` here, and go when the iPad no longer has them.
Run `add` and `apple-key` at the computer itself (or over Remote Desktop), not over SSH.
Passwords and sign-ins are kept in this computer's credential store only.";

/// The name of the scheduled task that starts the relay with Windows.
const TASK: &str = "MailCraft Relay";

fn folder() -> Result<PathBuf, String> {
    config::folder().ok_or_else(|| "this computer has no folder for the relay's settings".to_string())
}

/// The value after `flag` in `args`.
fn option<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).map(String::as_str)
}

/// Open `url` in the system's browser, as a program with the address as its one argument (never
/// through a shell).
fn open_browser(url: &str) -> bool {
    let started = if cfg!(target_os = "windows") {
        Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn()
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg(url).spawn()
    } else {
        Command::new("xdg-open").arg(url).spawn()
    };
    started.is_ok()
}

/// Run the command in `args` (the program's name left out).
pub fn run(args: &[String]) -> Result<(), String> {
    let Some(command) = args.first() else {
        println!("{HELP}");
        return Ok(());
    };
    let rest = args.get(1..).unwrap_or_default();
    let folder = folder()?;
    match command.as_str() {
        "add" => add(&folder, rest),
        "remove" => remove(&folder, rest),
        "apple-key" => apple_key(&folder, rest),
        "pair" => pair_ipad(&folder),
        "devices" => devices(&folder),
        "unpair" => unpair(&folder, rest),
        "code" => {
            show_pairing(&folder, rest.iter().any(|a| a == "--new"))?;
            Ok(())
        }
        "test" => test(&folder),
        "run" => {
            logfile::start(&folder, true);
            watcher::run(&folder)
        }
        "install" => install(),
        "uninstall" => uninstall(),
        "status" => status(&folder),
        "help" | "-h" | "--help" => {
            println!("{HELP}");
            Ok(())
        }
        other => Err(format!("there is no command \"{other}\"\n\n{HELP}")),
    }
}

fn add(folder: &Path, args: &[String]) -> Result<(), String> {
    let email =
        args.first().map(|s| s.trim().to_string()).filter(|s| s.contains('@')).ok_or("which address? `mailcraft-relay add you@example.com`")?;
    let path = config::file_in(folder);
    let mut config = Config::load(&path)?;
    if config.accounts.len() >= config::MAX_ACCOUNTS && config.account(&email).is_none() {
        return Err(format!("the relay watches {} accounts at most", config::MAX_ACCOUNTS));
    }
    let starttls = args.iter().any(|a| a == "--starttls");
    let (server, sign_in) = match option(args, "--server") {
        Some(typed) => (config::typed_server(typed, starttls).ok_or("that server isn't a host name, or host:port")?, None),
        // A Google Workspace address at a domain of its own signs in on Google's page too.
        None if args.iter().any(|a| a == "--google") => (mailcraft_model::provider::google(false).incoming, Some(OAuthProvider::Google)),
        None => config::server_for(&email).ok_or("that isn't an e-mail address")?,
    };
    let watched = Watched {
        email: email.clone(),
        username: option(args, "--user").unwrap_or_default().to_string(),
        server,
        sign_in,
        client_id: String::new(),
        by_hand: true,
    };
    let account = watched.account();
    let secret = if let Some(oauth) = &account.oauth {
        let whose = if oauth.provider == OAuthProvider::Google { "Google" } else { "Microsoft" };
        if oauth.client_id.is_empty() {
            return Err(format!("this build has no {whose} application id to sign in with"));
        }
        println!("Opening {whose}'s sign-in page in your browser. Sign in as {email} there.");
        let tokens = mailcraft_net::oauth::sign_in(
            oauth,
            &email,
            &|url| {
                if !open_browser(url) {
                    println!("Open this page in a browser:\n{url}");
                }
            },
            &AtomicBool::new(false),
        )
        .map_err(|e| {
            let e = e.to_string();
            if e.contains("did not grant Gmail") {
                // Seen in a Google Cloud project set up minutes before: run it again.
                "Google signed you in but left out Gmail access, which happens for a while after the app is set up in Google Cloud: run the same `add` again (and if Google shows a checkbox for Gmail, tick it)".to_string()
            } else {
                format!("the sign-in didn't finish: {e}")
            }
        })?;
        if tokens.refresh.is_empty() {
            return Err(format!("{whose} gave no lasting sign-in"));
        }
        print!("Checking the Inbox… ");
        let _ = std::io::stdout().flush();
        mailcraft_net::watch::open(&account, &tokens.access).map_err(|e| format!("it can't be opened: {e}"))?.close();
        tokens.refresh
    } else {
        if mailcraft_model::is_gmail_host(&watched.server.host) {
            return Err("this build can't sign in on Google's page (it was built without a Google application id)".into());
        }
        println!("Server: {}:{}", watched.server.host, watched.server.port);
        let password = rpassword::prompt_password(format!("Password for {email} (not shown): ")).map_err(|e| e.to_string())?;
        if password.is_empty() {
            return Err("no password given".into());
        }
        print!("Checking the Inbox… ");
        let _ = std::io::stdout().flush();
        mailcraft_net::watch::open(&account, &password).map_err(|e| format!("it can't be opened: {e}"))?.close();
        password
    };
    println!("it opens.");
    store::set(&watched.secret_key(), &secret)?;
    config.accounts.retain(|a| !a.email.eq_ignore_ascii_case(&email));
    config.accounts.push(watched);
    config.save(&path)?;
    println!("{email} is watched.");
    restart_if_installed();
    println!();
    show_pairing(folder, false)?;
    Ok(())
}

fn remove(folder: &Path, args: &[String]) -> Result<(), String> {
    let email = args.first().ok_or("which address? `mailcraft-relay remove you@example.com`")?;
    let path = config::file_in(folder);
    let mut config = Config::load(&path)?;
    let Some(watched) = config.account(email).cloned() else { return Err(format!("{email} isn't watched")) };
    store::delete(&watched.secret_key());
    config.accounts.retain(|a| !a.email.eq_ignore_ascii_case(email));
    config.seen.remove(&email.to_ascii_lowercase());
    config.save(&path)?;
    println!("{email} is no longer watched, and its password or sign-in is forgotten.");
    restart_if_installed();
    Ok(())
}

fn apple_key(folder: &Path, args: &[String]) -> Result<(), String> {
    let file = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .ok_or("which file? `mailcraft-relay apple-key AuthKey_XXXXXXXXXX.p8 --key-id … --team-id …`")?;
    let key_id = option(args, "--key-id").ok_or("the key id is missing: --key-id")?;
    let team_id = option(args, "--team-id").ok_or("the team id is missing: --team-id")?;
    let text = std::fs::read_to_string(file).map_err(|e| format!("{file} couldn't be read ({e})"))?;
    mailcraft_net::apns::Key::from_p8(&text, key_id, team_id)?;
    store::set(store::APPLE_KEY, &text)?;
    let path = config::file_in(folder);
    let mut config = Config::load(&path)?;
    config.apple = Some(AppleKey { key_id: key_id.trim().to_ascii_uppercase(), team_id: team_id.trim().to_ascii_uppercase() });
    config.save(&path)?;
    println!("The push key is kept in this computer's credential store. You may delete {file} now (keep a copy somewhere safe).");
    restart_if_installed();
    Ok(())
}

/// Whether a relay is running on this computer.
fn running() -> bool {
    std::net::TcpStream::connect(("127.0.0.1", crate::ONE_ONLY_PORT)).is_ok()
}

/// Print this computer's address and the pairing code (a new one when `renew`); the code.
fn show_pairing(folder: &Path, renew: bool) -> Result<String, String> {
    let code = config::pairing_code(&config::file_in(folder), renew)?;
    let address = pair::home_address().map(|ip| ip.to_string()).unwrap_or_else(|| "this computer's address".into());
    println!("To pair an iPad (on the same home network as this computer), open MailCraft on it:");
    println!("File › Options › Push from your computer, and enter");
    println!();
    println!("    Computer:  {address}");
    println!("    Code:      {code}");
    println!();
    if !running() {
        println!("(The relay listens for it while it runs: `mailcraft-relay run`, or `install`, or `pair` for ten minutes.)");
    }
    Ok(code)
}

fn pair_ipad(folder: &Path) -> Result<(), String> {
    let path = config::file_in(folder);
    let before = Config::load(&path)?.devices.iter().map(|d| (d.id.clone(), d.token.clone())).collect::<Vec<_>>();
    if running() {
        show_pairing(folder, false)?;
        println!("Waiting up to ten minutes for an iPad to pair…");
        let started = std::time::Instant::now();
        while started.elapsed() < pair::WAIT {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let config = Config::load(&path)?;
            if let Some(device) = config.devices.iter().find(|d| !before.contains(&(d.id.clone(), d.token.clone()))) {
                println!("Paired with {}.", device.name);
                return Ok(());
            }
        }
        return Err("no iPad paired within ten minutes (the relay goes on listening)".into());
    }
    let listener =
        std::net::TcpListener::bind(("0.0.0.0", crate::PAIR_PORT)).map_err(|e| format!("port {} can't be listened on ({e})", crate::PAIR_PORT))?;
    show_pairing(folder, false)?;
    println!("Waiting up to ten minutes… (Windows may ask whether to let the relay use the network: allow private networks.)");
    let name = pair::serve(&listener, &config::Settings::new(path))?;
    println!("Paired with {name}.");
    Ok(())
}

fn devices(folder: &Path) -> Result<(), String> {
    let config = Config::load(&config::file_in(folder))?;
    if config.devices.is_empty() {
        println!("No iPad is paired.");
    }
    for (n, d) in config.devices.iter().enumerate() {
        let version = if d.id.is_empty() { " (paired by an older MailCraft: pair it again)" } else { "" };
        println!("{}. {}{version}", n + 1, d.name);
        let mut accounts: Vec<String> = config.accounts.iter().filter(|a| d.wants(a)).map(|a| a.email.clone()).collect();
        accounts.sort();
        println!("   accounts: {}", if accounts.is_empty() { "none".to_string() } else { accounts.join(", ") });
    }
    Ok(())
}

fn unpair(folder: &Path, args: &[String]) -> Result<(), String> {
    let n: usize = args.first().and_then(|a| a.parse().ok()).ok_or("which one? `mailcraft-relay unpair 1` (numbers from `devices`)")?;
    let path = config::file_in(folder);
    let mut config = Config::load(&path)?;
    let index = n.checked_sub(1).filter(|i| *i < config.devices.len()).ok_or("there is no iPad with that number (see `devices`)")?;
    let device = config.devices.remove(index);
    let dropped = config.unwanted();
    config.accounts.retain(|a| !dropped.iter().any(|d| d.email.eq_ignore_ascii_case(&a.email)));
    for d in &dropped {
        config.seen.remove(&d.email.to_ascii_lowercase());
    }
    config.save(&path)?;
    if !device.id.is_empty() {
        store::delete(&config::Device::link_key_name(&device.id));
        store::delete(&config::Device::note_key_name(&device.id));
    }
    for d in &dropped {
        store::delete(&d.secret_key());
    }
    println!(
        "{} is forgotten{}.",
        device.name,
        if dropped.is_empty() { String::new() } else { format!(", and so are the accounts only it wanted ({})", dropped.len()) }
    );
    restart_if_installed();
    Ok(())
}

fn test(folder: &Path) -> Result<(), String> {
    let shared = watcher::Shared::open(config::file_in(folder))?;
    let test = Watched { email: "MailCraft relay".into(), by_hand: true, ..Default::default() };
    let news = watcher::News { count: 1, from: "MailCraft relay".into(), subject: "A test notification".into() };
    if !shared.notify(&test, &news) {
        return Err("no paired iPad took it (see `mailcraft-relay status`)".into());
    }
    println!("Sent. It should show on the iPad in a few seconds.");
    Ok(())
}

fn status(folder: &Path) -> Result<(), String> {
    let config = Config::load(&config::file_in(folder))?;
    println!("Settings: {}", config::file_in(folder).display());
    println!("Push key: {}", config.apple.as_ref().map_or("none yet".to_string(), |k| format!("{} (team {})", k.key_id, k.team_id)));
    println!("Accounts:");
    for a in &config.accounts {
        let kept = store::get(&a.secret_key()).is_some();
        let note = if kept {
            ""
        } else if std::env::var_os("SSH_CONNECTION").is_some() {
            "  — its sign-in can't be read over SSH (the running relay can)"
        } else {
            "  — nothing to sign in with: `add` it again"
        };
        println!("  {} ({}:{}){note}", a.email, a.server.host, a.server.port);
    }
    println!(
        "Paired iPads: {}",
        if config.devices.is_empty() { "none".to_string() } else { config.devices.iter().map(|d| d.name.clone()).collect::<Vec<_>>().join(", ") }
    );
    let running = std::net::TcpStream::connect(("127.0.0.1", crate::ONE_ONLY_PORT)).is_ok();
    println!("Running: {}", if running { "yes" } else { "no" });
    println!("Log ({}):", folder.join("relay.log").display());
    for line in logfile::tail(folder, 12) {
        println!("  {line}");
    }
    Ok(())
}

fn install() -> Result<(), String> {
    if !cfg!(target_os = "windows") {
        return Err("`install` is for Windows. Elsewhere, have `mailcraft-relay run` started at login (launchd on a Mac, systemd on Linux).".into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let service = exe.with_file_name("mailcraft-relay-service.exe");
    if !service.exists() {
        return Err(format!("{} is missing: keep it beside mailcraft-relay.exe", service.display()));
    }
    let made = Command::new("schtasks")
        // `/IT`: it runs in the owner's own session once they have signed in, the one where the
        // credential store holds their passwords (and no password is asked for the task itself).
        .args(["/Create", "/TN", TASK, "/SC", "ONLOGON", "/RL", "LIMITED", "/IT", "/F", "/TR"])
        .arg(format!("\"{}\"", service.display()))
        .status()
        .map_err(|e| e.to_string())?;
    if !made.success() {
        return Err("Windows didn't make the task that starts the relay".into());
    }
    let _ = Command::new("taskkill").args(["/IM", "mailcraft-relay-service.exe", "/F"]).output();
    let _ = Command::new("schtasks").args(["/Run", "/TN", TASK]).status();
    println!("The relay starts whenever you sign in to Windows, and is running now (`mailcraft-relay status`).");
    Ok(())
}

/// A relay that starts with Windows is started again, to take in changed settings.
fn restart_if_installed() {
    if !cfg!(target_os = "windows") {
        println!("Restart `mailcraft-relay run` for the change to count.");
        return;
    }
    let installed = Command::new("schtasks").args(["/Query", "/TN", TASK]).output().is_ok_and(|o| o.status.success());
    if installed {
        let _ = Command::new("taskkill").args(["/IM", "mailcraft-relay-service.exe", "/F"]).output();
        let _ = Command::new("schtasks").args(["/Run", "/TN", TASK]).output();
        println!("The relay was restarted with the change.");
    }
}

fn uninstall() -> Result<(), String> {
    if !cfg!(target_os = "windows") {
        return Err("`uninstall` is for Windows".into());
    }
    let _ = Command::new("schtasks").args(["/Delete", "/TN", TASK, "/F"]).status();
    let _ = Command::new("taskkill").args(["/IM", "mailcraft-relay-service.exe", "/F"]).output();
    println!("The relay no longer starts with Windows, and is stopped. Its settings and passwords are kept: `remove` an account to forget it.");
    Ok(())
}
