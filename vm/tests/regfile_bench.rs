//! M0 representation microbench for the register-VM rewrite (see
//! `notes/REGISTER_VM.md`). NOT a correctness test — it's a one-off decision
//! aid, marked `#[ignore]` so it stays out of `just test`.
//!
//! Run:
//!   cargo test -p vm --release --test regfile_bench -- --ignored --nocapture
//!
//! It sizes the prize of an unboxed register file by running the SAME register
//! program through the SAME dispatch loop over three storage representations,
//! differing ONLY in how a register is read/written:
//!
//!   * `boxed`  — `Vec<Value>` (the real 24-byte enum; today's storage as a
//!     register file). Captures the layout/tag cost the dispatch memory warned
//!     about.
//!   * `raw_i64` — `Vec<i64>` (parallel-arrays numeric path: no tag, direct).
//!   * `raw_u64` — `Vec<u64>` reinterpreted per the instruction's static type
//!     (the "single typed array" — the typed cousin of NaN-boxing, but with
//!     zero tagging because typed registers need no runtime discrimination).
//!
//! NaN-boxing proper is deliberately NOT benched: with statically-typed
//! registers its runtime-tag rationale evaporates, Pluma's full i64 can't be
//! NaN-boxed without a boxed fallback (>48-bit ints), and pointer registers
//! would need unsafe manual Rc juggling. The `raw_u64` row stands in for "what a
//! single typed array can do on numerics"; if it ties `raw_i64`, the
//! parallel-vs-single choice is about safety/memory, not speed.

use std::hint::black_box;
use std::time::Instant;
use vm::Value;

/// A tiny three-address register op for the bench. Mirrors the shape the real
/// register VM will use (dst, src, src), enough to host an arithmetic hot loop.
#[derive(Clone, Copy)]
enum Op {
	Mul(u8, u8, u8),       // dst = a * b
	Sub(u8, u8, u8),       // dst = a - b
	Add(u8, u8, u8),       // dst = a + b
	Lt(u8, u8, u8),        // dst = (a < b) as int
	JumpIfTrue(u8, usize), // if reg != 0, pc = target
	Halt,
}

/// Build the loop:
///   r0=i  r1=N  r2=acc  r3=tmp  r4=one  r5=cond
///   tmp = i * i; tmp = tmp - i; acc = acc + tmp; i = i + one;
///   cond = i < N; if cond goto top
/// Computes Σ_{i<N} (i*i - i) — several reg ops per iter so register access +
/// dispatch dominate, not loop overhead.
fn program() -> Vec<Op> {
	vec![
		Op::Mul(3, 0, 0),     // 0: tmp = i*i
		Op::Sub(3, 3, 0),     // 1: tmp = tmp - i
		Op::Add(2, 2, 3),     // 2: acc = acc + tmp
		Op::Add(0, 0, 4),     // 3: i = i + one
		Op::Lt(5, 0, 1),      // 4: cond = i < N
		Op::JumpIfTrue(5, 0), // 5: loop
		Op::Halt,             // 6
	]
}

fn run_boxed(prog: &[Op], n: i64) -> i64 {
	let mut r: Vec<Value> = vec![Value::Int(0); 8];
	r[1] = Value::Int(n);
	r[4] = Value::Int(1);
	let geti = |v: &Value| match v {
		Value::Int(x) => *x,
		_ => unreachable!(),
	};
	let mut pc = 0usize;
	loop {
		match prog[pc] {
			Op::Mul(d, a, b) => {
				r[d as usize] = Value::Int(geti(&r[a as usize]).wrapping_mul(geti(&r[b as usize])))
			}
			Op::Sub(d, a, b) => {
				r[d as usize] = Value::Int(geti(&r[a as usize]).wrapping_sub(geti(&r[b as usize])))
			}
			Op::Add(d, a, b) => {
				r[d as usize] = Value::Int(geti(&r[a as usize]).wrapping_add(geti(&r[b as usize])))
			}
			Op::Lt(d, a, b) => {
				r[d as usize] = Value::Int((geti(&r[a as usize]) < geti(&r[b as usize])) as i64)
			}
			Op::JumpIfTrue(c, t) => {
				if geti(&r[c as usize]) != 0 {
					pc = t;
					continue;
				}
			}
			Op::Halt => break,
		}
		pc += 1;
	}
	geti(&r[2])
}

