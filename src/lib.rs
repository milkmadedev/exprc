//! # exprc
//!
//! Compiles mathematical expressions into compact binary RPN bytecode.
//! `no_std`, zero allocation, no panics; all memory is caller-provided
//! and every limit is programmer-configured.
//!
//! ```
//! use exprc::{decode_into, parse_into, MAX_RPN};
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
//! # Organization
//!
//! * [`parse_into`] / [`compile_into`] — expression to bytecode. The
//!   latter takes a [`Config`] (output limit, nesting depth) and a
//!   [`Resolve`] implementation for variable substitution.
//! * [`Session`] — persistent single-letter variables: definition,
//!   substitution, constant folding, calculator-style lines.
//! * [`eval`] — postfix stack machine over caller memory with a
//!   [`Vars`] value map.
//! * [`solve`] — numeric equation solving by bracketed bisection.
//! * [`decode_into`] — renders bytecode back to readable RPN text.
//! * [`opcodes`] — the wire format.
//!
//! # Input grammar
//!
//! Numbers (`12`, `.5`, `1e-3`), variables `a..z`, constants `e`/`pi`,
//! functions `sin cos tan asin acos atan ln log`, operators `+ - * / ^`,
//! unary minus, implicit multiplication, and log-with-base via postfix
//! underscore (`log(100)_10`). See the README for the full table and
//! precedence rules.
//!
//! # Error handling
//!
//! Every failure mode is a typed [`Error`] carrying a byte offset where
//! relevant: malformed input, nesting beyond [`MAX_DEPTH`], output
//! beyond the configured limit, undersized buffers or scratch,
//! recursive definitions, evaluation stack exhaustion. No panics on any
//! path.

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
mod solve;
mod writer;

pub use config::{Config, DEFAULT_MAX_DEPTH, DEFAULT_OUTPUT_LIMIT};
pub use core::{compile_into, parse_into, NoResolve};
pub use decode::decode_into;
pub use error::{Error, Result};
pub use eval::{eval, Vars};
pub use session::{split_assignment, Line, Session};
pub use solve::{solve, SolveCfg};

/// Back-compat alias: the default configuration's output limit.
pub const MAX_RPN: usize = DEFAULT_OUTPUT_LIMIT;
/// Back-compat alias: the default configuration's nesting depth.
pub const MAX_DEPTH: u32 = DEFAULT_MAX_DEPTH;

// Compile-time sanity checks on the default limits.
const _: () = {
    assert!(MAX_RPN == 10 * 1024);
    assert!(MAX_DEPTH >= 16);
};
