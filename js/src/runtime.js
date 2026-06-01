// Pluma JS runtime preamble. Prepended to every emitted module. Defines the
// value model + the native primitive builtins (the leaves the pure-Pluma stdlib
// bottoms out in) + the program runner. Mirrors `vm::Value` semantics: Display
// (`__tostring`), structural equality (`__eq`), and the `print`/`io` host.
//
// Value representation (uniformly boxed, like the VM):
//   int      -> JS number (53-bit; full i64 deferred to a BigInt pass)
//   float    -> PFloat {v}              (tagged so `3` and `3.0` stay distinct)
//   bool     -> JS boolean
//   string   -> JS string               (native — free DOM/console interop)
//   bytes    -> Uint8Array
//   nothing  -> NOTHING sentinel
//   duration -> PDuration {ns}
//   list     -> JS array (mutable)
//   tuple    -> PTuple {e}              (tagged so it's distinct from a list)
//   record   -> plain object {field: v}
//   variant  -> PVariant {en, tag, name, p}
//   ref      -> PRef {v}                (compared by identity, like the VM)
//   closure  -> JS function taking the Pluma-level args (env closed over)

const NOTHING = Symbol("nothing");
class PFloat {
	constructor(v) {
		this.v = v;
	}
}
class PDuration {
	constructor(ns) {
		this.ns = ns;
	}
}
class PTuple {
	constructor(e) {
		this.e = e;
	}
}
class PVariant {
	constructor(en, tag, name, p) {
		this.en = en;
		this.tag = tag;
		this.name = name;
		this.p = p;
	}
}
class PRef {
	constructor(v) {
		this.v = v;
	}
}
class PlumaFail extends Error {} // program-controlled abort (`io.fail`)

const __enc = new TextEncoder();
const __dec = new TextDecoder("utf-8"); // lossy — matches the unchecked retag

// ---- output -------------------------------------------------------------
// Buffered; flushed by `__run`. `__out` is appended as a JS string; bytes are
// decoded lossily for the textual stream (raw-byte stdout is rare and only the
// `io-write-bytes` fixtures exercise it, which the JS allowlist defers).
let __stdout = "";
let __stderr = "";
function __writeOut(s) {
	__stdout += s;
}
function __writeErr(s) {
	__stderr += s;
}

// ---- formatting (mirrors vm::Value Display) -----------------------------
function __floatStr(n) {
	if (Number.isNaN(n)) return "NaN";
	if (n === Infinity) return "inf";
	if (n === -Infinity) return "-inf";
	if (Number.isInteger(n)) return n.toFixed(1); // 3 -> "3.0"
	return String(n);
}

function __bytesStr(b) {
	let s = "'";
	for (const byte of b) {
		if (byte === 0x5c) s += "\\\\";
		else if (byte === 0x27) s += "\\'";
		else if (byte >= 0x20 && byte <= 0x7e) s += String.fromCharCode(byte);
		else s += "\\x" + byte.toString(16).padStart(2, "0");
	}
	return s + "'";
}

const __DUR_UNITS = [
	[86400000000000n, "d"],
	[3600000000000n, "h"],
	[60000000000n, "m"],
	[1000000000n, "s"],
	[1000000n, "ms"],
	[1000n, "us"],
	[1n, "ns"],
];
function __durStr(ns) {
	let n = BigInt(ns);
	if (n === 0n) return "0s";
	let sign = "";
	if (n < 0n) {
		sign = "-";
		n = -n;
	}
	let out = sign;
	for (const [per, name] of __DUR_UNITS) {
		if (n >= per) {
			out += (n / per).toString() + name;
			n %= per;
		}
	}
	return out;
}

