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
# from other activity on the machine). Times include process startup and — for
# Pluma — compiling the source, because that is what "running the program" costs.
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

# Run "$@" RUNS times; print the minimum real (wall-clock) time in seconds.
# The command's stdout is captured into the file named by the first argument.
# Prints "n/a" if the command is missing or exits non-zero.
min_time() {
	local out="$1"
	shift
	if ! have "$1"; then
		echo "n/a"
		return
	fi
	local err best="" t
	err="$(mktemp)"
	local i
	for ((i = 0; i < RUNS; i++)); do
		# command stdout -> $out ; both command stderr and time's report -> $err
		if ! { /usr/bin/time -p "$@" 1>"$out" 2>"$err"; }; then
			rm -f "$err"
			echo "ERR"
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

echo "Pluma vs Python vs Ruby vs Node.js  —  best of $RUNS runs, seconds (lower is better)"
echo
printf '%-14s %10s %10s %10s %10s %12s   %s\n' \
	"benchmark" "pluma" "python3" "ruby" "node" "pluma vs best" "output"
printf '%s\n' "-------------------------------------------------------------------------------------------------"

po="$(mktemp)"
pyo="$(mktemp)"
rbo="$(mktemp)"
jso="$(mktemp)"

md_rows=""        # accumulated markdown table rows for RESULTS.md
mismatch="no"

for entry in "${BENCHES[@]}"; do
	dir="${entry%%:*}"
	rest="${entry#*:}"
	name="${rest%%:*}"
	desc="${rest#*:}"
	d="$ROOT/competition/$dir"

	pt="$(min_time "$po" "$PLUMA" "$d/$name")"
	pyt="$(min_time "$pyo" python3 "$d/$name.py")"
	rbt="$(min_time "$rbo" ruby "$d/$name.rb")"
	jt="$(min_time "$jso" node "$d/$name.js")"

	# Verify every backend produced the same output as Pluma.
	status="ok"
	for f in "$pyo" "$rbo" "$jso"; do
		if [ -s "$f" ] && ! diff -q "$po" "$f" >/dev/null 2>&1; then
			status="MISMATCH"
			mismatch="yes"
		fi
	done

	# Fastest of the three competitors, and how Pluma compares to it.
	best_other="$(printf '%s\n' "$pyt" "$rbt" "$jt" |
		awk '/^[0-9.]+$/ { if (m == "" || $1 < m) m = $1 } END { print (m == "" ? "n/a" : m) }')"
	vs="$(ratio "$pt" "$best_other")"

	printf '%-14s %10s %10s %10s %10s %12s   %s\n' "$name" "$pt" "$pyt" "$rbt" "$jt" "$vs" "$status"
	md_rows+="| \`$name\` | $desc | $pt | $pyt | $rbt | $jt | $vs | $status |"$'\n'
done

rm -f "$po" "$pyo" "$rbo" "$jso"

# ---- Write the markdown report -------------------------------------------------
os="$(uname -sm)"
pluma_ver="$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"
py_ver="$(python3 --version 2>&1 | head -1)"
rb_ver="$(ruby --version 2>&1 | awk '{ print $1, $2 }')"
node_ver="$(node --version 2>&1 | head -1)"
overall="all four implementations agreed on every output"
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
	echo "| benchmark | exercises | pluma | python3 | ruby | node | pluma vs best | output |"
	echo "|---|---|--:|--:|--:|--:|--:|:--:|"
	printf '%s' "$md_rows"
	echo
	echo "## How to read this"
	echo
	echo "- Times include process startup; for Pluma they also include front-end"
	echo "  compilation — the real cost of running the program."
	echo "- \`pluma vs best\` is Pluma's time divided by the fastest competitor's time"
	echo "  (greater than 1× means Pluma is slower; less than 1× means Pluma is faster)."
	echo "- \`output\` = \`ok\` means all four printed byte-identical results; \`MISMATCH\`"
	echo "  means they disagreed and the row should not be trusted."
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
echo "  - 'pluma vs best' is Pluma's time / the fastest competitor's time."
echo "    >1x means Pluma is slower; <1x means Pluma is faster."
echo "  - 'output' = MISMATCH means the four programs disagreed on their result."
echo "  - Pluma times include front-end compilation; the others include interpreter"
echo "    startup. That is the real cost of 'run this program', which is the point."
echo "  - core.dict is a persistent, structurally-shared map (O(log n) insert); list.sort"
echo "    is a Pluma-level merge sort and the string ops are Pluma-level too. The others"
echo "    use native mutable hash maps and C-level sort/string routines."
echo
echo "Wrote markdown report to ${REPORT#"$ROOT"/}"
