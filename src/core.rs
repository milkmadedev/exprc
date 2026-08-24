//! The engine: a fully *iterative* precedence-driven emitter.
//!
//! No recursion anywhere; an explicit operator stack lives in
//! caller-provided scratch memory, sized by [`Config::max_depth`]. A
//! `Resolver` may be injected so that variable atoms can expand into
//! previously compiled bodies (substitution) — see [`crate::Session`].
//!
//! # Binding powers
//!
//! | construct        | l_bp | r_bp | notes                           |
//! |------------------|------|------|---------------------------------|
//! | `+ -`            | 10   | 11   | left-assoc                      |
//! | `* /`, implicit  | 20   | 21   | left-assoc; implicit == `*`     |
//! | unary `-`        | —    | 25   | prefix                          |
//! | `^`              | 30   | 30   | right-assoc                     |
//! | `_base` postfix  | 41   | 41   | right-assoc; only after `log()` |

use crate::config::Config;
use crate::error::{Error, Result};
use crate::fold;
use crate::lex::{Scanner, Tok};
use crate::opcodes;
use crate::writer::Writer;

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

/// Variable-body lookup used for substitution. Implemented by
/// [`crate::Session`]; the stateless entry points use [`NoResolve`].
/// Implement this to supply your own definition store.
pub trait Resolve {
    /// Compiled bytecode body of `var`, if one has been defined.
    fn body(&self, var: u8) -> Option<&[u8]>;
}

/// Null resolver: every variable stays a `VAR` opcode.
pub struct NoResolve;

impl Resolve for NoResolve {
    #[inline]
    fn body(&self, _var: u8) -> Option<&[u8]> {
        None
    }
}

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

struct Engine<'b, 'r, R: Resolve> {
    w: Writer<'b>,
    /// Operator stack in caller scratch: `sp` entries of two bytes each
    /// (tag, aux). Only slots below `sp` are read.
    stack: &'r mut [u8],
    sp: usize,
    nest: u32,
    max_depth: u32,
    /// Bit per letter while its definition is being expanded; guards
    /// recursive definitions.
    chain: u32,
    resolver: &'r R,
    /// When false (definition bodies), variables stay `VAR` opcodes so
    /// storage stays lazy; consumer compilations pass true.
    substitute: bool,
    /// Set immediately after a complete `log(...)` call closes; cleared by
    /// every other event. Gates the `_base` postfix.
    log_pending: bool,
    expect_operand: bool,
    eof_pos: usize,
}

impl<'b, 'r, R: Resolve> Engine<'b, 'r, R> {
    fn new(
        w: Writer<'b>,
        stack: &'r mut [u8],
        resolver: &'r R,
        cfg: &Config,
        substitute: bool,
        eof_pos: usize,
    ) -> Self {
        Self {
            w,
            stack,
            sp: 0,
            // Parity with the historical engine: the base expression
            // occupies one depth unit.
            nest: 1,
            max_depth: cfg.get_max_depth(),
            chain: 0,
            resolver,
            substitute,
            log_pending: false,
            expect_operand: true,
            eof_pos,
        }
    }

    #[inline]
    fn cap_entries(&self) -> usize {
        self.stack.len() / 2
    }

    #[inline]
    fn peek(&self) -> Option<Ent> {
        if self.sp == 0 {
            return None;
        }
        let i = (self.sp - 1) * 2;
        Some(Ent {
            tag: self.stack[i],
            aux: self.stack[i + 1],
        })
    }

    #[inline]
    fn place(&mut self, ent: Ent) {
        let i = self.sp * 2;
        self.stack[i] = ent.tag;
        self.stack[i + 1] = ent.aux;
        self.sp += 1;
    }

    #[inline]
    fn push(&mut self, ent: Ent) -> Result<()> {
        let nested = matches!(
            ent.tag,
            TAG_LPAREN | TAG_LOGB | opcodes::POW | opcodes::NEG | opcodes::FUNC
        );
        if nested {
            if self.nest >= self.max_depth {
                return Err(Error::TooDeep);
            }
            self.nest += 1;
        }
        self.log_pending = false;
        if self.sp >= self.cap_entries() {
            return Err(Error::ScratchTooSmall {
                needed: self.cap_entries() * 2 + 2,
                got: self.stack.len(),
            });
        }
        self.place(ent);
        Ok(())
    }

    /// Parenthesis sentinel that belongs to a function frame. It shares
    /// the frame's single nesting level, so it must not bump `nest`.
    #[inline]
    fn push_frame_sentinel(&mut self) -> Result<()> {
        self.log_pending = false;
        if self.sp >= self.cap_entries() {
            return Err(Error::ScratchTooSmall {
                needed: self.cap_entries() * 2 + 2,
                got: self.stack.len(),
            });
        }
        self.place(Ent::LPAREN);
        Ok(())
    }

