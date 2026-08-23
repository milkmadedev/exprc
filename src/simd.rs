//! SIMD-assisted byte scanning with graceful degradation.
//!
//! Backend selection is purely compile-time, which keeps the crate
//! `no_std` without any CPUID machinery:
//!
//! * `x86_64`   → SSE2. Baseline for the entire target since 2003; every
//!   x86_64 CPU that can run Rust has it, so no runtime detection is
//!   needed for correctness or speed.
//! * `aarch64`  → NEON. Likewise baseline for the target.
//! * otherwise  → portable scalar loops (also used for sub-16-byte tails
//!   on vector backends).
//!
//! An AVX2 backend is a straightforward extension point (same masks,
//! 32-byte chunks) gated behind `target_feature = "avx2"` for builders
//! who know their deployment fleet.
//!
//! # Safety policy
//!
//! This is the *only* module in the crate permitted to use `unsafe`, and
//! the sole justification is aligned-for-us unaligned loads of chunk
//! buffers that have already been populated through entirely safe
//! `copy_from_slice` calls. Every `unsafe` block below dereferences a
//! pointer into a local `[u8; CHUNK]` array; no input memory is read
//! through raw pointers, so no out-of-bounds access is expressible here.
#![allow(unsafe_code)]

const CHUNK: usize = 16;

/// Hybrid drivers: scalar probe first (typical math input has short
/// tokens and no whitespace), SIMD chunking only once a span proves
/// long enough to amortize it.
#[inline]
#[allow(dead_code)] // unused on targets without a vector backend
fn skip_ws_hybrid(b: &[u8], mut i: usize, vec_skip: fn(&[u8], usize) -> usize) -> usize {
    if i >= b.len() || !matches!(b[i], b' ' | b'\t' | b'\r' | b'\n') {
        return i; // dominant path: no whitespace at all
    }
    while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\r' | b'\n') {
        i += 1;
    }
    vec_skip(b, i)
}

#[inline]
fn span_hybrid(
    b: &[u8],
    i: usize,
    is_member: fn(u8) -> bool,
    vec_span: fn(&[u8], usize) -> usize,
) -> usize {
    let limit = (i + CHUNK).min(b.len());
    let mut j = i;
    while j < limit && is_member(b[j]) {
        j += 1;
    }
    if j < limit {
        return j; // span ended inside the first chunk: done, scalar
    }
    vec_span(b, j)
}

// ---------------------------------------------------------------- scalars

/// Number of leading whitespace bytes starting at `i`.
/// End index of the run of ASCII digits starting at `i`.
fn span_digits_scalar(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    i
}

/// End index of the run of lowercase ASCII letters starting at `i`.
fn span_letters_scalar(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i].is_ascii_lowercase() {
        i += 1;
    }
    i
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
pub(crate) use self::scalar_only::*;

#[cfg(all(target_arch = "x86_64", not(any(target_arch = "aarch64"))))]
pub(crate) use self::sse2::*;

#[cfg(target_arch = "aarch64")]
pub(crate) use self::neon::*;

// ------------------------------------------------------------- scalar-only

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
mod scalar_only {
    #[inline]
    pub(crate) fn skip_ws(b: &[u8], mut i: usize) -> usize {
        while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\r' | b'\n') {
            i += 1;
        }
        i
    }
    #[inline]
    pub(crate) fn span_digits(b: &[u8], i: usize) -> usize {
        super::span_hybrid(b, i, |c| c.is_ascii_digit(), super::span_digits_scalar)
    }
    #[inline]
    pub(crate) fn span_letters(b: &[u8], i: usize) -> usize {
        super::span_hybrid(b, i, |c| c.is_ascii_lowercase(), super::span_letters_scalar)
    }
}

// ------------------------------------------------------------------- sse2

