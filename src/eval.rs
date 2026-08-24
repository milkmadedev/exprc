//! Bytecode evaluation: the calculator half.
//!
//! A compact postfix stack machine over caller-provided value stack —
//! zero allocation, IEEE semantics, `NaN` propagates loudly for unset
//! variables.
//!
//! Arithmetic (`+ - * / neg`, integral-exponent `^`) evaluates on every
//! target. Transcendental functions require feature `std` (they need a
//! libm); without it they evaluate to [`Error::FuncUnsupportedOnTarget`]
//! rather than silently wrong numbers.

use crate::error::{Error, Result};
use crate::opcodes;

/// Variable values by letter; unset letters read as `NaN`.
#[derive(Clone, Copy, Debug)]
pub struct Vars([f64; 26]);

impl Vars {
    pub fn new() -> Self {
        Self([f64::NAN; 26])
    }

    /// Set variable `var` (letter `a..=z`); returns false for non-letters.
    pub fn set(&mut self, var: u8, value: f64) -> bool {
        match var {
            b'a'..=b'z' => {
                self.0[(var - b'a') as usize] = value;
                true
            }
            _ => false,
        }
    }

    pub fn get(&self, var: u8) -> Option<f64> {
        self.0.get((var - b'a') as usize).copied()
    }

    pub const fn zeroed() -> Self {
        Self([0.0; 26])
    }
}

impl Default for Vars {
    fn default() -> Self {
        Self::new()
    }
}

/// Evaluate a bytecode stream produced by this crate.
///
/// `stack` is the value stack; expressions nestable within
/// [`crate::DEFAULT_MAX_DEPTH`] need at most ~64 slots, but the exact
/// requirement depends on shape — overflow reports
/// [`Error::EvalStackOverflow`] rather than panicking.
pub fn eval(rpn: &[u8], vars: &Vars, stack: &mut [f64]) -> Result<f64> {
    let mut sp = 0usize;

    macro_rules! push {
        ($v:expr) => {{
            if sp >= stack.len() {
                return Err(Error::EvalStackOverflow);
            }
            stack[sp] = $v;
            sp += 1;
        }};
    }
    macro_rules! pop {
        () => {{
            if sp == 0 {
                return Err(Error::MalformedRpn { offset: 0 });
            }
            sp -= 1;
            stack[sp]
        }};
    }

    let mut i = 0usize;
    while i < rpn.len() {
        match rpn[i] {
            opcodes::NUM => {
                let b = rpn
                    .get(i + 1..i + 9)
                    .ok_or(Error::MalformedRpn { offset: i })?;
                push!(f64::from_le_bytes(b.try_into().unwrap()));
                i += 9;
            }
            opcodes::VAR => {
                let name = *rpn.get(i + 1).ok_or(Error::MalformedRpn { offset: i })?;
                if !name.is_ascii_lowercase() {
                    return Err(Error::MalformedRpn { offset: i });
                }
                push!(vars.get(name).unwrap_or(f64::NAN));
                i += 2;
            }
            opcodes::CONST => {
                let id = *rpn.get(i + 1).ok_or(Error::MalformedRpn { offset: i })?;
                match id {
                    opcodes::CONST_E => push!(core::f64::consts::E),
                    opcodes::CONST_PI => push!(core::f64::consts::PI),
                    _ => return Err(Error::MalformedRpn { offset: i }),
                }
                i += 2;
            }
            opcodes::FUNC => {
                let id = *rpn.get(i + 1).ok_or(Error::MalformedRpn { offset: i })?;
                if id == opcodes::FUNC_LOGB {
                    // Stack holds [ln(arg), base] (the LOG opcode fired
                    // first), and log_b(arg) = ln(arg) / ln(base).
                    let base = pop!();
                    let logged = pop!();
                    push!(logged / apply_func(opcodes::FUNC_LN, base)?);
                } else {
                    let x = pop!();
                    push!(apply_func(id, x)?);
                }
                i += 2;
            }
            op => {
                match op {
                    opcodes::NEG => {
                        let x = pop!();
                        push!(-x);
                    }
                    opcodes::ADD | opcodes::SUB | opcodes::MUL | opcodes::DIV | opcodes::POW => {
                        let b = pop!();
                        let a = pop!();
                        push!(match op {
                            opcodes::ADD => a + b,
                            opcodes::SUB => a - b,
                            opcodes::MUL => a * b,
                            opcodes::DIV => a / b,
                            _ => eval_pow(a, b)?,
                        });
                    }
                    _ => return Err(Error::MalformedRpn { offset: i }),
                }
                i += 1;
            }
        }
    }
    if sp != 1 {
        return Err(Error::MalformedRpn { offset: 0 });
    }
    Ok(stack[0])
}

fn eval_pow(a: f64, b: f64) -> Result<f64> {
    // Exact iterated multiplication for small integral exponents works
    // everywhere; general powers need libm (std builds only).
    if b.is_finite() && b % 1.0 == 0.0 && b.abs() <= 32.0 {
        let n = b as i32;
        let mut acc = 1.0;
        for _ in 0..n.unsigned_abs() {
            acc *= a;
        }
        Ok(if n < 0 { 1.0 / acc } else { acc })
    } else {
        #[cfg(feature = "std")]
        {
            Ok(a.powf(b))
        }
        #[cfg(not(feature = "std"))]
        {
            let _ = a;
            Err(Error::FuncUnsupportedOnTarget { func: opcodes::POW })
        }
    }
}

fn apply_func(id: u8, x: f64) -> Result<f64> {
    #[cfg(feature = "std")]
    {
        Ok(match id {
            opcodes::FUNC_SIN => x.sin(),
            opcodes::FUNC_COS => x.cos(),
            opcodes::FUNC_TAN => x.tan(),
            opcodes::FUNC_ASIN => x.asin(),
            opcodes::FUNC_ACOS => x.acos(),
            opcodes::FUNC_ATAN => x.atan(),
            opcodes::FUNC_LN | opcodes::FUNC_LOG => x.ln(),
            opcodes::FUNC_LOGB => unreachable!("handled with arity 2"),
            _ => return Err(Error::MalformedRpn { offset: 0 }),
        })
    }
    #[cfg(not(feature = "std"))]
    {
        let _ = x;
        Err(Error::FuncUnsupportedOnTarget { func: id })
    }
}
