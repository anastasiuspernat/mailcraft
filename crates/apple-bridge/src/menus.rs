// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The iPadOS menu bar, the one that shows when the pointer goes to the top of the screen (or
//! the keyboard's Command key is held): MailCraft's own menus in place of the system's File,
//! Edit, View, Format and Help, and Settings… in the app menu. The window says what the menus
//! hold ([`install`]) and which items can be chosen now ([`set_enabled`]); UIKit asks the app's
//! delegate ([`crate::delegate`]) to build them and sends it the item chosen, which waits here
//! for the window to [`take`] it. Every item is a number the window gave; nothing here knows what
//! it runs.
//!
//! Also the iPad's pasteboard ([`paste_text`], [`copy_text`]), which the Edit menu's Cut, Copy
//! and Paste use: the window library has none on iPadOS.

use std::cell::RefCell;

/// A key equivalent: ⌘ and `input` (one lower-case letter or digit, `\r` or `,`), with ⇧ and ⌥
/// where asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key {
    pub input: String,
    pub shift: bool,
    pub alt: bool,
}

/// An item: what it says, its key, and the number the window knows it by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub title: String,
    pub key: Option<Key>,
    pub tag: usize,
}

/// A line of a menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    Item(Item),
    Separator,
}

/// A menu of the bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Menu {
    pub title: String,
    pub entries: Vec<Entry>,
}

/// The whole bar: Settings… in the app menu, then the menus in order (the last one goes after
/// the system's Window menu, as Help does).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bar {
    pub settings: Option<Item>,
    pub menus: Vec<Menu>,
}

#[derive(Default)]
struct State {
    bar: Bar,
    enabled: Vec<bool>,
    chosen: Vec<usize>,
    wake: Option<Box<dyn Fn()>>,
}

/// The most choices kept until the window takes them.
const MOST_CHOSEN: usize = 32;

thread_local! {
    // (UIKit builds and runs the menus on the main thread, where the window runs too.)
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Put `bar` in the menu bar (again, after a change of language).
pub fn install(bar: Bar) {
    STATE.with(|s| s.borrow_mut().bar = bar);
    #[cfg(target_os = "ios")]
    ios::rebuild();
}

/// Which items can be chosen now, by their numbers; the menus are looked at again when it
/// changed.
pub fn set_enabled(enabled: Vec<bool>) {
    let changed = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let changed = s.enabled != enabled;
        s.enabled = enabled;
        changed
    });
    #[cfg(target_os = "ios")]
    if changed {
        ios::revalidate();
    }
    #[cfg(not(target_os = "ios"))]
    let _ = changed;
}

/// `wake` is called when an item is chosen (from the main thread).
pub fn on_choice(wake: impl Fn() + 'static) {
    STATE.with(|s| s.borrow_mut().wake = Some(Box::new(wake)));
}

/// The numbers of the items chosen since the last call, in order.
pub fn take() -> Vec<usize> {
    STATE.with(|s| std::mem::take(&mut s.borrow_mut().chosen))
}

/// Whether item `tag` can be chosen, as last said.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
fn is_enabled(tag: usize) -> bool {
    STATE.with(|s| s.borrow().enabled.get(tag).copied().unwrap_or(false))
}

/// Item `tag` was chosen.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
fn choose(tag: usize) {
    let wake = STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.chosen.len() < MOST_CHOSEN {
            s.chosen.push(tag);
        }
        s.wake.take()
    });
    if let Some(wake) = wake {
        wake();
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            if s.wake.is_none() {
                s.wake = Some(wake);
            }
        });
    }
}

/// The text on the iPad's pasteboard (none: empty).
pub fn paste_text() -> String {
    #[cfg(target_os = "ios")]
    return ios::paste_text();
    #[cfg(not(target_os = "ios"))]
    String::new()
}

/// Put `text` on the iPad's pasteboard.
pub fn copy_text(text: &str) {
    #[cfg(target_os = "ios")]
    ios::copy_text(text);
    #[cfg(not(target_os = "ios"))]
    let _ = text;
}

/// The tag a command's property list carries (`mailcraft.<n>`).
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
fn tag_of(text: &str) -> Option<usize> {
    text.strip_prefix(TAG_PREFIX)?.parse().ok()
}

const TAG_PREFIX: &str = "mailcraft.";

#[cfg(target_os = "ios")]
pub(crate) use ios::{build, chosen, validate};

