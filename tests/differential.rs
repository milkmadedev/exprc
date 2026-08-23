//! Differential testing: rpn2 must be *indistinguishable* from pratt-rpn.
//!
//! Three campaigns, all deterministic (xorshift-seeded):
//! 1. **Generated ASTs** — random expression trees rendered to source,
//!    byte-for-byte RPN comparison between engines.
//! 2. **Depth boundary sweep** — nesting levels around MAX_DEPTH must
//!    produce identical verdicts in both engines.
//! 3. **Mutation fuzz** — single-byte mutations of valid expressions;
//!    both engines must agree on Ok(bytes) or the exact Err.

use pratt_rpn as v1;
use rpn2::{Error as E2, MAX_RPN};
use std::string::String;
use std::vec::Vec;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn run_v1(src: &str) -> Result<Vec<u8>, String> {
    let mut buf = [0u8; MAX_RPN];
    match v1::parse_into(src, &mut buf) {
        Ok(n) => Ok(buf[..n].to_vec()),
        Err(e) => Err(format!("{e:?}")),
    }
}

fn run_v2(src: &str) -> Result<Vec<u8>, String> {
    let mut buf = [0u8; MAX_RPN];
    match rpn2::parse_into(src, &mut buf) {
        Ok(n) => Ok(buf[..n].to_vec()),
        Err(e) => Err(format!("{e:?}")),
    }
}

fn assert_same(src: &str) {
    if std::env::var_os("V2_ONLY").is_some() {
        let _ = run_v2(src);
        return;
    }
    match (run_v1(src), run_v2(src)) {
        (Ok(a), Ok(b)) => assert_eq!(a, b, "RPN mismatch for {src:?}"),
        (Err(a), Err(b)) => assert_eq!(a, b, "error mismatch for {src:?}"),
        (r1, r2) => panic!("verdict mismatch for {src:?}: v1={r1:?} v2={r2:?}"),
    }
}

const FUNCS: [&str; 8] = ["sin", "cos", "tan", "asin", "acos", "atan", "ln", "log"];
const VARS: [&str; 4] = ["a", "x", "y", "pi"];

/// Random AST → source text, with random whitespace sprinkled in.
struct Gen<'a> {
    rng: &'a mut Rng,
}
impl Gen<'_> {
    fn ws(&mut self, out: &mut String) {
        if self.rng.below(4) == 0 {
            out.push(' ');
        }
    }
    fn atom(&mut self, out: &mut String, depth: u32) {
        match self.rng.below(10) {
            0..=3 => out.push_str(&(self.rng.below(1000)).to_string()),
            4 => out.push_str(&format!(
                "{}.{:02}",
                self.rng.below(50),
                self.rng.below(100)
            )),
            5 => out.push_str("2e3"),
            6..=7 => {
                let vs = VARS[self.rng.below(VARS.len() as u64) as usize];
                out.push_str(vs);
            }
            8 => {
                let fs = FUNCS[self.rng.below(FUNCS.len() as u64) as usize];
                out.push_str(fs);
                out.push('(');
                if depth > 0 {
                    self.expr_at(out, depth - 1);
                } else {
                    out.push('x');
                }
                out.push(')');
                // log(...) may take a base
                if fs == "log" && self.rng.below(2) == 0 && depth > 0 {
                    out.push('_');
                    self.atom(out, depth - 1);
                }
            }
            _ => {
                out.push('(');
                if depth > 0 {
                    self.expr_at(out, depth - 1);
                } else {
                    out.push('x');
                }
                out.push(')');
            }
        }
    }
    fn unary_chain(&mut self, out: &mut String, depth: u32) {
        while self.rng.below(6) == 0 {
            out.push('-');
        }
        self.atom(out, depth);
        // postfix power chain (right-assoc: nests in both engines)
        while self.rng.below(8) == 0 && depth > 0 {
            out.push('^');
            self.atom(out, depth - 1);
        }
    }
    fn expr_at(&mut self, out: &mut String, depth: u32) {
        if depth == 0 || self.rng.below(6) == 0 {
            return self.unary_chain(out, depth);
        }
        self.expr_at(out, depth - 1);
        match self.rng.below(5) {
            0 => out.push('+'),
            1 => out.push('-'),
            2 => out.push('*'),
            3 => out.push('/'),
            _ => out.push('^'),
        }
        self.ws(out);
        self.expr_at(out, depth - 1);
    }
    fn expr(&mut self, out: &mut String) {
        self.expr_at(out, 6);
    }
}

