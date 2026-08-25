//! Tests for programmer-owned limits, variable sessions (substitution,
//! solving, cycles), and evaluation.

use exprc::{
    compile_into, decode_into, eval, split_assignment, Config, Error, NoResolve, Session, Vars,
};

fn text(rpn: &[u8]) -> String {
    let mut out = [0u8; 4096];
    let m = decode_into(rpn, &mut out).unwrap();
    core::str::from_utf8(&out[..m]).unwrap().to_string()
}

// ------------------------------------------------------------- limits

#[test]
fn default_parse_unchanged() {
    let mut buf = [0u8; exprc::MAX_RPN];
    let n = exprc::parse_into("3x+2", &mut buf).unwrap();
    assert_eq!(text(&buf[..n]), "3 x * 2 +");
}

#[test]
fn tiny_budget_rejects_with_buffer_too_small() {
    let cfg = Config::new().output_limit(64);
    let mut out = vec![0u8; 32]; // buffer below configured budget
    let mut stack = vec![0u8; cfg.scratch_len()];
    assert_eq!(
        compile_into(&cfg, &NoResolve, "1*2*3*4*5", &mut out, &mut stack),
        Err(Error::BufferTooSmall)
    );
    // Same budget, adequate buffer: compiles.
    let mut out = [0u8; 128];
    assert!(compile_into(&cfg, &NoResolve, "1*2*3", &mut out, &mut stack).is_ok());
}

#[test]
fn budget_limit_is_the_programmers_not_a_constant() {
    // An expression that exceeds a 16-byte budget even with a huge buffer:
    let cfg = Config::new().output_limit(16);
    let mut out = vec![0u8; 1024 * 1024];
    let mut stack = vec![0u8; cfg.scratch_len()];
    assert_eq!(
        compile_into(&cfg, &NoResolve, "123456+2", &mut out, &mut stack),
        Err(Error::OutputLimitExceeded)
    );
}

#[test]
fn one_megabyte_budget_compiles_large_expressions() {
    let cfg = Config::new().output_limit(1024 * 1024).max_depth(512);
    let expr = format!("1{}", "+1".repeat(20_000)); // ~60 KB of bytecode
    let mut out = vec![0u8; cfg.get_output_limit()];
    let mut stack = vec![0u8; cfg.scratch_len()];
    let n = compile_into(&cfg, &NoResolve, &expr, &mut out, &mut stack).unwrap();
    assert!(n > 60_000);
}

#[test]
fn deeper_nesting_needs_and_uses_scratch() {
    let cfg = Config::new().max_depth(600);
    let mut out = vec![0u8; cfg.get_output_limit()];
    let mut small_stack = vec![0u8; Config::new().scratch_len()];
    let deep = format!("{}x{}", "(".repeat(400), ")".repeat(400));

    assert!(matches!(
        compile_into(&cfg, &NoResolve, &deep, &mut out.clone(), &mut small_stack),
        Err(Error::ScratchTooSmall { .. })
    ));

    let mut big_stack = vec![0u8; cfg.scratch_len()];
    assert!(compile_into(&cfg, &NoResolve, &deep, &mut out, &mut big_stack).is_ok());
}

#[test]
fn scratch_too_small_reports_requirements() {
    let cfg = Config::new().max_depth(1000);
    let mut out = [0u8; 256];
    let mut stack = [0u8; 10];
    match compile_into(&cfg, &NoResolve, "x", &mut out, &mut stack) {
        Err(Error::ScratchTooSmall { needed, got }) => {
            assert_eq!(got, 10);
            assert!(needed >= 2000);
        }
        other => panic!("expected ScratchTooSmall, got {other:?}"),
    }
}

#[test]
fn depth_still_enforced_within_custom_budget() {
    let cfg = Config::new().max_depth(50);
    let mut out = vec![0u8; 1024];
    let mut stack = vec![0u8; cfg.scratch_len()];
    let deep = format!("{}x{}", "(".repeat(60), ")".repeat(60));
    assert_eq!(
        compile_into(&cfg, &NoResolve, &deep, &mut out, &mut stack),
        Err(Error::TooDeep)
    );
}

// ------------------------------------------------------- assignments

