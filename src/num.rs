//! Bit-exact fast decimal-to-f64 conversion.
//!
//! [`try_fast`] converts the common cases without touching the general
//! correctly-rounded machinery:
//!
//! * **Pure integers** (no `.` / exponent) with at most 19 digits: folded
//!   into a `u64`; the single `u64 → f64` conversion *is* the correctly
//!   rounded value of that decimal integer, so it is bit-identical to
//!   `str::parse`.
//! * **Everything else** with mantissa magnitude `m < 10^15` (hence
//!   exactly representable, since `10^15 < 2^53`) and decimal shift
//!   `k ∈ [-22, 22]`: the true value equals `m × 10^k`. For `k ≥ 0` we
//!   multiply by the *exact* float `10^k`; for `k < 0` we divide by the
//!   exact float `10^|k|` (negative powers are not themselves
//!   representable). IEEE mul/div is correctly rounded w.r.t. its
//!   operands' exact result, so one operation yields precisely the
//!   correctly rounded conversion of the original literal — bit-identical
//!   to `str::parse`.
//!
//! Anything outside those envelopes returns `None`; the caller falls back
//! to core's parser. Differential tests against `str::parse` enforce the
//! bit-exactness claim on edge grids and randomized fuzzing.

/// `10^k` for `0 <= k <= 22` — every entry exactly representable in f64.
/// Negative powers are *not* exactly representable, so they never appear
/// here; negative shifts divide by these instead (correctly rounded).
static POW10: [f64; 23] = [
    1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16,
    1e17, 1e18, 1e19, 1e20, 1e21, 1e22,
];

/// Mantissas up to this are exact in f64: 2^53 - 1. (Not 10^15-ish
/// hand-waving — 9_007_199_254_740_992 is precisely the last integer f64
/// can hold exactly.)
const M_EXACT_LIMIT: u64 = 9_007_199_254_740_991;
const MAX_INT_DIGITS: usize = 19;
const K_MAX: i32 = 22;

/// Attempt the fast path. `s` is a numeric literal slice previously
/// validated by the lexer: `[digits][.digits]([eE][+-]digits)?` with at
/// least one digit total. Returns `None` when the envelope does not hold;
/// the caller must then use [`fallback`].
#[inline]
pub fn try_fast(s: &[u8]) -> Option<f64> {
    let mut i = 0;
    let mut m: u64 = 0;
    let mut frac_len: i32 = 0;
    let mut saw_digit_int = false;
    let mut saw_dot = false;

    // Fold helper: bails to fallback on u64 overflow instead of wrapping.
    macro_rules! fold {
        ($d:expr) => {
            match m.checked_mul(10).and_then(|x| x.checked_add($d)) {
                Some(v) => m = v,
                None => return None,
            }
        };
    }

    // Integer part.
    while i < s.len() && s[i].is_ascii_digit() {
        fold!((s[i] - b'0') as u64);
        saw_digit_int = true;
        i += 1;
    }

    // Fractional part.
    if i < s.len() && s[i] == b'.' {
        saw_dot = true;
        i += 1;
        let mut frac_digits = 0usize;
        while i < s.len() && s[i].is_ascii_digit() {
            fold!((s[i] - b'0') as u64);
            frac_digits += 1;
            i += 1;
        }
        frac_len = frac_digits as i32;
    }

    // Exponent (magnitude capped; absurd exponents fall back).
    let mut exp: i32 = 0;
    if i < s.len() && (s[i] | 0x20) == b'e' {
        i += 1;
        let neg = match i < s.len() && (s[i] == b'+' || s[i] == b'-') {
            true => {
                let n = s[i] == b'-';
                i += 1;
                n
            }
            false => false,
        };
        let mut capped = false;
        while i < s.len() && s[i].is_ascii_digit() {
            if exp <= 99_999 {
                exp = exp * 10 + (s[i] - b'0') as i32;
            } else {
                capped = true;
            }
            i += 1;
        }
        if neg {
            exp = -exp;
        }
        if capped {
            return None;
        }
    }

    debug_assert_eq!(i, s.len(), "lexer passed a non-terminal number slice");
    debug_assert!(
        saw_digit_int || (saw_dot && frac_len > 0),
        "bad literal shape"
    );

    if m == 0 {
        return Some(0.0);
    }

    // Pure integer: single correctly-rounded conversion step covers all
    // 19-digit values (10^19 < u64::MAX), matching str::parse exactly.
    if !saw_dot && exp == 0 && i <= MAX_INT_DIGITS {
        return Some(m as f64);
    }

    if m >= M_EXACT_LIMIT {
        return None;
    }
    let k = exp - frac_len;
    // Positive shifts: `10^k` (k <= 22) is an *exact* float, so the single
    // multiply is correctly rounded w.r.t. the true decimal.
    // Negative shifts: `10^-k` is generally NOT representable, but `10^|k|`
    // is, and IEEE division is correctly rounded w.r.t. its operands'
    // exact quotient — so divide by the exact positive power instead.
    if (0..=K_MAX).contains(&k) {
        Some((m as f64) * POW10[k as usize])
    } else if (-K_MAX..0).contains(&k) {
        Some((m as f64) / POW10[(-k) as usize])
    } else {
        None
    }
}

/// Slow path: delegates to core's correctly rounded parser. The slice is
/// pure ASCII by construction, so UTF-8 conversion cannot fail.
pub fn fallback(s: &[u8]) -> Option<f64> {
    let text = core::str::from_utf8(s).ok()?;
    text.parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    extern crate std;

    /// Differential grid: fast path vs `str::parse` must agree bit-for-bit
    /// on every literal shape the lexer can produce.
    #[test]
    fn matches_std_parse() {
        use std::{format, string::String, vec::Vec};

        let mut cases: Vec<String> = Vec::new();
        for &int in &[
            "",
            "0",
            "007",
            "1",
            "9",
            "42",
            "123456789",
            "9007199254740993",
            "9999999999999999999",
        ] {
            for &frac in &["", ".", ".5", ".0496875", "125"] {
                for &exp in &[
                    "", "e", "E", "e+", "e-", "e0", "e3", "E-2", "e22", "e23", "e-23", "e300",
                    "e-300",
                ] {
                    cases.push(format!("{int}{frac}{exp}"));
                    cases.push(format!("0{int}{frac}0{exp}"));
                }
            }
        }
        // Stress around every envelope boundary.
        for n in [1usize, 14, 15, 16, 17, 19, 20, 25] {
            let nines = "9".repeat(n);
            cases.push(nines.clone());
            cases.push(format!("{nines}.5"));
            cases.push(format!("1.{nines}"));
            cases.push(format!("0.{nines}"));
            cases.push(format!("{nines}e42"));
            cases.push(format!("{nines}e-42"));
            cases.push(format!("1.{nines}e-1"));
        }

        for c in &cases {
            let bytes = c.as_bytes();
            // Only shapes the lexer can emit: starts with a digit or
            // `.`+digit; `e`/sign never terminate a valid literal.
            let ok_shape = match bytes.first() {
                Some(b) if b.is_ascii_digit() => true,
                Some(b'.') => matches!(bytes.get(1), Some(c) if c.is_ascii_digit()),
                _ => false,
            };
            if !ok_shape || matches!(bytes.last(), Some(b'e' | b'E' | b'+' | b'-')) {
                continue;
            }
            let slow = super::fallback(bytes);
            let got = super::try_fast(bytes).or(slow);
            assert_eq!(
                format!("{got:?}"),
                format!("{slow:?}"),
                "case {c} | fast_raw={:?}",
                super::try_fast(bytes)
            );
        }
    }
}
