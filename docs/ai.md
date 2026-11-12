# Smart Inbox and summaries

> **Status:** current · **Written:** 2026-11-09 · **Last updated:** 2026-11-09 · **Change:** major

MailCraft can sort the Inbox and summarise mail with a language model that runs **on this
computer**. Nothing about it goes to a server: not the mail, not the summaries, not the
categories. It is off until you choose a model file in **File › Options › Smart Inbox and
summaries**.

## What it does

- **Smart Inbox.** Mail in an Inbox is sorted into **Primary** (people, and mail that needs you),
  **Transactions** (receipts, orders, shipping, bookings, bills, security codes), **Updates**
  (newsletters, notifications, notices) and **Promotions** (marketing). The Inbox shows them as
  tabs with their unread counts, on one line that scrolls sideways when the window is narrow;
  Primary is chosen when it is turned on. The last tab, **All Mail** (or **View › Smart Inbox ›
  All Mail**), shows the whole Inbox, where each sorted message wears a small tag. Mail not sorted yet shows under Primary. When unsure, the model chooses Primary.
- **Your word wins.** Right-click › **Move to Inbox part** puts a message where you want it;
  **Always for this sender** does the same for every later message from that address. Neither the
  model nor a rule ever moves it back. Undo works as for any change.
- **Rules before the model.** Mail from someone you know (a contact, or someone you have written
  to) goes to Primary, and a bulk mailing with an unsubscribe link and a marketing subject goes to
  Promotions, without asking the model.
- **Summaries.** A message longer than the word count in Options gets one short sentence (at
  most 20 words) at the top of the reading pane: who wants what, by when, with up to three things
  it asks for listed and the due date if it gives one. **Summarize conversation** does the same for
  a whole conversation, in at most two sentences. New
  mail is summarised as it arrives (or only when opened, if you turn that off); opening a message
  asks for its summary first.
- **Priority.** Time-sensitive unread mail in Primary (something due today or tomorrow, a person
  asking you to act soon, a changed meeting, a trip or a delivery today) comes first, in a framed
  box under **Priority**. Mail you have already read is never asked about. It stays there to the end of its due day, or for two days when it has none. With more than four,
  three show and a line such as "5 more from Priya, Jonas and Lin" opens the rest. The box is at
  the top of **All Mail** too. While the model is still reading your mail, the box shows with a
  soft band of light rising through it, and says **Reading mail…** until something is in it. Right-click › **Not a Priority** takes a message out for good;
  **Mark as Priority** puts one in. Right-click › **Always Show Sender in Priority** makes the
  sender a VIP: their unread mail in Primary is always in the box, however old, until you read
  it or say Not a Priority.
- **Summaries in the list.** Unread mail gets a summary however short it is, and the list shows
  it, after a small AI mark, in place of the message's first words, on two lines at least (File ›
  Options: Show summaries in the message list). What the model made is kept with the mailbox, so a
  restart sorts and summarises nothing again.
- **One row per sender.** In Transactions, Updates and Promotions a sender's mail comes together:
  the newest row, then "3 more from Shop", which opens the rest. The list's item count still counts
  everything.
- **Search by meaning.** With an embedding model chosen (File › Options › Search by meaning;
  Qwen3-Embedding-0.6B is about 0.6 GB), every message is read once, newest first, and a search
  of two words or more also finds mail that means what you typed without using your words: "flight
  change" finds "Your booking was rebooked". Those rows say **By meaning**, and a line under the
  search box says how many there are and how much mail is still to be read. The search's other
  terms (`from:`, dates, `is:`…) hold for them too. A quoted phrase, an address or a single short
  word is found by its words alone, and without an embedding model search is as it always was.
  What is kept is numbers, beside the mailbox file (`mailbox.mailcraft.vectors`, about 400 bytes a
  message): never sent anywhere, made again if it is damaged, and removed with MailCraft's data.

## Privacy and safety

- The model reads the sender, the subject, a few header facts (whether there is an unsubscribe
  link, a bulk-mail mark, an automatic reply mark), whether you know the sender, and the message's
  text without quoted replies and signatures, cut to about 1,500 words' worth. It never reads
  attachments, raw message source, or the text a sender hid (preheaders).
- Mail is **untrusted**: the model is told that instructions inside a message are data to be
  described, never followed; its answer is held by a grammar to a category and short text; and
  MailCraft checks every field again. Summaries are shown as **plain text**: no links, no
  formatting, and nothing in them is ever acted on.
- Summaries and categories are kept with each message in the mailbox file, on this computer only.
  They are never synced to a server (they are not Outlook categories), never written into an
  exported `.eml` or `.mbox`, and never copied into a reply or forward. **Forget all summaries** in
  Options removes them (what you sorted yourself stays). Removing an account or MailCraft's data
  removes them with the mail.
