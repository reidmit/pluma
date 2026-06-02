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
# Pluma is measured three ways, because it ships two backends over one IR and the
# WasmGC artifact's speed depends heavily on which collector wasmtime runs it under:
#
#   pluma-vm    `pluma run <src>`          — the reference VM interpreter; the
#                                            time includes front-end compilation,
#                                            because that is what the dev loop
#                                            actually costs every run.
#   wasm-null   `pluma build` once, run    — the WasmGC artifact under wasmtime's
#               under PLUMA_WASM_GC=null      null collector (allocate, never free):
#                                            a no-GC *floor*. Fastest possible, but
#                                            it OOMs any long-lived deploy, so it is
#                                            a best-case bound, not a deploy figure.
#   wasm-drc    same artifact, run under   — the same artifact under wasmtime's
#               PLUMA_WASM_GC=drc            deferred-reference-counting collector,
#                                            the only *real* WasmGC collector it
#                                            ships: the deploy *ceiling*. Costly
#                                            here because Pluma boxes every value,
#                                            so refcounting churns on every
#                                            transient; the true deploy cost sits
#                                            between the two bounds. The one-time
#                                            build cost is reported separately.
#
# Usage:  competition/run.sh [RUNS]      (default RUNS=5)

set -u

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PLUMA="$ROOT/target/release/cli"
RUNS="${1:-5}"
REPORT="$ROOT/competition/RESULTS.md"

# The WasmGC artifact is timed under both collectors `host::engine()` exposes via
# the PLUMA_WASM_GC env var (set per-invocation below): `null` (no-GC floor) and
# `drc` (real-collector ceiling). See the header comment for why both are reported.

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

echo "Pluma (VM + WasmGC) vs Python vs Ruby vs Node.js  —  best of $RUNS runs, seconds (lower is better)"
echo
printf '%-12s %8s %9s %9s %8s %7s %7s %10s %12s   %s\n' \
	"benchmark" "pluma-vm" "wasm-null" "wasm-drc" "python3" "ruby" "node" "vm vs best" "wasm vs best" "output"
printf '%s\n' "----------------------------------------------------------------------------------------------------------------"

po="$(mktemp)"
pwo="$(mktemp)"
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

	pt="$(min_time "$po" "$PLUMA" run "$d/$name")"

	# Compile to a WasmGC artifact once (build cost is a one-time price, not part
	# of the per-run number), then time executing that one artifact under BOTH
	# collectors: null = the no-GC floor (allocate, never free — fastest, but OOMs
	# any real deploy), drc = wasmtime's only real collector, the deploy ceiling.
	# Same `.wasm`, same output; only the host's collector differs.
	bt="$(build_wasm "$d/$name" "$WASMDIR/$name")"
	if [ "$bt" = "ERR" ]; then
		pwt_null="n/a"
		pwt_drc="n/a"
		: >"$pwo"      # clear stale output so the diff below doesn't false-match
	else
		[[ "$bt" =~ ^[0-9.]+$ ]] && build_total="$(awk "BEGIN { print $build_total + $bt }")"
		pwt_null="$(min_time "$pwo" env PLUMA_WASM_GC=null "$PLUMA" run "$WASMDIR/$name.wasm")"
		pwt_drc="$(min_time "$pwo" env PLUMA_WASM_GC=drc "$PLUMA" run "$WASMDIR/$name.wasm")"
	fi

	pyt="$(min_time "$pyo" python3 "$d/$name.py")"
	rbt="$(min_time "$rbo" ruby "$d/$name.rb")"
	jt="$(min_time "$jso" node "$d/$name.js")"

	# Verify every other backend produced the same output as Pluma's VM (the
	# reference). The WasmGC artifact is held to the same bar as the competitors.
	status="ok"
	for f in "$pwo" "$pyo" "$rbo" "$jso"; do
		if [ -s "$f" ] && ! diff -q "$po" "$f" >/dev/null 2>&1; then
			status="MISMATCH"
			mismatch="yes"
		fi
	done

	# Fastest of the three competitors, and how each Pluma backend compares to it.
	best_other="$(printf '%s\n' "$pyt" "$rbt" "$jt" |
		awk '/^[0-9.]+$/ { if (m == "" || $1 < m) m = $1 } END { print (m == "" ? "n/a" : m) }')"
	vs_vm="$(ratio "$pt" "$best_other")"
	# `wasm vs best` uses the drc (real-collector) number — the deploy reality, not
	# the no-GC floor.
	vs_wa="$(ratio "$pwt_drc" "$best_other")"

	printf '%-12s %8s %9s %9s %8s %7s %7s %10s %12s   %s\n' \
		"$name" "$pt" "$pwt_null" "$pwt_drc" "$pyt" "$rbt" "$jt" "$vs_vm" "$vs_wa" "$status"
	md_rows+="| \`$name\` | $desc | $pt | $pwt_null | $pwt_drc | $pyt | $rbt | $jt | $vs_vm | $vs_wa | $status |"$'\n'
