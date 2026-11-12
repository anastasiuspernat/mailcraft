// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The meaning index: kept and read back, a damaged file thrown away, another model's numbers
//! starting again, messages gone taken out, and scores close to full precision.

use mailcraft_model::insight::EMBED_DIM;

use super::*;

fn unit(seed: u32) -> Vec<f32> {
    let mut v: Vec<f32> = (0..EMBED_DIM).map(|i| (((i as u32).wrapping_mul(2_654_435_761) ^ seed) % 1000) as f32 / 1000.0 - 0.5).collect();
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.iter_mut().for_each(|x| *x /= n);
    v
}

fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("mailcraft-vectors-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::create_dir_all(&d);
    d
}

#[test]
fn numbers_kept_as_bytes_score_as_the_numbers_do() {
    let mut v = Vectors::in_memory();
    let a = unit(1);
    v.insert(7, 1, &a, "embed");
    let exact: f32 = a.iter().map(|x| x * x).sum();
    let got = v.scores(&a, [7, 8].into_iter());
    assert_eq!(got.len(), 1);
    assert!((got[0].1 - exact).abs() < 0.01, "{got:?}");
    let b = unit(99);
    let exact: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
    assert!((v.scores(&b, [7].into_iter())[0].1 - exact).abs() < 0.01);
    assert!(v.current(7, 1) && !v.current(7, 2) && !v.current(8, 1));
}

#[test]
fn the_file_is_read_back_and_damage_starts_it_again() {
    let d = dir("file");
    let mailbox = d.join("m.mailcraft");
    let mut v = Vectors::open(&mailbox);
    for id in 1..=5 {
        v.insert(id, id * 10, &unit(id as u32), "qwen3-embedding");
    }
    v.flush();
    v.insert(6, 60, &unit(6), "qwen3-embedding");
    v.retain(|id| id != 2);
    v.flush();
    let back = Vectors::open(&mailbox);
    assert_eq!(back.len(), 5);
    assert_eq!(back.model(), "qwen3-embedding");
    assert!(back.current(6, 60) && !back.current(2, 20) && back.current(1, 10));
    // A record cut short at the end is left out.
    let file = path_for(&mailbox);
    let bytes = std::fs::read(&file).unwrap();
    std::fs::write(&file, &bytes[..bytes.len() - 5]).unwrap();
    // (It was the record that took message 2 out.)
    assert_eq!(Vectors::open(&mailbox).len(), 6);
    // Damage inside: thrown away, never an error.
    let mut bytes = std::fs::read(&file).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0x55;
    std::fs::write(&file, &bytes).unwrap();
    let mut v = Vectors::open(&mailbox);
    assert!(v.is_empty());
    v.insert(1, 10, &unit(1), "qwen3-embedding");
    v.flush();
    assert_eq!(Vectors::open(&mailbox).len(), 1);
    // Garbage, or another version.
    std::fs::write(&file, b"not a vector file").unwrap();
    assert!(Vectors::open(&mailbox).is_empty());
    Vectors::remove_file(&mailbox);
    assert!(!file.exists());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn another_models_numbers_start_it_again() {
    let mut v = Vectors::in_memory();
    v.insert(1, 1, &unit(1), "one");
    v.insert(2, 2, &unit(2), "one");
    v.insert(3, 3, &unit(3), "two");
    assert_eq!(v.len(), 1);
    assert_eq!(v.model(), "two");
    // Of another length: let go of.
    v.insert(4, 4, &[1.0, 0.0], "two");
    assert_eq!(v.len(), 1);
}
