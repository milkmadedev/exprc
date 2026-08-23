//! (module docs continue at lib)
#![allow(unsafe_code)]
//! The v2 engine: a fully *iterative* precedence-driven emitter.
//!
//! Where v1 (Pratt / precedence climbing) used the call stack as its
//! operator stack — costing a frame per nesting level and capping out at
//! ~19 KB of stack for legal-but-deep input — v2 keeps an explicit,
//! fixed-size operator stack of 2-byte entries and emits instructions in
//! the exact same single pass, with **no recursion anywhere**.
//!
//! # Correspondence with v1's binding powers
//!
//! | construct        | l_bp | r_bp | notes                           |
//! |------------------|------|------|---------------------------------|
//! | `+ -`            | 10   | 11   | left-assoc                      |
//! | `* /`, implicit  | 20   | 21   | left-assoc; implicit == `*`     |
//! | unary `-`        | —    | 25   | prefix                          |
//! | `^`              | 30   | 30   | right-assoc                     |
//! | `_base` postfix  | 41   | 41   | right-assoc; only after `log()` |
//!
//! Nesting accounting mirrors v1's recursion depth exactly (enforced by
//! boundary-sweep tests against the v1 crate): parens, function calls,
//! unary minus, `^` chains, and `_base` each contribute one level; flat
//! left-assoc chains contribute none.

use core::mem::MaybeUninit;

use crate::error::{Error, Result};
use crate::lex::{Scanner, Tok};
use crate::opcodes;
use crate::writer::Writer;
use crate::MAX_DEPTH;

// Binding powers (see module docs).
const BP_ADD: (u8, u8) = (10, 11);
const BP_MUL: (u8, u8) = (20, 21);
const BP_POW: (u8, u8) = (30, 30);
const BP_UNARY_RHS: u8 = 25;
const BP_BASE: u8 = 41;

// Stack entry tags. Operator entries reuse their opcode byte as the tag;
// sentinel/frame entries use reserved high tags.
const TAG_LPAREN: u8 = 0xE0;
const TAG_LOGB: u8 = 0xE1;

#[derive(Clone, Copy)]
struct Ent {
    tag: u8,
    aux: u8, // rbp for operators, function id for FUNC frames
}

impl Ent {
    #[inline]
    const fn op(opcode: u8, rbp: u8) -> Self {
        Self {
            tag: opcode,
            aux: rbp,
        }
    }

    const LPAREN: Self = Self {
        tag: TAG_LPAREN,
        aux: 0,
    };

    /// Sentinel / frame entries halt the collapse loop by tag.
    #[inline]
    const fn is_barrier(&self) -> bool {
        self.tag == TAG_LPAREN || self.tag == opcodes::FUNC
    }
}

/// Fixed operator stack: bounded by structural nesting (`MAX_DEPTH`),
/// never by expression length. Worst legal case holds ≤4 entries per
/// nesting level (`(`, unary `-`, function frame, `_base`) ⇒
/// `4 × MAX_DEPTH + slack`.
const STACK_CAP: usize = 4 * 128 + 16;

struct Engine<'b> {
    w: Writer<'b>,
    /// Operator stack. Only slots `0..sp` are ever read; construction
    /// therefore skips initialization (a ~1 KB memset per parse that
    /// showed up prominently in benchmarks). See `push`/`pop_emit` for
    /// the discipline that makes this sound.
    stack: [MaybeUninit<Ent>; STACK_CAP],
    sp: usize,
    nest: u32,
    /// Set immediately after a complete `log(...)` call closes; cleared by
    /// every other event. Gates the `_base` postfix.
    log_pending: bool,
    expect_operand: bool,
    eof_pos: usize,
}

impl<'b> Engine<'b> {
    fn new(w: Writer<'b>, eof_pos: usize) -> Self {
        // SAFETY: no element is read before being written by `push`;
        // all reads are strictly below `sp`.
        let stack = [const { MaybeUninit::uninit() }; STACK_CAP];
        Self {
            w,
            stack,
            sp: 0,
            // Parity with v1: the base expression occupies one depth unit.
            nest: 1,
            log_pending: false,
            expect_operand: true,
            eof_pos,
        }
    }

