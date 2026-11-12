// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! What the program and its worker process say to each other: one JSON object a line, the
//! program's on the worker's standard input ([`Request`]), the worker's on its standard output
//! ([`Response`]). Both sides read the other's lines as hostile (a line at most [`MAX_LINE`]).

use mailcraft_model::insight::{Job, JobResult};
use serde::{Deserialize, Serialize};

use crate::backend::{LoadInfo, ModelConfig};

/// The longest line either side reads.
pub const MAX_LINE: usize = 1 << 20;

/// What the program asks of the worker.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "do")]
pub enum Request {
    Load {
        config: ModelConfig,
    },
    Run {
        job: Job,
    },
    /// Run the self-test ([`crate::prompt::self_test_job`]).
    SelfTest,
    Unload,
    Quit,
}

/// What the worker answers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "is")]
pub enum Response {
    /// The first line, once the worker runs: the backend it was built with.
    Ready {
        backend: String,
    },
    Loaded {
        info: LoadInfo,
    },
    Done {
        result: JobResult,
    },
    SelfTest {
        passed: bool,
        millis: u32,
    },
    Unloaded,
    /// Something failed (in English, never the model's own words).
    Error {
        error: String,
    },
}

/// One line of `value` as JSON.
pub fn line<T: Serialize>(value: &T) -> String {
    let mut s = serde_json::to_string(value).unwrap_or_else(|_| "{}".into());
    s.push('\n');
    s
}

/// A line read back, `None` when it is too long or isn't one.
pub fn read<T: for<'de> Deserialize<'de>>(line: &str) -> Option<T> {
    if line.len() > MAX_LINE {
        return None;
    }
    serde_json::from_str(line.trim()).ok()
}