#[cfg(target_os = "ios")]
mod ios {
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, ProtocolObject};
    use objc2::{MainThreadMarker, sel};
    use objc2_foundation::{NSArray, NSString};
    use objc2_ui_kit::{
        UICommand, UIKeyCommand, UIKeyModifierFlags, UIMenu, UIMenuApplication, UIMenuBuilder, UIMenuEdit, UIMenuElement, UIMenuElementAttributes,
        UIMenuFile, UIMenuFormat, UIMenuHelp, UIMenuOptions, UIMenuPreferences, UIMenuSystem, UIMenuView, UIMenuWindow, UIPasteboard,
    };

    use super::{Entry, Item, STATE, TAG_PREFIX, choose, is_enabled, tag_of};

    pub fn rebuild() {
        // (When the app has another delegate, the menu bar isn't ours to fill.)
        if !crate::delegate::ensure() {
            return;
        }
        let Some(mtm) = MainThreadMarker::new() else { return };
        UIMenuSystem::mainSystem(mtm).setNeedsRebuild();
    }

    pub fn revalidate() {
        let Some(mtm) = MainThreadMarker::new() else { return };
        UIMenuSystem::mainSystem(mtm).setNeedsRevalidate();
    }

    /// One item as a command (with its key, when it has one).
    fn command(item: &Item, mtm: MainThreadMarker) -> Retained<UIMenuElement> {
        let title = NSString::from_str(&item.title);
        let tag = NSString::from_str(&format!("{TAG_PREFIX}{}", item.tag));
        let list: &AnyObject = tag.as_ref();
        match &item.key {
            Some(key) => {
                let mut flags = UIKeyModifierFlags::Command;
                if key.shift {
                    flags |= UIKeyModifierFlags::Shift;
                }
                if key.alt {
                    flags |= UIKeyModifierFlags::Alternate;
                }
                // SAFETY: `mailcraftMenu:` is the delegate's method that takes a command; the
                // property list is a string.
                let c = unsafe {
                    UIKeyCommand::commandWithTitle_image_action_input_modifierFlags_propertyList(
                        &title,
                        None,
                        sel!(mailcraftMenu:),
                        &NSString::from_str(&key.input),
                        flags,
                        Some(list),
                        mtm,
                    )
                };
                Retained::into_super(Retained::into_super(c))
            }
            None => {
                // SAFETY: as above.
                let c = unsafe { UICommand::commandWithTitle_image_action_propertyList(&title, None, sel!(mailcraftMenu:), Some(list), mtm) };
                Retained::into_super(c)
            }
        }
    }

    /// A menu's lines: the items between separators as groups shown in line.
    fn children(entries: &[Entry], mtm: MainThreadMarker) -> Retained<NSArray<UIMenuElement>> {
        let mut groups: Vec<Vec<Retained<UIMenuElement>>> = vec![Vec::new()];
        for entry in entries {
            match entry {
                Entry::Item(item) => {
                    if let Some(group) = groups.last_mut() {
                        group.push(command(item, mtm));
                    }
                }
                Entry::Separator => groups.push(Vec::new()),
            }
        }
        let groups: Vec<Retained<UIMenuElement>> = groups
            .into_iter()
            .filter(|g| !g.is_empty())
            .map(|g| {
                let menu = UIMenu::menuWithTitle_image_identifier_options_children(
                    &NSString::new(),
                    None,
                    None,
                    UIMenuOptions::DisplayInline,
                    &NSArray::from_retained_slice(&g),
                    mtm,
                );
                Retained::into_super(menu)
            })
            .collect();
        NSArray::from_retained_slice(&groups)
    }

    pub fn build(builder: &ProtocolObject<dyn UIMenuBuilder>) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let bar = STATE.with(|s| s.borrow().bar.clone());
        if bar.menus.is_empty() {
            return;
        }
        // SAFETY: the identifiers are UIKit's constant strings, there for the life of the app.
        let (application, file, edit, format, view, window, help, preferences) =
            unsafe { (UIMenuApplication, UIMenuFile, UIMenuEdit, UIMenuFormat, UIMenuView, UIMenuWindow, UIMenuHelp, UIMenuPreferences) };
        for system in [file, edit, format, view, help] {
            builder.removeMenuForIdentifier(system);
        }
        if let Some(settings) = &bar.settings {
            let children = children(&[Entry::Item(settings.clone())], mtm);
            let ours = UIMenu::menuWithTitle_image_identifier_options_children(
                &NSString::new(),
                None,
                Some(&NSString::from_str("app.mailcraft.settings")),
                UIMenuOptions::DisplayInline,
                &children,
                mtm,
            );
            if builder.menuForIdentifier(preferences).is_some() {
                builder.replaceMenuForIdentifier_withMenu(preferences, &ours);
            } else {
                builder.insertChildMenu_atStartOfMenuForIdentifier(&ours, application);
            }
        }
        let last = bar.menus.len().saturating_sub(1);
        let mut after: Retained<NSString> = NSString::from_str(&application.to_string());
        for (n, menu) in bar.menus.iter().enumerate() {
            let id = NSString::from_str(&format!("app.mailcraft.menu.{n}"));
            let made = UIMenu::menuWithTitle_image_identifier_options_children(
                &NSString::from_str(&menu.title),
                None,
                Some(&id),
                UIMenuOptions::empty(),
                &children(&menu.entries, mtm),
                mtm,
            );
            // The last (Help) after the system's Window menu, when there is one.
            if n == last && n > 0 && builder.menuForIdentifier(window).is_some() {
                builder.insertSiblingMenu_afterMenuForIdentifier(&made, window);
            } else {
                builder.insertSiblingMenu_afterMenuForIdentifier(&made, &after);
            }
            after = id;
        }
    }

    /// The tag a command carries, when it is one of ours.
    fn tag(command: &UICommand) -> Option<usize> {
        let list = command.propertyList()?;
        let text = list.downcast_ref::<NSString>()?;
        tag_of(&text.to_string())
    }

    pub fn validate(command: &UICommand) {
        let Some(tag) = tag(command) else { return };
        let mut attributes = command.attributes();
        attributes.set(UIMenuElementAttributes::Disabled, !is_enabled(tag));
        command.setAttributes(attributes);
    }

    pub fn chosen(command: &UICommand) {
        if let Some(tag) = tag(command)
            && is_enabled(tag)
        {
            choose(tag);
        }
    }

    pub fn paste_text() -> String {
        // SAFETY: read on the main thread, where the window runs.
        unsafe { UIPasteboard::generalPasteboard().string() }.map(|s| s.to_string()).unwrap_or_default()
    }

    pub fn copy_text(text: &str) {
        // SAFETY: written on the main thread, where the window runs.
        unsafe { UIPasteboard::generalPasteboard().setString(Some(&NSString::from_str(text))) };
    }
}

#[cfg(test)]
#[path = "menus_tests.rs"]
mod menus_tests;