#[cfg(target_arch = "x86_64")]
mod sse2 {
    use core::arch::x86_64::{
        __m128i, _mm_and_si128, _mm_cmpeq_epi8, _mm_cmpgt_epi8, _mm_cmplt_epi8, _mm_loadu_si128,
        _mm_movemask_epi8, _mm_or_si128, _mm_set1_epi8,
    };

    /// Load a staged local chunk into a vector (see module safety notes).
    #[inline(always)]
    fn vec(chunk: &[u8; super::CHUNK]) -> __m128i {
        // SAFETY: pointer derives from a fully initialized local array.
        unsafe { _mm_loadu_si128(chunk.as_ptr().cast()) }
    }

    /// True when every lane of `mask` is set.
    #[inline(always)]
    fn all(mask: __m128i) -> bool {
        // SAFETY: value-only intrinsic.
        unsafe { _mm_movemask_epi8(mask) == 0xFFFF_u32 as i32 }
    }

    #[inline]
    fn skip_ws_vec(b: &[u8], mut i: usize) -> usize {
        let (sp, tab, lf, cr) = unsafe {
            (
                // SAFETY: value-only intrinsics.
                _mm_set1_epi8(b' ' as i8),
                _mm_set1_epi8(b'\t' as i8),
                _mm_set1_epi8(b'\n' as i8),
                _mm_set1_epi8(b'\r' as i8),
            )
        };
        let mut chunk = [0u8; super::CHUNK];
        while b.len() - i >= super::CHUNK {
            chunk.copy_from_slice(&b[i..i + super::CHUNK]);
            let v = vec(&chunk);
            let ws = unsafe {
                _mm_or_si128(
                    _mm_or_si128(_mm_cmpeq_epi8(v, sp), _mm_cmpeq_epi8(v, tab)),
                    _mm_or_si128(_mm_cmpeq_epi8(v, lf), _mm_cmpeq_epi8(v, cr)),
                )
            };
            if !all(ws) {
                break;
            }
            i += super::CHUNK;
        }
        i
    }

    /// All-lanes test for `lo <= byte < hi_excl`. Signed compares are safe
    /// here because every byte >= 0x80 is negative and falls outside any
    /// ASCII range we query.
    #[inline(always)]
    fn range_all(v: __m128i, lo: u8, hi_excl: u8) -> bool {
        // SAFETY: value-only intrinsics.
        unsafe {
            let m = _mm_and_si128(
                _mm_cmpgt_epi8(v, _mm_set1_epi8(lo.wrapping_sub(1) as i8)),
                _mm_cmplt_epi8(v, _mm_set1_epi8(hi_excl as i8)),
            );
            _mm_movemask_epi8(m) == 0xFFFF_u32 as i32
        }
    }

    #[inline]
    fn span_digits_vec(b: &[u8], mut i: usize) -> usize {
        let mut chunk = [0u8; super::CHUNK];
        while b.len() - i >= super::CHUNK {
            chunk.copy_from_slice(&b[i..i + super::CHUNK]);
            if !range_all(vec(&chunk), b'0', b'9' + 1) {
                break;
            }
            i += super::CHUNK;
        }
        super::span_digits_scalar(b, i)
    }

    #[inline]
    pub(crate) fn span_digits(b: &[u8], i: usize) -> usize {
        super::span_hybrid(b, i, |c| c.is_ascii_digit(), span_digits_vec)
    }

    #[inline]
    fn span_letters_vec(b: &[u8], mut i: usize) -> usize {
        let mut chunk = [0u8; super::CHUNK];
        while b.len() - i >= super::CHUNK {
            chunk.copy_from_slice(&b[i..i + super::CHUNK]);
            if !range_all(vec(&chunk), b'a', b'z' + 1) {
                break;
            }
            i += super::CHUNK;
        }
        super::span_letters_scalar(b, i)
    }

    #[inline]
    pub(crate) fn span_letters(b: &[u8], i: usize) -> usize {
        super::span_hybrid(b, i, |c| c.is_ascii_lowercase(), span_letters_vec)
    }