done

rm -f "$po" "$pwo" "$pyo" "$rbo" "$jso"
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
	echo "| benchmark | exercises | pluma-vm | wasm (null) | wasm (drc) | python3 | ruby | node | vm vs best | wasm vs best | output |"
	echo "|---|---|--:|--:|--:|--:|--:|--:|--:|--:|:--:|"
	printf '%s' "$md_rows"
	echo
	echo "One-time cost to compile all ${#BENCHES[@]} benchmarks to WasmGC artifacts: **${build_total}s** total (not included in the per-run \`wasm\` times)."
	echo
	echo "## How to read this"
	echo
	echo "- Pluma ships **two backends over one IR**, and the WasmGC artifact's speed"
	echo "  depends heavily on the collector it runs under, so it appears three times:"
	echo "  - \`pluma-vm\` — \`pluma run <src>\`, the reference VM interpreter. The time"
	echo "    includes front-end compilation, because that is what the dev loop costs"
	echo "    every run."
	echo "  - \`wasm (null)\` — the WasmGC artifact (\`pluma build\` once, then"
	echo "    \`pluma run <out>.wasm\`) run under wasmtime's **null collector**:"
	echo "    allocate, never free. This is a **no-GC floor** — the fastest the artifact"
	echo "    can possibly go, but it OOMs any long-lived program, so it is a best-case"
	echo "    bound and **not a real deploy configuration**."
	echo "  - \`wasm (drc)\` — the *same* artifact under wasmtime's **deferred-reference-"
	echo "    counting collector**, the only real WasmGC collector wasmtime ships. This is"
	echo "    the **deploy ceiling**. It is costly here because Pluma's IR boxes every"
	echo "    value (every \`int\` is a heap object), so reference counting churns on every"
	echo "    transient — the worst-fit collector for this allocation pattern. A tracing /"
	echo "    generational collector (which wasmtime does not yet offer for WasmGC) would"
	echo "    bulk-free instead and land much closer to the floor. **The true deploy cost"
	echo "    sits between \`null\` and \`drc\`**; until wasmtime ships a tracing GC, \`drc\` is"
	echo "    what a deploy actually pays."
	echo "  - The one-time build cost is reported separately above; \`build once, run many\`,"
	echo "    so the per-run times measure *executing* the artifact, not compiling it."
	echo "- \`vm vs best\` / \`wasm vs best\` divide a Pluma time by the fastest competitor's"
	echo "  time (greater than 1× means Pluma is slower; less than 1× means faster)."
	echo "  \`wasm vs best\` uses the \`drc\` number — the deploy reality, not the no-GC floor."
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
echo "  - 'pluma-vm' runs the source through the reference VM (compile + interpret each"
echo "    run). 'wasm-null' and 'wasm-drc' run the SAME precompiled WasmGC artifact in"
echo "    wasmtime under two collectors: null (allocate-never-free; a no-GC floor that"
echo "    OOMs any real deploy) and drc (wasmtime's only real collector; the deploy"
echo "    ceiling). The true deploy cost sits between. Compiling all ${#BENCHES[@]} benchmarks took"
echo "    ${build_total}s total, one time, and is NOT in the per-run wasm numbers."
echo "  - drc is costly because Pluma boxes every value, so refcounting churns on every"
echo "    transient; a tracing GC (not yet in wasmtime for WasmGC) would land near null."
echo "  - 'vm vs best' / 'wasm vs best' are a Pluma time / the fastest competitor's time"
echo "    ('wasm vs best' uses drc). >1x means Pluma is slower; <1x means faster."
echo "  - 'output' = MISMATCH means the programs disagreed on their result."
echo "  - core.dict is a persistent, structurally-shared map (O(log n) insert); list.sort"
echo "    is a Pluma-level merge sort and the string ops are Pluma-level too. The others"
echo "    use native mutable hash maps and C-level sort/string routines."
echo
echo "Wrote markdown report to ${REPORT#"$ROOT"/}"
