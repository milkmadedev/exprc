//! # rpn2 — math expressions to binary RPN bytecode
//!
//! Compile mathematical expressions (as `&str`) into compact binary
//! Reverse-Polish-Notation bytecode, written into buffers **you** own,
//! under limits **you** choose. Built for live recompilation: zero
//! allocation, no panics, bounded everything.
//!
//! ## One-shot compilation
//!
//! ```
//! use rpn2::{decode_into, parse_into, MAX_RPN};
//!
//! let mut buf = [0u8; MAX_RPN];
//! let n = parse_into("3x+2", &mut buf).unwrap();
//! assert_eq!(n, 22);
//!
//! let mut text = [0u8; 4 * MAX_RPN];
//! let m = decode_into(&buf[..n], &mut text).unwrap();
//! assert_eq!(&text[..m], b"3 x * 2 +");
//! ```
//!
//! ## Programmer-owned limits
//!
//! [`Config`] replaces every hard-coded bound: the output budget and
//! nesting depth are yours. Scratch requirements derive from them.
//!
//! ```
//! # use rpn2::{compile_into, Config, NoResolve};
//! # fn main() -> Result<(), rpn2::Error> {
//! // 1 MiB of bytecode budget, deep nesting:
//! let cfg = Config::new().output_limit(1024 * 1024).max_depth(2048);
//! let mut out = vec![0u8; cfg.get_output_limit()];
//! let mut stack = vec![0u8; cfg.scratch_len()];
//! let n = compile_into(&cfg, &NoResolve, "1+1", &mut out, &mut stack)?;
//! assert!(n > 0);
//! # Ok(())
//! # }
//! ```
//!
//! [`NoResolve`] keeps every variable symbolic; pass your own
//! `Resolve` implementation (or a [`Session`]) to substitute bodies.
//!
//! ## Variables, substitution, solving
//!
//! [`Session`] stores single-letter definitions and splices them into
//! later compilations; fully-numeric chains collapse to literals.
//!
//! ```
//! use rpn2::{decode_into, Config, Session};
//!
//! let mut s = Session::<256>::new(Config::new());
//! let mut stack = [0u8; Config::new().scratch_len()];
//! s.compile_line("a = 6", &mut [], &mut stack).unwrap();
//! s.compile_line("b = 7", &mut [], &mut stack).unwrap();
//! s.compile_line("x = a*b", &mut [], &mut stack).unwrap();
//!
//! let mut out = [0u8; 128];
//! let n = s.compile("x+1", &mut out, &mut stack).unwrap();
//! let mut text = [0u8; 512];
//! let m = decode_into(&out[..n], &mut text).unwrap();
//! assert_eq!(&text[..m], b"42 1 +"); // solved at compile time
//! ```
//!
//! ## Evaluation
//!
//! Pair compiled bytecode with a [`Vars`] map and a value stack:
//!
//! ```
//! use rpn2::{eval, parse_into, Vars};
//! # fn main() -> Result<(), rpn2::Error> {
//! let mut buf = [0u8; 256];
//! let n = parse_into("2x^2+1", &mut buf)?;
//! let mut vars = Vars::zeroed();
//! vars.set(b'x', 3.0);
//! let y = eval(&buf[..n], &vars, &mut [0.0; 64])?;
//! assert_eq!(y, 19.0);
//! # Ok(())
//! # }
//! ```
//!
//! ## Contract
//!
//! * **Input**: ASCII expressions — numbers (`12`, `.5`, `1e-3`),
//!   variables `a..z`, constants `e`/`pi`, functions
//!   `sin cos tan asin acos atan ln log`, operators `+ - * / ^`,
//!   unary minus, implicit multiplication (`3x`, `2sin(x)`), log-base
//!   via postfix underscore (`log(100)_10`).
//! * **Output**: little-endian opcode stream ([`opcodes`]), fixed-width
//!   instructions, O(1) skip/decode.
//! * **Failure is a value** ([`Error`]) with offsets; nothing panics;
//!   nothing allocates on any compile or evaluate path.

#![no_std]
#![deny(unsafe_code)]

#[cfg(any(test, feature = "std"))]
extern crate std;

pub mod opcodes;

mod class;
mod config;
mod core;
mod decode;
mod error;
mod eval;
mod fold;
mod lex;
mod num;
mod session;
mod simd;
mod writer;

pub use config::{Config, DEFAULT_MAX_DEPTH, DEFAULT_OUTPUT_LIMIT};
pub use core::{compile_into, parse_into, NoResolve};
pub use decode::decode_into;
pub use error::{Error, Result};
pub use eval::{eval, Vars};
pub use session::{split_assignment, Line, Session};

/// Back-compat alias: the default configuration's output limit.
pub const MAX_RPN: usize = DEFAULT_OUTPUT_LIMIT;
/// Back-compat alias: the default configuration's nesting depth.
pub const MAX_DEPTH: u32 = DEFAULT_MAX_DEPTH;

// Compile-time sanity checks on the default limits.
const _: () = {
    assert!(MAX_RPN == 10 * 1024);
    assert!(MAX_DEPTH >= 16);
};
