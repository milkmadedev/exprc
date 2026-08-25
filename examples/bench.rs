//! Side-by-side benchmark: pratt-rpn (v1) vs exprc (v2).
//!
//! Run with: cargo run --release --features std --example bench

use std::time::Instant;

const ITERS: u32 = 200_000;

fn time<F: FnMut()>(mut f: F) -> f64 {
    // warmup
    for _ in 0..10_000 {
        f();
    }
    let start = Instant::now();
    for _ in 0..ITERS {
        f();
    }
    start.elapsed().as_nanos() as f64 / ITERS as f64
}

fn main() {
    let cases: [(&str, String); 6] = [
        ("short        3x+2", "3x+2".into()),
        ("func         atan(2)", "atan(2)".into()),
        ("log-base     log(8)_2", "log(8)_2".into()),
        ("numeric-heavy", "12.5*3.75e-2+0.125/7".into()),
        ("medium mixed ", "2sin(x)^2+atan(x*3)/log(100)_10".into()),
        ("long chain   ", format!("1+{}", "1+".repeat(500) + "1")),
    ];

    println!(
        "{:<22} {:>12} {:>12} {:>8}",
        "case", "v1 ns", "v2 ns", "speedup"
    );
    println!("{}", "-".repeat(58));
    for (name, expr) in &cases {
        let mut b1 = [0u8; 10240];
        let mut b2 = [0u8; 10240];
        let t1 = time(|| {
            let _ = pratt_rpn::parse_into(expr, &mut b1);
        });
        let t2 = time(|| {
            let _ = exprc::parse_into(expr, &mut b2);
        });
        println!("{name:<22} {t1:>10.1} {t2:>10.1} {:>7.2}x", t1 / t2);
    }
}
