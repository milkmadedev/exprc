use rpn2::{decode_into, parse_into, MAX_RPN};
use std::process::ExitCode;

const USAGE: &str = "usage: rpn <expression>\nexample: rpn \"3x+2\"";

fn main() -> ExitCode {
    let Some(expr) = std::env::args().nth(1) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };

    let mut buf = [0u8; MAX_RPN];
    let n = match parse_into(&expr, &mut buf) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut text = [0u8; 4 * MAX_RPN];
    let text = decode_into(&buf[..n], &mut text).map_or_else(
        |e| format!("<decode failed: {e}>"),
        |m| String::from_utf8_lossy(&text[..m]).into_owned(),
    );

    println!("input : {expr}");
    println!("rpn   : {text}");
    println!("bytes : {n}/{MAX_RPN}");

    print!("hex   :");
    for (i, b) in buf[..n].iter().enumerate() {
        if i % 32 == 0 {
            println!();
            print!("  {i:04x}: ");
        }
        print!("{b:02x} ");
    }
    println!();

    ExitCode::SUCCESS
}