function __tostring(x) {
	switch (typeof x) {
		case "number":
			return String(x);
		case "string":
			return x;
		case "boolean":
			return x ? "true" : "false";
		case "function":
			return "<closure>";
	}
	if (x === NOTHING) return "()";
	if (x instanceof PFloat) return __floatStr(x.v);
	if (x instanceof PDuration) return __durStr(x.ns);
	if (x instanceof Uint8Array) return __bytesStr(x);
	if (Array.isArray(x)) return "[" + x.map(__tostring).join(", ") + "]";
	if (x instanceof PTuple) return "(" + x.e.map(__tostring).join(", ") + ")";
	if (x instanceof PVariant) {
		const dot = x.en.lastIndexOf(".");
		const bare = dot >= 0 ? x.en.slice(dot + 1) : x.en;
		let s = bare + "." + x.name;
		for (const a of x.p) s += " " + __tostring(a);
		return s;
	}
	if (x instanceof PRef) return "ref " + __tostring(x.v);
	if (x instanceof PDict) return __dictStr(x);
	// plain object = record; keys sorted (matches Display)
	const keys = Object.keys(x).sort();
	return "{" + keys.map((k) => k + ": " + __tostring(x[k])).join(", ") + "}";
}

// ---- structural equality (mirrors vm::values_eq) ------------------------
function __eq(a, b) {
	const ta = typeof a;
	if (ta === "number" || ta === "string" || ta === "boolean") return a === b;
	if (a === NOTHING) return b === NOTHING;
	if (a instanceof PFloat) return b instanceof PFloat && a.v === b.v; // nan != nan
	if (a instanceof PDuration) return b instanceof PDuration && a.ns === b.ns;
	if (a instanceof Uint8Array) {
		if (!(b instanceof Uint8Array) || a.length !== b.length) return false;
		for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
		return true;
	}
	if (Array.isArray(a)) {
		if (!Array.isArray(b) || a.length !== b.length) return false;
		for (let i = 0; i < a.length; i++) if (!__eq(a[i], b[i])) return false;
		return true;
	}
	if (a instanceof PTuple) {
		if (!(b instanceof PTuple) || a.e.length !== b.e.length) return false;
		for (let i = 0; i < a.e.length; i++)
			if (!__eq(a.e[i], b.e[i])) return false;
		return true;
	}
	if (a instanceof PVariant) {
		if (!(b instanceof PVariant) || a.en !== b.en || a.tag !== b.tag)
			return false;
		if (a.p.length !== b.p.length) return false;
		for (let i = 0; i < a.p.length; i++)
			if (!__eq(a.p[i], b.p[i])) return false;
		return true;
	}
	if (a instanceof PRef) return a === b; // identity, like the VM
	if (a instanceof PDict) return b instanceof PDict && __dictEq(a, b);
	// records (plain objects)
	if (a && b && typeof a === "object" && typeof b === "object") {
		const ka = Object.keys(a);
		if (ka.length !== Object.keys(b).length) return false;
		for (const k of ka) {
			if (!(k in b) || !__eq(a[k], b[k])) return false;
		}
		return true;
	}
	return false;
}

// ---- hashing (FNV-1a, mirrors vm::primitive_hash) -----------------------
function __fnv1a(bytes) {
	let h = 0xcbf29ce484222325n;
	const MASK = 0xffffffffffffffffn;
	for (const b of bytes) {
		h = ((h ^ BigInt(b)) * 0x100000001b3n) & MASK;
	}
	return BigInt.asIntN(64, h);
}
// hash builtins return an int; we surface FNV's i64 as a JS number (53-bit).
function __strHash(s) {
	return Number(BigInt.asIntN(53, __fnv1a(__enc.encode(s))));
}
function __bytesHash(b) {
	return Number(BigInt.asIntN(53, __fnv1a(b)));
}

