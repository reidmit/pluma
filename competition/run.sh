#!/usr/bin/env bash
#
# Benchmark Pluma against a spread of other languages.
#
# Each benchmark folder holds the same program written once per language,
# idiomatically. Every implementation prints identical output. We invoke each
# the way you actually would from a shell and report the best wall-clock time
# over several runs (best-of-N rejects noise from other activity on the machine).
#
# The columns, and how each is run:
#
#   pluma-v8   `pluma build` once, then     — the WasmGC artifact you deploy, run
#              `pluma run <out>.wasm`          under V8 (the default `pluma run`
#                                              engine — run what you ship). The
#                                              per-run time measures *executing*
#                                              the artifact; the one-time build
#                                              cost is summed and reported
#                                              separately. V8's generational GC is
#                                              what makes Pluma's boxed-value IR
#                                              fast here.
#   pluma-src  `pluma run <prog>.pa`        — the same thing from source: every run
#                                              re-does the full pipeline (tokenize →
#                                              parse → analyze → IR → wasm) and then
#                                              executes, in one process. The gap
#                                              over `pluma-v8` is the compile cost
#                                              you pay per `pluma run` of a source
#                                              file — the dev-loop reality, where
#                                              `pluma-v8` is the deploy reality.
#   grain-wasm `grain compile --release`,    — Grain also compiles to wasm (linear
#              then `grain run <out>.gr.wasm`   memory + a refcounting runtime, not
#                                              WasmGC). The mirror of `pluma-v8`:
#                                              build the release artifact once
#                                              (build cost summed separately), then
#                                              time executing it under Grain's WASI
#                                              runner. n/a when `grain` is absent.
#   grain-src  `grain <prog>.gr`            — the mirror of `pluma-src`: compile
#                                              from source *and* run every
#                                              invocation, so its time folds in the
#                                              Grain compiler pipeline each run.
#   python3    python3 <prog>.py            — CPython, a bytecode interpreter.
#   ruby       ruby <prog>.rb               — CRuby (MRI), a bytecode interpreter.
#   node       node <prog>.js               — V8, a JIT.
#   bun        bun <prog>.js                — JavaScriptCore, a *different* JIT —
#                                              the one cross-engine JS data point.
#   deno       deno run <prog>.js           — V8 again (same engine as node).
#
# Pluma's WasmGC/V8 output is the reference every other column is diffed against,
# so a row is only trusted once all the languages agree byte-for-byte.
#
# Usage:  competition/run.sh [RUNS]      (default RUNS=5)

set -u

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PLUMA="$ROOT/target/release/pluma"
RUNS="${1:-5}"
REPORT="$ROOT/competition/RESULTS.md"

# benchmark-dir : base-filename : label : one-line description
BENCHES=(
	"01-fib:fib:fib:naive recursion"
	"02-mandelbrot:mandelbrot:mandelbrot:float64 escape loop"
	"03-primes:primes:primes:integer trial division"
	"04-sort:sort:sort:sort + checksum"
	"05-dict:dict:dict:hash-map tally"
	"06-string:string:string:join / split / upcase"
	"07-tree:tree:tree:build + fold a tree"
	"08-collections:collections:collections:map / filter / fold"
	"09-interp:interp:interp:AST interpreter"
	"10-nbody:nbody:nbody:n-body float sim"
	"11-sieve:sieve:sieve:sieve of Eratosthenes"
	"12-json:jsonrt:json:JSON round-trip"
	"13-regex:regex:regex:regex scan + extract"
)

have() { command -v "$1" >/dev/null 2>&1; }

# A pathological run (e.g. a workload a backend handles far slower than the rest)
# must never wedge the whole suite, so every timed command runs under a wall-clock
# cap when a `timeout`/`gtimeout` is available. Tune with RUN_TIMEOUT (seconds);
# normal benchmarks finish in ~1–2 s, so the default leaves a wide margin.
RUN_TIMEOUT="${RUN_TIMEOUT:-30}"
TIMEOUT_CMD=""
if have timeout; then
	TIMEOUT_CMD="timeout"
elif have gtimeout; then
	TIMEOUT_CMD="gtimeout"
fi

