// Microbench runner. For each program under benchmarks/programs/<name>/main.pa,
// compiles + runs it through the VM, wall-clocks it, and prints the average
// time — for BOTH codegen backends side by side:
//   * `ast` — the original fused AST->bytecode walk (`codegen::compile`)
//   * `ir`  — the new AST->IR->bytecode path (`ir::lower` + `compile_from_ir`)
// so we can see whether the IR refactor costs anything before cutover.
//
// Usage:
//   cargo run -p bench --release                 # timing comparison (both)
//   cargo run -p bench --release -- --profile <name> [ast|ir]  # opcode counts
//   BENCH_ITERS=20 cargo run -p bench --release  # more iterations
//
// Output captured-but-discarded so we measure execution, not stdout I/O.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq)]
enum Backend {
	Ast,
	Ir,
}

impl Backend {
	fn label(self) -> &'static str {
		match self {
			Backend::Ast => "ast",
			Backend::Ir => "ir",
		}
	}
}

fn main() {
	let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
	let programs_dir = workspace.join("benchmarks/programs");
	std::env::set_current_dir(workspace).ok();

	// `cargo run -p bench -- --profile <name> [backend]` dumps opcode counts for
	// one benchmark and exits, instead of running the timing comparison.
	let mut args = std::env::args().skip(1);
	if let Some(arg) = args.next() {
		if arg == "--dump-ir" {
			let name = args.next().expect("--dump-ir takes a benchmark name");
			let main_pa = programs_dir.join(&name).join("main.pa");
			let mut compiler =
				match compiler::Compiler::from_entry_path(main_pa.to_str().unwrap().to_string()) {
					Ok(c) => c,
					Err(_) => panic!("compile error"),
				};
			vm::stdlib::register_compiler(&mut compiler);
			if compiler.check().is_err() {
				panic!("check error");
			}
			let program = ir::lower(&compiler).unwrap();
			for (i, f) in program.functions.iter().enumerate() {
				println!("--- FuncId({i}) {} (async={}) ---", f.name, f.is_async);
				println!("{:#?}", f.body);
			}
			return;
		}
		if arg == "--profile" {
			let name = args.next().expect("--profile takes a benchmark name");
			let backend = match args.next().as_deref() {
				None | Some("ast") => Backend::Ast,
				Some("ir") => Backend::Ir,
				Some(other) => panic!("unknown backend `{other}` (expected ast|ir)"),
			};
			let main_pa = programs_dir.join(&name).join("main.pa");
			profile_one(&main_pa, backend);
			return;
		}
	}

	let iterations = std::env::var("BENCH_ITERS")
		.ok()
		.and_then(|s| s.parse::<usize>().ok())
		.unwrap_or(5);

	let mut benchmarks: Vec<_> = std::fs::read_dir(&programs_dir)
		.expect("benchmarks/programs not found")
		.filter_map(|e| e.ok())
		.filter(|e| e.path().is_dir())
		.collect();
	benchmarks.sort_by_key(|e| e.file_name());

	println!("benchmark            iters    ast (avg)        ir (avg)     ir/ast");
	println!("-------------------  ------  --------------  --------------  ------");

	for entry in benchmarks {
		let name = entry.file_name().to_string_lossy().to_string();
		let main_pa = entry.path().join("main.pa");
		if !main_pa.exists() {
			continue;
		}

		let ast_path = main_pa.clone();
		let ast_time = time_runs(iterations, move || run_program(&ast_path, Backend::Ast));
		let ir_path = main_pa.clone();
		let ir_time = time_runs(iterations, move || run_program(&ir_path, Backend::Ir));

		print!("{:<21}{:>4}    ", name, iterations);
		print!("{:>12}    ", fmt(ast_time));
		print!("{:>12}    ", fmt(ir_time));
		match (ast_time, ir_time) {
			(Some(a), Some(i)) if a.as_secs_f64() > 0.0 => {
				println!("{:>5.2}x", i.as_secs_f64() / a.as_secs_f64())
			}
			_ => println!("{:>6}", "-"),
		}
	}
}