    #[inline]
    fn push(&mut self, ent: Ent) -> Result<()> {
        let nested = matches!(
            ent.tag,
            TAG_LPAREN | TAG_LOGB | opcodes::POW | opcodes::NEG | opcodes::FUNC
        );
        if nested {
            if self.nest >= MAX_DEPTH {
                return Err(Error::TooDeep);
            }
            self.nest += 1;
        }
        self.log_pending = false;
        self.stack[self.sp] = MaybeUninit::new(ent);
        self.sp += 1;
        Ok(())
    }

    /// Parenthesis sentinel that belongs to a function frame. It shares
    /// the frame's single nesting level, so it must not bump `nest`.
    #[inline]
    fn push_frame_sentinel(&mut self) {
        self.log_pending = false;
        self.stack[self.sp] = MaybeUninit::new(Ent::LPAREN);
        self.sp += 1;
    }

    /// Pop one entry and emit its instruction. Callers guarantee it is
    /// not a barrier.
    #[inline]
    fn pop_emit(&mut self) -> Result<()> {
        self.sp -= 1;
        // SAFETY: slot was written by push/push_frame_sentinel before this
        // read (sp discipline).
        let ent = unsafe { self.stack[self.sp].assume_init() };
        debug_assert!(!ent.is_barrier());
        self.log_pending = false;
        match ent.tag {
            opcodes::ADD..=opcodes::POW | opcodes::NEG => self.w.push(ent.tag),
            opcodes::FUNC => self.w.emitn([opcodes::FUNC, ent.aux]),
            TAG_LOGB => self.w.emitn([opcodes::FUNC, opcodes::FUNC_LOGB]),
            _ => unreachable!("no other tags are ever pushed"),
        }
    }

    /// Pop-and-emit while the top operator binds at least as tightly as
    /// `lbp` (honoring associativity). Stops at barriers or drain.
    #[inline]
    fn collapse(&mut self, lbp: u8, left_assoc: bool) -> Result<()> {
        while self.sp > 0 {
            // SAFETY: peek strictly below sp (written earlier).
            let top = unsafe { self.stack[self.sp - 1].assume_init() };
            if top.is_barrier() {
                break;
            }
            let rbp = top.aux;
            if rbp < lbp || (rbp == lbp && !left_assoc) {
                break;
            }
            self.pop_emit()?;
        }
        Ok(())
    }

    #[inline]
    fn infix(&mut self, opcode: u8, bp: (u8, u8), left_assoc: bool) -> Result<()> {
        self.collapse(bp.0, left_assoc)?;
        self.push(Ent::op(opcode, bp.1))
    }

    /// An operand arrived directly after another operand: splice in a
    /// multiply (implicit multiplication carries `*`'s precedence).
    #[inline]
    fn implicit_mul(&mut self) -> Result<()> {
        self.infix(opcodes::MUL, BP_MUL, true)
    }

    fn finish(mut self) -> Result<usize> {
        if self.expect_operand {
            return Err(Error::UnexpectedToken { pos: self.eof_pos });
        }
        // Drain. A surviving barrier means an unclosed parenthesis.
        while self.sp > 0 {
            // SAFETY: peek strictly below sp (written earlier).
            if unsafe { self.stack[self.sp - 1].assume_init() }.is_barrier() {
                return Err(Error::ExpectedRparen { pos: self.eof_pos });
            }
            self.pop_emit()?;
        }
        Ok(self.w.finish())
    }
}

