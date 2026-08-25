//! The lexer.
//!
//! Produces tokens in a single left-to-right scan of the input. Token
//! kind is decided by one lookup into the byte-class table; number and
//! identifier spans are measured with hybrid scalar/SIMD scans (see
//! [`crate::simd`]).
//!
//! Number literals follow the shapes accepted by [`crate::num`]:
//! `[digits][.digits]([eE][+-]digits)?`. Identifiers are matched as
//! whole words — reserved words name functions and constants, any other
//! single letter is a variable, and multi-letter unknown words are an
//! error rather than implicit multiplication.

use crate::class::{class, Class};
use crate::error::{Error, Result};
use crate::num;
use crate::opcodes;
use crate::simd;

pub(crate) struct Scanner<'s> {
    src: &'s [u8],
    pos: usize,
}

pub(crate) enum Tok {
    Num(f64),
    Var(u8),
    Const(u8),
    Func(u8),
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Underscore,
    LParen,
    RParen,
}

impl Tok {
    /// True for tokens that may begin an operand (drives implicit
    /// multiplication detection).
    pub(crate) fn starts_operand(&self) -> bool {
        matches!(
            self,
            Tok::Num(_) | Tok::Var(_) | Tok::Const(_) | Tok::Func(_) | Tok::LParen
        )
    }
}

impl<'s> Scanner<'s> {
    pub(crate) fn new(src: &'s str) -> Self {
        Self {
            src: src.as_bytes(),
            pos: 0,
        }
    }

    #[inline]
    fn skip_ws(&mut self) {
        self.pos = simd::skip_ws(self.src, self.pos);
    }

    /// Skip whitespace and peek the next byte without lexing a token.
    #[inline(always)]
    pub(crate) fn skip_peek(&mut self) -> Option<u8> {
        self.skip_ws();
        self.src.get(self.pos).copied()
    }

    #[inline(always)]
    pub(crate) fn bump(&mut self) {
        self.pos += 1;
    }

    pub(crate) fn next(&mut self) -> Result<Option<(usize, Tok)>> {
        self.skip_ws();
        let start = self.pos;
        let Some(&b) = self.src.get(start) else {
            return Ok(None);
        };
        match class(b) {
            Class::DIGIT => self.scan_number(start).map(Some),
            // A leading dot only starts a number when a digit follows.
            Class::DOT if matches!(self.src.get(start + 1), Some(c) if c.is_ascii_digit()) => {
                self.scan_number(start).map(Some)
            }
            Class::LETTER => self.scan_ident(start).map(Some),
            Class::ADD => Ok(self.single(start, Tok::Add)),
            Class::SUB => Ok(self.single(start, Tok::Sub)),
            Class::MUL => Ok(self.single(start, Tok::Mul)),
            Class::DIV => Ok(self.single(start, Tok::Div)),
            Class::POW => Ok(self.single(start, Tok::Pow)),
            Class::UNDER => Ok(self.single(start, Tok::Underscore)),
            Class::LPAREN => Ok(self.single(start, Tok::LParen)),
            Class::RPAREN => Ok(self.single(start, Tok::RParen)),
            _ => Err(Error::UnexpectedByte {
                pos: start,
                byte: b,
            }),
        }
    }

    #[inline]
    fn single(&mut self, start: usize, tok: Tok) -> Option<(usize, Tok)> {
        self.pos = start + 1;
        Some((start, tok))
    }

    fn scan_number(&mut self, start: usize) -> Result<(usize, Tok)> {
        let src = self.src;
        let mut end = simd::span_digits(src, start);

        // Fraction: once entered via a leading digit, the dot is consumed
        // even without following digits (`5.` == 5). A leading dot needs a
        // digit after it (guaranteed by the entry condition above).
        if src.get(end) == Some(&b'.') {
            end += 1;
            end = simd::span_digits(src, end);
        }

        // Exponent: only when at least one digit follows the (optional)
        // sign; otherwise `e` terminates this literal and is lexed as the
        // constant `e` next.
        if matches!(src.get(end), Some(b'e' | b'E')) {
            let mut j = end + 1;
            if matches!(src.get(j), Some(b'+' | b'-')) {
                j += 1;
            }
            if matches!(src.get(j), Some(c) if c.is_ascii_digit()) {
                end = simd::span_digits(src, j);
            }
        }

        let slice = &src[start..end];
        let v = num::try_fast(slice)
            .or_else(|| num::fallback(slice))
            .ok_or(Error::InvalidNumber { pos: start })?;
        if !v.is_finite() {
            return Err(Error::InvalidNumber { pos: start });
        }
        self.pos = end;
        Ok((start, Tok::Num(v)))
    }

    fn scan_ident(&mut self, start: usize) -> Result<(usize, Tok)> {
        let end = simd::span_letters(self.src, start);
        self.pos = end;
        let word = &self.src[start..end];
        let tok = match word {
            b"sin" => Tok::Func(opcodes::FUNC_SIN),
            b"cos" => Tok::Func(opcodes::FUNC_COS),
            b"tan" => Tok::Func(opcodes::FUNC_TAN),
            b"asin" => Tok::Func(opcodes::FUNC_ASIN),
            b"acos" => Tok::Func(opcodes::FUNC_ACOS),
            b"atan" => Tok::Func(opcodes::FUNC_ATAN),
            b"ln" => Tok::Func(opcodes::FUNC_LN),
            b"log" => Tok::Func(opcodes::FUNC_LOG),
            b"e" => Tok::Const(opcodes::CONST_E),
            b"pi" => Tok::Const(opcodes::CONST_PI),
            _ if word.len() == 1 => Tok::Var(word[0]),
            _ => return Err(Error::UnknownIdentifier { pos: start }),
        };
        Ok((start, tok))
    }
}
