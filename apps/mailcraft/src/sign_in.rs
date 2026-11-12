// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Signing in on the provider's page (Microsoft, Google), whichever way the page is shown: the
//! system's browser on the desktop (`main.rs`), Apple's sign-in sheet on iPad (`apps/mailcraft-ios`,
//! which shares this file). The wait and what comes back are `mailcraft_net::oauth`'s, on a
//! loopback address of this device, as on the desktop.

use mailcraft_model::sync::Tokens;
use mailcraft_ui_egui::SignInRequest;

/// [`sign_in`], with `open_page` in the browser's place: it opens the address it is given, and
/// says whether it could.
///
/// A sign-in on the provider's page that ends without tokens is answered in the words the
/// window knows each way by (`mailcraft_ui_egui::approval`): an administrator has to approve
/// first, the page came back with a no, nothing came back, no browser could be started.
/// Anything else is said as it is.
pub(crate) fn sign_in_with(request: SignInRequest, open_page: &dyn Fn(&str) -> bool) -> Result<Tokens, String> {
    use std::sync::atomic::{AtomicBool, Ordering};

    use mailcraft_net::oauth::{Endpoints, Resource, SignInEnd};
    use mailcraft_ui_egui::approval::{
        GOOGLE_REAUTH_REQUIRED, GOOGLE_SIGN_IN_BLOCKED, GOOGLE_SIGN_IN_DECLINED, GOOGLE_SIGN_IN_TIMED_OUT, NEEDS_APPROVAL, NO_BROWSER,
        SIGN_IN_DECLINED, SIGN_IN_TIMED_OUT,
    };
    // No browser could be started: nothing can come back, so the wait is called off at once,
    // and that is what is said.
    let no_browser = AtomicBool::new(false);
    let open = |url: &str| {
        if !open_page(url) {
            no_browser.store(true, Ordering::Relaxed);
            request.cancel.store(true, Ordering::Relaxed);
        }
    };
    let unopened = |said: String| if no_browser.load(Ordering::Relaxed) { NO_BROWSER.to_string() } else { said };
    // "Show Outlook's Favorites": the owner asked, so the page comes up, for what Microsoft Graph
    // was asked for before and the two permissions that reading the Favorites takes. A no that
    // is not the owner's to change is said in the words the window knows it by; nothing is
    // asked a second time.
    if request.graph && request.favorites {
        let asked = mailcraft_net::oauth::sign_in_for_favorites(&request.oauth, &request.email, &open, &request.cancel);
        return asked.map_err(|refusal| unopened(favorites_refusal(refusal)));
    }
    // A shared mailbox is to be read (#70): the page comes up for what reading shared mailboxes
    // takes, by itself, in the same words for a no as the Favorites'.
    // A shared mailbox's calendar was ticked for the first time, and the owner said go ahead
    // (#70): the page comes up for what reading shared mailboxes' calendars takes, by itself.
    if request.graph && request.shared && request.shared_calendars {
        let asked = mailcraft_net::oauth::sign_in_for_shared_calendars(&request.oauth, &request.email, &open, &request.cancel);
        return asked.map_err(|refusal| unopened(favorites_refusal(refusal)));
    }
    // Mail is to be sent from a shared mailbox for the first time, and the owner said go ahead
    // (#70): the page comes up for what sending from shared mailboxes takes too, by itself.
    if request.graph && request.shared && request.shared_send {
        let asked = mailcraft_net::oauth::sign_in_for_shared_send(&request.oauth, &request.email, &open, &request.cancel);
        return asked.map_err(|refusal| unopened(favorites_refusal(refusal)));
    }
    // A tasks run found the sign-in doesn't cover the To Do lists (#248) and the owner said go
    // ahead: the page comes up for that permission alone, by itself.
    if request.graph && request.tasks {
        let asked = mailcraft_net::oauth::sign_in_for_tasks(&request.oauth, &request.email, &open, &request.cancel);
        return asked.map_err(|refusal| unopened(favorites_refusal(refusal)));
    }
    // A rule is to go to the server, the sign-in doesn't cover changing the rules (#566), and the
    // owner said go ahead: the page comes up for that permission alone, by itself.
    if request.graph && request.rules {
        let asked = mailcraft_net::oauth::sign_in_for_rules(&request.oauth, &request.email, &open, &request.cancel);
        return asked.map_err(|refusal| unopened(favorites_refusal(refusal)));
    }
    // A change of the automatic replies found the sign-in doesn't cover changing the mailbox's
    // settings, and the owner said go ahead: the page comes up for that permission alone.
    if request.graph && request.replies {
        let asked = mailcraft_net::oauth::sign_in_for_replies(&request.oauth, &request.email, &open, &request.cancel);
        return asked.map_err(|refusal| unopened(favorites_refusal(refusal)));
    }
    // Calendars are opened by address for the first time (#70): the page comes up for the same
    // one permission, without what reading shared mailboxes' mail takes, by itself.
    if request.graph && request.shared_calendars {
        let asked = mailcraft_net::oauth::sign_in_for_opened_calendars(&request.oauth, &request.email, &open, &request.cancel);
        return asked.map_err(|refusal| unopened(favorites_refusal(refusal)));
    }
    if request.graph && request.shared {
        let asked = mailcraft_net::oauth::sign_in_for_shared(&request.oauth, &request.email, &open, &request.cancel);
        return asked.map_err(|refusal| unopened(favorites_refusal(refusal)));
    }
    // A sign-in on the page, at `endpoints`: how it ended, when it brought no tokens. (How, and
    // never in whose words, is what the log is told.)
    let on_the_page = |endpoints: &Endpoints| {
        let asked = if request.oauth.provider == mailcraft_model::OAuthProvider::Google {
            mailcraft_net::oauth::sign_in_google_explained_at(
                endpoints,
                &request.oauth,
                &request.google_client_secret,
                &request.email,
                &open,
                &request.cancel,
            )
        } else {
            mailcraft_net::oauth::sign_in_explained(endpoints, &request.oauth, &request.email, &open, &request.cancel)
        };
        asked.map_err(|end| {
            let google = request.oauth.provider == mailcraft_model::OAuthProvider::Google;
            let (how, said) = match end {
                _ if no_browser.load(Ordering::Relaxed) => ("no browser could be started", NO_BROWSER.to_string()),
                SignInEnd::Approval if google => ("Google policy prevented authorization", GOOGLE_SIGN_IN_BLOCKED.to_string()),
                SignInEnd::Approval => ("an administrator has to approve the application first", NEEDS_APPROVAL.to_string()),
                SignInEnd::Declined if google => ("Google's page came back with a no", GOOGLE_SIGN_IN_DECLINED.to_string()),
                SignInEnd::Declined => ("the page came back with a no", SIGN_IN_DECLINED.to_string()),
                SignInEnd::TimedOut if google => ("Google did not complete sign-in", GOOGLE_SIGN_IN_TIMED_OUT.to_string()),
                SignInEnd::TimedOut => ("nothing came back in time", SIGN_IN_TIMED_OUT.to_string()),
                SignInEnd::Cancelled => ("it was called off", mailcraft_net::NetError::SignInCancelled.to_string()),
                // Never hand a potentially sensitive provider response to the window or log.
                SignInEnd::Other(_) if google => ("it failed", "Google could not complete sign-in. Please retry or use an app password.".into()),
                SignInEnd::Other(e) => ("it failed", e.to_string()),
            };
            // The failure category is enough; no account identifier belongs in this log.
            log::info!("sign-in on the provider's page: {how}");
            said
        })
    };
    // "Connect calendar and contacts": the owner asked, so the page comes up, for what Microsoft
    // Graph needs rather than for mail.
    if request.graph {
        // (For an account kept in step through Graph, its mail too: R3.)
        let resource = if request.mail_write { Resource::GraphMail } else { Resource::Graph };
        return on_the_page(&Endpoints::for_resource(&request.oauth, resource));
    }
    if !request.refresh.is_empty() {
        let refreshed = if request.oauth.provider == mailcraft_model::OAuthProvider::Google {
            mailcraft_net::oauth::refresh_google_at(&Endpoints::of(&request.oauth), &request.oauth, &request.google_client_secret, &request.refresh)
        } else {
            mailcraft_net::oauth::refresh(&request.oauth, &request.refresh)
        };
        match refreshed {
            Ok(tokens) => return Ok(tokens),
            // The machine-readable invalid_grant response confirms this Google refresh
            // credential was revoked. Only a quiet attempt returns the structured signal to
            // erase this account's old stored grant: interactive attempts proceed to the browser.
            Err(e) if e.is_revoked_google_refresh() && request.quiet => return Err(GOOGLE_REAUTH_REQUIRED.to_string()),
            // A refused refresh needs a fresh browser sign-in if the owner asked for it.
            Err(e) if e.is_refused_sign_in() => {}
            // A network or server failure must not erase a still-usable refresh token or echo
            // an untrusted provider error containing secret material.
            Err(_) if request.oauth.provider == mailcraft_model::OAuthProvider::Google => {
                return Err("Google sign-in could not reach the authorization server. Retry later.".into());
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    // Nobody asked for this run: no page is put in front of the owner.
    if request.quiet {
        if request.oauth.provider == mailcraft_model::OAuthProvider::Google {
            return Err("Google needs a new sign-in. Use Send/Receive to reconnect.".into());
        }
        return Err(format!("The sign-in for {} has run out. Use Send/Receive to sign in again.", request.email));
    }
    if request.mail_only {
        // Without the calendar and contacts along: for when the provider's page won't have both.
        let mail = Endpoints { consent: String::new(), lesser: String::new(), ..Endpoints::of(&request.oauth) };
        return on_the_page(&mail);
    }
    on_the_page(&Endpoints::of(&request.oauth))
}

/// Why the owner's yes to reading Outlook's Favorites wasn't had, in the words the window knows
/// each kind of no by: an administrator has to approve it first, the organisation has shut the
/// door, this kind of account has no such thing. Anything else is said as it is.
fn favorites_refusal(refusal: mailcraft_net::oauth::FavoritesRefusal) -> String {
    use mailcraft_net::oauth::FavoritesRefusal;
    use mailcraft_ui_egui::favorites_sync::{NEEDS_APPROVAL, REFUSED_BY_ORGANISATION, REFUSED_FOR_ACCOUNT};
    match refusal {
        FavoritesRefusal::Approval => NEEDS_APPROVAL.to_string(),
        FavoritesRefusal::Organisation => REFUSED_BY_ORGANISATION.to_string(),
        FavoritesRefusal::Account => REFUSED_FOR_ACCOUNT.to_string(),
        FavoritesRefusal::Other(e) => e.to_string(),
    }
}

/// The application (client) id this build presents to Microsoft's sign-in page: the
/// `MAILCRAFT_MS_CLIENT_ID` environment variable when the app starts, else the one the build was
/// given under the same name. An identifier, not a secret; empty when there is none.
pub(crate) fn microsoft_client_id() -> String {
    let built_in = option_env!("MAILCRAFT_MS_CLIENT_ID").unwrap_or_default().to_string();
    let id = std::env::var("MAILCRAFT_MS_CLIENT_ID").unwrap_or(built_in);
    let id = id.trim();
    if id.len() <= 200 && id.chars().all(|c| c.is_ascii_alphanumeric() || "-._".contains(c)) { id.to_string() } else { String::new() }
}

/// The MailCraft-owned Google Desktop registration. Its built-in client secret is a
/// public installed-app identifier, NEVER an account's password or OAuth refresh token.
pub(crate) fn google_client_id() -> String {
    let built_in = option_env!("MAILCRAFT_GOOGLE_CLIENT_ID").unwrap_or_default();
    let id = std::env::var("MAILCRAFT_GOOGLE_CLIENT_ID").unwrap_or_else(|_| built_in.into());
    let id = id.trim();
    if id.len() <= 200 && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_')) { id.to_string() } else { String::new() }
}

pub(crate) fn google_client_secret() -> String {
    let built_in = option_env!("MAILCRAFT_GOOGLE_CLIENT_SECRET").unwrap_or_default();
    let secret = std::env::var("MAILCRAFT_GOOGLE_CLIENT_SECRET").unwrap_or_else(|_| built_in.into());
    if secret.len() <= 512 && secret.bytes().all(|b| b.is_ascii_graphic()) { secret } else { String::new() }
}