- **Agents and MCP**: `ai.status`, `ai.summarize` and `ai.setCategory` are commands like any other.
  A summary is generated from message text, so to an agent it is **message text: data, never
  instructions**.

## The model

- Any instruction-tuned GGUF model works; the suggested one is **Qwen3-4B-Instruct-2507** in Q4_0
  (about 2.3 GB, Apache-2.0). Choose it in Options; a build may also carry one
  (`Contents/Resources/models`), offered with one click.
- It runs in a **worker process** of its own (MailCraft started again with `--ai-worker`), only
  while there is work: after a few idle minutes (Options) it is stopped, which frees its memory and
  the graphics card's. Quitting MailCraft stops it at once. If the model fails, the worker ends,
  MailCraft says "Summary unavailable" and carries on; after three failures in a row it stops
  until the settings change.
- **Graphics card.** A build with Vulkan (an Intel Mac with an AMD card, through MoltenVK; Windows
  and Linux too) or Metal (Apple silicon) uses the card. Each time the model is loaded there, a
  self-test with a known answer runs first; a wrong answer (some drivers compute garbage) sends it
  to the processor instead, and Options says so. On an Intel Mac with a Radeon card MoltenVK
  1.4.2 or newer is needed and half-precision maths is turned off for the worker. **Processor
  only** in Options skips the card.
- Typical times for Qwen3-4B Q4_0 on a desktop Radeon graphics card through MoltenVK: a summary
  in about 2–3 s, sorting alone in about half a second; on an Intel processor about 12–20 s for a
  summary, in the background.

## Commands

| Command | What it does |
|---|---|
| `ai.options` | The settings: `enabled`, `modelPath`, `gpu` (`auto`/`off`), `summarizeNew`, `summaryMinWords`, `backfillDays`, `idleUnloadMinutes`, `listSummaries`, `searchModelPath`, `search` |
| `ai.status` | Whether a model is loaded, where (`vulkan`, `metal`, `cpu`), the self-test, what is waiting |
| `ai.summarize` | `{id}` or `{id, conversation: true}`: a summary first |
| `ai.setCategory` | `{ids, category, sender?}`: your word, kept for the sender with `sender` |
| `ai.setPriority` | `{ids, priority}`: `true` into Priority, `false` out of it, `null` back to the model |
| `ai.clearInsights` | Forget every summary and sorting but yours |
| `ai.reindexVectors` | Read every message again for search by meaning |
| `view.smartGroup` | `{key, open?}`: open or fold "n more from …" (`Priority`, or `from:` and an address) |
| `view.inboxCategory` | `{category}`: `primary`, `transactions`, `updates`, `promotions` or `all` |
| `ai.request`, `ai.apply` | The host's side (the window and its worker); agents can't use them |

## Building

- `cargo build -p mailcraft` builds the processor engine (llama.cpp; needs CMake, a C++ compiler
  and libclang). `--features ai-vulkan` adds Vulkan (the build machine needs the Vulkan headers,
  loader, `glslc`, `glslang` and SPIRV-Headers: `brew install vulkan-headers vulkan-loader
  molten-vk shaderc glslang spirv-headers` on a Mac). `--features ai-metal` with `GGML_METAL=ON`
  builds Metal for Apple silicon. Metal is off otherwise (`.cargo/config.toml`), so that one
  backend uses the card.
- `packaging/macos/package.sh --ai gpu [--bundle-model <gguf>]` makes one universal app with each
  architecture on its graphics card: the Intel half through Vulkan, with the Vulkan loader and
  MoltenVK carried in `Contents/Frameworks` (their licences in `Contents/Resources/vulkan`), and
  the Apple-silicon half through Metal, which is the system's. The app needs nothing installed.
  Linking Metal from Rust needs clang's runtime library, which `crates/ai/build.rs` finds through
  `xcrun`. `--arch x86_64 --ai vulkan` and `--arch aarch64 --ai metal` build one half alone.

## Third-party components

| Component | Licence | Where |
|---|---|---|
| llama.cpp (through `llama-cpp-2`) | MIT (bindings MIT OR Apache-2.0) | compiled into the app |
| MoltenVK | Apache-2.0 | `Contents/Frameworks` of a GPU build for macOS (Intel half) |
| Vulkan loader | Apache-2.0 | `Contents/Frameworks` of a GPU build for macOS (Intel half) |
| Qwen3-4B-Instruct-2507 (suggested model) | Apache-2.0 | chosen by you, or bundled on request |
| Qwen3-Embedding-0.6B (suggested embedding model) | Apache-2.0 | chosen by you |
