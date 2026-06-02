#!/usr/bin/env bash
#
# Benchmark Pluma against Python, Ruby, and Node.js.
#
# Each benchmark folder holds the same program written four times, idiomatically,
# in each language. Every implementation prints identical output. We invoke each
# the way you actually would from a shell:
#
#     ./target/release/cli <prog>      python3 <prog>.py
#     ruby <prog>.rb                   node <prog>.js
#
# and report the best wall-clock time over several runs (best-of-N rejects noise
# from other activity on the machine).
#
# Pluma ships two backends over one IR, so it is measured two ways:
#
#   pluma-vm    `pluma run --vm <src>`     — the reference bytecode interpreter: the
#                                            dev/test oracle, NOT a deploy target. The
#                                            time includes front-end compilation,
#                                            because that is what the dev loop actually
#                                            costs every run.
#   pluma-v8    `pluma build` once, then    — the WasmGC artifact you deploy, run under
#               `pluma run <out>.wasm`        V8 (the default `pluma run` engine — run
#                                            what you ship). The same `.wasm` `pluma
#                                            build` emits; the per-run time measures
#                                            *executing* it, with the one-time build
#                                            cost reported separately. V8's generational
#                                            GC is what makes Pluma's boxed-value IR fast
#                                            here — it bulk-frees the per-iteration
#                                            transients a reference-counting collector
#                                            would churn on.
#
# (Earlier revisions also timed the artifact under wasmtime's `null` and `drc`
# collectors. Wasmtime has since been retired entirely — every WasmGC artifact runs
# under V8, the deploy engine, both here and in the `conformance` differential — so
# those collector columns are gone. See git history for the old three-column form.)
#
# Usage:  competition/run.sh [RUNS]      (default RUNS=5)

set -u

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PLUMA="$ROOT/target/release/cli"
RUNS="${1:-5}"
REPORT="$ROOT/competition/RESULTS.md"