#[test]
fn split_assignment_shapes() {
    assert_eq!(split_assignment("x=1"), Some((b'x', "1")));
    assert_eq!(split_assignment(" y = a+b "), Some((b'y', " a+b ")));
    assert_eq!(split_assignment("xy = 1"), None); // multi-letter: not an assignment
    assert_eq!(split_assignment("a+b"), None);
    assert_eq!(split_assignment(""), None);
}

#[test]
fn define_then_substitute_splices_body() {
    let mut s = Session::<256>::new(Config::new());
    let mut stack = vec![0u8; Config::new().scratch_len()];
    s.define(b'f', "a*b", &mut stack).unwrap();

    let mut out = [0u8; 128];
    let n = s.compile("f+1", &mut out, &mut stack).unwrap();
    assert_eq!(text(&out[..n]), "a b * 1 +");
}

#[test]
fn substitution_is_recursive() {
    let mut s = Session::<256>::new(Config::new());
    let mut stack = vec![0u8; Config::new().scratch_len()];
    s.define(b'c', "pi", &mut stack).unwrap();
    s.define(b'q', "2*c", &mut stack).unwrap();

    let mut out = [0u8; 256];
    let n = s.compile("q/2", &mut out, &mut stack).unwrap();
    assert_eq!(text(&out[..n]), "2 pi * 2 /");
}

#[test]
fn known_parts_solve_to_literals() {
    let mut s = Session::<256>::new(Config::new());
    let mut stack = vec![0u8; Config::new().scratch_len()];
    for (v, e) in [(b'a', "3"), (b'b', "4")] {
        s.define(v, e, &mut stack).unwrap();
    }
    s.define(b'x', "a^2+b", &mut stack).unwrap(); // 9 + 4
    s.define(b'y', "x*2", &mut stack).unwrap(); // folds through x

    let mut out = [0u8; 128];
    let n = s.compile("y", &mut out, &mut stack).unwrap();
    assert_eq!(text(&out[..n]), "26");

    // Bodies are lazy: after undefining b, x re-expands symbolically.
    s.define(b'z', "x+a", &mut stack).unwrap();
    let n = s.compile("z", &mut out, &mut stack).unwrap();
    assert_eq!(text(&out[..n]), "16"); // x folds to 13, + a=3

    s.undefine(b'b');
    let n = s.compile("z", &mut out, &mut stack).unwrap();
    assert_eq!(text(&out[..n]), "3 2 ^ b + 3 +"); // a stays solved, b symbolic
}

#[test]
fn definitions_are_lazy_so_tweaking_propagates() {
    let mut s = Session::<256>::new(Config::new());
    let mut stack = vec![0u8; Config::new().scratch_len()];
    s.define(b'a', "1", &mut stack).unwrap();
    s.define(b'x', "a+a", &mut stack).unwrap();

    let mut out = [0u8; 128];
    let n = s.compile("x", &mut out, &mut stack).unwrap();
    assert_eq!(text(&out[..n]), "2"); // solved

    s.define(b'a', "5", &mut stack).unwrap(); // tweak
    let n = s.compile("x", &mut out, &mut stack).unwrap();
    assert_eq!(text(&out[..n]), "10"); // re-solved from new a

    s.undefine(b'a');
    let n = s.compile("x", &mut out, &mut stack).unwrap();
    assert_eq!(text(&out[..n]), "a a +"); // fully symbolic again
}

#[test]
fn recursive_definitions_are_rejected_without_clobbering() {
    let mut s = Session::<256>::new(Config::new());
    let mut stack = vec![0u8; Config::new().scratch_len()];
    s.define(b'y', "2", &mut stack).unwrap();

    assert_eq!(
        s.define(b'x', "x+1", &mut stack),
        Err(Error::RecursiveDefinition { var: b'x' })
    );
    assert_eq!(
        s.define(b'x', "y+y+x", &mut stack),
        Err(Error::RecursiveDefinition { var: b'x' })
    );
    // Mutual recursion:
    s.define(b'p', "q+1", &mut stack).unwrap();
    assert_eq!(
        s.define(b'q', "p*2", &mut stack),
        Err(Error::RecursiveDefinition { var: b'q' }) // cycle closes back at q
    );
    // Previous definitions survived untouched:
    let mut out = [0u8; 128];
    let n = s.compile("y", &mut out, &mut stack).unwrap();
    assert_eq!(text(&out[..n]), "2");
}

