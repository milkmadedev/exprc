use core::fmt;

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    UnexpectedByte {
        pos: usize,
        byte: u8,
    },
    UnexpectedToken {
        pos: usize,
    },
    ExpectedLparen {
        pos: usize,
    },
    ExpectedRparen {
        pos: usize,
    },
    UnknownIdentifier {
        pos: usize,
    },
    InvalidNumber {
        pos: usize,
    },
    TooDeep,
    BufferTooSmall,
    OutputLimitExceeded,
    MalformedRpn {
        offset: usize,
    },
    /// The caller-provided operator-stack scratch is smaller than
    /// `Config::scratch_len` for the configured depth.
    ScratchTooSmall {
        needed: usize,
        got: usize,
    },
    /// A variable definition (transitively) references itself.
    RecursiveDefinition {
        var: u8,
    },
    /// An assignment's left-hand side is not a single variable letter.
    BadAssignment {
        pos: usize,
    },
    /// Evaluation ran out of caller-provided value-stack space.
    EvalStackOverflow,
    /// This target has no libm: the function/transcendental cannot be
    /// evaluated here. Compile with the `std` feature for full math.
    FuncUnsupportedOnTarget {
        func: u8,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Error::UnexpectedByte { pos, byte } => {
                write!(f, "unexpected byte 0x{byte:02x} at offset {pos}")
            }
            Error::UnexpectedToken { pos } => write!(f, "unexpected token at offset {pos}"),
            Error::ExpectedLparen { pos } => {
                write!(f, "expected '(' after function name at offset {pos}")
            }
            Error::ExpectedRparen { pos } => {
                write!(
                    f,
                    "expected ')' at offset {pos}, found end of input or other token"
                )
            }
            Error::UnknownIdentifier { pos } => write!(f, "unknown identifier at offset {pos}"),
            Error::InvalidNumber { pos } => write!(f, "invalid number literal at offset {pos}"),
            Error::TooDeep => write!(
                f,
                "expression nesting exceeds MAX_DEPTH ({})",
                crate::MAX_DEPTH
            ),
            Error::BufferTooSmall => write!(
                f,
                "output buffer too small; retry with a larger buffer up to {} bytes",
                crate::MAX_RPN
            ),
            Error::OutputLimitExceeded => write!(
                f,
                "RPN output exceeds the hard limit of {} bytes",
                crate::MAX_RPN
            ),
            Error::MalformedRpn { offset } => write!(f, "malformed RPN stream at offset {offset}"),
            Error::ScratchTooSmall { needed, got } => {
                write!(
                    f,
                    "operator-stack scratch is {got} bytes but the configured depth needs {needed}"
                )
            }
            Error::RecursiveDefinition { var } => {
                write!(f, "variable {} recursively references itself", var as char)
            }
            Error::BadAssignment { pos } => {
                write!(
                    f,
                    "assignment target must be one variable letter at offset {pos}"
                )
            }
            Error::EvalStackOverflow => write!(f, "evaluation value stack exhausted"),
            Error::FuncUnsupportedOnTarget { func } => {
                write!(
                    f,
                    "function/opcode 0x{func:02x} needs a libm (build with feature \"std\")"
                )
            }
        }
    }
}

impl core::error::Error for Error {}
