// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
#[test]
fn hex_is_lower_case_pairs() {
    assert_eq!(crate::push::hex(&[0x0a, 0xff, 0x10]), "0aff10");
    assert_eq!(crate::push::hex(&[]), "");
}
