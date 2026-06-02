// Throwaway: run a .wasm under the V8 backend. `cargo run -p host --features v8 --example v8smoke -- <file.wasm>`
fn main() {
	let path = std::env::args().nth(1).expect("usage: v8smoke <file.wasm>");
	let bytes = std::fs::read(&path).expect("read wasm");
	let r = host::run_wasm_v8(&bytes, b"");
	eprintln!("[v8 status] {}", r.status);
	print!("{}", r.stdout);
}
