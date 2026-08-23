use crate::error::{Error, Result};
use crate::opcodes::*;
use crate::writer::Writer;
use core::fmt::Write;

pub fn decode_into(rpn: &[u8], out: &mut [u8]) -> Result<usize> {
    let mut w = Writer::new(out);
    let mut i = 0;
    let mut first = true;

    while i < rpn.len() {
        let op = rpn[i];
        i += 1;
        if !first {
            w.push(b' ')?;
        }
        first = false;
        match op {
            NUM => {
                if rpn.len() - i < 8 {
                    return Err(Error::MalformedRpn { offset: i });
                }
                let v = f64::from_le_bytes(rpn[i..i + 8].try_into().unwrap());
                i += 8;
                if !v.is_finite() {
                    return Err(Error::MalformedRpn { offset: i - 8 });
                }
                let mut fw = FmtWriter {
                    w: &mut w,
                    err: None,
                };
                if write!(fw, "{v}").is_err() {
                    return Err(fw.err.take().unwrap());
                }
            }
            VAR => {
                let name = *rpn.get(i).ok_or(Error::MalformedRpn { offset: i })?;
                i += 1;
                if !name.is_ascii_lowercase() {
                    return Err(Error::MalformedRpn { offset: i - 1 });
                }
                w.push(name)?;
            }
            CONST => {
                let id = *rpn.get(i).ok_or(Error::MalformedRpn { offset: i })?;
                i += 1;
                match id {
                    CONST_E => w.extend(b"e")?,
                    CONST_PI => w.extend(b"pi")?,
                    _ => return Err(Error::MalformedRpn { offset: i - 1 }),
                }
            }
            FUNC => {
                let id = *rpn.get(i).ok_or(Error::MalformedRpn { offset: i })?;
                i += 1;
                let name: &[u8] = match id {
                    FUNC_SIN => b"sin",
                    FUNC_COS => b"cos",
                    FUNC_TAN => b"tan",
                    FUNC_ASIN => b"asin",
                    FUNC_ACOS => b"acos",
                    FUNC_ATAN => b"atan",
                    FUNC_LN => b"ln",
                    FUNC_LOG => b"log",
                    FUNC_LOGB => b"logb",
                    _ => return Err(Error::MalformedRpn { offset: i - 1 }),
                };
                w.extend(name)?;
            }
            ADD => w.push(b'+')?,
            SUB => w.push(b'-')?,
            MUL => w.push(b'*')?,
            DIV => w.push(b'/')?,
            POW => w.push(b'^')?,
            NEG => w.extend(b"neg")?,
            _ => return Err(Error::MalformedRpn { offset: i - 1 }),
        }
    }
    Ok(w.finish())
}

struct FmtWriter<'a, 'b> {
    w: &'a mut Writer<'b>,
    err: Option<Error>,
}

impl Write for FmtWriter<'_, '_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        match self.w.extend(s.as_bytes()) {
            Ok(()) => Ok(()),
            Err(e) => {
                self.err = Some(e);
                Err(core::fmt::Error)
            }
        }
    }
}
