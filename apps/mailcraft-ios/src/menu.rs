// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The iPadOS menu bar, from the same table as the Mac's (`mailcraft_ui_egui::menubar`): File,
//! Edit, View, Folder, Message and Help, and Settings… (⌘,) in the app menu, which opens File ›
//! Options. `mailcraft_apple_bridge::menus` puts them in UIKit's menu bar; here each item is
//! numbered, what is chosen is run as the Mac's menus run it, and the items are kept enabled as
//! their commands are. Cut, Copy and Paste go through the iPad's pasteboard.

use egui::{Context, OutputCommand};
use mailcraft_apple_bridge::menus::{self as bar, Bar, Entry as BarEntry, Item as BarItem, Key};
use mailcraft_ui_egui::MailApp;
use mailcraft_ui_egui::i18n::{self, Lang};
use mailcraft_ui_egui::menubar::{self, Action, Chord, Entry, Item};

/// The most choices run in one frame.
const PER_FRAME: usize = 32;

pub struct Menus {
    /// What each numbered item runs.
    actions: Vec<Action>,
    lang: Lang,
}

/// The key UIKit is given for `chord`.
fn key(chord: &Chord) -> Option<Key> {
    let input = match chord.key.as_str() {
        "Enter" => "\r".to_string(),
        "," => ",".to_string(),
        k if k.chars().count() == 1 => k.to_lowercase(),
        _ => return None,
    };
    Some(Key { input, shift: chord.shift, alt: chord.alt })
}

fn number(item: &Item, actions: &mut Vec<Action>) -> BarItem {
    actions.push(item.action.clone());
    BarItem { title: item.label.clone(), key: item.chord.as_ref().and_then(key), tag: actions.len() - 1 }
}

/// The bar for `app`, and what each of its items runs.
fn made(app: &MailApp) -> (Bar, Vec<Action>) {
    let mut actions = Vec::new();
    let settings = menubar::app_items().first().map(|item| number(item, &mut actions));
    let menus = menubar::menus(app)
        .into_iter()
        .map(|block| bar::Menu {
            title: block.title,
            entries: block
                .entries
                .iter()
                .map(|entry| match entry {
                    Entry::Item(item) => BarEntry::Item(number(item, &mut actions)),
                    Entry::Separator => BarEntry::Separator,
                })
                .collect(),
        })
        .collect();
    (Bar { settings, menus }, actions)
}

impl Menus {
    /// Fill the menu bar; `ctx` is woken when an item is chosen.
    pub fn install(ctx: &Context, app: &MailApp) -> Menus {
        let (made, actions) = made(app);
        bar::install(made);
        let wake = ctx.clone();
        bar::on_choice(move || wake.request_repaint());
        Menus { actions, lang: i18n::current() }
    }

    /// Once a frame: run what was chosen, keep the items enabled as their commands are, and
    /// make the menus again in a new language.
    pub fn pump(&mut self, app: &mut MailApp, ctx: &Context) {
        if i18n::current() != self.lang {
            self.lang = i18n::current();
            let (made, actions) = made(app);
            bar::install(made);
            self.actions = actions;
        }
        for tag in bar::take().into_iter().take(PER_FRAME) {
            let Some(action) = self.actions.get(tag).cloned() else { continue };
            if menubar::enabled(app, &action) {
                menubar::run(app, &action, bar::paste_text);
                ctx.request_repaint();
            }
        }
        bar::set_enabled(self.actions.iter().map(|a| menubar::enabled(app, a)).collect());
    }
}

/// What the window copied this frame goes on the iPad's pasteboard (the window library has none
/// on iPadOS). Called at the end of the frame.
pub fn copy_out(ctx: &Context) {
    let copied = ctx.output(|o| {
        o.commands.iter().rev().find_map(|c| match c {
            OutputCommand::CopyText(text) => Some(text.clone()),
            _ => None,
        })
    });
    if let Some(text) = copied {
        bar::copy_text(&text);
    }
}