# benchmark-dir : base-filename : one-line description
BENCHES=(
	"01-fib:fib:naive recursion"
	"02-mandelbrot:mandelbrot:float64 escape loop"
	"03-primes:primes:integer trial division"
	"04-sort:sort:sort + checksum"
	"05-dict:dict:hash-map tally"
	"06-string:string:join / split / upcase"
	"07-tree:tree:build + fold a tree"
	"08-collections:collections:map / filter / fold"
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

# Run "$@" RUNS times; print the minimum real (wall-clock) time in seconds.
# The command's stdout is captured into the file named by the first argument.
# Prints "n/a" if the command is missing, "ERR" if it exits non-zero, or
# ">Ns" if it blew past the RUN_TIMEOUT cap (slow, not crashed).
min_time() {
	local out="$1"
	shift
	if ! have "$1"; then
		echo "n/a"
		return
	fi
	local err best="" t rc
	err="$(mktemp)"
	local i
	for ((i = 0; i < RUNS; i++)); do
		# command stdout -> $out ; both command stderr and time's report -> $err
		if [ -n "$TIMEOUT_CMD" ]; then
			/usr/bin/time -p "$TIMEOUT_CMD" "$RUN_TIMEOUT" "$@" 1>"$out" 2>"$err"
		else
			/usr/bin/time -p "$@" 1>"$out" 2>"$err"
		fi
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

# Compile <src> to a WasmGC artifact at <outbase>.wasm (the deploy artifact).
# Prints the build's wall-clock seconds, or "ERR" if compilation failed.
build_wasm() {
	local src="$1" outbase="$2" err t
	err="$(mktemp)"
	if [ -n "$TIMEOUT_CMD" ]; then
		/usr/bin/time -p "$TIMEOUT_CMD" "$RUN_TIMEOUT" "$PLUMA" build --target server "$src" -o "$outbase" 1>/dev/null 2>"$err"
	else
		/usr/bin/time -p "$PLUMA" build --target server "$src" -o "$outbase" 1>/dev/null 2>"$err"
	fi
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
	echo "error: $PLUMA not found. Build it first:  cargo build --release --bin cli" >&2
	exit 1
fi

echo "Pluma (VM + WasmGC/V8) vs Python vs Ruby vs Node.js  —  best of $RUNS runs, seconds (lower is better)"
echo
printf '%-12s %8s %9s %8s %7s %7s %10s %10s   %s\n' \
	"benchmark" "pluma-vm" "pluma-v8" "python3" "ruby" "node" "vm vs best" "v8 vs best" "output"
printf '%s\n' "----------------------------------------------------------------------------------------------"

po="$(mktemp)"
pv8o="$(mktemp)"
pyo="$(mktemp)"
rbo="$(mktemp)"
jso="$(mktemp)"
WASMDIR="$(mktemp -d)"

md_rows=""        # accumulated markdown table rows for RESULTS.md
mismatch="no"
build_total=0     # summed one-time compile-to-wasm cost across all benchmarks

for entry in "${BENCHES[@]}"; do
	dir="${entry%%:*}"
	rest="${entry#*:}"
	name="${rest%%:*}"
	desc="${rest#*:}"
	d="$ROOT/competition/$dir"

	# `pluma-vm`: force the bytecode VM (the reference interpreter). Its output is
	# the oracle every other backend is diffed against below.
	pt="$(min_time "$po" "$PLUMA" run --vm "$d/$name")"

	# `pluma-v8`: compile to the WasmGC deploy artifact once (build cost is a one-
	# time price, not part of the per-run number), then time executing that artifact
	# under V8 — the same thing `pluma run <out>.wasm` does, the engine you deploy.
	bt="$(build_wasm "$d/$name" "$WASMDIR/$name")"
	if [ "$bt" = "ERR" ]; then
		pv8="n/a"
		: >"$pv8o"     # clear stale output so the diff below doesn't false-match
	else
		[[ "$bt" =~ ^[0-9.]+$ ]] && build_total="$(awk "BEGIN { print $build_total + $bt }")"
		pv8="$(min_time "$pv8o" "$PLUMA" run "$WASMDIR/$name.wasm")"
	fi

	pyt="$(min_time "$pyo" python3 "$d/$name.py")"
	rbt="$(min_time "$rbo" ruby "$d/$name.rb")"
	jt="$(min_time "$jso" node "$d/$name.js")"

	# Verify every other backend produced the same output as Pluma's VM (the
	# reference). The WasmGC/V8 artifact is held to the same bar as the competitors.
	status="ok"
	for f in "$pv8o" "$pyo" "$rbo" "$jso"; do
		if [ -s "$f" ] && ! diff -q "$po" "$f" >/dev/null 2>&1; then
			status="MISMATCH"
			mismatch="yes"
		fi
	done

	# Fastest of the three competitors, and how each Pluma backend compares to it.
	best_other="$(printf '%s\n' "$pyt" "$rbt" "$jt" |
		awk '/^[0-9.]+$/ { if (m == "" || $1 < m) m = $1 } END { print (m == "" ? "n/a" : m) }')"
	vs_vm="$(ratio "$pt" "$best_other")"
	# `v8 vs best` is the deploy reality — the artifact you ship vs the fastest competitor.
	vs_v8="$(ratio "$pv8" "$best_other")"

	printf '%-12s %8s %9s %8s %7s %7s %10s %10s   %s\n' \
		"$name" "$pt" "$pv8" "$pyt" "$rbt" "$jt" "$vs_vm" "$vs_v8" "$status"
	md_rows+="| \`$name\` | $desc | $pt | $pv8 | $pyt | $rbt | $jt | $vs_vm | $vs_v8 | $status |"$'\n'
done

rm -f "$po" "$pv8o" "$pyo" "$rbo" "$jso"
rm -rf "$WASMDIR"

build_total="$(awk "BEGIN { printf \"%.2f\", $build_total }")"

# ---- Write the markdown report -------------------------------------------------
os="$(uname -sm)"
pluma_ver="$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"
py_ver="$(python3 --version 2>&1 | head -1)"
rb_ver="$(ruby --version 2>&1 | awk '{ print $1, $2 }')"
node_ver="$(node --version 2>&1 | head -1)"
overall="every implementation agreed on every output"
[ "$mismatch" = "yes" ] && overall="**one or more benchmarks produced mismatched output — results are not comparable**"

{
	echo "# Pluma vs Python, Ruby, and Node.js — benchmark results"
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
	echo
	echo "## Results"
	echo
	echo "| benchmark | exercises | pluma-vm | pluma-v8 | python3 | ruby | node | vm vs best | v8 vs best | output |"
	echo "|---|---|--:|--:|--:|--:|--:|--:|--:|:--:|"
	printf '%s' "$md_rows"
	echo
	echo "One-time cost to compile all ${#BENCHES[@]} benchmarks to WasmGC artifacts: **${build_total}s** total (not included in the per-run \`pluma-v8\` times)."
	echo
	echo "## How to read this"
	echo
	echo "- Pluma ships **two backends over one IR**, so it appears twice:"
	echo "  - \`pluma-vm\` — \`pluma run --vm <src>\`, the reference bytecode interpreter."
	echo "    It is the dev/test oracle (and the differential reference the deploy backend"
	echo "    is cross-checked against), **not** a deploy target. The time includes front-end"
	echo "    compilation, because that is what the dev loop costs every run."
	echo "  - \`pluma-v8\` — the WasmGC artifact you deploy (\`pluma build\` once, then"
	echo "    \`pluma run <out>.wasm\`), executed under **V8** — the default \`pluma run\`"
	echo "    engine, so this is *run what you ship*. The per-run time measures executing"
	echo "    the artifact; the one-time compile-to-wasm cost is reported separately above."
	echo "    V8's **generational garbage collector** is what makes Pluma's boxed-value IR"
	echo "    fast here: it bulk-frees the short-lived per-iteration allocations that a"
	echo "    reference-counting collector would churn on one at a time."
	echo "- \`vm vs best\` / \`v8 vs best\` divide a Pluma time by the fastest competitor's"
	echo "  time (greater than 1× means Pluma is slower; less than 1× means faster)."
	echo "  \`v8 vs best\` is the deploy reality — the artifact you ship vs the field."
	echo "- \`output\` = \`ok\` means Pluma (both backends) and all three competitors printed"
	echo "  byte-identical results; \`MISMATCH\` means they disagreed and the row should not"
	echo "  be trusted."
	echo "- A time cell may instead read \`n/a\` (tool not installed), \`ERR\` (exited non-zero),"
	echo "  or \`>${RUN_TIMEOUT}s\` (still running when the per-run cap fired — the workload is"
	echo "  far slower on that backend, not crashed). Such cells are excluded from the"
	echo "  ratio and the output check."
	echo "- This compares **idiomatic code in each language**. \`core.dict\` is a persistent,"
	echo "  structurally-shared map (O(log n) insert, immutable, insertion-ordered);"
	echo "  \`list.sort\` is a Pluma-level merge sort and the string ops are Pluma-level too,"
	echo "  versus the other languages' native mutable maps and C-level sort/string routines."
	echo "- Where a competitor finishes in well under ~0.1 s it is essentially measuring"
	echo "  interpreter startup, not the workload."
	echo "- Regenerate with \`competition/run.sh [RUNS]\`."
} >"$REPORT"

echo
echo "Notes:"
echo "  - 'pluma-vm' forces the reference bytecode VM ('pluma run --vm <src>'): it compiles"
echo "    and interprets each run, so the time is the dev-loop cost. It is the oracle every"
echo "    other backend's output is diffed against — not a deploy target."
echo "  - 'pluma-v8' is the WasmGC artifact you deploy: 'pluma build' once, then"
echo "    'pluma run <out>.wasm', executed under V8 (the default 'pluma run' engine). V8's"
echo "    generational GC is what makes the boxed-value IR fast. Compiling all ${#BENCHES[@]}"
echo "    benchmarks took ${build_total}s total, one time, and is NOT in the per-run numbers."
echo "  - 'vm vs best' / 'v8 vs best' are a Pluma time / the fastest competitor's time."
echo "    >1x means Pluma is slower; <1x means faster. 'v8 vs best' is the deploy reality."
echo "  - 'output' = MISMATCH means the programs disagreed on their result."
echo "  - core.dict is a persistent, structurally-shared map (O(log n) insert); list.sort"
echo "    is a Pluma-level merge sort and the string ops are Pluma-level too. The others"
echo "    use native mutable hash maps and C-level sort/string routines."
echo
echo "Wrote markdown report to ${REPORT#"$ROOT"/}"
