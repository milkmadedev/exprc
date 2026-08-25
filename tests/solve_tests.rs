//! Numeric equation solving.

use rpn2::{eval, parse_into, solve, Session, SolveCfg, Vars};

fn roots_of(lhs: &str, rhs: &str) -> Vec<f64> {
    let mut l = [0u8; 512];
    let mut r = [0u8; 512];
    let ln = parse_into(lhs, &mut l).unwrap();
    let rn = parse_into(rhs, &mut r).unwrap();
    let mut found = [f64::NAN; 8];
    let k = solve(
        &l[..ln],
        &r[..rn],
        b'x',
        &Vars::new(),
        SolveCfg {
            range: (-50.0, 50.0),
            steps: 4096,
        },
        &mut [0.0; 64],
        &mut found,
    )
    .unwrap();
    found[..k].to_vec()
}

fn residual(lhs: &[u8], ln: usize, rhs: &[u8], rn: usize, x: f64) -> f64 {
    let mut vars = Vars::zeroed();
    vars.set(b'x', x);
    let mut st = [0.0; 64];
    eval(&lhs[..ln], &vars, &mut st).unwrap() - eval(&rhs[..rn], &vars, &mut st).unwrap()
}

#[test]
fn the_users_example() {
    // y = 2x+3 meets y = 2x^3+10:
    let roots = roots_of("2x+3", "2x^3+10");
    assert_eq!(roots.len(), 1, "{roots:?}");
    let mut l = [0u8; 128];
    let mut r = [0u8; 128];
    let ln = parse_into("2x+3", &mut l).unwrap();
    let rn = parse_into("2x^3+10", &mut r).unwrap();
    assert!(residual(&l, ln, &r, rn, roots[0]).abs() < 1e-9);
}

#[test]
fn linear_exact() {
    // x+1 = 3 -> x = 2
    let roots = roots_of("x+1", "3");
    assert_eq!(roots.len(), 1);
    assert!((roots[0] - 2.0).abs() < 1e-12);
}

#[test]
fn quadratic_two_roots() {
    // x^2 = 4 -> ±2
    let mut roots = roots_of("x^2", "4");
    roots.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(roots.len(), 2);
    assert!((roots[0] + 2.0).abs() < 1e-9);
    assert!((roots[1] - 2.0).abs() < 1e-9);
}

#[test]
fn cubic_three_roots() {
    // (x+3)(x)(x-2) = x^3 - x^2 - 6x
    let roots = roots_of("x^3+x^2-6x", "0");
    let sorted = {
        let mut v = roots.clone();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v
    };
    assert_eq!(sorted.len(), 3, "{sorted:?}");
    for (got, want) in sorted.iter().zip([-3.0, 0.0, 2.0]) {
        assert!((got - want).abs() < 1e-7, "{got} vs {want}");
    }
}

#[test]
fn no_root_in_range_reports_none() {
    let roots = roots_of("x^2+100", "0");
    assert!(roots.is_empty());
}

#[test]
fn tangential_root_is_invisible_like_a_calculator() {
    // (x-1)^2 = 0 touches zero without crossing: bracketing cannot see it.
    let roots = roots_of("(x-1)^2", "0");
    assert!(roots.is_empty());
}

#[test]
fn session_definitions_participate() {
    let mut s = Session::<256>::new(rpn2::Config::new());
    let mut stack = vec![0u8; rpn2::Config::new().scratch_len()];
    s.compile_line("m = 2x+3", &mut [], &mut stack).unwrap();
    s.compile_line("k = 2x^3+10", &mut [], &mut stack).unwrap();

    let mut l = [0u8; 256];
    let mut r = [0u8; 256];
    let ln = s.compile("m", &mut l, &mut stack).unwrap();
    let rn = s.compile("k", &mut r, &mut stack).unwrap();
    let mut found = [f64::NAN; 4];
    let n = solve(
        &l[..ln],
        &r[..rn],
        b'x',
        &Vars::new(),
        SolveCfg {
            range: (-10.0, 10.0),
            steps: 1024,
        },
        &mut [0.0; 64],
        &mut found,
    )
    .unwrap();
    assert_eq!(n, 1);
    assert!(residual(&l, ln, &r, rn, found[0]).abs() < 1e-9);
}

#[test]
fn other_vars_come_from_the_map() {
    // a*x + 1 = 5 with a = 2 -> x = 2
    let mut l = [0u8; 128];
    let mut r = [0u8; 128];
    let ln = parse_into("a*x+1", &mut l).unwrap();
    let rn = parse_into("5", &mut r).unwrap();
    let mut vars = Vars::zeroed();
    vars.set(b'a', 2.0);
    let mut found = [f64::NAN; 4];
    let n = solve(
        &l[..ln],
        &r[..rn],
        b'x',
        &vars,
        SolveCfg {
            range: (-20.0, 20.0),
            steps: 512,
        },
        &mut [0.0; 32],
        &mut found,
    )
    .unwrap();
    assert_eq!(n, 1);
    assert!((found[0] - 2.0).abs() < 1e-12);
}