    #[inline]
    pub(crate) fn skip_ws(b: &[u8], i: usize) -> usize {
        super::skip_ws_hybrid(b, i, skip_ws_vec)
    }
}

// ------------------------------------------------------------------- neon

#[cfg(target_arch = "aarch64")]
mod neon {
    use core::arch::aarch64::{
        uint8x16_t, vandq_u8, vceqq_u8, vcleq_u8, vdupq_n_u8, vld1q_u8, vminvq_u8, vorrq_u8,
        vsubq_u8,
    };

    #[inline(always)]
    fn vec(chunk: &[u8; super::CHUNK]) -> uint8x16_t {
        // SAFETY: pointer derives from a fully initialized local array.
        unsafe { vld1q_u8(chunk.as_ptr()) }
    }

    /// Minimum lane == 0xFF ⇔ every lane is all-ones.
    #[inline(always)]
    fn all(mask: uint8x16_t) -> bool {
        vminvq_u8(mask) == 0xFF
    }

    #[inline]
    fn skip_ws_vec(b: &[u8], mut i: usize) -> usize {
        let mut chunk = [0u8; super::CHUNK];
        while b.len() - i >= super::CHUNK {
            chunk.copy_from_slice(&b[i..i + super::CHUNK]);
            let v = vec(&chunk);
            let ws = vorrq_u8(
                vorrq_u8(
                    vceqq_u8(v, vdupq_n_u8(b' ')),
                    vceqq_u8(v, vdupq_n_u8(b'\t')),
                ),
                vorrq_u8(
                    vceqq_u8(v, vdupq_n_u8(b'\n')),
                    vceqq_u8(v, vdupq_n_u8(b'\r')),
                ),
            );
            if !all(ws) {
                break;
            }
            i += super::CHUNK;
        }
        i
    }

    #[inline]
    pub(crate) fn skip_ws(b: &[u8], i: usize) -> usize {
        super::skip_ws_hybrid(b, i, skip_ws_vec)
    }

    /// Unsigned trick: `d = byte - lo` wraps for out-of-range values, so
    /// `d <= span` selects exactly the wanted range.
    #[inline(always)]
    fn in_range(v: uint8x16_t, lo: u8, hi_excl: u8) -> uint8x16_t {
        let d = vsubq_u8(v, vdupq_n_u8(lo));
        vcleq_u8(d, vdupq_n_u8(hi_excl - lo))
    }

    #[inline]
    fn span_digits_vec(b: &[u8], mut i: usize) -> usize {
        let mut chunk = [0u8; super::CHUNK];
        while b.len() - i >= super::CHUNK {
            chunk.copy_from_slice(&b[i..i + super::CHUNK]);
            if !all(in_range(vec(&chunk), b'0', b'9' + 1)) {
                break;
            }
            i += super::CHUNK;
        }
        super::span_digits_scalar(b, i)
    }

    #[inline]
    pub(crate) fn span_digits(b: &[u8], i: usize) -> usize {
        super::span_hybrid(b, i, |c| c.is_ascii_digit(), span_digits_vec)
    }

    #[inline]
    fn span_letters_vec(b: &[u8], mut i: usize) -> usize {
        let mut chunk = [0u8; super::CHUNK];
        while b.len() - i >= super::CHUNK {
            chunk.copy_from_slice(&b[i..i + super::CHUNK]);
            if !all(in_range(vec(&chunk), b'a', b'z' + 1)) {
                break;
            }
            i += super::CHUNK;
        }
        super::span_letters_scalar(b, i)
    }

    #[inline]
    pub(crate) fn span_letters(b: &[u8], i: usize) -> usize {
        super::span_hybrid(b, i, |c| c.is_ascii_lowercase(), span_letters_vec)
    }

    #[inline]
    pub(crate) fn skip_ws(b: &[u8], i: usize) -> usize {
        super::skip_ws_hybrid(b, i, skip_ws_vec)
    }
}