# Time "$@" once, capturing its stdout to $1 and the `time -p` report to $2,
# honoring the wall-clock cap when a timeout command is available. Returns the
# command's exit status (124 when the cap killed it).
timed_run() {
	local out="$1" err="$2"
	shift 2
	if [ -n "$TIMEOUT_CMD" ]; then
		/usr/bin/time -p "$TIMEOUT_CMD" "$RUN_TIMEOUT" "$@" 1>"$out" 2>"$err"
	else
		/usr/bin/time -p "$@" 1>"$out" 2>"$err"
	fi
}

# Run "$@" RUNS times; print the minimum real (wall-clock) time in seconds.
# The command's stdout is captured into the file named by the first argument.
# Prints "n/a" if the command is missing, "ERR" if it exits non-zero, or
# ">Ns" if it blew past the RUN_TIMEOUT cap (slow, not crashed).
min_time() {
	local out="$1"
	shift
	if ! have "$1"; then
		: >"$out"
		echo "n/a"
		return
	fi
	local err best="" t rc
	err="$(mktemp)"
	local i
	for ((i = 0; i < RUNS; i++)); do
		# command stdout -> $out ; both command stderr and time's report -> $err
		timed_run "$out" "$err" "$@"
		rc=$?
		if [ "$rc" -ne 0 ]; then
			rm -f "$err"
			: >"$out" # drop any partial stdout so it can't false-match the diff check
			# 124 = killed by the timeout cap; report distinctly so a slow run reads
			# as ">Ns" rather than masquerading as a crash.
			if [ "$rc" -eq 124 ]; then echo ">${RUN_TIMEOUT}s"; else echo "ERR"; fi
			return
		fi
		t="$(awk '/^real/ { print $2 }' "$err")"
		if [ -z "$best" ] || awk "BEGIN { exit !($t < $best) }"; then
			best="$t"
		fi
	done
	rm -f "$err"
	echo "$best"
}

# Like min_time, but first checks that the source file exists; a deliberately
# absent port (e.g. a language that skips a benchmark) reads as "n/a", not "ERR".
time_src() {
	local out="$1" src="$2"
	shift 2
	if [ ! -f "$src" ]; then
		: >"$out"
		echo "n/a"
		return
	fi
	min_time "$out" "$@"
}

# Compile <src> to a WasmGC artifact at <outbase>.wasm (the deploy artifact).
# Prints the build's wall-clock seconds, or "ERR" if compilation failed.
build_wasm() {
	local src="$1" outbase="$2" err t
	err="$(mktemp)"
	timed_run /dev/null "$err" "$PLUMA" build "$src" -o "$outbase"
	if [ "$?" -ne 0 ]; then
		rm -f "$err"
		echo "ERR"
		return 1
	fi
	t="$(awk '/^real/ { print $2 }' "$err")"
	rm -f "$err"
	echo "$t"
}

# Compile a Grain <src> to a release WasmGC artifact at <out> (the deploy
# artifact). Prints the build's wall-clock seconds, or "ERR" if it failed.
grain_build_wasm() {
	local src="$1" out="$2" err t
	err="$(mktemp)"
	timed_run /dev/null "$err" grain compile --release -o "$out" "$src"
	if [ "$?" -ne 0 ]; then
		rm -f "$err"
		echo "ERR"
		return 1
	fi
	t="$(awk '/^real/ { print $2 }' "$err")"
	rm -f "$err"
	echo "$t"
}

# Pretty ratio "x.y×" of $1/$2, or "—" if either is non-numeric.
ratio() {
	if [[ "$1" =~ ^[0-9.]+$ && "$2" =~ ^[0-9.]+$ ]]; then
		awk "BEGIN { printf \"%.1fx\", $1 / $2 }"
	else
		echo "—"
	fi
}

if [ ! -x "$PLUMA" ]; then
	echo "error: $PLUMA not found. Build it first:  cargo build --release --bin pluma" >&2
	exit 1
fi

echo "Pluma (WasmGC/V8) vs Grain (wasm/refcount), Python, Ruby, Node, Bun, Deno  —  best of $RUNS runs, seconds (lower is better)"
echo
printf '%-12s %8s %9s %10s %9s %8s %6s %6s %6s %6s %9s   %s\n' \
	"benchmark" "pluma-v8" "pluma-src" "grain-wasm" "grain-src" "python3" "ruby" "node" "bun" "deno" "vs best" "output"
printf '%s\n' "----------------------------------------------------------------------------------------------------------------"

