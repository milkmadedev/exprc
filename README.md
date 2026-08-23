# rpn2

Ground-up second engine for math-expression → binary **RPN**. Same wire
format, same API, same error semantics as
[`pratt-rpn`](https://github.com/milkmadedev/pratt-rpn) — output buffers are
interchangeable and a large differential suite enforces bit-equality — but a
completely different internal design.

```text
input : 3x+2
rpn   : 3 x * 2 +        (emitted as 22 bytes of binary opcodes)
```

## Design differences vs v1

| dimension       | pratt-rpn (v1)                | rpn2 (v2)                                |
|-----------------|-------------------------------|------------------------------------------|
| parse strategy  | recursive Pratt               | fully iterative shunting-yard core       |
| aux memory      | call stack, ~19 KB worst case | fixed 528-byte operator stack            |
| number parsing  | `str::parse::<f64>` always    | provably bit-exact fast path + fallback  |
| lexer scanning  | byte-at-a-time `match`        | class table + hybrid scalar/SIMD spans   |
| output writes   | bounds check per byte         | one check per instruction                |
| recursion       | one frame per nesting level   | none, anywhere                           |

## Measured performance

Default `--release` profile, single core, this repository's
`cargo run --release --features std --example bench` (numbers from the
development machine; run it yourself on yours):

| case                        | v1 ns | v2 ns | speedup |
|-----------------------------|------:|------:|--------:|
| short      `3x+2`           |  35.3 |  36.8 | 0.96x   |
| function   `atan(2)`        |  24.4 |  28.0 | 0.87x   |
| log-base   `log(8)_2`       |  39.9 |  37.7 | 1.06x   |
| numeric-heavy (4 decimals)  |  71.3 |  61.2 | 1.16x   |
| medium mixed expression     | 121.0 | 100.3 | 1.21x   |
| long chain (1001 operands)  | 8244.8| 5787.2| **1.42x** |

Honest summary: v2 wins as expressions grow and on numeric input; v1 keeps a
~3 ns edge on trivial two-operand expressions because its fused recursive loop
materializes nothing. The structural win of v2 is memory: worst-case auxiliary
usage drops from unbounded recursion frames (~19 KB at `MAX_DEPTH`) to a fixed
528-byte stack, which is what matters for embedded deployment.

## SIMD

Compile-time dispatch, no runtime detection needed:

* `x86_64` → SSE2 (baseline for the entire target since 2003)
* `aarch64` → NEON (baseline for the target)
* everything else → portable scalar loops (also used for sub-16-byte tails)

The hybrid scanner probes scalar-first and only engages vector chunking once a
span proves long enough to amortize it, so short expressions pay nothing for
the SIMD machinery while KB-scale inputs get full-width scans. An AVX2 backend
is a drop-in extension point behind `target-feature = "avx2"`.

All `unsafe` is confined to `src/simd.rs` load wrappers (pointers into local,
fully initialized `[u8; 16]` staging arrays); everything else in the crate is
safe code.

## Fast float parsing

`num::try_fast` converts literals without touching general decimal machinery:

* pure integers ≤19 digits: single `u64 → f64` conversion (itself correctly
  rounded ⇒ bit-identical to `str::parse`);
* other literals with mantissa `< 2^53` and shift `k ∈ [-22, 22]`: one IEEE
  multiply (exact `10^k`) or divide (exact `10^|k|`; negative powers are not
  representable, so division is required), each correctly rounded w.r.t. the
  true decimal value ⇒ bit-identical to `str::parse`.

Everything outside the envelope falls back to core's parser. A differential
grid test pins fast-path results to `str::parse` across boundary shapes
(`2^53±1`, 15/16/19/20-digit mantissas, exponent edges).

## Grammar & format

Identical to v1: functions `sin cos tan asin acos atan ln log`, constants
`e pi`, variables `a..z` (except reserved letters), implicit multiplication,
`^` right-assoc, `log(arg)_base` postfix base. Wire format documented in
[`src/opcodes.rs`](src/opcodes.rs); golden-vector tests protect it.

## Correctness enforcement

```
cargo test                     # unit + integration suites
cargo test --features std      # adds differential campaigns vs pratt-rpn:
                               #  - 20k generated ASTs, byte-for-byte equal RPN
                               #  - depth-boundary sweeps around MAX_DEPTH
                               #  - 36k single-byte mutation fuzz cases
                               #    (identical Ok bytes or identical Err)
```

## CLI

```console
$ cargo run --features std --release -- "log(100)_10"
input : log(100)_10
rpn   : 100 log 10 logb
bytes : 22/10240
```

## License

MIT
