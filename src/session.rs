//! Persistent variables: define once, substitute everywhere.
//!
//! A [`Session`] owns up to 26 single-letter variable definitions. Each
//! definition is a compiled bytecode body; using that variable in any
//! later compilation splices the body in place (recursively, with cycle
//! rejection), and definitions that are fully numeric collapse to a
//! single literal — the "solve" behavior:
//!
//! ```
//! use exprc::{decode_into, Config, Session};
//!
//! let mut s = Session::<256>::new(Config::new());
//! let mut stack = [0u8; Config::new().scratch_len()];
//!
//! // x = 2*a + b
//! let n = s.define(b'x', "2a+b", &mut stack).unwrap();
//! assert!(n > 0);
//!
//! let mut out = [0u8; 256];
//! let n = s.compile("x^2", &mut out, &mut stack).unwrap();
//! let mut text = [0u8; 1024];
//! let m = decode_into(&out[..n], &mut text).unwrap();
//! // Storage is lazy: undefined a/b stay VAR opcodes.
//! assert_eq!(&text[..m], b"2 a * b + 2 ^");
//!
//! // Once a and b are known numbers, x solves itself:
//! s.define(b'a', "3", &mut stack).unwrap();
//! s.define(b'b', "4", &mut stack).unwrap();
//! let n = s.compile("x", &mut out, &mut stack).unwrap();
//! let m = decode_into(&out[..n], &mut text).unwrap();
//! assert_eq!(&text[..m], b"10");
//! ```
//!
//! Definitions are stored **lazily**: bodies keep their `VAR` opcodes,
//! so redefining `a` changes what later compilations of `x` produce —
//! everything stays tweakable.

use crate::config::Config;
use crate::core::{compile_into, compile_into_ex, Resolve};
use crate::error::{Error, Result};

/// Outcome of [`Session::compile_line`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Line {
    /// The line was `x = expr`; the definition's body is `len` bytes.
    Defined { var: u8, len: usize },
    /// The line was a plain expression compiling to `len` bytes.
    Expression(usize),
}

/// Persistent single-letter variable store with substitution.
///
/// `VAR_BYTES` is each definition's storage budget (default 256 bytes).
/// Total session size is `26 * (VAR_BYTES + 2)` bytes — inline arrays,
/// no allocation anywhere.
pub struct Session<const VAR_BYTES: usize = 256> {
    cfg: Config,
    lens: [u16; 26],
    bodies: [[u8; VAR_BYTES]; 26],
}

impl<const VAR_BYTES: usize> Session<VAR_BYTES> {
    pub fn new(cfg: Config) -> Self {
        Self {
            cfg,
            lens: [0; 26],
            bodies: [[0; VAR_BYTES]; 26],
        }
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// True when `var` has a definition (any letter `a..=z`).
    pub fn is_defined(&self, var: u8) -> bool {
        var.is_ascii_lowercase() && self.lens[(var - b'a') as usize] > 0
    }

    /// Number of currently defined variables.
    pub fn defined_count(&self) -> usize {
        self.lens.iter().filter(|&&l| l > 0).count()
    }

    /// Compile `src` as the body of `var`, replacing any previous
    /// definition ("tweak it"). Recursive definitions are rejected and
    /// leave the previous definition intact.
    ///
    /// Returns the stored body length in bytes.
    pub fn define(&mut self, var: u8, src: &str, stack: &mut [u8]) -> Result<usize> {
        if !var.is_ascii_lowercase() {
            return Err(Error::BadAssignment { pos: 0 });
        }
        let idx = (var - b'a') as usize;

        // Compile into a staging buffer so the session stays immutably
        // borrowed while its own definitions may be substituted.
        let mut tmp = [0u8; VAR_BYTES];
        // Lazy storage: keep VAR opcodes intact in the definition body.
        let n = compile_into_ex(&self.cfg, self, src, &mut tmp, stack, false)?;

        struct Probe<'a, const V: usize> {
            s: &'a Session<V>,
            var: u8,
            body: &'a [u8],
        }
        impl<const V: usize> Resolve for Probe<'_, V> {
            fn body(&self, v: u8) -> Option<&[u8]> {
                if v == self.var {
                    Some(self.body)
                } else {
                    self.s.body(v)
                }
            }
        }
        crate::fold::definition_acyclic(
            &Probe {
                s: self,
                var,
                body: &tmp[..n],
            },
            var,
        )?;

        self.bodies[idx][..n].copy_from_slice(&tmp[..n]);
        self.lens[idx] = n as u16;
        Ok(n)
    }

    /// Remove a definition; later uses of `var` become plain symbols.
    pub fn undefine(&mut self, var: u8) -> bool {
        if !var.is_ascii_lowercase() {
            return false;
        }
        let idx = (var - b'a') as usize;
        let had = self.lens[idx] > 0;
        self.lens[idx] = 0;
        had
    }

    /// Compile an expression under current definitions.
    pub fn compile(&self, src: &str, out: &mut [u8], stack: &mut [u8]) -> Result<usize> {
        compile_into(&self.cfg, self, src, out, stack)
    }

    /// One source line, calculator-style:
    ///
    /// * `"x = 2a+b"` → defines `x` → [`Line::Defined`]
    /// * anything else → [`Line::Expression`] with its bytecode length
    ///
    /// An `=` whose target is not a single letter is [`Error::BadAssignment`].
    pub fn compile_line(&mut self, line: &str, out: &mut [u8], stack: &mut [u8]) -> Result<Line> {
        if let Some((var, rhs)) = split_assignment(line) {
            let len = self.define(var, rhs, stack)?;
            Ok(Line::Defined { var, len })
        } else {
            // Surface '=' misuse precisely when present but malformed.
            let t = line.trim_start();
            if let Some(eq) = t.find('=') {
                let before = t[..eq].trim_end();
                let bad = !(before.len() == 1 && before.as_bytes()[0].is_ascii_lowercase());
                if bad {
                    return Err(Error::BadAssignment {
                        pos: line.len() - line.find('=').unwrap(),
                    });
                }
            }
            let n = self.compile(line, out, stack)?;
            Ok(Line::Expression(n))
        }
    }
}

/// Split `"x = rest"` into `(b'x', "rest")`; `None` without an assignment
/// prefix. Only exact single-letter targets count.
pub fn split_assignment(line: &str) -> Option<(u8, &str)> {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    let var = *b.get(i)?;
    if !var.is_ascii_lowercase() {
        return None;
    }
    let mut j = i + 1;
    while j < b.len() && (b[j] == b' ' || b[j] == b'\t') {
        j += 1;
    }
    if b.get(j) == Some(&b'=') {
        Some((var, &line[j + 1..]))
    } else {
        None
    }
}

impl<const VAR_BYTES: usize> Resolve for Session<VAR_BYTES> {
    fn body(&self, var: u8) -> Option<&[u8]> {
        let idx = (var - b'a') as usize;
        let len = *self.lens.get(idx)?;
        if len == 0 {
            None
        } else {
            Some(&self.bodies[idx][..len as usize])
        }
    }
}
