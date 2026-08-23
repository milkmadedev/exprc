//! Byte-classification table for the lexer.
//!
//! A single `const` 256-entry table turns every byte into a class code in
//! one load, replacing the per-byte `match` cascade of v1. The table is
//! built at compile time by a `const fn`, so there is no initialization
//! cost and it lives in `.rodata`.

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Class(pub(crate) u8);

impl Class {
    /// Byte introduces no legal token.
    pub(crate) const ILLEGAL: Class = Class(0);
    pub(crate) const WS: Class = Class(1);
    pub(crate) const DIGIT: Class = Class(2);
    pub(crate) const LETTER: Class = Class(3);
    pub(crate) const DOT: Class = Class(4);
    // Single-char operator/paren classes; identity doubles as a dispatch key.
    pub(crate) const ADD: Class = Class(5);
    pub(crate) const SUB: Class = Class(6);
    pub(crate) const MUL: Class = Class(7);
    pub(crate) const DIV: Class = Class(8);
    pub(crate) const POW: Class = Class(9);
    pub(crate) const UNDER: Class = Class(10);
    pub(crate) const LPAREN: Class = Class(11);
    pub(crate) const RPAREN: Class = Class(12);
}

const fn build() -> [u8; 256] {
    let mut t = [Class::ILLEGAL.0; 256];
    let mut i = 0;
    while i < 256 {
        let b = i as u8;
        t[i] = match b {
            b' ' | b'\t' | b'\r' | b'\n' => Class::WS.0,
            b'0'..=b'9' => Class::DIGIT.0,
            b'a'..=b'z' => Class::LETTER.0,
            b'.' => Class::DOT.0,
            b'+' => Class::ADD.0,
            b'-' => Class::SUB.0,
            b'*' => Class::MUL.0,
            b'/' => Class::DIV.0,
            b'^' => Class::POW.0,
            b'_' => Class::UNDER.0,
            b'(' => Class::LPAREN.0,
            b')' => Class::RPAREN.0,
            _ => Class::ILLEGAL.0,
        };
        i += 1;
    }
    t
}

pub(crate) static CLASSES: [u8; 256] = build();

#[inline(always)]
pub(crate) fn class(b: u8) -> Class {
    Class(CLASSES[b as usize])
}