#[test]
fn compile_line_handles_calculator_input() {
    let mut s = Session::<256>::new(Config::new());
    let mut stack = vec![0u8; Config::new().scratch_len()];
    let mut out = [0u8; 256];

    use exprc::Line;
    assert!(matches!(
        s.compile_line("m = 2n", &mut out, &mut stack),
        Ok(Line::Defined { var: b'm', .. })
    ));
    let n = match s.compile_line("m+n", &mut out, &mut stack).unwrap() {
        Line::Expression(n) => n,
        _ => panic!(),
    };
    assert_eq!(text(&out[..n]), "2 n * n +");

    assert!(matches!(
        s.compile_line("ab = 1", &mut out, &mut stack),
        Err(Error::BadAssignment { .. })
    ));
    assert!(matches!(
        s.compile_line("a+b = 2", &mut out, &mut stack),
        Err(Error::BadAssignment { .. })
    ));
}

#[test]
fn undefined_variables_stay_symbols() {
    let s = Session::<256>::new(Config::new());
    let mut stack = vec![0u8; Config::new().scratch_len()];
    let mut out = [0u8; 64];
    let n = s.compile("z x a b c", &mut out, &mut stack).unwrap();
    assert_eq!(text(&out[..n]), "z x * a * b * c *");
}

#[test]
fn oversized_bodies_report_buffer_too_small() {
    let mut s = Session::<8>::new(Config::new()); // tiny per-var storage
    let mut stack = vec![0u8; Config::new().scratch_len()];
    assert_eq!(
        s.define(b'x', "111111+222222", &mut stack),
        Err(Error::BufferTooSmall)
    );
}

// -------------------------------------------------------------- eval

#[test]
fn eval_arithmetic_and_vars() {
    let mut buf = [0u8; 256];
    let cases = [
        ("2x^2+1", 3.0, 19.0),
        ("-x^2", 3.0, -9.0),
        ("2^-2+0.5", 0.0, 0.75),
        ("log(8)_2", 0.0, 3.0),
        ("e-2", 0.0, std::f64::consts::E - 2.0),
        ("atan(1)*4-pi", 0.0, 0.0),
    ];
    for (src, xv, want) in cases {
        let n = exprc::parse_into(src, &mut buf).unwrap();
        let mut vars = Vars::zeroed();
        vars.set(b'x', xv);
        let got = eval(&buf[..n], &vars, &mut [0.0; 64]).unwrap();
        assert!((got - want).abs() < 1e-12, "{src}: {got} vs {want}");
    }
}

#[test]
fn eval_unset_var_is_nan() {
    let mut buf = [0u8; 64];
    let n = exprc::parse_into("x+1", &mut buf).unwrap();
    let got = eval(&buf[..n], &Vars::new(), &mut [0.0; 8]).unwrap();
    assert!(got.is_nan());
}

#[test]
fn eval_session_solved_output_is_exact() {
    let mut s = Session::<256>::new(Config::new());
    let mut stack = vec![0u8; Config::new().scratch_len()];
    s.compile_line("r = 2", &mut [], &mut stack).unwrap();
    s.compile_line("k = pi*r^2", &mut [], &mut stack).unwrap();

    let mut out = [0u8; 128];
    let n = s.compile("k", &mut out, &mut stack).unwrap();
    let area = eval(&out[..n], &Vars::new(), &mut [0.0; 16]).unwrap();
    assert!((area - core::f64::consts::PI * 4.0).abs() < 1e-15);
}

#[test]
fn eval_stack_overflow_is_an_error() {
    let mut buf = [0u8; 64];
    let n = exprc::parse_into("1+2", &mut buf).unwrap();
    let r = eval(&buf[..n], &Vars::zeroed(), &mut []);
    assert!(matches!(r, Err(Error::EvalStackOverflow)));
}

#[test]
fn transcendental_eval_requires_std_feature_here_it_has_it() {
    // Built with `std` in this test harness, so sin works:
    let mut buf = [0u8; 64];
    let n = exprc::parse_into("sin(0)", &mut buf).unwrap();
    let v = eval(&buf[..n], &Vars::new(), &mut [0.0; 8]).unwrap();
    assert_eq!(v, 0.0);
}
