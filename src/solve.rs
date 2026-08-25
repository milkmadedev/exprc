//! Numeric equation solving: `lhs = rhs`, find `var`.
//!
//! The honest kind of solver — the one every physical calculator ships.
//! Given compiled bytecodes for both sides, sample the difference over
//! a range, bracket every sign change, and bisect to machine precision.
//! Exact closed-form algebra (a CAS) is explicitly out of scope; what
//! you get are the *numbers* a calculator would give you.
//!
//! Works on continuous functions over the chosen range — polynomials,
//! anything built from `+ - * / ^ neg` — including expressions using
//! [`crate::Session`] substitutions, since those fold into plain
//! bytecode before reaching here.

use crate::error::{Error, Result};
use crate::eval::{eval, Vars};

/// Solve `lhs(var) = rhs(var)` for `var` over `[lo, hi]`.
///
/// * `steps`: uniform samples used to bracket sign changes (≥ 2).
///   Use more for wiggly functions: `steps ≥ 64` finds all real roots
///   of typical polynomials; even roots touching zero tangentially may
///   need thousands.
/// * `found`: caller-owned output slice; up to `found.len()` distinct
///   roots are written, returned count is the number written. Roots are
///   ascending, deduplicated near-equal duplicates.
/// * Other variables come from `vars`; the solved `var` overrides any
///   value stored there.
///
/// Tangential (even-multiplicity) roots never change sign and are
/// invisible to bracketing — the classic calculator limitation,
/// inherited deliberately rather than hidden.
/// Scan parameters for [`solve`].
#[derive(Clone, Copy, Debug)]
pub struct SolveCfg {
    /// Search window `[lo, hi]`, finite, `lo < hi`.
    pub range: (f64, f64),
    /// Uniform samples used to bracket sign changes (>= 2). More samples
    /// find more roots of wiggly functions; 512-4096 suits polynomials.
    pub steps: u32,
}

pub fn solve(
    lhs: &[u8],
    rhs: &[u8],
    var: u8,
    vars: &Vars,
    cfg: SolveCfg,
    stack: &mut [f64],
    found: &mut [f64],
) -> Result<usize> {
    let SolveCfg { range, steps } = cfg;
    if !var.is_ascii_lowercase() {
        return Err(Error::BadAssignment { pos: 0 });
    }
    let (lo, hi) = range;
    if !(lo.is_finite() && hi.is_finite()) || steps < 2 || hi <= lo {
        return Err(Error::MalformedRpn { offset: 0 });
    }

    let d = Diff {
        lhs,
        rhs,
        vars: *vars,
        var,
    };

    let step = (hi - lo) / steps as f64;
    let mut found_n = 0usize;
    let mut x_prev = lo;
    let mut f_prev = d.at(lo, stack)?;

    for s in 1..=steps {
        let x = lo + step * s as f64;
        let x = if s == steps { hi } else { x }; // exact endpoint
        let f = d.at(x, stack)?;

        let hit = if f == 0.0 {
            Some(x)
        } else if f_prev != 0.0 && f.signum() != f_prev.signum() {
            Some(d.bisect(x_prev, x, f_prev, stack)?)
        } else {
            None
        };
        if let Some(root) = hit {
            // Deduplicate near-equal neighbors (bracket edges landing on
            // the same root twice).
            let dup = found_n > 0 && (found[found_n - 1] - root).abs() <= 1e-7 * (1.0 + root.abs());
            if !dup && found_n < found.len() {
                found[found_n] = root;
                found_n += 1;
                if found_n == found.len() {
                    break;
                }
            }
        }
        x_prev = x;
        f_prev = f;
    }
    Ok(found_n)
}

/// `lhs(x) - rhs(x)` over a working copy of the variable map.
struct Diff<'a> {
    lhs: &'a [u8],
    rhs: &'a [u8],
    vars: Vars,
    var: u8,
}

impl Diff<'_> {
    #[inline]
    fn at(&self, x: f64, stack: &mut [f64]) -> Result<f64> {
        let mut w = self.vars;
        w.set(self.var, x);
        // Sequential reuse of the caller's scratch; eval leaves it clean.
        let a = eval(self.lhs, &w, stack)?;
        let b = eval(self.rhs, &w, stack)?;
        Ok(a - b)
    }

    /// Bisection to full f64 convergence on a known sign change.
    fn bisect(&self, mut a: f64, mut b: f64, fa: f64, stack: &mut [f64]) -> Result<f64> {
        debug_assert!(fa != 0.0);
        loop {
            let m = 0.5 * (a + b);
            if m == a || m == b {
                return Ok(m); // interval collapsed: converged
            }
            let fm = self.at(m, stack)?;
            if fm == 0.0 {
                return Ok(m);
            }
            if fm.signum() == fa.signum() {
                a = m;
            } else {
                b = m;
            }
        }
    }
}
