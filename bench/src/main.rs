// Microbench runner. For each program under benchmarks/programs/<name>/main.pa,
// runs it through both backends (VM and interpreter), wall-clocks each, and
// prints a comparison.
//
// Usage:
//   cargo run -p bench --release
//
// Output captured-but-discarded so we measure execution, not stdout I/O. The
// interpreter is skipped for programs that would overflow its stack.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

fn main() {
	let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
	let programs_dir = workspace.join("benchmarks/programs");
	std::env::set_current_dir(workspace).ok();

	// `cargo run -p bench -- --profile <name>` dumps opcode counts for one
	// benchmark and exits, instead of running the timing comparison.
	let mut args = std::env::args().skip(1);
	if let Some(arg) = args.next() {
		if arg == "--profile" {
			let name = args.next().expect("--profile takes a benchmark name");
			let main_pa = programs_dir.join(&name).join("main.pa");
			profile_one(&main_pa);
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

	println!(
		"benchmark            iters   vm (avg)        interp (avg)    speedup"
	);
	println!(
		"-------------------  ------  --------------  --------------  -------"
	);

	for entry in benchmarks {
		let name = entry.file_name().to_string_lossy().to_string();
		let main_pa = entry.path().join("main.pa");
		if !main_pa.exists() {
			continue;
		}

		let path_a = main_pa.clone();
		let path_b = main_pa.clone();
		let vm_time = time_runs(iterations, move || run_via_vm(&path_a));
		let interp_time = time_runs(iterations, move || run_via_interp(&path_b));

		print!("{:<21}{:>4}    ", name, iterations);
		match vm_time {
			Some(d) => print!("{:>12}  ", format_duration(d)),
			None => print!("{:>12}  ", "ERROR"),
		}
		match interp_time {
			Some(d) => print!("{:>12}  ", format_duration(d)),
			None => print!("{:>12}  ", "n/a"),
		}
		match (vm_time, interp_time) {
			(Some(vm), Some(interp)) => {
				let ratio = interp.as_secs_f64() / vm.as_secs_f64();
				println!("{:>5.2}x", ratio);
			}
			_ => println!(""),
		}
	}
}

fn time_runs<F>(iterations: usize, f: F) -> Option<Duration>
where
	F: Fn() -> Result<(), String> + Send + Sync + 'static + Clone,
{
	// Run on a thread with a big stack so the interpreter doesn't overflow
	// on deep-but-fine recursion benchmarks. The VM is unaffected, but it's
	// simpler to use the same harness for both. Anything that still
	// overflows reports as None and is shown as ERROR/OVERFLOW.
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
		.stack_size(64 * 1024 * 1024)
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

fn run_via_vm(path: &PathBuf) -> Result<(), String> {
	let mut compiler = compiler::Compiler::from_entry_path(path.to_str().unwrap().to_string())
		.map_err(|d| format!("compile: {:?} diagnostics", d.len()))?;
	vm::stdlib::register_compiler(&mut compiler);
	compiler.check().map_err(|d| format!("check: {:?} diagnostics", d.len()))?;
	let program = codegen::compile(&compiler).map_err(|e| format!("codegen: {}", e))?;
	let buf = Rc::new(RefCell::new(Vec::<u8>::new()));
	let mut vm_instance = vm::VM::new(program).with_stdout(vm::StdoutSink::Buffer(buf));
	vm_instance.run().map(|_| ()).map_err(|e| e.message)
}

fn profile_one(path: &Path) {
	let mut compiler = compiler::Compiler::from_entry_path(path.to_str().unwrap().to_string())
		.unwrap_or_else(|_| panic!("compile error"));
	vm::stdlib::register_compiler(&mut compiler);
	compiler.check().unwrap_or_else(|_| panic!("check error"));
	let program = codegen::compile(&compiler).expect("codegen error");
	let buf = Rc::new(RefCell::new(Vec::<u8>::new()));
	let mut vm_instance = vm::VM::new(program).with_stdout(vm::StdoutSink::Buffer(buf));
	vm_instance.profile = Some(std::collections::HashMap::new());
	vm_instance
		.run()
		.unwrap_or_else(|e| panic!("run error: {}", e.message));
	let mut counts: Vec<_> = vm_instance.profile.unwrap().into_iter().collect();
	counts.sort_by(|a, b| b.1.cmp(&a.1));
	let total: u64 = counts.iter().map(|(_, n)| n).sum();
	println!("opcode             count       pct");
	println!("-----------------  ----------  -----");
	for (name, n) in &counts {
		let pct = (*n as f64 / total as f64) * 100.0;
		println!("{:<19}{:>10}  {:>5.1}%", name, n, pct);
	}
	println!("{:<19}{:>10}", "TOTAL", total);
}

fn run_via_interp(path: &PathBuf) -> Result<(), String> {
	let mut compiler = compiler::Compiler::from_entry_path(path.to_str().unwrap().to_string())
		.map_err(|d| format!("compile: {:?} diagnostics", d.len()))?;
	interpreter::stdlib::register_compiler(&mut compiler);
	compiler.check().map_err(|d| format!("check: {:?} diagnostics", d.len()))?;
	let buf = Rc::new(RefCell::new(Vec::<u8>::new()));
	let mut interp = interpreter::Interpreter::new(&compiler)
		.with_stdout(interpreter::StdoutSink::Buffer(buf));
	interpreter::stdlib::register_runtime(&mut interp);
	interp.run().map(|_| ()).map_err(|e| e.message)
}