// ---- dict (native) ------------------------------------------------------
// Mirrors vm::Value::Dict: insertion-ordered entries + hash buckets, so equal
// keys collapse and lookup matches dict semantics. Each entry stores its hash
// (`[k, v, h]`) so map/filter/remove rebuild buckets without re-hashing. The key
// hash is supplied by the dict builtins via the key type's `hash` trait dict, so
// enum / derived-hash keys work — not just primitives.
function __hashOf(hashDict, k) {
	return hashDict[0](k); // the `hash` trait's sole method
}
class PDict {
	constructor() {
		this.entries = []; // [k, v, h]
		this.buckets = new Map(); // h -> [index]
	}
	_clone() {
		const d = new PDict();
		d.entries = this.entries.slice();
		for (const [h, idxs] of this.buckets) d.buckets.set(h, idxs.slice());
		return d;
	}
	_find(h, k) {
		const idxs = this.buckets.get(h);
		if (!idxs) return -1;
		for (const i of idxs) if (__eq(this.entries[i][0], k)) return i;
		return -1;
	}
	_raw(k, v, h) {
		const idx = this.entries.length;
		this.entries.push([k, v, h]);
		if (!this.buckets.has(h)) this.buckets.set(h, []);
		this.buckets.get(h).push(idx);
	}
	insert(h, k, v) {
		const d = this._clone();
		const i = d._find(h, k);
		if (i >= 0) d.entries[i] = [k, v, h];
		else d._raw(k, v, h);
		return d;
	}
	lookup(h, k) {
		const i = this._find(h, k);
		return i >= 0 ? __opt_some(this.entries[i][1]) : __opt_none();
	}
	remove(h, k) {
		const i = this._find(h, k);
		if (i < 0) return this;
		const d = new PDict();
		for (const [ek, ev, eh] of this.entries) if (!__eq(ek, k)) d._raw(ek, ev, eh);
		return d;
	}
	mapValues(f) {
		const d = new PDict();
		for (const [k, v, h] of this.entries) d._raw(k, f(v), h);
		return d;
	}
	filterEntries(f) {
		const d = new PDict();
		for (const [k, v, h] of this.entries) if (f(k, v)) d._raw(k, v, h);
		return d;
	}
}
function __dictStr(d) {
	return (
		"{" +
		d.entries.map(([k, v]) => __tostring(k) + ": " + __tostring(v)).join(", ") +
		"}"
	);
}
function __dictEq(a, b) {
	if (a.entries.length !== b.entries.length) return false;
	for (const [k, v, h] of a.entries) {
		const i = b._find(h, k);
		if (i < 0 || !__eq(v, b.entries[i][1])) return false;
	}
	return true;
}

// ---- option/result constructors (for native builtins that return them) --
// Filled in by the emitter via `__bindEnums` once the program's enum table is
// known (the qualified names are program-specific).
let __OPTION = null,
	__RESULT = null,
	__ORDERING = null;
function __bindEnums(optionEnum, resultEnum, orderingEnum) {
	__OPTION = optionEnum;
	__RESULT = resultEnum;
	__ORDERING = orderingEnum;
}
// An `ordering` variant from a sign (-1/0/+1); `lt`/`eq`/`gt` (tags 0/1/2).
function __ord(c) {
	if (c < 0) return new PVariant(__ORDERING, 0, "lt", []);
	if (c > 0) return new PVariant(__ORDERING, 2, "gt", []);
	return new PVariant(__ORDERING, 1, "eq", []);
}
// Lexicographic byte comparison of two Uint8Arrays (matches Rust's str/[u8] cmp).
function __byteCmp(a, b) {
	const n = Math.min(a.length, b.length);
	for (let i = 0; i < n; i++) {
		if (a[i] !== b[i]) return a[i] < b[i] ? -1 : 1;
	}
	return a.length === b.length ? 0 : a.length < b.length ? -1 : 1;
}
function __opt_some(v) {
	return new PVariant(__OPTION, 0, "some", [v]);
}
function __opt_none() {
	return new PVariant(__OPTION, 1, "none", []);
}
function __res_ok(v) {
	return new PVariant(__RESULT, 0, "ok", [v]);
}
function __res_err(v) {
	return new PVariant(__RESULT, 1, "err", [v]);
}

// A record-pattern `...rest`: a copy of `rec` without the matched field names.
function __recordRest(rec, excluded) {
	const out = {};
	for (const k of Object.keys(rec)) if (!excluded.includes(k)) out[k] = rec[k];
	return out;
}

// ---- closures + globals -------------------------------------------------
function __mkclosure(fn, env) {
	return (...args) => fn(env, ...args);
}

// Globals are lazily forced on first access (matches the VM's Pending->Evaluated
// thunk slots), so inter-global references resolve regardless of init order. A
// `Thunk` marker distinguishes a slot to force from a pre-evaluated value that
// merely happens to be callable (a builtin / closure global).
class Thunk {
	constructor(f) {
		this.f = f;
	}
}
const __GINIT = [];
const __GCACHE = [];
function __gload(i) {
	if (i in __GCACHE) return __GCACHE[i];
	const init = __GINIT[i];
	const v = init instanceof Thunk ? init.f() : init;
	__GCACHE[i] = v;
	return v;
}

