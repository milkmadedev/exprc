//! Opcode constants for the binary RPN stream.
//!
//! A stream is a flat sequence of instructions. Each instruction starts with
//! one of the four framing opcodes ([`NUM`], [`VAR`], [`CONST`], [`FUNC`])
//! followed by its fixed-size payload, or is one of the single-byte operator
//! opcodes. Because every opcode has a statically known payload width, any
//! instruction can be skipped or decoded in O(1).
//!
//! # Layout (little-endian)
//!
//! | Opcode | Payload   | Size | Meaning                        |
//! |--------|-----------|------|--------------------------------|
//! | `NUM`  | `f64`     | 9    | push number                    |
//! | `VAR`  | `u8` name | 2    | push variable (`a`..`z`)       |
//! | `CONST`| `u8` id   | 2    | push constant                  |
//! | `FUNC` | `u8` id   | 2    | call function (arity 1, [`FUNC_LOGB`] arity 2) |
//! | `ADD`..`NEG` | —   | 1    | apply operator                 |

/// Push an `f64` literal; followed by 8 little-endian bytes.
pub const NUM: u8 = 0x01;
/// Push a variable; followed by one ASCII lowercase byte.
pub const VAR: u8 = 0x02;
/// Push a constant; followed by one id byte.
pub const CONST: u8 = 0x03;
/// Call a function; followed by one id byte.
pub const FUNC: u8 = 0x04;

/// Constant ids for [`CONST`].
pub mod consts {
    /// Euler's number.
    pub const E: u8 = 0x01;
    /// Pi.
    pub const PI: u8 = 0x02;
}

/// Function ids for [`FUNC`].
pub mod funcs {
    pub const SIN: u8 = 0x01;
    pub const COS: u8 = 0x02;
    pub const TAN: u8 = 0x03;
    pub const ASIN: u8 = 0x04;
    pub const ACOS: u8 = 0x05;
    pub const ATAN: u8 = 0x06;
    pub const LN: u8 = 0x07;
    /// Natural log alias: same semantics as [`LN`].
    pub const LOG: u8 = 0x08;
    /// Binary log-with-base; consumes base then argument (arity 2).
    pub const LOGB: u8 = 0x09;
}

/// Single-byte operator opcodes.
pub mod ops {
    pub const ADD: u8 = 0x10;
    pub const SUB: u8 = 0x11;
    pub const MUL: u8 = 0x12;
    pub const DIV: u8 = 0x13;
    pub const POW: u8 = 0x14;
    /// Unary negation.
    pub const NEG: u8 = 0x15;
}

// Flat aliases: most call sites want `opcodes::FUNC_SIN`, not
// `opcodes::funcs::SIN`.
pub use consts::{E as CONST_E, PI as CONST_PI};
pub use funcs::{
    ACOS as FUNC_ACOS, ASIN as FUNC_ASIN, ATAN as FUNC_ATAN, COS as FUNC_COS, LN as FUNC_LN,
    LOG as FUNC_LOG, LOGB as FUNC_LOGB, SIN as FUNC_SIN, TAN as FUNC_TAN,
};
pub use ops::{ADD, DIV, MUL, NEG, POW, SUB};
