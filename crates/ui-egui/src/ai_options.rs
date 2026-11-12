// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! File › Options › Smart Inbox and summaries: the switch, the model file, the graphics card,
//! when summaries are made, and what the model is doing (`ai.status`), with Forget all summaries.

use egui::Ui;
use serde_json::{Value, json};

use crate::MailApp;
use crate::theme::Tokens;

pub(crate) fn options(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let prefs = app.session.prefs.ai.clone();
    let host = app.session.ai.host.clone();
    let set = |app: &mut MailApp, params: Value| {
        if let Err(e) = app.run("ai.options", params) {
            app.status(e);
        }
    };
    let dim = |ui: &mut Ui, text: &str| {
        ui.label(egui::RichText::new(text).color(t.text_dim));
    };
    if app.services.ai.is_none() {
        dim(ui, tl!("This version of MailCraft has no model engine: the smart Inbox and summaries are in the desktop app."));
        return;
    }
    dim(
        ui,
        tl!(
            "A language model on this computer sorts the Inbox into Primary, Transactions, Updates and Promotions, and writes short summaries. Your mail never leaves this computer, and summaries are kept only here."
        ),
    );
    let mut on = prefs.enabled;
    let switch =
        ui.add_enabled(!prefs.model_path.is_empty() || prefs.enabled, egui::Checkbox::new(&mut on, tl!("Sort the Inbox and summarise mail")));
    app.hit.add("optionAiEnabled", 0, switch.rect);
    if switch.changed() {
        set(app, json!({"enabled": on}));
    }
    crate::backstage::row(ui, tl!("Model file"), |ui| {
        let shown = if prefs.model_path.is_empty() {
            tl!("None chosen").to_string()
        } else {
            std::path::Path::new(&prefs.model_path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        };
        ui.label(shown).on_hover_text(prefs.model_path.clone());
        if ui.button(tl!("Choose…")).clicked() {
            crate::files::pick_model(app);
        }
        // A model that came with MailCraft, one click away while none is chosen.
        if prefs.model_path.is_empty()
            && let Some(bundled) = app.services.ai.as_ref().and_then(|h| h.bundled_model())
            && ui.button(tl!("Use the model that came with MailCraft")).clicked()
        {
            set(app, json!({"modelPath": bundled}));
        }
    });
    if !prefs.model_path.is_empty()
        && let Some(h) = app.services.ai.as_ref()
    {
        match h.check_model(&prefs.model_path) {
            // (A model that is no file, as Apple's on-device model on iPad, has no size.)
            Ok(0) => dim(ui, tl!("Apple's on-device model, on this device.")),
            Ok(bytes) => dim(ui, &crate::i18n::fmt(tl!("A GGUF model, {size}."), &[("size", &mailcraft_model::size_label(bytes))])),
            Err(e) => {
                ui.label(egui::RichText::new(crate::i18n::fmt(tl!("This file can't be used: {why}"), &[("why", &e)])).color(t.red));
            }
        }
    }
    crate::backstage::row(ui, tl!("Graphics card"), |ui| {
        let label = |g: mailcraft_engine::ai::GpuChoice| match g {
            mailcraft_engine::ai::GpuChoice::Auto => tl!("Use it when it works"),
            mailcraft_engine::ai::GpuChoice::Off => tl!("Processor only"),
        };
        egui::ComboBox::from_id_salt("opt_ai_gpu").selected_text(label(prefs.gpu)).show_ui(ui, |ui| {
            for (g, code) in [(mailcraft_engine::ai::GpuChoice::Auto, "auto"), (mailcraft_engine::ai::GpuChoice::Off, "off")] {
                if ui.selectable_label(g == prefs.gpu, label(g)).clicked() {
                    set(app, json!({"gpu": code}));
                }
            }
        });
    });
    let mut list = prefs.list_summaries;
    if ui.checkbox(&mut list, tl!("Show summaries in the message list, in place of the first words")).changed() {
        set(app, json!({"listSummaries": list}));
    }
    let mut summarize = prefs.summarize_new;
    if ui.checkbox(&mut summarize, tl!("Summarise new mail as it arrives (otherwise when it is opened)")).changed() {
        set(app, json!({"summarizeNew": summarize}));
    }
    crate::backstage::row(ui, tl!("Summarise messages longer than"), |ui| {
        let label = |n: u32| tln!(n as usize, "{n} word", "{n} words");
        egui::ComboBox::from_id_salt("opt_ai_words").selected_text(label(prefs.summary_min_words)).show_ui(ui, |ui| {
            for n in [60u32, 120, 250, 500] {
                if ui.selectable_label(n == prefs.summary_min_words, label(n)).clicked() {
                    set(app, json!({"summaryMinWords": n}));
                }
            }
        });
    });
    crate::backstage::row(ui, tl!("Sort mail already here from the last"), |ui| {
        let label = |n: u32| tln!(n as usize, "{n} day", "{n} days");
        egui::ComboBox::from_id_salt("opt_ai_days").selected_text(label(prefs.backfill_days)).show_ui(ui, |ui| {
            for n in [7u32, 30, 90, 365] {
                if ui.selectable_label(n == prefs.backfill_days, label(n)).clicked() {
                    set(app, json!({"backfillDays": n}));
                }
            }
        });
    });
    crate::backstage::row(ui, tl!("Let go of the model after"), |ui| {
        let label = |n: u32| tln!(n as usize, "{n} idle minute", "{n} idle minutes");
        egui::ComboBox::from_id_salt("opt_ai_idle").selected_text(label(prefs.idle_unload_minutes)).show_ui(ui, |ui| {
            for n in [1u32, 5, 15, 60] {
                if ui.selectable_label(n == prefs.idle_unload_minutes, label(n)).clicked() {
                    set(app, json!({"idleUnloadMinutes": n}));
                }
            }
        });
    });
    // What it is doing.
    let state = if !host.available {
        tl!("This build has no model engine.").to_string()
    } else if host.loaded {
        let on = match host.backend.as_str() {
            "vulkan" | "metal" => tl!("the graphics card"),
            _ => tl!("the processor"),
        };
        crate::i18n::fmt(tl!("{model} is loaded, on {where}."), &[("model", &host.model), ("where", on)])
    } else if prefs.enabled {
        tl!("The model loads when there is mail to sort.").to_string()
    } else {
        tl!("Off.").to_string()
    };
    dim(ui, &state);
    if !host.self_test.is_empty() {
        dim(ui, &crate::i18n::fmt(tl!("Graphics card self-test: {result}"), &[("result", &host.self_test)]));
    }
    let left = app.session.ai.backlog + host.working;
    if prefs.enabled && left > 0 {
        dim(ui, &tln!(left, "{n} message left to sort", "{n} messages left to sort"));
    }
    if host.last_millis > 0 {
        dim(ui, &crate::i18n::fmt(tl!("The last one took {seconds} s."), &[("seconds", &format!("{:.1}", f64::from(host.last_millis) / 1000.0))]));
    }
    if !host.error.is_empty() {
        ui.label(egui::RichText::new(&host.error).color(t.red));
    }
    if ui
        .button(tl!("Forget all summaries"))
        .on_hover_text(tl!("Forgets every summary and how mail was sorted, but for what you sorted yourself. Undo brings them back."))
        .clicked()
    {
        match app.run("ai.clearInsights", json!({})) {
            Ok(v) => {
                let n = v.get("messages").and_then(Value::as_u64).unwrap_or(0) as usize;
                app.status(tln!(n, "Forgot what was made of {n} message", "Forgot what was made of {n} messages"));
            }
            Err(e) => app.status(e),
        }
    }
    search(app, ui, t);
}

/// Search by meaning: the embedding model, its switch, how far the index is, and Rebuild.
fn search(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let prefs = app.session.prefs.ai.clone();
    let set = |app: &mut MailApp, params: Value| {
        if let Err(e) = app.run("ai.options", params) {
            app.status(e);
        }
    };
    let dim = |ui: &mut Ui, text: &str| {
        ui.label(egui::RichText::new(text).color(t.text_dim));
    };
    ui.add_space(10.0);
    ui.label(egui::RichText::new(tl!("Search by meaning")).strong());
    dim(
        ui,
        tl!(
            "An embedding model on this computer reads each message once, so that a search also finds mail that means what you typed without using your words. What it keeps of your mail stays beside the mailbox on this computer."
        ),
    );
    let mut on = prefs.search;
    let switch =
        ui.add_enabled(!prefs.search_model_path.is_empty(), egui::Checkbox::new(&mut on, tl!("Find mail by meaning as well as by its words")));
    app.hit.add("optionAiSearch", 0, switch.rect);
    if switch.changed() {
        set(app, json!({"search": on}));
    }
    crate::backstage::row(ui, tl!("Embedding model"), |ui| {
        let shown = if prefs.search_model_path.is_empty() {
            tl!("None chosen").to_string()
        } else {
            std::path::Path::new(&prefs.search_model_path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        };
        ui.label(shown).on_hover_text(prefs.search_model_path.clone());
        if ui.button(tl!("Choose…")).clicked() {
            crate::files::pick_embed_model(app);
        }
        if !prefs.search_model_path.is_empty() && ui.button(tl!("None")).clicked() {
            set(app, json!({"searchModelPath": ""}));
        }
    });
    if !prefs.search_model_path.is_empty()
        && let Some(h) = app.services.ai.as_ref()
        && let Err(e) = h.check_model(&prefs.search_model_path)
    {
        ui.label(egui::RichText::new(crate::i18n::fmt(tl!("This file can't be used: {why}"), &[("why", &e)])).color(t.red));
    }
    if mailcraft_engine::ai::searches_by_meaning(&app.session) {
        let indexed = app.session.ai.vectors.as_ref().map_or(0, |v| v.len());
        let left = app.session.ai.embed_backlog;
        let said = if left > 0 {
            crate::i18n::fmt(
                &tln!(left, "{indexed} messages read for meaning, {n} to go", "{indexed} messages read for meaning, {n} to go"),
                &[("indexed", &indexed.to_string())],
            )
        } else {
            tln!(indexed, "{n} message read for meaning", "{n} messages read for meaning")
        };
        dim(ui, &said);
        if ui.button(tl!("Read every message again")).on_hover_text(tl!("Forgets what search by meaning has read, and reads it all again.")).clicked()
        {
            match app.run("ai.reindexVectors", json!({})) {
                Ok(_) => app.status(tl!("Every message will be read again for search by meaning.")),
                Err(e) => app.status(e),
            }
        }
    }
}

/// An embedding model file was picked: checked, then used for search by meaning.
pub(crate) fn use_embed_model(app: &mut MailApp, path: &str) -> Result<Value, String> {
    if let Some(h) = app.services.ai.as_ref() {
        h.check_model(path).map_err(|e| crate::i18n::fmt(tl!("This file can't be used: {why}"), &[("why", &e)]))?;
    }
    app.run("ai.options", json!({"searchModelPath": path, "search": true}))
}

/// A model file was picked: checked, then used.
pub(crate) fn use_model(app: &mut MailApp, path: &str) -> Result<Value, String> {
    if let Some(h) = app.services.ai.as_ref() {
        h.check_model(path).map_err(|e| crate::i18n::fmt(tl!("This file can't be used: {why}"), &[("why", &e)]))?;
    }
    app.run("ai.options", json!({"modelPath": path}))
}
