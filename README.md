# rpn2

A `no_std` library that compiles mathematical expressions into compact
binary [RPN](https://en.wikipedia.org/wiki/Reverse_Polish_notation)
bytecode. Zero allocation, no panics, bounded memory, single pass.

```text
input : 3x+2
rpn   : 3 x * 2 +          (22 bytes of binary opcodes in your buffer)
```

## Installation

```toml
[dependencies]
rpn2 = "0.1"
```

Enable the optional `std` feature for the CLI binary and transcendental
evaluation (`sin`, `cos`, `ln`, ...):

```toml
[dependencies]
rpn2 = { version = "0.1", features = ["std"] }
```

The library core is `no_std` and never allocates on any compile or
evaluate path; all memory is caller-provided.

## Usage

### Compile

```rust
use rpn2::{decode_into, parse_into, MAX_RPN};

let mut buf = [0u8; MAX_RPN];
let n = parse_into("atan(2)x + log(8)_2", &mut buf)?;

// buf[..n] holds binary bytecode. Render it back to text:
let mut text = [0u8; 4 * MAX_RPN];
let m = decode_into(&buf[..n], &mut text)?;
assert_eq!(&text[..m], b"2 atan x * 8 log 2 logb +");
# Ok::<(), rpn2::Error>(())
```

### Choose your own limits

Nothing about output size or nesting is hard-coded. A `Config` states
the budget and depth; scratch requirements derive from it.

```rust
use rpn2::{compile_into, Config, NoResolve};

let cfg = Config::new().output_limit(1024 * 1024).max_depth(2048);

let mut out = vec![0u8; cfg.get_output_limit()];
let mut stack = vec![0u8; cfg.scratch_len()];
let n = compile_into(&cfg, &NoResolve, "1+1", &mut out, &mut stack)?;
# Ok::<(), rpn2::Error>(())
```

Exceeding the configured limit returns `Error::OutputLimitExceeded`;
a buffer smaller than the limit that an expression outgrows returns
`Error::BufferTooSmall`. The operator stack lives in `stack`
(`Config::scratch_len()` bytes: 4 per nesting level) and nothing else
is touched beyond your buffers.

### Variables: define, substitute, tweak, solve

A `Session` stores up to 26 single-letter definitions. Using a defined
variable splices its compiled body into later compilations, recursively.
Storage keeps variable references intact, so redefining a letter changes
every downstream result on the next compile.

When every part of a chain is numeric, it folds to a literal:

```rust
use rpn2::{decode_into, Config, Session};

let mut s = Session::<256>::new(Config::new());
let mut stack = [0u8; Config::new().scratch_len()];

s.compile_line("a = 6", &mut [], &mut stack)?;
s.compile_line("b = 7", &mut [], &mut stack)?;
s.compile_line("x = a*b", &mut [], &mut stack)?;

let mut out = [0u8; 128];
let n = s.compile("x+1", &mut out, &mut stack)?;

let mut text = [0u8; 512];
let m = decode_into(&out[..n], &mut text)?;
assert_eq!(&text[..m], b"42 1 +");
# Ok::<(), rpn2::Error>(())
```

Recursive definitions are rejected with `Error::RecursiveDefinition`
and leave existing definitions untouched. Transcendental functions stay
symbolic (no bundled libm). `Session::compile_line` accepts
calculator-style input and distinguishes assignments from expressions.

### Evaluate

```rust
use rpn2::{eval, parse_into, Vars};

let mut buf = [0u8; 256];
let n = parse_into("2x^2+1", &mut buf)?;

let mut vars = Vars::zeroed();
vars.set(b'x', 3.0);
assert_eq!(eval(&buf[..n], &vars, &mut [0.0; 64])?, 19.0);
# Ok::<(), rpn2::Error>(())
```

A postfix stack machine over caller-provided memory. Unset variables
read as `NaN`. Arithmetic evaluates everywhere; transcendentals need
the `std` feature and return `Error::FuncUnsupportedOnTarget` without
it.

### Solve equations numerically

```rust
use rpn2::{parse_into, solve, SolveCfg, Vars};

// y = 2x+3 meets y = 2x^3+10 — where?
let mut l = [0u8; 128];
let mut r = [0u8; 128];
let ln = parse_into("2x+3", &mut l)?;
let rn = parse_into("2x^3+10", &mut r)?;

let mut roots = [0.0; 4];
let k = solve(
    &l[..ln], &r[..rn], b'x', &Vars::new(),
    SolveCfg { range: (-10.0, 10.0), steps: 512 },
    &mut [0.0; 64],
    &mut roots,
)?;
// roots[..k]: ascending, deduplicated, converged to f64 precision.
# assert_eq!(k, 1);
```

Bracketed sign-change search plus bisection over continuous functions —
the same contract as handheld calculators. Even-multiplicity roots do
not change sign and are not reported.

## Grammar

| Construct | Syntax |
|---|---|
| Numbers | `123`, `.5`, `1e3`, `2E-2` |
| Variables | single letters `a..z` (except reserved) |
| Constants | `e`, `pi` |
| Functions | `sin cos tan asin acos atan ln log` — parentheses required |
| Logarithm with base | `log(arg)_base` |
| Operators | `+ - * / ^`, unary `-` |
| Implicit multiplication | `3x`, `2sin(x)`, `(a)(b)` |

Precedence, low to high: `+ -` < `* /` and implicit multiplication <
unary `-` < `^` (right-assoc) < `_base`.

Identifiers match greedily as whole words: `xsin` is an error, not
`x*sin`. Input is ASCII lowercase; whitespace is ignored.

## Binary format

Little-endian, self-delimiting by opcode, fixed-width payloads so any
instruction can be skipped or decoded in O(1):

| Opcode | Payload | Size | Meaning |
|---|---|---|---|
| `0x01` | `f64` | 9 | push number |
| `0x02` | `u8` letter | 2 | push variable |
| `0x03` | `u8` id | 2 | push constant |
| `0x04` | `u8` id | 2 | call function |
| `0x10`–`0x14` | — | 1 | `+ - * / ^` |
| `0x15` | — | 1 | unary negate |

Constants live in [`rpn2::opcodes`](src/opcodes.rs).

## Errors

Every failure mode is a typed value carrying a byte offset where
relevant — malformed input, nesting overflow, capacity limits, cycle
detection, evaluation stack exhaustion. Nothing panics.

## Performance

Single fused pass: O(n) time, O(1) auxiliary memory (operator stack +
fixed-size staging), direct-to-buffer emission. The lexer combines a
256-entry class table with hybrid scalar/SIMD scanning — SSE2 on
`x86_64` and NEON on `aarch64` (both baseline for their targets),
portable scalar elsewhere, including all sub-16-byte tails. Number
literals take a bit-exact fast path proven equivalent to core's parser;
anything outside its envelope falls back to `str::parse`.

Run the comparison harness against `pratt-rpn` yourself:

```console
cargo run --release --features std --example bench
```

## License

MIT