// ---- native builtins (the primitive leaves) -----------------------------
const RT = {
	// stdout / stderr / abort
	print: (x) => {
		__writeOut(__tostring(x) + "\n");
		return NOTHING;
	},
	"io-print": (x) => {
		__writeOut(__tostring(x) + "\n");
		return NOTHING;
	},
	"io-print-err": (x) => {
		__writeErr(__tostring(x) + "\n");
		return NOTHING;
	},
	"io-write": (x) => {
		__writeOut(__tostring(x));
		return NOTHING;
	},
	"io-write-err": (x) => {
		__writeErr(__tostring(x));
		return NOTHING;
	},
	"io-write-bytes": (b) => {
		__writeOut(__dec.decode(b));
		return NOTHING;
	},
	"io-write-err-bytes": (b) => {
		__writeErr(__dec.decode(b));
		return NOTHING;
	},
	"io-fail": (msg) => {
		throw new PlumaFail(msg);
	},
	debug: (x) => {
		__writeOut(__tostring(x) + "\n");
		return x;
	}, // call-site prefix TODO
	"to-string": (x) => __tostring(x),

	// numeric trait dict methods (concrete arith uses BinOp; these are for the
	// polymorphic `numeric` dispatch path)
	"int-add": (a, b) => a + b,
	"int-sub": (a, b) => a - b,
	"int-mul": (a, b) => a * b,
	"int-div": (a, b) => {
		if (b === 0) throw new PlumaFail("integer division by zero");
		return Math.trunc(a / b);
	},
	"int-negate": (a) => -a,
	"float-add": (a, b) => new PFloat(a.v + b.v),
	"float-sub": (a, b) => new PFloat(a.v - b.v),
	"float-mul": (a, b) => new PFloat(a.v * b.v),
	"float-div": (a, b) => new PFloat(a.v / b.v),
	"float-negate": (a) => new PFloat(-a.v),

	// comparison trait dict methods (`ord.compare` -> ordering)
	"int-compare": (a, b) => __ord(a < b ? -1 : a > b ? 1 : 0),
	"float-compare": (a, b) => __ord(a.v < b.v ? -1 : a.v > b.v ? 1 : 0),
	"string-compare": (a, b) => __ord(__byteCmp(__enc.encode(a), __enc.encode(b))),
	"bytes-compare": (a, b) => __ord(__byteCmp(a, b)),

	// hashing
	"int-hash": (a) => a,
	"float-hash": (a) =>
		Number(BigInt.asIntN(53, BigInt(new Float64Array([a.v]).length ? 0 : 0))) ||
		__strHash(String(a.v)),
	"bool-hash": (a) => (a ? 1 : 0),
	"string-hash": (a) => __strHash(a),
	"bytes-hash": (a) => __bytesHash(a),

	// lists
	"list-get": (xs, i) => xs[i],
	"list-length": (xs) => xs.length,
	"list-set": (xs, i, v) => {
		xs[i] = v;
		return xs;
	},
	"list-push": (xs, v) => {
		xs.push(v);
		return xs;
	},
	"list-build": (n, f) => {
		const a = new Array(n);
		for (let i = 0; i < n; i++) a[i] = f(i);
		return a;
	},
	"list-collect": (n, f) => {
		const a = [];
		for (let i = 0; i < n; i++) {
			const r = f(i);
			if (r instanceof PVariant && r.tag === 0) a.push(r.p[0]);
		}
		return a;
	},

	// bytes
	"bytes-get": (b, i) => b[i],
	"bytes-length": (b) => b.length,
	"bytes-set": (b, i, v) => {
		b[i] = v & 0xff;
		return b;
	},
	"bytes-build": (n, f) => {
		const a = new Uint8Array(n);
		for (let i = 0; i < n; i++) a[i] = f(i) & 0xff;
		return a;
	},
	"bytes-concat": (a, b) => {
		const out = new Uint8Array(a.length + b.length);
		out.set(a, 0);
		out.set(b, a.length);
		return out;
	},
	"bytes-as-string": (b) => __dec.decode(b),
	"string-to-bytes": (s) => __enc.encode(s),

	// refs
	"ref-new": (v) => new PRef(v),
	"ref-get": (r) => r.v,
	"ref-set": (r, v) => {
		r.v = v;
		return NOTHING;
	},
	"ref-update": (r, f) => {
		r.v = f(r.v);
		return NOTHING;
	},

	// dict (the leading arg of insert/lookup/remove is the key type's `hash`
	// trait dict, used to hash the key — so enum / derived-hash keys work)
	"dict-empty": (_u) => new PDict(),
	"dict-insert": (hd, m, k, v) => m.insert(__hashOf(hd, k), k, v),
	"dict-lookup": (hd, m, k) => m.lookup(__hashOf(hd, k), k),
	"dict-remove": (hd, m, k) => m.remove(__hashOf(hd, k), k),
	"dict-size": (m) => m.entries.length,
	"dict-entries": (m) => m.entries.map(([k, v]) => new PTuple([k, v])),
	"dict-map": (m, f) => m.mapValues(f),
	"dict-filter": (m, f) => m.filterEntries(f),

	// time (durations carry nanos as a BigInt)
	"time-duration-as-nanos": (d) => Number(d.ns),
	"time-duration-of-nanos": (n) => new PDuration(BigInt(n)),

	// math
	"math-sqrt": (a) => new PFloat(Math.sqrt(a.v)),
	"math-sin": (a) => new PFloat(Math.sin(a.v)),
	"math-cos": (a) => new PFloat(Math.cos(a.v)),
	"math-exp": (a) => new PFloat(Math.exp(a.v)),
	"math-log": (a) => new PFloat(Math.log(a.v)),
	"math-log10": (a) => new PFloat(Math.log10(a.v)),
	"math-log2": (a) => new PFloat(Math.log2(a.v)),
	"math-to-float": (a) => new PFloat(a),
	"math-to-int": (a) => Math.trunc(a.v),
};