fn run_raw_i64(prog: &[Op], n: i64) -> i64 {
	let mut r: Vec<i64> = vec![0; 8];
	r[1] = n;
	r[4] = 1;
	let mut pc = 0usize;
	loop {
		match prog[pc] {
			Op::Mul(d, a, b) => r[d as usize] = r[a as usize].wrapping_mul(r[b as usize]),
			Op::Sub(d, a, b) => r[d as usize] = r[a as usize].wrapping_sub(r[b as usize]),
			Op::Add(d, a, b) => r[d as usize] = r[a as usize].wrapping_add(r[b as usize]),
			Op::Lt(d, a, b) => r[d as usize] = (r[a as usize] < r[b as usize]) as i64,
			Op::JumpIfTrue(c, t) => {
				if r[c as usize] != 0 {
					pc = t;
					continue;
				}
			}
			Op::Halt => break,
		}
		pc += 1;
	}
	r[2]
}

fn run_raw_u64(prog: &[Op], n: i64) -> i64 {
	// Single untyped u64 array; the op's static type says "these bits are i64".
	let mut r: Vec<u64> = vec![0; 8];
	r[1] = n as u64;
	r[4] = 1u64;
	let geti = |x: u64| x as i64;
	let mut pc = 0usize;
	loop {
		match prog[pc] {
			Op::Mul(d, a, b) => {
				r[d as usize] = geti(r[a as usize]).wrapping_mul(geti(r[b as usize])) as u64
			}
			Op::Sub(d, a, b) => {
				r[d as usize] = geti(r[a as usize]).wrapping_sub(geti(r[b as usize])) as u64
			}
			Op::Add(d, a, b) => {
				r[d as usize] = geti(r[a as usize]).wrapping_add(geti(r[b as usize])) as u64
			}
			Op::Lt(d, a, b) => {
				r[d as usize] = ((geti(r[a as usize]) < geti(r[b as usize])) as i64) as u64
			}
			Op::JumpIfTrue(c, t) => {
				if r[c as usize] != 0 {
					pc = t;
					continue;
				}
			}
			Op::Halt => break,
		}
		pc += 1;
	}
	r[2] as i64
}

fn time<F: Fn() -> i64>(label: &str, iters: u32, f: F) -> (i64, f64) {
	// warmup
	let mut last = 0;
	for _ in 0..2 {
		last = black_box(f());
	}
	let start = Instant::now();
	for _ in 0..iters {
		last = black_box(f());
	}
	let ms = start.elapsed().as_secs_f64() * 1000.0 / iters as f64;
	println!("  {label:<10} {ms:>9.2} ms   (check = {last})");
	(last, ms)
}

#[test]
#[ignore = "microbench; run with --ignored --nocapture --release"]
fn regfile_representation() {
	let n: i64 = std::env::var("REGBENCH_N")
		.ok()
		.and_then(|s| s.parse().ok())
		.unwrap_or(20_000_000);
	let iters: u32 = std::env::var("REGBENCH_ITERS")
		.ok()
		.and_then(|s| s.parse().ok())
		.unwrap_or(11);
	let prog = black_box(program());

	println!(
		"\nregister-file representation microbench  (N={n}, {iters} iters, sizeof(Value)={}B)\n",
		std::mem::size_of::<Value>()
	);
	let (cb, boxed) = time("boxed", iters, || run_boxed(&prog, n));
	let (cr, raw_i64) = time("raw_i64", iters, || run_raw_i64(&prog, n));
	let (cu, raw_u64) = time("raw_u64", iters, || run_raw_u64(&prog, n));

	assert_eq!(cb, cr, "boxed vs raw_i64 disagree");
	assert_eq!(cr, cu, "raw_i64 vs raw_u64 disagree");

	println!(
		"\n  boxed / raw_i64 = {:.2}x   (unboxed numeric register speedup)",
		boxed / raw_i64
	);
	println!(
		"  raw_u64 / raw_i64 = {:.2}x  (single typed array vs parallel raw; ~1.0 => no penalty)\n",
		raw_u64 / raw_i64
	);
}
