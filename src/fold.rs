//! Variable-body expansion: substitution with constant solving.
//!
//! Two passes over a stored definition:
//!
//! 1. **Acyclicity walk** — follow every defined-variable reference and
//!    reject any cycle (`x = y`, `y = x`) before any emission happens.
//! 2. **Fold attempt** — interpret the postfix stream abstractly. If
//!    every leaf resolves to a number (literals, `e`/`pi`, definitions
//!    that folded earlier, computable operators), the whole definition
//!    collapses to a single `NUM` instruction. Transcendental functions
//!    do not fold — the crate carries no libm — and remain symbolic.
//!
//! When folding is impossible the body's instructions are copied
//! through verbatim, recursing into nested defined variables.

use crate::core::Resolve;
use crate::error::{Error, Result};
use crate::opcodes;
use crate::writer::Writer;

/// Fold-stack depth cap. Deferentially symbolic beyond this — never an
/// error — so pathological shapes degrade instead of failing.
const FOLD_CAP: usize = 64;

/// Small-int exponent bound for iterated-multiply power folding.
const POW_FOLD_MAX_ABS: i32 = 32;

#[derive(Clone, Copy)]
enum Val {
    Num(f64),
    Sym,
}

/// Emit `body` in place of a variable atom: folded to a literal when
/// fully known, otherwise expanded (recursively substituting).
pub(crate) fn emit_var_body<R: Resolve>(
    body: &[u8],
    res: &R,
    w: &mut Writer,
    chain: u32,
) -> Result<()> {
    // Pass 0: reject cycles before emitting anything.
    let mut visited = 0u32;
    check_acyclic(body, res, chain, &mut visited)?;

    match try_fold(body, res, chain) {
        Some(v) => {
            let mut ins = [0u8; 9];
            ins[0] = opcodes::NUM;
            ins[1..].copy_from_slice(&v.to_le_bytes());
            w.emitn(ins)
        }
        None => copy_stream(body, res, w, chain),
    }
}

/// Follow all defined-variable references inside `body`; any reference
/// to a var whose `chain` bit is set is a recursive definition.
fn check_acyclic<R: Resolve>(body: &[u8], res: &R, chain: u32, visited: &mut u32) -> Result<()> {
    let mut i = 0usize;
    while i < body.len() {
        match body[i] {
            opcodes::NUM => i += 9,
            opcodes::VAR => {
                let v = *body.get(i + 1).ok_or(Error::MalformedRpn { offset: i })?;
                let bit = 1u32 << (v - b'a');
                if chain & bit != 0 {
                    return Err(Error::RecursiveDefinition { var: v });
                }
                if res.body(v).is_some() && *visited & bit == 0 {
                    *visited |= bit;
                    if let Some(b) = res.body(v) {
                        check_acyclic(b, res, chain, visited)?;
                    }
                }
                i += 2;
            }
            opcodes::CONST | opcodes::FUNC => i += 2,
            _ => i += 1,
        }
    }
    Ok(())
}