pv8o="$(mktemp)"
psrco="$(mktemp)"
gwo="$(mktemp)"
gso="$(mktemp)"
pyo="$(mktemp)"
rbo="$(mktemp)"
jso="$(mktemp)"
buno="$(mktemp)"
denoo="$(mktemp)"
WASMDIR="$(mktemp -d)"

md_rows=""          # accumulated markdown table rows for RESULTS.md
mismatch="no"
build_total=0       # summed one-time pluma compile-to-wasm cost across all benchmarks
grain_build_total=0 # summed one-time grain compile-to-wasm cost across all benchmarks

for entry in "${BENCHES[@]}"; do
	dir="${entry%%:*}"
	rest="${entry#*:}"
	base="${rest%%:*}"
	rest="${rest#*:}"
	label="${rest%%:*}"
	desc="${rest#*:}"
	d="$ROOT/competition/$dir"

	# `pluma-v8`: compile to the WasmGC deploy artifact once (build cost is a one-
	# time price, not part of the per-run number), then time executing that artifact
	# under V8 — the same thing `pluma run <out>.wasm` does, the engine you deploy.
	# This artifact's stdout is the oracle every competitor is diffed against below.
	bt="$(build_wasm "$d/$base" "$WASMDIR/$base")"
	if [ "$bt" = "ERR" ]; then
		pv8="n/a"
		: >"$pv8o" # clear stale output so the diff below has no oracle to match
	else
		[[ "$bt" =~ ^[0-9.]+$ ]] && build_total="$(awk "BEGIN { print $build_total + $bt }")"
		pv8="$(min_time "$pv8o" "$PLUMA" run "$WASMDIR/$base.wasm")"
	fi

	# `pluma-src`: `pluma run <source>` — compiles from source *and* executes every
	# invocation, so its time folds the full tokenize/parse/analyze/IR/wasm pipeline
	# into each run. The delta over `pluma-v8` is what compiling the source costs.
	psrct="$(min_time "$psrco" "$PLUMA" run "$d/$base")"

	# `grain-wasm`: Grain also compiles to WasmGC. Mirror `pluma-v8` — compile to a
	# release `.wasm` artifact once (build cost summed separately), then time
	# executing that artifact under Grain's WASI runner. `grain-src`: `grain <src>`
	# compiles *and* runs from source every invocation, the analog of `pluma-src`.
	if [ -f "$d/$base.gr" ] && have grain; then
		gbt="$(grain_build_wasm "$d/$base.gr" "$WASMDIR/$base.gr.wasm")"
		if [ "$gbt" = "ERR" ]; then
			gwasm="ERR"
			: >"$gwo"
		else
			[[ "$gbt" =~ ^[0-9.]+$ ]] && grain_build_total="$(awk "BEGIN { print $grain_build_total + $gbt }")"
			gwasm="$(min_time "$gwo" grain run "$WASMDIR/$base.gr.wasm")"
		fi
		gsrc="$(min_time "$gso" grain "$d/$base.gr")"
	else
		gwasm="n/a"
		gsrc="n/a"
		: >"$gwo"
		: >"$gso"
	fi

	pyt="$(time_src "$pyo" "$d/$base.py" python3 "$d/$base.py")"
	rbt="$(time_src "$rbo" "$d/$base.rb" ruby "$d/$base.rb")"
	jt="$(time_src "$jso" "$d/$base.js" node "$d/$base.js")"
	bunt="$(time_src "$buno" "$d/$base.js" bun "$d/$base.js")"
	denot="$(time_src "$denoo" "$d/$base.js" deno run --quiet "$d/$base.js")"

	# Verify every competitor that ran produced the same output as the WasmGC/V8
	# artifact (the reference). If the v8 build/run failed there is no oracle to
	# compare against, so the diff check is skipped rather than false-flagging.
	if [ "$pv8" = "n/a" ] || [ ! -s "$pv8o" ]; then
		status="no-ref"
	else
		status="ok"
		for f in "$psrco" "$gwo" "$gso" "$pyo" "$rbo" "$jso" "$buno" "$denoo"; do
			if [ -s "$f" ] && ! diff -q "$pv8o" "$f" >/dev/null 2>&1; then
				status="MISMATCH"
				mismatch="yes"
			fi
		done
	fi

	# Fastest of all the competitors, and how the deploy artifact compares to it.
	# Grain's deploy artifact (grain-wasm) is in the pool; grain-src, like
	# pluma-src, folds in compile cost and is excluded — the same rule as above.
	best_other="$(printf '%s\n' "$gwasm" "$pyt" "$rbt" "$jt" "$bunt" "$denot" |
		awk '/^[0-9.]+$/ { if (m == "" || $1 < m) m = $1 } END { print (m == "" ? "n/a" : m) }')"
	# `vs best` is the deploy reality — the artifact you ship vs the fastest column.
	vs_v8="$(ratio "$pv8" "$best_other")"

	printf '%-12s %8s %9s %10s %9s %8s %6s %6s %6s %6s %9s   %s\n' \
		"$label" "$pv8" "$psrct" "$gwasm" "$gsrc" "$pyt" "$rbt" "$jt" "$bunt" "$denot" "$vs_v8" "$status"
	md_rows+="| \`$label\` | $desc | $pv8 | $psrct | $gwasm | $gsrc | $pyt | $rbt | $jt | $bunt | $denot | $vs_v8 | $status |"$'\n'
