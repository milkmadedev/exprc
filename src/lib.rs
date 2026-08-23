//! # rpn2 — ground-up second engine for math-expression → binary RPN
//!
//! Same wire format, same API, same error semantics as [`pratt-rpn`]
//! (drop-in compatible; differential tests enforce bit-equality), but a
//! completely different internal design:
//!
//! | dimension        | pratt-rpn (v1)             | rpn2 (v2)                     |
//! |------------------|----------------------------|-------------------------------|
//! | parse strategy   | recursive Pratt            | iterative shunting-yard core  |
//! | aux memory       | call stack, ≤ ~19 KB deep  | fixed 2 B/entry op stack      |
//! | number parsing   | `str::parse::<f64>` always | bit-exact fast path, fallback |
//! | lexer scanning   | byte-at-a-time `match`     | 256-way class table + SIMD    |
//! | output writes    | bounds check per byte      | one check per instruction     |
//!
//! SIMD is compile-time dispatched: SSE2 on `x86_64` and NEON on
//! `aarch64` are baseline for those targets (every CPU that runs the
//! target has them); all other architectures use portable scalar loops,
//! which are also used for sub-16-byte tails everywhere. There is no
//! runtime feature detection to configure and no code path that can be
//! "missing" on a supported CPU.
//!
//! [`pratt-rpn`]: https://github.com/milkmadedev/pratt-rpn

#![no_std]
#![deny(unsafe_code)]

#[cfg(test)]
extern crate std;

pub mod opcodes;

mod class;
mod core;
mod decode;
mod error;
mod lex;
mod num;
mod simd;
mod writer;

pub use core::parse_into;
pub use decode::decode_into;
pub use error::{Error, Result};

/// Hard cap on generated RPN output, in bytes. Identical to v1 so output
/// buffers are interchangeable between engines.
pub const MAX_RPN: usize = 10 * 1024;

/// Maximum expression nesting depth. Semantics match v1 exactly.
pub const MAX_DEPTH: u32 = 128;

/// Measurement hook (not part of the public API): tokenize only.
#[cfg(feature = "std")]
#[doc(hidden)]
pub fn __scan_only(src: &str) -> usize {
    use lex::Scanner;
    let mut s = Scanner::new(src);
    let mut n = 0usize;
    while let Ok(Some((_, _t))) = s.next() {
        n += 1;
    }
    ::core::hint::black_box(n)
}

/// Measurement hook (not part of the public API): parse with output discarded.
#[cfg(feature = "std")]
#[doc(hidden)]
pub fn __parse_discard(src: &str, buf: &mut [u8]) -> usize {
    parse_into(src, buf).unwrap_or(0)
}

// Compile-time sanity checks on the public limits.
const _: () = {
    assert!(MAX_RPN == 10 * 1024);
    assert!(MAX_DEPTH >= 16);
};