// fix float-hash: reinterpret the f64 bit pattern as i64, then clamp to 53 bits.
RT["float-hash"] = (a) => {
	const buf = new ArrayBuffer(8);
	new Float64Array(buf)[0] = a.v;
	const bits = new BigInt64Array(buf)[0];
	return Number(BigInt.asIntN(53, bits));
};

// ---- DOM host (browser only; no-op/throws under node) -------------------
RT["dom-set-text"] = (id, text) => {
	if (typeof document === "undefined")
		throw new PlumaFail("dom.set-text: no DOM");
	const el = document.getElementById(id);
	if (el) el.textContent = text;
	return NOTHING;
};
RT["dom-set-html"] = (id, html) => {
	if (typeof document === "undefined")
		throw new PlumaFail("dom.set-html: no DOM");
	const el = document.getElementById(id);
	if (el) el.innerHTML = html;
	return NOTHING;
};
RT["dom-get-value"] = (id) => {
	if (typeof document === "undefined")
		throw new PlumaFail("dom.get-value: no DOM");
	const el = document.getElementById(id);
	return el && "value" in el ? el.value : "";
};

// ---- program runner -----------------------------------------------------
// Runs the entry thunk, applies the "main returns `err`" convention, flushes
// output. Returns {status, stdout, stderr}. Under node it also writes the
// streams and sets the exit code; in the browser the caller consumes the result.
function __run(entryThunk) {
	let status = "ok";
	try {
		const result = entryThunk();
		if (
			result instanceof PVariant &&
			result.en === __RESULT &&
			result.name === "err"
		) {
			status = "runtime error: " + __tostring(result.p[0]);
		}
	} catch (e) {
		if (e instanceof PlumaFail) status = "runtime error: " + e.message;
		else throw e;
	}
	const out = { status, stdout: __stdout, stderr: __stderr };
	if (typeof process !== "undefined" && process.stdout) {
		if (__stdout) process.stdout.write(__stdout);
		if (__stderr) process.stderr.write(__stderr);
		if (status !== "ok") {
			process.stderr.write(status + "\n");
			process.exitCode = 1;
		}
	}
	return out;
}