done

rm -f "$pv8o" "$psrco" "$gwo" "$gso" "$pyo" "$rbo" "$jso" "$buno" "$denoo"
rm -rf "$WASMDIR"

build_total="$(awk "BEGIN { printf \"%.2f\", $build_total }")"
grain_build_total="$(awk "BEGIN { printf \"%.2f\", $grain_build_total }")"

# ---- Write the markdown report -------------------------------------------------
os="$(uname -sm)"
pluma_ver="$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"
py_ver="$(python3 --version 2>&1 | head -1)"
rb_ver="$(ruby --version 2>&1 | awk '{ print $1, $2 }')"
node_ver="$(node --version 2>&1 | head -1)"
bun_ver="$(bun --version 2>&1 | head -1)"
deno_ver="$(deno --version 2>&1 | head -1)"
grain_ver="$(grain --version 2>&1 | head -1)"
overall="every implementation agreed on every output"
[ "$mismatch" = "yes" ] && overall="**one or more benchmarks produced mismatched output — results are not comparable**"

{
	echo "# Pluma vs Grain, Python, Ruby, Node, Bun, and Deno — benchmark results"
	echo
	echo "_Best of $RUNS runs, wall-clock seconds (lower is better). Generated $(date '+%Y-%m-%d %H:%M:%S %Z')._"
	echo
	echo "Correctness: $overall."
	echo
	echo "## Environment"
	echo
	echo "| component | version |"
	echo "|---|---|"
	echo "| host | \`$os\` |"
	echo "| pluma | git \`$pluma_ver\` (release build) |"
	echo "| python3 | $py_ver |"
	echo "| ruby | $rb_ver |"
	echo "| node | $node_ver |"
	echo "| bun | $bun_ver |"
	echo "| deno | $deno_ver |"
	echo "| grain | $grain_ver |"
	echo
	echo "## Results"
	echo
	echo "| benchmark | exercises | pluma-v8 | pluma-src | grain-wasm | grain-src | python3 | ruby | node | bun | deno | vs best | output |"
	echo "|---|---|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|:--:|"
	printf '%s' "$md_rows"
	echo
	echo "\`pluma-v8\` runs a prebuilt WasmGC artifact (\`pluma run <out>.wasm\`); \`pluma-src\` runs from source"
	echo "(\`pluma run <prog>.pa\`), so its time folds the full tokenize/parse/analyze/IR/wasm pipeline into every"
	echo "run — the gap between the two columns is that compile cost. \`grain-wasm\` is the analogous Grain deploy"
	echo "artifact (\`grain compile --release\`, then \`grain run <out>.gr.wasm\`); \`grain-src\` is \`grain <prog>.gr\`,"
	echo "compiling and running from source each invocation. \`vs best\` compares the deploy artifact (\`pluma-v8\`)"
	echo "against the fastest other language (including \`grain-wasm\`)."
	echo
	echo "One-time build cost, summed across all ${#BENCHES[@]} benchmarks and **not** included in the per-run times above:"
	echo "Pluma compile-to-WasmGC **${build_total}s**; Grain compile-to-wasm **${grain_build_total}s**."
	echo
	echo "Regenerate with \`competition/run.sh [RUNS]\`."
} >"$REPORT"

echo
echo "Wrote markdown report to ${REPORT#"$ROOT"/}"