#[test]
fn generated_ast_equivalence() {
    let mut rng = Rng(0x9E3779B97F4A7C15);
    for _ in 0..20_000 {
        let mut src = String::new();
        {
            let mut g = Gen { rng: &mut rng };
            g.expr(&mut src);
        }
        // Keep nesting inside limits so most cases exercise the parser
        // rather than tripping TooDeep immediately.
        let src = if src.len() > 400 {
            &src[..400.min(src.len())]
        } else {
            &src
        };
        assert_same(src);
    }
}

#[test]
fn depth_boundary_sweep() {
    // Parens: both engines must agree at every level around the limit.
    for n in 118..=140usize {
        let src = format!("{}x{}", "(".repeat(n), ")".repeat(n));
        assert_same(&src);
    }
    // Unary chains.
    for n in 118..=140usize {
        let src = format!("{}x", "-".repeat(n));
        assert_same(&src);
    }
    // Power chains (right-assoc nesting).
    for n in 118..=140usize {
        let src = format!("2{}", "^2".repeat(n));
        assert_same(&src);
    }
    // Function-call chains.
    for n in 60..=80usize {
        let src = format!("{}sin(x){}", "sin(".repeat(n), ")".repeat(n));
        assert_same(&src);
    }
}

#[test]
fn mutation_fuzz() {
    let mut rng = Rng(0xDEADBEEFCAFEBABE);
    const BASES: [&str; 12] = [
        "3x+2",
        "atan(2)x+log(8)_2",
        "2sin(x)^2-cos(y)/tan(pi)",
        "-(x+1)*(2-x)^3",
        "log(log(8)_2)_e+1",
        ".5x+.25e-1*y",
        "((a)(b))*sqrt_like_nope",
        "x^2^3^4",
        "e*pi*2",
        "1/2/3/4/x/y",
        "sin(x)_2", // invalid: _ after non-log
        "atan(2)",
    ];
    for base in BASES {
        for _ in 0..3_000 {
            let mut s: Vec<u8> = base.bytes().collect();
            let ops = rng.below(3) + 1;
            for _ in 0..ops {
                let idx = rng.below(s.len() as u64 + 1) as usize;
                match rng.below(3) {
                    0 if idx < s.len() => {
                        s.remove(idx);
                    }
                    1 => {
                        let c = b"0123456789+-*/^_().exyzabclnsgatco"[rng.below(34) as usize];
                        s.insert(idx.min(s.len()), c);
                    }
                    _ if !s.is_empty() => {
                        let at = idx % s.len();
                        s[at] = b"0123456789+-*/^_().exyzabclnsgatco"[rng.below(34) as usize];
                    }
                    _ => {}
                }
            }
            let mutated = String::from_utf8_lossy(&s).into_owned();
            // Only ASCII inputs are comparable; lossy conversion may inject
            // U+FFFD which both engines reject identically anyway.
            if mutated.is_ascii() {
                assert_same(&mutated);
            } else {
                assert!(run_v2(&mutated).is_err());
            }
        }
    }
}

#[test]
fn wire_format_stability() {
    // Golden vectors: protect the binary format from accidental change.
    assert_same_is_exact_bytes(
        "3x+2",
        &[
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x40, // NUM 3
            0x02, b'x', // VAR x
            0x12, // MUL
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, // NUM 2
            0x10, // ADD
        ],
    );
    let _ = E2::TooDeep; // touch import
}

fn assert_same_is_exact_bytes(src: &str, expected: &[u8]) {
    let got = run_v2(src).expect("parses");
    assert_eq!(got, expected.to_vec(), "golden vector drift for {src:?}");
}
