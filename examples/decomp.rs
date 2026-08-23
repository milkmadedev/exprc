use std::time::Instant;

fn t(name: &str, iters: u64, mut f: impl FnMut()) {
    for _ in 0..(iters / 10) {
        f();
    }
    let st = Instant::now();
    for _ in 0..iters {
        f();
    }
    println!(
        "{name:<14} {:.1} ns",
        st.elapsed().as_nanos() as f64 / iters as f64
    );
}

fn main() {
    let mut buf = [0u8; 10240];
    t("scan atan(2)", 500_000, || {
        rpn2::__scan_only("atan(2)");
    });
    t("full atan(2)", 500_000, || {
        let _ = rpn2::__parse_discard("atan(2)", &mut buf);
    });
    t("scan 3x+2", 500_000, || {
        rpn2::__scan_only("3x+2");
    });
    t("full 3x+2", 500_000, || {
        let _ = rpn2::__parse_discard("3x+2", &mut buf);
    });
}