/// Abstract interpretation of a postfix stream. Returns `Some(value)`
/// when the entire stream is numeric, `None` when symbolic (or too deep
/// for the fold stack).
fn try_fold<R: Resolve>(body: &[u8], res: &R, chain: u32) -> Option<f64> {
    let mut st: [Val; FOLD_CAP] = [Val::Sym; FOLD_CAP];
    let mut sp = 0usize;

    macro_rules! push {
        ($v:expr) => {
            if sp >= FOLD_CAP {
                return None;
            } else {
                st[sp] = $v;
                sp += 1;
            }
        };
    }
    macro_rules! pop {
        () => {
            if sp == 0 {
                return None;
            } else {
                sp -= 1;
                st[sp]
            }
        };
    }

    let mut i = 0usize;
    while i < body.len() {
        match *body.get(i)? {
            opcodes::NUM => {
                let b = body.get(i + 1..i + 9)?;
                let v = f64::from_le_bytes(b.try_into().ok()?);
                push!(Val::Num(v));
                i += 9;
            }
            opcodes::VAR => {
                let name = *body.get(i + 1)?;
                let bit = 1u32 << (name - b'a');
                if chain & bit != 0 {
                    return None; // unreachable after acyclicity; be safe
                }
                match res.body(name) {
                    Some(b) => match try_fold(b, res, chain | bit) {
                        Some(v) => push!(Val::Num(v)),
                        None => push!(Val::Sym),
                    },
                    None => push!(Val::Sym),
                }
                i += 2;
            }
            opcodes::CONST => match *body.get(i + 1)? {
                opcodes::CONST_E => push!(Val::Num(core::f64::consts::E)),
                opcodes::CONST_PI => push!(Val::Num(core::f64::consts::PI)),
                _ => return None,
            },
            opcodes::FUNC => {
                let id = *body.get(i + 1)?;
                let arity = if id == opcodes::FUNC_LOGB { 2 } else { 1 };
                for _ in 0..arity {
                    pop!();
                }
                // Transcendentals never fold without libm; logb needs ln.
                push!(Val::Sym);
                i += 2;
            }
            op => {
                let r = pop!();
                match op {
                    opcodes::NEG => {
                        push!(match r {
                            Val::Num(x) => Val::Num(-x),
                            Val::Sym => Val::Sym,
                        });
                    }
                    opcodes::ADD | opcodes::SUB | opcodes::MUL | opcodes::DIV | opcodes::POW => {
                        let l = pop!();
                        push!(match (l, r) {
                            (Val::Num(a), Val::Num(b)) => {
                                let v = match op {
                                    opcodes::ADD => a + b,
                                    opcodes::SUB => a - b,
                                    opcodes::MUL => a * b,
                                    opcodes::DIV => a / b,
                                    _ => fold_pow(a, b)?,
                                };
                                Val::Num(v)
                            }
                            _ => Val::Sym,
                        });
                    }
                    _ => return None,
                }
                i += 1;
                continue;
            }
        }
    }
    if sp != 1 {
        return None;
    }
    match st[0] {
        Val::Num(v) => Some(v),
        Val::Sym => None,
    }
}

/// Power folding: exact iterated multiplication for small integral
/// exponents; anything else stays symbolic (`None`).
fn fold_pow(a: f64, b: f64) -> Option<f64> {
    if b.is_finite() && b % 1.0 == 0.0 && b.abs() <= POW_FOLD_MAX_ABS as f64 {
        let n = b as i32;
        let mut acc = 1.0;
        for _ in 0..n.unsigned_abs() {
            acc *= a;
        }
        Some(if n < 0 { 1.0 / acc } else { acc })
    } else {
        None
    }
}

/// Post-define cycle probe: does `var`'s body reach back to `var`?
pub(crate) fn definition_acyclic<R: Resolve>(res: &R, var: u8) -> Result<()> {
    match res.body(var) {
        Some(body) => {
            let mut visited = 0u32;
            check_acyclic(body, res, 1u32 << (var - b'a'), &mut visited)
        }
        None => Ok(()),
    }
}

/// Verbatim copy with recursive substitution of defined variables.
fn copy_stream<R: Resolve>(body: &[u8], res: &R, w: &mut Writer, chain: u32) -> Result<()> {
    let mut i = 0usize;
    while i < body.len() {
        match body[i] {
            opcodes::NUM => {
                let end = i + 9;
                let sl = body.get(i..end).ok_or(Error::MalformedRpn { offset: i })?;
                w.emit(sl)?;
                i = end;
            }
            opcodes::VAR => {
                let v = *body.get(i + 1).ok_or(Error::MalformedRpn { offset: i })?;
                match res.body(v) {
                    Some(b) => emit_var_body(b, res, w, chain)?,
                    None => w.emitn([opcodes::VAR, v])?,
                }
                i += 2;
            }
            opcodes::CONST | opcodes::FUNC => {
                let end = i + 2;
                let sl = body.get(i..end).ok_or(Error::MalformedRpn { offset: i })?;
                w.emit(sl)?;
                i = end;
            }
            _ => {
                w.push(body[i])?;
                i += 1;
            }
        }
    }
    Ok(())
}