pub fn parse_into(src: &str, out: &mut [u8]) -> Result<usize> {
    let mut scanner = Scanner::new(src);
    let mut eng = Engine::new(Writer::new(out), src.len());

    while let Some((pos, tok)) = scanner.next()? {
        // Fast path: operand following another operand splices an
        // implicit multiply before the generic operand emission.
        let operand_arrival = tok.starts_operand();
        if operand_arrival && !eng.expect_operand {
            eng.implicit_mul()?;
        }

        if !operand_arrival {
            match tok {
                Tok::Add if eng.expect_operand => continue, // unary plus: no-op

                Tok::Sub if eng.expect_operand => {
                    eng.push(Ent::op(opcodes::NEG, BP_UNARY_RHS))?;
                    continue;
                }

                Tok::RParen => {
                    if eng.expect_operand {
                        return Err(Error::UnexpectedToken { pos });
                    }
                    eng.log_pending = false;
                    loop {
                        if eng.sp == 0 {
                            return Err(Error::UnexpectedToken { pos });
                        }
                        // SAFETY: peek strictly below sp (written earlier).
                        if unsafe { eng.stack[eng.sp - 1].assume_init() }.tag == TAG_LPAREN {
                            break;
                        }
                        eng.pop_emit()?;
                    }
                    eng.sp -= 1; // sentinel (shares its frame's nesting unit)
                                 // A function frame directly beneath closes its call.
                                 // SAFETY: as above, peek below sp.
                    if eng.sp > 0
                        && unsafe { eng.stack[eng.sp - 1].assume_init() }.tag == opcodes::FUNC
                    {
                        let id = unsafe { eng.stack[eng.sp - 1].assume_init() }.aux;
                        eng.sp -= 1;
                        eng.nest -= 1;
                        eng.w.emit(&[opcodes::FUNC, id])?;
                        eng.log_pending = id == opcodes::FUNC_LOG;
                    }
                    eng.expect_operand = false;
                }

                Tok::Underscore => {
                    if eng.expect_operand || !eng.log_pending {
                        return Err(Error::UnexpectedToken { pos });
                    }
                    eng.push(Ent {
                        tag: TAG_LOGB,
                        aux: BP_BASE,
                    })?;
                    eng.expect_operand = true;
                }

                Tok::Add => {
                    eng.infix(opcodes::ADD, BP_ADD, true)?;
                    eng.expect_operand = true;
                }
                Tok::Sub => {
                    eng.infix(opcodes::SUB, BP_ADD, true)?;
                    eng.expect_operand = true;
                }
                Tok::Mul | Tok::Div | Tok::Pow if eng.expect_operand => {
                    // Binary-only operators cannot begin an expression.
                    return Err(Error::UnexpectedToken { pos });
                }

                Tok::Mul => {
                    eng.infix(opcodes::MUL, BP_MUL, true)?;
                    eng.expect_operand = true;
                }
                Tok::Div => {
                    eng.infix(opcodes::DIV, BP_MUL, true)?;
                    eng.expect_operand = true;
                }
                Tok::Pow => {
                    eng.infix(opcodes::POW, BP_POW, false)?;
                    eng.expect_operand = true;
                }

                Tok::Num(_) | Tok::Var(_) | Tok::Const(_) | Tok::Func(_) | Tok::LParen => {
                    unreachable!("operand-start tokens handled below")
                }
            }
            continue;
        }

        match tok {
            Tok::Num(v) => {
                let mut ins = [0u8; 9];
                ins[0] = opcodes::NUM;
                ins[1..].copy_from_slice(&v.to_le_bytes());
                eng.w.emitn(ins)?;
                eng.log_pending = false;
                eng.expect_operand = false;
            }
            Tok::Var(name) => {
                eng.w.emitn([opcodes::VAR, name])?;
                eng.log_pending = false;
                eng.expect_operand = false;
            }
            Tok::Const(id) => {
                eng.w.emitn([opcodes::CONST, id])?;
                eng.log_pending = false;
                eng.expect_operand = false;
            }
            Tok::Func(id) => {
                // Happy path: `(` directly follows. Slow path: fully lex
                // the offending token so lexer errors surface before
                // grammar errors, matching v1's token-stream semantics.
                match scanner.skip_peek() {
                    Some(b'(') => scanner.bump(),
                    _ => {
                        return match scanner.next()? {
                            Some((p, _)) => Err(Error::ExpectedLparen { pos: p }),
                            None => Err(Error::ExpectedLparen { pos: eng.eof_pos }),
                        };
                    }
                }
                eng.push(Ent {
                    tag: opcodes::FUNC,
                    aux: id,
                })?;
                eng.push_frame_sentinel();
                eng.expect_operand = true;
            }
            Tok::LParen => {
                eng.push(Ent::LPAREN)?;
                eng.expect_operand = true;
            }
            _ => unreachable!("non-operand tokens routed earlier"),
        }
    }

    eng.finish()
}
