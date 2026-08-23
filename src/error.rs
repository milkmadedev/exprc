use core::fmt;

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    UnexpectedByte { pos: usize, byte: u8 },
    UnexpectedToken { pos: usize },
    ExpectedLparen { pos: usize },
    ExpectedRparen { pos: usize },
    UnknownIdentifier { pos: usize },
    InvalidNumber { pos: usize },
    TooDeep,
    BufferTooSmall,
    OutputLimitExceeded,
    MalformedRpn { offset: usize },
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
        }
    }
}

impl core::error::Error for Error {}
