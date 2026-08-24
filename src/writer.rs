//! Output writer: one capacity check per *instruction*, not per byte.
//!
//! v1 paid a bounds check for every single byte emitted (9 checks per
//! f64 literal). Here [`Writer::emit`] reserves the whole instruction
//! payload at once — a single comparison — then copies via
//! `copy_from_slice`, which lowers to memcpy for the fixed-size cases.

use crate::error::{Error, Result};
pub(crate) struct Writer<'b> {
    buf: &'b mut [u8],
    len: usize,
    limit: usize,
}

impl<'b> Writer<'b> {
    pub(crate) fn new(buf: &'b mut [u8]) -> Self {
        Self {
            buf,
            len: 0,
            limit: crate::DEFAULT_OUTPUT_LIMIT,
        }
    }

    pub(crate) fn with_limit(buf: &'b mut [u8], limit: usize) -> Self {
        Self { buf, len: 0, limit }
    }

    /// Reserve `n` bytes and return the writable window.
    #[inline]
    pub(crate) fn reserve(&mut self, n: usize) -> Result<&mut [u8]> {
        // Effective capacity is min(buffer, configured limit): exceeding
        // the limit is OutputLimitExceeded even when the buffer is huge.
        let eff = self.buf.len().min(self.limit);
        if self.len + n <= eff {
            Ok(&mut self.buf[self.len..self.len + n])
        } else {
            Err(overflow(self.buf.len(), self.limit))
        }
    }

    #[inline]
    pub(crate) fn emit(&mut self, bytes: &[u8]) -> Result<()> {
        let dst = self.reserve(bytes.len())?;
        dst.copy_from_slice(bytes);
        self.len += bytes.len();
        Ok(())
    }

    /// Const-length emit: `N` known at the call site lowers the copy to
    /// scalar stores instead of a memcpy call.
    #[inline]
    pub(crate) fn emitn<const N: usize>(&mut self, bytes: [u8; N]) -> Result<()> {
        let dst = self.reserve(N)?;
        dst.copy_from_slice(&bytes);
        self.len += N;
        Ok(())
    }

    #[inline]
    pub(crate) fn push(&mut self, byte: u8) -> Result<()> {
        self.emit(&[byte])
    }

    pub(crate) fn extend(&mut self, bytes: &[u8]) -> Result<()> {
        self.emit(bytes)
    }

    pub(crate) fn finish(&self) -> usize {
        self.len
    }
}

/// `BufferTooSmall` when the caller's buffer is below the configured
/// budget (retry bigger); `OutputLimitExceeded` when the expression
/// inherently exceeds the configured budget.
pub(crate) fn overflow(cap: usize, limit: usize) -> Error {
    if cap >= limit {
        Error::OutputLimitExceeded
    } else {
        Error::BufferTooSmall
    }
}
