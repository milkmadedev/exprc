use rpn2::{decode_into, parse_into, Error, MAX_RPN};

fn rpn(src: &str) -> Result<String, Error> {
    let mut buf = [0u8; MAX_RPN];
    let n = parse_into(src, &mut buf)?;
    let mut out = [0u8; 4 * MAX_RPN];
    let m = decode_into(&buf[..n], &mut out)?;
    Ok(core::str::from_utf8(&out[..m]).unwrap().to_string())
}

#[test]
fn arithmetic_and_precedence() {
    assert_eq!(rpn("3x+2").unwrap(), "3 x * 2 +");
    assert_eq!(rpn("3+x*2").unwrap(), "3 x 2 * +");
    assert_eq!(rpn("(3+x)*2").unwrap(), "3 x + 2 *");
    assert_eq!(rpn("2^3^2").unwrap(), "2 3 2 ^ ^");
    assert_eq!(rpn("-x^2").unwrap(), "x 2 ^ neg");
    assert_eq!(rpn("(-x)^2").unwrap(), "x neg 2 ^");
    assert_eq!(rpn("1/2/4").unwrap(), "1 2 / 4 /");
    assert_eq!(rpn("+5--6").unwrap(), "5 6 neg -");
}

#[test]
fn implicit_multiplication() {
    assert_eq!(rpn("3x+2").unwrap(), "3 x * 2 +");
    assert_eq!(rpn("2sin(x)^2").unwrap(), "2 x sin 2 ^ *");
    assert_eq!(rpn("(1+2)x").unwrap(), "1 2 + x *");
    assert_eq!(rpn("(a)(b)").unwrap(), "a b *");
    assert_eq!(rpn("2pi").unwrap(), "2 pi *");
    assert_eq!(rpn("2e").unwrap(), "2 e *");
    assert_eq!(rpn(".5x").unwrap(), "0.5 x *");
    assert_eq!(rpn("-2x").unwrap(), "2 neg x *");
    assert_eq!(rpn("2pi x").unwrap(), "2 pi * x *");
}

#[test]
fn functions() {
    assert_eq!(rpn("atan(2)").unwrap(), "2 atan");
    assert_eq!(
        rpn("sin(x)+cos(x)+tan(x)").unwrap(),
        "x sin x cos + x tan +"
    );
    assert_eq!(rpn("asin(x)acos(x)").unwrap(), "x asin x acos *");
    assert_eq!(rpn("ln(e)").unwrap(), "e ln");
    assert_eq!(rpn("log(100)").unwrap(), "100 log");
    assert_eq!(rpn("sin ( x )").unwrap(), "x sin");
}

#[test]
fn log_with_base() {
    assert_eq!(rpn("log(2)_2").unwrap(), "2 log 2 logb");
    assert_eq!(rpn("log(x)_10+1").unwrap(), "x log 10 logb 1 +");
    assert_eq!(rpn("log(log(8)_2)_e").unwrap(), "8 log 2 logb log e logb");
    assert_eq!(rpn("2^log(x)_2").unwrap(), "2 x log 2 logb ^");
}

#[test]
fn numbers() {
    assert_eq!(rpn("1e3").unwrap(), "1000");
    assert_eq!(rpn("1.5E-2").unwrap(), "0.015");
    assert_eq!(rpn("007").unwrap(), "7");
}

#[test]
fn errors_are_positioned() {
    assert_eq!(rpn(""), Err(Error::UnexpectedToken { pos: 0 }));
    assert_eq!(rpn("sin"), Err(Error::ExpectedLparen { pos: 3 }));
    assert_eq!(rpn("sinx"), Err(Error::UnknownIdentifier { pos: 0 }));
    assert_eq!(rpn("3+"), Err(Error::UnexpectedToken { pos: 2 }));
    assert_eq!(rpn("(1+2"), Err(Error::ExpectedRparen { pos: 4 }));
    assert_eq!(rpn("1)",), Err(Error::UnexpectedToken { pos: 1 }));
    assert_eq!(rpn("cos(x)_2"), Err(Error::UnexpectedToken { pos: 6 }));
    assert_eq!(rpn("*3"), Err(Error::UnexpectedToken { pos: 0 }));
    assert_eq!(
        rpn("3@2"),
        Err(Error::UnexpectedByte { pos: 1, byte: b'@' })
    );
    assert_eq!(rpn("1e9999"), Err(Error::InvalidNumber { pos: 0 }));
}

#[test]
fn depth_limit() {
    let deep = format!(
        "{}x{}",
        "(".repeat(MAX_DEPTH_NEST),
        ")".repeat(MAX_DEPTH_NEST)
    );
    assert_eq!(rpn(&deep), Err(Error::TooDeep));
    let ok = format!("{}x{}", "(".repeat(64), ")".repeat(64));
    assert!(rpn(&ok).is_ok());
}

const MAX_DEPTH_NEST: usize = pratt_rpn::MAX_DEPTH as usize + 1;

#[test]
fn output_limits() {
    let mut huge = vec![0u8; MAX_RPN * 4];
    let expr = format!("{}1", "1*".repeat(20_000));
    assert_eq!(
        parse_into(&expr, &mut huge),
        Err(Error::OutputLimitExceeded)
    );

    let mut tiny = [0u8; 3];
    assert_eq!(parse_into("123456", &mut tiny), Err(Error::BufferTooSmall));

    let mut exact = [0u8; 9];
    assert_eq!(parse_into("123456", &mut exact), Ok(9));
}

#[test]
fn decode_rejects_malformed_streams() {
    let mut out = [0u8; 64];
    assert!(matches!(
        decode_into(&[pratt_rpn::opcodes::NUM], &mut out),
        Err(Error::MalformedRpn { .. })
    ));
    assert!(matches!(
        decode_into(&[pratt_rpn::opcodes::VAR, b'Z'], &mut out),
        Err(Error::MalformedRpn { .. })
    ));
    assert!(matches!(
        decode_into(&[0xFF], &mut out),
        Err(Error::MalformedRpn { offset: 0 })
    ));
}
