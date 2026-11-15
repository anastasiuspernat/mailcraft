// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! "Your name" on a new account is the owner's from an account of their own, never the sample
//! mailbox's invented person at a made-up address.

use super::*;

#[test]
fn the_sample_persons_name_is_not_offered_for_a_new_account() {
    let a = app(true);
    let w = AccountSetup::new(&a, &json!({}));
    assert_eq!(w.name, "");
}

#[test]
fn the_owners_own_name_is_offered_for_another_address() {
    let mailbox = mailcraft_model::Mailbox::with_local_account("Noor Haddad", "noor@lumen.co");
    let a = MailApp::new(Session::at(mailbox, NOW, 0), Services::default());
    assert_eq!(AccountSetup::new(&a, &json!({})).name, "Noor Haddad");
}

#[test]
fn addresses_at_example_domains_are_made_up() {
    for email in ["avery@lumen.example", "dev@example.org", "a@mail.example.com", "x@host.test", "y@z.invalid", "q@EXAMPLE.NET."] {
        assert!(made_up(email), "{email}");
    }
    for email in ["noor@lumen.co", "someone@gmail.com", "a@example.org.uk", "b@myexample.com", "nobody"] {
        assert!(!made_up(email), "{email}");
    }
}

/// The wizard over the owner's mailbox, with an address at a provider that has typed servers.
fn owners_wizard() -> AccountSetup {
    let mailbox = mailcraft_model::Mailbox::with_local_account("Noor Haddad", "noor@lumen.co");
    let a = MailApp::new(Session::at(mailbox, NOW, 0), Services::default());
    let mut w = AccountSetup::new(&a, &json!({"email": "noor@example.org"}));
    w.incoming.host = "imap.example.org".into();
    w.outgoing.host = "smtp.example.org".into();
    w
}

#[test]
fn the_name_the_provider_gives_replaces_one_filled_in_from_another_account() {
    let w = owners_wizard();
    assert_eq!(w.name, "Noor Haddad");
    let (_, params) = w.add_command("Noor H. Haddad").unwrap();
    assert_eq!(params["displayName"], "Noor H. Haddad");
    // With no name from the provider, the one filled in stays.
    assert_eq!(w.add_command("").unwrap().1["displayName"], "Noor Haddad");
}

#[test]
fn a_name_the_owner_typed_is_kept_whatever_the_provider_says() {
    let mut w = owners_wizard();
    w.name = "Noor".into();
    assert_eq!(w.add_command("Noor H. Haddad").unwrap().1["displayName"], "Noor");
}