    /// Pop one entry and emit its instruction. Callers guarantee it is
    /// not a barrier.
    #[inline]
    fn pop_emit(&mut self) -> Result<()> {
        self.sp -= 1;
        let i = self.sp * 2;
        let ent = Ent {
            tag: self.stack[i],
            aux: self.stack[i + 1],
        };
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
        while let Some(top) = self.peek() {
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

    /// Emit a variable atom: substituted-and-folded when defined, a raw
    /// `VAR` opcode otherwise.
    #[inline]
    fn emit_variable(&mut self, name: u8) -> Result<()> {
        let resolved = if self.substitute {
            self.resolver.body(name)
        } else {
            None
        };
        match resolved {
            None => self.w.emitn([opcodes::VAR, name])?,
            Some(body) => {
                let bit = 1u32 << (name - b'a');
                if self.chain & bit != 0 {
                    return Err(Error::RecursiveDefinition { var: name });
                }
                self.chain |= bit;
                fold::emit_var_body(body, self.resolver, &mut self.w, self.chain)?;
                self.chain &= !bit;
            }
        }
        self.log_pending = false;
        self.expect_operand = false;
        Ok(())
    }

    fn finish(mut self) -> Result<usize> {
        if self.expect_operand {
            return Err(Error::UnexpectedToken { pos: self.eof_pos });
        }
        // Drain. A surviving barrier means an unclosed parenthesis.
        while self.sp > 0 {
            if self.peek().is_some_and(|e| e.is_barrier()) {
                return Err(Error::ExpectedRparen { pos: self.eof_pos });
            }
            self.pop_emit()?;
        }
        Ok(self.w.finish())
    }
}

/// Stateless compilation under an explicit [`Config`].
///
/// `stack` must be at least [`Config::scratch_len`] bytes; it is used as
/// the operator stack and its contents are irrelevant on entry.
pub fn compile_into<R: Resolve>(
    cfg: &Config,
    resolver: &R,
    src: &str,
    out: &mut [u8],
    stack: &mut [u8],
) -> Result<usize> {
    compile_into_ex(cfg, resolver, src, out, stack, true)
}

/// Like [`compile_into`] with explicit substitution control: definition
/// bodies are compiled with `substitute = false` so they stay lazy.
pub fn compile_into_ex<R: Resolve>(
    cfg: &Config,
    resolver: &R,
    src: &str,
    out: &mut [u8],
    stack: &mut [u8],
    substitute: bool,
) -> Result<usize> {
    let need = cfg.scratch_len();
    if stack.len() < need {
        return Err(Error::ScratchTooSmall {
            needed: need,
            got: stack.len(),
        });
    }
    let mut scanner = Scanner::new(src);
    let mut eng = Engine::new(
        Writer::with_limit(out, cfg.get_output_limit()),
        stack,
        resolver,
        cfg,
        substitute,
        src.len(),
    );

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
                        let top = eng.peek().ok_or(Error::UnexpectedToken { pos })?;
                        if top.tag == TAG_LPAREN {
                            break;
                        }
                        eng.pop_emit()?;
                    }
                    eng.sp -= 1; // sentinel (shares its frame's nesting unit)
                                 // A function frame directly beneath closes its call.
                    if let Some(f) = eng.peek() {
                        if f.tag == opcodes::FUNC {
                            let id = f.aux;
                            eng.sp -= 1;
                            eng.nest -= 1;
                            eng.w.emitn([opcodes::FUNC, id])?;
                            eng.log_pending = id == opcodes::FUNC_LOG;
                        }
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

                Tok::Mul | Tok::Div | Tok::Pow if eng.expect_operand => {
                    // Binary-only operators cannot begin an expression.
                    return Err(Error::UnexpectedToken { pos });
                }

                Tok::Add => {
                    eng.infix(opcodes::ADD, BP_ADD, true)?;
                    eng.expect_operand = true;
                }
                Tok::Sub => {
                    eng.infix(opcodes::SUB, BP_ADD, true)?;
                    eng.expect_operand = true;
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
            Tok::Var(name) => eng.emit_variable(name)?,
            Tok::Const(id) => {
                eng.w.emitn([opcodes::CONST, id])?;
                eng.log_pending = false;
                eng.expect_operand = false;
            }
            Tok::Func(id) => {
                // Happy path: `(` directly follows. Slow path: fully lex
                // the offending token so lexer errors surface before
                // grammar errors.
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
                eng.push_frame_sentinel()?;
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

/// Convenience wrapper with the default [`Config`], no variables, and an
/// inline operator stack sized for [`crate::DEFAULT_MAX_DEPTH`].
pub fn parse_into(src: &str, out: &mut [u8]) -> Result<usize> {
    let mut stack = [0u8; crate::DEFAULT_MAX_DEPTH as usize * 4 + 32];
    compile_into(&Config::new(), &NoResolve, src, out, &mut stack)
}
