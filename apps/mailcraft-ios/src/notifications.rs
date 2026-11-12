// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Reminders and new mail through iPadOS's notifications (the UserNotifications framework,
//! through its generated bindings' safe calls only).
//!
//! iPadOS suspends a program in the background, so a reminder can't be posted as it falls due:
//! the reminders still to come are handed over in advance, each with a time trigger, and the
//! system shows them by itself, with MailCraft closed too ([`AlertHost::schedule`]). What falls due
//! while MailCraft runs is posted at once as well. The owner is asked for permission the first
//! time; until they answer, what comes waits (a few), and after a no nothing is posted.

use std::sync::{Arc, Mutex};

use block2::RcBlock;
use mailcraft_ui_egui::alerts::{AlertHost, AlertTrouble, DesktopAlert, Scheduled};
use objc2::runtime::Bool;
use objc2_foundation::{NSError, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest, UNNotificationSound, UNTimeIntervalNotificationTrigger,
    UNUserNotificationCenter,
};

/// How many alerts wait for the owner's answer about permission; more are dropped.
const WAITING: usize = 20;

/// What is held until the owner answers: alerts to post, and the last list to schedule.
#[derive(Default)]
struct Held {
    alerts: Vec<DesktopAlert>,
    coming: Option<Vec<Scheduled>>,
}

#[derive(Default)]
enum Permission {
    #[default]
    Unknown,
    Asking(Held),
    Granted,
    Refused,
}

#[derive(Default)]
pub struct Notifications {
    permission: Arc<Mutex<Permission>>,
}

fn content(title: &str, body: &str) -> objc2::rc::Retained<UNMutableNotificationContent> {
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    content.setSound(Some(&UNNotificationSound::defaultSound()));
    content
}

/// Post one alert now.
fn post(alert: &DesktopAlert) {
    let id = NSString::from_str(&format!("mailcraft-alert-{}", alert.key));
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&id, &content(&alert.title, &alert.body), None);
    UNUserNotificationCenter::currentNotificationCenter().addNotificationRequest_withCompletionHandler(&request, None);
}

/// Replace what is scheduled with `coming`.
fn schedule_now(coming: &[Scheduled]) {
    let center = UNUserNotificationCenter::currentNotificationCenter();
    center.removeAllPendingNotificationRequests();
    let now = mailcraft_time::now();
    for c in coming {
        let seconds = c.at.saturating_sub(now);
        if seconds <= 0 {
            continue;
        }
        let trigger = UNTimeIntervalNotificationTrigger::triggerWithTimeInterval_repeats(seconds as f64, false);
        let id = NSString::from_str(&format!("mailcraft-reminder-{}", c.id));
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&id, &content(&c.title, &c.body), Some(&trigger));
        center.addNotificationRequest_withCompletionHandler(&request, None);
    }
}

impl Notifications {
    /// Ask the owner, once: what was held is posted and scheduled on a yes.
    fn ask(&self, held: Held) {
        let Ok(mut permission) = self.permission.lock() else { return };
        *permission = Permission::Asking(held);
        drop(permission);
        let shared = self.permission.clone();
        let answered = RcBlock::new(move |granted: Bool, _: *mut NSError| {
            let Ok(mut permission) = shared.lock() else { return };
            let held = match std::mem::take(&mut *permission) {
                Permission::Asking(held) => held,
                _ => Held::default(),
            };
            *permission = if granted.as_bool() { Permission::Granted } else { Permission::Refused };
            if granted.as_bool() {
                held.alerts.iter().for_each(post);
                if let Some(coming) = &held.coming {
                    schedule_now(coming);
                }
            }
        });
        let options = UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound | UNAuthorizationOptions::Badge;
        UNUserNotificationCenter::currentNotificationCenter().requestAuthorizationWithOptions_completionHandler(options, &answered);
    }
}

impl AlertHost for Notifications {
    fn show(&self, alert: &DesktopAlert) -> Result<(), String> {
        let Ok(mut permission) = self.permission.lock() else { return Err("the permission's state is lost".into()) };
        match &mut *permission {
            Permission::Granted => {
                post(alert);
                Ok(())
            }
            Permission::Refused => Err("turned off in Settings".into()),
            Permission::Asking(held) => {
                if held.alerts.len() < WAITING {
                    held.alerts.push(alert.clone());
                }
                Ok(())
            }
            Permission::Unknown => {
                drop(permission);
                self.ask(Held { alerts: vec![alert.clone()], coming: None });
                Ok(())
            }
        }
    }
    fn trouble(&self) -> Option<AlertTrouble> {
        matches!(self.permission.lock().as_deref(), Ok(Permission::Refused)).then_some(AlertTrouble::TurnedOff)
    }
    fn schedule(&self, coming: &[Scheduled]) {
        let Ok(mut permission) = self.permission.lock() else { return };
        match &mut *permission {
            Permission::Granted => schedule_now(coming),
            Permission::Refused => {}
            Permission::Asking(held) => held.coming = Some(coming.to_vec()),
            Permission::Unknown => {
                drop(permission);
                self.ask(Held { alerts: Vec::new(), coming: Some(coming.to_vec()) });
            }
        }
    }
}
