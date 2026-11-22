// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! A host about to be stopped without warning (iPadOS, going to the background) hands what
//! changed to the background saver at once, and never waits for it to be written.

use std::sync::{Arc, Mutex};

use mailcraft_engine::Session;
use mailcraft_model::Mailbox;
use serde_json::json;

use crate::{MailApp, Services};

/// A saver that writes nothing and keeps what it was handed; its flush would hang a test.
#[derive(Clone, Default)]
struct Saver {
    handed: Arc<Mutex<Vec<u64>>>,
}

impl mailcraft_engine::io::BackgroundSave for Saver {
    fn follow(&self, _: &std::path::Path) {}
    fn submit(&self, _: &std::path::Path, _: &Mailbox, rev: u64) {
        self.handed.lock().unwrap().push(rev);
    }
    fn reports(&self) -> Vec<mailcraft_engine::io::SaveReport> {
        Vec::new()
    }
    fn flush(&self) {
        panic!("save_soon never waits for the writing");
    }
}

fn window(saver: &Saver) -> MailApp {
    let mut app = MailApp::new(Session::at(mailcraft_engine::sample::mailbox(1_791_993_600, 0), 1_791_993_600, 0), Services::default());
    app.services.saver = Some(Box::new(saver.clone()));
    app.session.path = Some(std::path::PathBuf::from("kept.mailcraft"));
    app.autosave = true;
    app
}

#[test]
fn a_change_goes_to_the_saver_at_once() {
    let saver = Saver::default();
    let mut app = window(&saver);
    app.save_soon();
    assert!(saver.handed.lock().unwrap().is_empty(), "nothing changed: nothing handed");
    let id = app.session.mailbox.messages.values().next().map(|m| m.id).unwrap();
    app.run("mail.flag", json!({"id": id})).unwrap();
    app.save_soon();
    assert_eq!(*saver.handed.lock().unwrap(), vec![app.session.rev()], "the newest state, without waiting the autosave's seconds");
}

#[test]
fn nothing_goes_without_autosave() {
    let saver = Saver::default();
    let mut app = window(&saver);
    app.autosave = false;
    let id = app.session.mailbox.messages.values().next().map(|m| m.id).unwrap();
    app.run("mail.flag", json!({"id": id})).unwrap();
    app.save_soon();
    assert!(saver.handed.lock().unwrap().is_empty());
}