fn fmt(d: Option<Duration>) -> String {
	match d {
		Some(d) => format_duration(d),
		None => "ERROR".to_string(),
	}
}

fn time_runs<F>(iterations: usize, f: F) -> Option<Duration>
where
	F: Fn() -> Result<(), String> + Send + Sync + 'static + Clone,
{
	// Run on a dedicated thread with a big stack so deep-but-fine recursion
	// benchmarks don't overflow. The IR path doesn't yet emit TailCall, so its
	// tail-recursive benchmarks nest frames — extra insurance matters there.
	let inner = move || {
		if f().is_err() {
			return None;
		}
		let mut total = Duration::ZERO;
		for _ in 0..iterations {
			let start = Instant::now();
			if f().is_err() {
				return None;
			}
			total += start.elapsed();
		}
		Some(total / iterations as u32)
	};
	let handle = std::thread::Builder::new()
		.stack_size(256 * 1024 * 1024)
		.spawn(inner)
		.unwrap();
	handle.join().ok().flatten()
}

fn format_duration(d: Duration) -> String {
	let ms = d.as_secs_f64() * 1000.0;
	if ms < 1.0 {
		format!("{:.2} ms", ms)
	} else if ms < 1000.0 {
		format!("{:.1} ms", ms)
	} else {
		format!("{:.2} s", ms / 1000.0)
	}
}

// Compile `path` with the chosen backend, then run it on the VM. Both compile
// and run are timed together (compilation is cheap relative to these loops, but
// see `--profile` for a pure opcode-count view).
fn run_program(path: &PathBuf, backend: Backend) -> Result<(), String> {
	let program = compile(path, backend)?;
	let buf = Rc::new(RefCell::new(Vec::<u8>::new()));
	let mut vm_instance = vm::VM::new(program).with_stdout(vm::OutputSink::Buffer(buf));
	vm_instance.run().map(|_| ()).map_err(|e| e.message)
}

fn compile(path: &PathBuf, backend: Backend) -> Result<vm::Program, String> {
	let mut compiler = compiler::Compiler::from_entry_path(path.to_str().unwrap().to_string())
		.map_err(|d| format!("compile: {:?} diagnostics", d.len()))?;
	vm::stdlib::register_compiler(&mut compiler);
	compiler
		.check()
		.map_err(|d| format!("check: {:?} diagnostics", d.len()))?;
	match backend {
		Backend::Ast => codegen::compile(&compiler).map_err(|e| format!("codegen: {e}")),
		Backend::Ir => {
			let program = ir::lower(&compiler).map_err(|e| format!("ir::lower: {e}"))?;
			codegen::compile_from_ir(&program).map_err(|e| format!("compile_from_ir: {e}"))
		}
	}
}

fn profile_one(path: &Path, backend: Backend) {
	let program = compile(&path.to_path_buf(), backend).expect("compile error");
	let buf = Rc::new(RefCell::new(Vec::<u8>::new()));
	let mut vm_instance = vm::VM::new(program).with_stdout(vm::OutputSink::Buffer(buf));
	vm_instance.profile = Some(HashMap::new());
	vm_instance
		.run()
		.unwrap_or_else(|e| panic!("run error: {}", e.message));
	let mut counts: Vec<_> = vm_instance.profile.unwrap().into_iter().collect();
	counts.sort_by(|a, b| b.1.cmp(&a.1));
	let total: u64 = counts.iter().map(|(_, n)| n).sum();
	println!("opcode counts ({} backend)", backend.label());
	println!("opcode             count       pct");
	println!("-----------------  ----------  -----");
	for (name, n) in &counts {
		let pct = (*n as f64 / total as f64) * 100.0;
		println!("{:<19}{:>10}  {:>5.1}%", name, n, pct);
	}
	println!("{:<19}{:>10}", "TOTAL", total);
}
