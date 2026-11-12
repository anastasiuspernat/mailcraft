// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The window's model host: the worker's environment, and the window and the engine with a fake
//! model end to end (new mail gets an insight; quitting is quick).

use std::time::{Duration, Instant};

use mailcraft_ai::client::{Client, InProcess};
use mailcraft_ai::{Backend, FakeBackend};
use mailcraft_engine::Session;
use mailcraft_model::{Address, Body, FolderKind, InboxCategory, Message};
use serde_json::json;

use super::*;

#[test]
fn the_worker_gets_the_graphics_settings_and_its_argument() {
    let cmd = worker_command().unwrap_or_default();
    assert_eq!(cmd.args, vec![WORKER_ARG.to_string()]);
    assert!(cmd.envs.iter().any(|(k, v)| k == "GGML_VK_DISABLE_F16" && v == "1"));
    assert!(cmd.envs.iter().any(|(k, _)| k == "GGML_VK_VISIBLE_DEVICES"));
}

#[test]
fn a_bundles_moltenvk_is_found_beside_the_program() {
    let dir = std::env::temp_dir().join(format!("mailcraft-icd-{}", std::process::id()));
    let program = dir.join("MailCraft.app/Contents/MacOS/mailcraft");
    let icd = dir.join("MailCraft.app/Contents/Resources/vulkan/icd.d/MoltenVK_icd.json");
    let _ = std::fs::create_dir_all(icd.parent().unwrap_or(&dir));
    let _ = std::fs::write(&icd, "{}");
    let found = moltenvk_icd(&program);
    if cfg!(target_os = "macos") {
        assert_eq!(found, Some(icd));
    } else {
        assert_eq!(found, None);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// New mail in the Inbox gets its category and summary through the engine's commands and a fake
/// model, as the window drives them.
#[test]
fn new_mail_gets_an_insight_through_a_fake_model() {
    let mut s = Session::new(mailcraft_model::Mailbox::with_local_account("Owner", "owner@example.org"));
    let a = s.mailbox.accounts[0].id;
    let inbox = s.mailbox.folder_of_kind(a, FolderKind::Inbox).unwrap_or_default();
    s.run("ai.options", &json!({"modelPath": "/m.gguf", "enabled": true, "summaryMinWords": 3})).unwrap();
    let body = "Your order 1234 has shipped and arrives on Monday. Track it in your account.";
    let id = s
        .mailbox
        .add_message(Message {
            folder: inbox,
            from: Address::new("Shop", "orders@shop.example"),
            subject: "Your order shipped".into(),
            body: Body { text: body.into(), html: None },
            received: s.now(),
            ..Default::default()
        })
        .unwrap();
    let client = Client::start(Box::new(InProcess(|| Box::new(FakeBackend::default()) as Box<dyn Backend>)), Box::new(|| {})).unwrap();
    client.configure(Some(mailcraft_ai::ModelConfig::new("/m.gguf".into(), mailcraft_ai::GpuChoice::Off)), Duration::from_secs(60));
    let jobs: Vec<mailcraft_model::insight::Job> = serde_json::from_value(s.run("ai.request", &json!({"max": 4})).unwrap()["jobs"].clone()).unwrap();
    assert_eq!(jobs.len(), 1);
    client.submit(jobs);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut results = Vec::new();
    while results.is_empty() && Instant::now() < deadline {
        results = client.take_results();
        std::thread::sleep(Duration::from_millis(5));
    }
    let applied = s.run("ai.apply", &json!({"results": results})).unwrap();
    assert_eq!(applied["kept"], 1, "{applied}");
    let insight = s.mailbox.message(id).and_then(|m| m.insight.as_deref().cloned()).unwrap_or_default();
    assert_eq!(insight.category, Some(InboxCategory::Transactions));
    assert!(insight.summary.is_some());
}

#[test]
fn a_bundled_model_is_found_in_resources() {
    let dir = std::env::temp_dir().join(format!("mailcraft-model-{}", std::process::id()));
    let program = dir.join("MailCraft.app/Contents/MacOS/mailcraft");
    let models = dir.join("MailCraft.app/Contents/Resources/models");
    let _ = std::fs::create_dir_all(&models);
    assert_eq!(bundled_model_beside(&program), None);
    let _ = std::fs::write(models.join("qwen.gguf"), b"GGUF");
    let _ = std::fs::write(models.join("notes.txt"), b"x");
    assert_eq!(bundled_model_beside(&program), Some(models.join("qwen.gguf").to_string_lossy().into_owned()));
    let _ = std::fs::remove_dir_all(&dir);
}
