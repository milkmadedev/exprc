//! Programmer-owned limits.
//!
//! Nothing about output size or nesting is hard-coded: a [`Config`]
//! states the bytecode budget and nesting depth *you* are willing to
//! spend, and every bound derives from those two numbers.
//!
//! ```
//! use rpn2::Config;
//!
//! // Default posture (10 KiB output, depth 128):
//! let cfg = Config::new();
//!
//! // A 1 MiB budget with deep nesting allowed:
//! let big = Config::new().output_limit(1024 * 1024).max_depth(2048);
//! assert!(big.scratch_len() > cfg.scratch_len());
//! ```

/// Bytecode output limit used by [`Config::new`] and by the stateless
/// convenience entry point [`crate::parse_into`].
pub const DEFAULT_OUTPUT_LIMIT: usize = 10 * 1024;

/// Nesting depth used by [`Config::new`] and [`crate::parse_into`].
pub const DEFAULT_MAX_DEPTH: u32 = 128;

/// Operator-stack entries needed per nesting level in the worst case
/// (a function frame holds two entries; every other construct one).
const ENTRIES_PER_LEVEL: usize = 2;

/// Slack for entries that can coexist with a maximal nest
/// (pending binary operators of the outermost frame).
const ENTRY_SLACK: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    pub(crate) output_limit: usize,
    pub(crate) max_depth: u32,
}

impl Config {
    /// The default posture: [`DEFAULT_OUTPUT_LIMIT`] bytes of bytecode,
    /// [`DEFAULT_MAX_DEPTH`] nesting.
    pub const fn new() -> Self {
        Self {
            output_limit: DEFAULT_OUTPUT_LIMIT,
            max_depth: DEFAULT_MAX_DEPTH,
        }
    }

    /// Set the bytecode output budget. This — not a library constant —
    /// is the limit compilations are held to. Choose 2 KiB for a tiny
    /// display widget or 1 MiB for a computer-algebra scratchpad.
    ///
    /// Depth also scales, because deeper expressions need more operator
    /// stack; see [`Config::max_depth`].
    pub const fn output_limit(mut self, bytes: usize) -> Self {
        self.output_limit = bytes;
        self
    }

    /// Set the maximum nesting depth. Each unit costs
    /// `2 * ENTRIES_PER_LEVEL` bytes of caller-provided scratch during
    /// compilation; see [`Config::scratch_len`].
    pub const fn max_depth(mut self, depth: u32) -> Self {
        self.max_depth = depth;
        self
    }

    pub const fn get_output_limit(&self) -> usize {
        self.output_limit
    }

    pub const fn get_max_depth(&self) -> u32 {
        self.max_depth
    }

    /// Scratch bytes (`&mut [u8]`) the compiler needs for its operator
    /// stack at this configuration's depth. Pass a buffer of at least
    /// this length to [`crate::compile_into`] / `Session::compile`, or
    /// the call fails with [`Error::ScratchTooSmall`].
    pub const fn scratch_len(&self) -> usize {
        let entries = self.max_depth as usize * ENTRIES_PER_LEVEL + ENTRY_SLACK;
        entries * 2 // each entry: tag byte + aux byte
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}
