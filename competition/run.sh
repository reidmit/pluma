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
# Pluma has one backend over its IR, the WasmGC/V8 deploy artifact:
#
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
# under V8, the deploy engine. See git history for the old multi-column form.)
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

echo "Pluma (WasmGC/V8) vs Python vs Ruby vs Node.js  —  best of $RUNS runs, seconds (lower is better)"
echo
printf '%-12s %9s %8s %7s %7s %10s   %s\n' \
	"benchmark" "pluma-v8" "python3" "ruby" "node" "v8 vs best" "output"
printf '%s\n' "------------------------------------------------------------------------------"

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

	# `pluma-v8`: compile to the WasmGC deploy artifact once (build cost is a one-
	# time price, not part of the per-run number), then time executing that artifact
	# under V8 — the same thing `pluma run <out>.wasm` does, the engine you deploy.
	# This artifact's stdout is the oracle every competitor is diffed against below.
	bt="$(build_wasm "$d/$name" "$WASMDIR/$name")"
	if [ "$bt" = "ERR" ]; then
		pv8="n/a"
		: >"$pv8o"     # clear stale output so the diff below has no oracle to match
	else
		[[ "$bt" =~ ^[0-9.]+$ ]] && build_total="$(awk "BEGIN { print $build_total + $bt }")"
		pv8="$(min_time "$pv8o" "$PLUMA" run "$WASMDIR/$name.wasm")"
	fi

	pyt="$(min_time "$pyo" python3 "$d/$name.py")"
	rbt="$(min_time "$rbo" ruby "$d/$name.rb")"
	jt="$(min_time "$jso" node "$d/$name.js")"

	# Verify every competitor produced the same output as the WasmGC/V8 artifact (the
	# reference). If the v8 build/run failed there is no oracle to compare against, so
	# the diff check is skipped rather than false-flagging an empty oracle file.
	if [ "$pv8" = "n/a" ] || [ ! -s "$pv8o" ]; then
		status="no-ref"
	else
		status="ok"
		for f in "$pyo" "$rbo" "$jso"; do
			if [ -s "$f" ] && ! diff -q "$pv8o" "$f" >/dev/null 2>&1; then
				status="MISMATCH"
				mismatch="yes"
			fi
		done
	fi

	# Fastest of the three competitors, and how the deploy artifact compares to it.
	best_other="$(printf '%s\n' "$pyt" "$rbt" "$jt" |
		awk '/^[0-9.]+$/ { if (m == "" || $1 < m) m = $1 } END { print (m == "" ? "n/a" : m) }')"
	# `v8 vs best` is the deploy reality — the artifact you ship vs the fastest competitor.
	vs_v8="$(ratio "$pv8" "$best_other")"

	printf '%-12s %9s %8s %7s %7s %10s   %s\n' \
		"$name" "$pv8" "$pyt" "$rbt" "$jt" "$vs_v8" "$status"
	md_rows+="| \`$name\` | $desc | $pv8 | $pyt | $rbt | $jt | $vs_v8 | $status |"$'\n'
done

rm -f "$pv8o" "$pyo" "$rbo" "$jso"
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
	echo "| benchmark | exercises | pluma-v8 | python3 | ruby | node | v8 vs best | output |"
	echo "|---|---|--:|--:|--:|--:|--:|:--:|"
	printf '%s' "$md_rows"
	echo
	echo "One-time cost to compile all ${#BENCHES[@]} benchmarks to WasmGC artifacts: **${build_total}s** total (not included in the per-run \`pluma-v8\` times)."
	echo
	echo "Regenerate with \`competition/run.sh [RUNS]\`."
} >"$REPORT"

echo
echo "Wrote markdown report to ${REPORT#"$ROOT"/}"
