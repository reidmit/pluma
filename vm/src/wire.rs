// The `wire` binary codec.
//
// A `wire a` dictionary is reified at runtime as a *schema descriptor* — a
// compact tree mirroring `a`'s static structure. The `wire-encode` /
// `wire-decode` builtins interpret that schema to turn values into a tight
// positional binary encoding and back. The schema is what makes decode
// possible: a byte buffer doesn't know whether it's an `option int` or a
// record, so the target type has to drive the parse.
//
// Format:
//   int       varint, zigzag (LEB128 over the zigzag transform)
//   float     8 raw bytes, IEEE-754 little-endian
//   bool      1 byte (0/1)
//   string    varint byte-length prefix + UTF-8 bytes
//   bytes     varint length prefix + raw bytes
//   list      varint element count + each element
//   tuple     fields in order (arity is in the schema, no prefix)
//   record    fields in schema order (no field names on the wire)
//   enum      varint tag (variant index) + payload fields in order
//   duration  its constant int repr (zigzag varint of the i64 nanos)
//   nothing   zero bytes
//
// The encoding is positional and deterministic: identical values produce
// identical bytes, so a payload is content-addressable. It leans hard on the
// end-to-end type guarantee — which is the whole point.

use crate::value::{DictData, Value, VariantData, primitive_hash};
use std::collections::HashMap;
use std::rc::Rc;

/// The runtime reification of a `wire a` dictionary. Built by codegen from a
/// type's static structure (see the compiler's `WireShape` / `Resolved::WireSchema`)
/// and threaded as the trait dictionary; parsed back here via `schema_from_value`.
#[derive(Debug, Clone, PartialEq)]
pub enum Schema {
	Int,
	Float,
	Bool,
	String,
	Bytes,
	Duration,
	Nothing,
	List(Box<Schema>),
	Tuple(Vec<Schema>),
	// `dict k v` as a length-prefixed sequence of (key, value) pairs. The key
	// type is always primitive (int/float/bool/string/bytes) — wire-derivation
	// rejects compound keys — so `decode` can rehash keys via
	// `value::primitive_hash` to rebuild buckets identically to `dict.lookup`.
	Dict(Box<Schema>, Box<Schema>),
	// Field name (for HashMap lookup on encode / construction on decode) +
	// field schema, in a canonical order shared by both directions.
	Record(Vec<(String, Schema)>),
	Enum {
		// Fully-qualified enum name, e.g. `__prelude__.option`, needed to
		// reconstruct a `Value::Variant` on decode.
		qualified: String,
		// One entry per variant in declaration order; the index is the wire
		// tag. Each carries the variant name + its payload field schemas.
		variants: Vec<(String, Vec<Schema>)>,
	},
	// A back-reference to a recursive enum being expanded by an enclosing
	// `Enum` node — the cycle-cut that keeps a recursive type's schema finite.
	// Resolved against the enum context the codec threads as it descends.
	EnumRef(String),
}

/// A failure decoding (or, rarely, encoding) a wire payload. Mirrors the
/// `wire-error` enum in `core.wire`; `to_value` builds the matching variant.
#[derive(Debug, Clone, PartialEq)]
pub enum WireError {
	/// Ran out of bytes mid-decode.
	UnexpectedEnd,
	/// An enum tag had no corresponding variant in the schema.
	InvalidTag(i64),
	/// A string field was not valid UTF-8.
	InvalidUtf8,
	/// Bytes remained after a complete top-level decode.
	TrailingBytes(i64),
	/// A varint was malformed (overlong / unterminated).
	Malformed,
}

const WIRE_ERROR_ENUM: &str = "__prelude__.wire-error";

impl WireError {
	pub fn to_value(&self) -> Value {
		let (variant, payload): (&str, Vec<Value>) = match self {
			WireError::UnexpectedEnd => ("unexpected-end", vec![]),
			WireError::InvalidTag(t) => ("invalid-tag", vec![Value::Int(*t)]),
			WireError::InvalidUtf8 => ("invalid-utf8", vec![]),
			WireError::TrailingBytes(n) => ("trailing-bytes", vec![Value::Int(*n)]),
			WireError::Malformed => ("malformed", vec![]),
		};
		Value::Variant(Rc::new(VariantData {
			qualified_enum: Rc::new(WIRE_ERROR_ENUM.to_string()),
			variant: Rc::new(variant.to_string()),
			payload,
		}))
	}
}

// ---- varint / zigzag ------------------------------------------------------

fn zigzag(n: i64) -> u64 {
	((n << 1) ^ (n >> 63)) as u64
}

fn unzigzag(u: u64) -> i64 {
	((u >> 1) as i64) ^ -((u & 1) as i64)
}

fn write_uvarint(out: &mut Vec<u8>, mut v: u64) {
	loop {
		let byte = (v & 0x7f) as u8;
		v >>= 7;
		if v == 0 {
			out.push(byte);
			break;
		}
		out.push(byte | 0x80);
	}
}

/// Read a LEB128 unsigned varint, advancing the cursor. Caps at 10 bytes (the
/// max for a 64-bit value); anything longer is `Malformed`.
fn read_uvarint(cur: &mut &[u8]) -> Result<u64, WireError> {
	let mut result: u64 = 0;
	let mut shift = 0u32;
	for i in 0..10 {
		let byte = *cur.first().ok_or(WireError::UnexpectedEnd)?;
		*cur = &cur[1..];
		// On the 10th byte only the lowest bit is meaningful for a 64-bit int.
		if i == 9 && byte > 0x01 {
			return Err(WireError::Malformed);
		}
		result |= ((byte & 0x7f) as u64) << shift;
		if byte & 0x80 == 0 {
			return Ok(result);
		}
		shift += 7;
	}
	Err(WireError::Malformed)
}

fn read_len(cur: &mut &[u8]) -> Result<usize, WireError> {
	usize::try_from(read_uvarint(cur)?).map_err(|_| WireError::Malformed)
}

// ---- encode ---------------------------------------------------------------

// As the codec walks the schema it records each inline `Enum`'s variants by
// qualified name, so a recursive back-reference (`EnumRef`) further down can
// resolve to its enclosing definition without the schema being infinite. The
// *value*'s finite depth drives termination. Borrows the variant lists out of
// the `Schema` tree (lifetime `'a`), so no cloning.
type EnumCtx<'a> = HashMap<&'a str, &'a [(String, Vec<Schema>)]>;

/// Encode `value` per `schema`, appending to `out`. The value is assumed to
/// match the schema (the type checker guarantees it); a mismatch is a compiler
/// bug, caught by `debug_assert`/`unreachable` in debug builds.
pub fn encode(schema: &Schema, value: &Value, out: &mut Vec<u8>) {
	encode_in(schema, value, out, &mut EnumCtx::new());
}

fn encode_in<'a>(schema: &'a Schema, value: &Value, out: &mut Vec<u8>, ctx: &mut EnumCtx<'a>) {
	match (schema, value) {
		(Schema::Int, Value::Int(n)) => write_uvarint(out, zigzag(*n)),
		(Schema::Duration, Value::Duration(n)) => write_uvarint(out, zigzag(*n)),
		(Schema::Float, Value::Float(f)) => out.extend_from_slice(&f.to_le_bytes()),
		(Schema::Bool, Value::Bool(b)) => out.push(*b as u8),
		(Schema::String, Value::String(s)) => {
			let bytes = s.as_bytes();
			write_uvarint(out, bytes.len() as u64);
			out.extend_from_slice(bytes);
		}
		(Schema::Bytes, Value::Bytes(b)) => {
			write_uvarint(out, b.len() as u64);
			out.extend_from_slice(b);
		}
		(Schema::Nothing, Value::Nothing) => {}
		(Schema::List(inner), Value::List(xs)) => {
			let xs = xs.borrow();
			write_uvarint(out, xs.len() as u64);
			for x in xs.iter() {
				encode_in(inner, x, out, ctx);
			}
		}
		(Schema::Tuple(schemas), Value::Tuple(xs)) => {
			debug_assert_eq!(schemas.len(), xs.len(), "tuple arity vs schema");
			for (s, x) in schemas.iter().zip(xs.iter()) {
				encode_in(s, x, out, ctx);
			}
		}
		(Schema::Dict(ksch, vsch), Value::Dict(data)) => {
			// Encode entries in a canonical order (sorted by encoded-key bytes)
			// so logically-equal dicts produce identical bytes regardless of
			// insertion order — insertion order isn't part of a dict's identity.
			// (Keys are primitive, so they carry no enum context.)
			let mut items: Vec<(Vec<u8>, &Value)> = data
				.entries
				.iter()
				.map(|(k, v)| {
					let mut kb = Vec::new();
					encode_in(ksch, k, &mut kb, ctx);
					(kb, v)
				})
				.collect();
			items.sort_by(|a, b| a.0.cmp(&b.0));
			write_uvarint(out, items.len() as u64);
			for (kb, v) in items {
				out.extend_from_slice(&kb);
				encode_in(vsch, v, out, ctx);
			}
		}
		(Schema::Record(fields), Value::Record(map)) => {
			for (name, s) in fields {
				let v = map
					.get(name)
					.unwrap_or_else(|| unreachable!("record missing field `{}`", name));
				encode_in(s, v, out, ctx);
			}
		}
		(
			Schema::Enum {
				qualified,
				variants,
			},
			Value::Variant(v),
		) => {
			ctx.insert(qualified.as_str(), variants.as_slice());
			encode_variant(variants, v, out, ctx);
		}
		(Schema::EnumRef(qualified), Value::Variant(v)) => {
			let variants = *ctx
				.get(qualified.as_str())
				.unwrap_or_else(|| unreachable!("unregistered enum ref `{}`", qualified));
			encode_variant(variants, v, out, ctx);
		}
		(s, v) => unreachable!("wire encode: schema {:?} vs value {}", s, v),
	}
}

fn encode_variant<'a>(
	variants: &'a [(String, Vec<Schema>)],
	v: &VariantData,
	out: &mut Vec<u8>,
	ctx: &mut EnumCtx<'a>,
) {
	let tag = variants
		.iter()
		.position(|(name, _)| name.as_str() == v.variant.as_str())
		.unwrap_or_else(|| unreachable!("variant `{}` not in schema", v.variant));
	write_uvarint(out, tag as u64);
	let (_, field_schemas) = &variants[tag];
	debug_assert_eq!(
		field_schemas.len(),
		v.payload.len(),
		"variant payload arity"
	);
	for (s, x) in field_schemas.iter().zip(v.payload.iter()) {
		encode_in(s, x, out, ctx);
	}
}

// ---- decode ---------------------------------------------------------------

fn decode_in<'a>(
	schema: &'a Schema,
	cur: &mut &[u8],
	ctx: &mut EnumCtx<'a>,
) -> Result<Value, WireError> {
	match schema {
		Schema::Int => Ok(Value::Int(unzigzag(read_uvarint(cur)?))),
		Schema::Duration => Ok(Value::Duration(unzigzag(read_uvarint(cur)?))),
		Schema::Float => {
			let bytes = take(cur, 8)?;
			let arr: [u8; 8] = bytes.try_into().unwrap();
			Ok(Value::Float(f64::from_le_bytes(arr)))
		}
		Schema::Bool => match take(cur, 1)?[0] {
			0 => Ok(Value::Bool(false)),
			_ => Ok(Value::Bool(true)),
		},
		Schema::String => {
			let len = read_len(cur)?;
			let bytes = take(cur, len)?;
			let s = std::str::from_utf8(bytes).map_err(|_| WireError::InvalidUtf8)?;
			Ok(Value::String(Rc::new(s.to_string())))
		}
		Schema::Bytes => {
			let len = read_len(cur)?;
			Ok(Value::Bytes(Rc::new(take(cur, len)?.to_vec())))
		}
		Schema::Nothing => Ok(Value::Nothing),
		Schema::List(inner) => {
			let n = read_len(cur)?;
			let mut xs = Vec::with_capacity(n.min(1024));
			for _ in 0..n {
				xs.push(decode_in(inner, cur, ctx)?);
			}
			Ok(Value::list(xs))
		}
		Schema::Tuple(schemas) => {
			let mut xs = Vec::with_capacity(schemas.len());
			for s in schemas {
				xs.push(decode_in(s, cur, ctx)?);
			}
			Ok(Value::Tuple(Rc::new(xs)))
		}
		Schema::Dict(ksch, vsch) => {
			let n = read_len(cur)?;
			let mut data = DictData::new();
			for _ in 0..n {
				let key = decode_in(ksch, cur, ctx)?;
				let value = decode_in(vsch, cur, ctx)?;
				// Keys are primitive by construction (wire-derivation rejects
				// compound dict keys), so this matches `dict.lookup`'s hashing.
				let h = primitive_hash(&key)
					.unwrap_or_else(|| unreachable!("wire dict key not primitive-hashable"));
				data = data.inserted(h, key, value);
			}
			Ok(Value::Dict(Rc::new(data)))
		}
		Schema::Record(fields) => {
			let mut map = HashMap::with_capacity(fields.len());
			for (name, s) in fields {
				map.insert(name.clone(), decode_in(s, cur, ctx)?);
			}
			Ok(Value::Record(Rc::new(map)))
		}
		Schema::Enum {
			qualified,
			variants,
		} => {
			ctx.insert(qualified.as_str(), variants.as_slice());
			decode_variant(qualified, variants, cur, ctx)
		}
		Schema::EnumRef(qualified) => {
			let variants = *ctx.get(qualified.as_str()).ok_or(WireError::Malformed)?;
			decode_variant(qualified, variants, cur, ctx)
		}
	}
}

fn decode_variant<'a>(
	qualified: &str,
	variants: &'a [(String, Vec<Schema>)],
	cur: &mut &[u8],
	ctx: &mut EnumCtx<'a>,
) -> Result<Value, WireError> {
	let tag = read_uvarint(cur)?;
	let idx = usize::try_from(tag).ok().filter(|i| *i < variants.len());
	let idx = idx.ok_or(WireError::InvalidTag(tag as i64))?;
	let (name, field_schemas) = &variants[idx];
	let mut payload = Vec::with_capacity(field_schemas.len());
	for s in field_schemas {
		payload.push(decode_in(s, cur, ctx)?);
	}
	Ok(Value::Variant(Rc::new(VariantData {
		qualified_enum: Rc::new(qualified.to_string()),
		variant: Rc::new(name.clone()),
		payload,
	})))
}

fn take<'a>(cur: &mut &'a [u8], n: usize) -> Result<&'a [u8], WireError> {
	if cur.len() < n {
		return Err(WireError::UnexpectedEnd);
	}
	let (head, rest) = cur.split_at(n);
	*cur = rest;
	Ok(head)
}

/// Top-level decode entry: decode a full value and require the buffer to be
/// fully consumed (no trailing bytes).
pub fn decode_all(schema: &Schema, bytes: &[u8]) -> Result<Value, WireError> {
	let mut cur = bytes;
	let value = decode_in(schema, &mut cur, &mut EnumCtx::new())?;
	if !cur.is_empty() {
		return Err(WireError::TrailingBytes(cur.len() as i64));
	}
	Ok(value)
}

// ---- schema fingerprint ---------------------------------------------------
//
// A stable structural hash of a schema. Two
// types with the same wire shape hash identically; ANY drift that would change
// the bytes — a renamed/reordered/retyped record field, an added/renamed/
// re-arity'd enum variant, a different enum identity, different nesting —
// changes the hash. It rides in a per-request header so a stale client decoding
// a new server's payload fails cleanly instead of decoding garbage.
//
// FNV-1a over a canonical token stream. Lengths/counts are mixed in
// length-prefixed so neighbouring strings can't run together (field set
// `["ab","c"]` must not collide with `["a","bc"]`). Deterministic across runs
// and platforms — no reliance on Rust's randomized HashMap order (records and
// variants arrive already in canonical order from `build_wire_shape`).

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn mix_byte(h: u64, b: u8) -> u64 {
	(h ^ b as u64).wrapping_mul(FNV_PRIME)
}

fn mix_len(h: u64, n: usize) -> u64 {
	(n as u64)
		.to_le_bytes()
		.iter()
		.fold(h, |h, &b| mix_byte(h, b))
}

fn mix_str(h: u64, s: &str) -> u64 {
	let h = mix_len(h, s.len());
	s.bytes().fold(h, mix_byte)
}

fn mix_schema(h: u64, schema: &Schema) -> u64 {
	// Each arm leads with a distinct kind tag so structurally-different schemas
	// can't alias (e.g. an empty tuple vs `nothing`).
	match schema {
		Schema::Int => mix_byte(h, 1),
		Schema::Float => mix_byte(h, 2),
		Schema::Bool => mix_byte(h, 3),
		Schema::String => mix_byte(h, 4),
		Schema::Bytes => mix_byte(h, 5),
		Schema::Duration => mix_byte(h, 6),
		Schema::Nothing => mix_byte(h, 7),
		Schema::List(inner) => mix_schema(mix_byte(h, 8), inner),
		Schema::Tuple(elems) => {
			let h = mix_len(mix_byte(h, 9), elems.len());
			elems.iter().fold(h, mix_schema)
		}
		Schema::Dict(k, v) => mix_schema(mix_schema(mix_byte(h, 12), k), v),
		Schema::EnumRef(qualified) => mix_str(mix_byte(h, 13), qualified),
		Schema::Record(fields) => {
			let h = mix_len(mix_byte(h, 10), fields.len());
			fields
				.iter()
				.fold(h, |h, (name, s)| mix_schema(mix_str(h, name), s))
		}
		Schema::Enum {
			qualified,
			variants,
		} => {
			let h = mix_len(mix_str(mix_byte(h, 11), qualified), variants.len());
			variants.iter().fold(h, |h, (name, fields)| {
				let h = mix_len(mix_str(h, name), fields.len());
				fields.iter().fold(h, mix_schema)
			})
		}
	}
}

/// The schema's structural fingerprint, as an `i64` (the full 64-bit FNV-1a
/// hash reinterpreted into Pluma's `int`). Stable across runs/platforms.
pub fn fingerprint(schema: &Schema) -> i64 {
	mix_schema(FNV_OFFSET, schema) as i64
}

// ---- schema reification ---------------------------------------------------
//
// At runtime the schema arrives as an ordinary `wire-schema` value tree (built
// by codegen out of variant/list/tuple/string constructors). We parse it once
// into the Rust `Schema` above before encode/decode. The variant names below
// mirror the `wire-schema` enum declared in `core.wire`.

fn as_list(v: &Value) -> Option<Vec<Value>> {
	match v {
		Value::List(xs) => Some(xs.borrow().clone()),
		_ => None,
	}
}

fn as_string(v: &Value) -> Option<&str> {
	match v {
		Value::String(s) => Some(s.as_str()),
		_ => None,
	}
}

/// Parse a `wire-schema` value (as produced by codegen) into a Rust `Schema`.
/// Returns `None` only on a malformed schema, which is a compiler bug.
pub fn schema_from_value(v: &Value) -> Option<Schema> {
	let Value::Variant(d) = v else { return None };
	let payload = &d.payload;
	match d.variant.as_str() {
		"s-int" => Some(Schema::Int),
		"s-float" => Some(Schema::Float),
		"s-bool" => Some(Schema::Bool),
		"s-string" => Some(Schema::String),
		"s-bytes" => Some(Schema::Bytes),
		"s-duration" => Some(Schema::Duration),
		"s-nothing" => Some(Schema::Nothing),
		"s-list" => Some(Schema::List(Box::new(schema_from_value(payload.first()?)?))),
		"s-dict" => Some(Schema::Dict(
			Box::new(schema_from_value(payload.first()?)?),
			Box::new(schema_from_value(payload.get(1)?)?),
		)),
		"s-enum-ref" => Some(Schema::EnumRef(as_string(payload.first()?)?.to_string())),
		"s-tuple" => {
			let items = as_list(payload.first()?)?;
			let schemas = items
				.iter()
				.map(schema_from_value)
				.collect::<Option<Vec<_>>>()?;
			Some(Schema::Tuple(schemas))
		}
		"s-record" => {
			let items = as_list(payload.first()?)?;
			let fields = items
				.iter()
				.map(|item| {
					let Value::Tuple(pair) = item else {
						return None;
					};
					let name = as_string(pair.first()?)?.to_string();
					let s = schema_from_value(pair.get(1)?)?;
					Some((name, s))
				})
				.collect::<Option<Vec<_>>>()?;
			Some(Schema::Record(fields))
		}
		"s-enum" => {
			let qualified = as_string(payload.first()?)?.to_string();
			let items = as_list(payload.get(1)?)?;
			let variants = items
				.iter()
				.map(|item| {
					let Value::Tuple(pair) = item else {
						return None;
					};
					let name = as_string(pair.first()?)?.to_string();
					let field_vals = as_list(pair.get(1)?)?;
					let fields = field_vals
						.iter()
						.map(schema_from_value)
						.collect::<Option<Vec<_>>>()?;
					Some((name, fields))
				})
				.collect::<Option<Vec<_>>>()?;
			Some(Schema::Enum {
				qualified,
				variants,
			})
		}
		_ => None,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn int(n: i64) -> Value {
		Value::Int(n)
	}
	fn string(s: &str) -> Value {
		Value::String(Rc::new(s.to_string()))
	}
	fn variant(qualified: &str, name: &str, payload: Vec<Value>) -> Value {
		Value::Variant(Rc::new(VariantData {
			qualified_enum: Rc::new(qualified.to_string()),
			variant: Rc::new(name.to_string()),
			payload,
		}))
	}

	/// Encode then decode-all, asserting the value survives the round trip.
	fn round_trip(schema: &Schema, value: &Value) {
		let mut bytes = Vec::new();
		encode(schema, value, &mut bytes);
		let back = decode_all(schema, &bytes).expect("decode");
		assert!(
			crate::value::values_eq(&back, value),
			"round trip: got {} want {} (bytes {:?})",
			back,
			value,
			bytes
		);
	}

	#[test]
	fn zigzag_round_trips() {
		for n in [0i64, -1, 1, -2, 2, 63, -64, i64::MIN, i64::MAX, 1234567] {
			assert_eq!(unzigzag(zigzag(n)), n);
		}
	}

	#[test]
	fn uvarint_round_trips() {
		for v in [0u64, 1, 127, 128, 300, 16384, u64::MAX, 1 << 63] {
			let mut out = Vec::new();
			write_uvarint(&mut out, v);
			let mut cur = &out[..];
			assert_eq!(read_uvarint(&mut cur).unwrap(), v);
			assert!(cur.is_empty(), "varint consumed exactly");
		}
	}

	#[test]
	fn small_int_is_one_byte() {
		let mut out = Vec::new();
		encode(&Schema::Int, &int(0), &mut out);
		assert_eq!(out, vec![0]);
		out.clear();
		encode(&Schema::Int, &int(-1), &mut out);
		assert_eq!(out, vec![1]); // zigzag(-1) == 1
	}

	#[test]
	fn primitives_round_trip() {
		round_trip(&Schema::Int, &int(0));
		round_trip(&Schema::Int, &int(-1234567));
		round_trip(&Schema::Int, &Value::Int(i64::MIN));
		round_trip(&Schema::Float, &Value::Float(3.14159));
		round_trip(&Schema::Float, &Value::Float(-0.0));
		round_trip(&Schema::Bool, &Value::Bool(true));
		round_trip(&Schema::Bool, &Value::Bool(false));
		round_trip(&Schema::String, &string("hello, 世界 🌍"));
		round_trip(&Schema::String, &string(""));
		round_trip(&Schema::Bytes, &Value::Bytes(Rc::new(vec![0, 255, 1, 128])));
		round_trip(&Schema::Duration, &Value::Duration(86_400_000_000_000));
		round_trip(&Schema::Nothing, &Value::Nothing);
	}

	#[test]
	fn list_round_trips() {
		let s = Schema::List(Box::new(Schema::Int));
		round_trip(&s, &Value::list(vec![int(1), int(2), int(-3)]));
		round_trip(&s, &Value::list(vec![]));
	}

	#[test]
	fn tuple_round_trips() {
		let s = Schema::Tuple(vec![Schema::Int, Schema::String, Schema::Bool]);
		round_trip(
			&s,
			&Value::Tuple(Rc::new(vec![int(42), string("x"), Value::Bool(true)])),
		);
	}

	#[test]
	fn dict_round_trips() {
		let s = Schema::Dict(Box::new(Schema::String), Box::new(Schema::Int));
		let mk = |pairs: &[(&str, i64)]| {
			let mut d = DictData::new();
			for (k, v) in pairs {
				let key = string(k);
				let h = primitive_hash(&key).unwrap();
				d = d.inserted(h, key, Value::Int(*v));
			}
			Value::Dict(Rc::new(d))
		};
		round_trip(&s, &mk(&[("a", 1), ("b", 2), ("c", 3)]));
		round_trip(&s, &mk(&[]));

		// Deterministic: insertion order doesn't change the bytes.
		let mut b1 = Vec::new();
		encode(&s, &mk(&[("x", 1), ("y", 2)]), &mut b1);
		let mut b2 = Vec::new();
		encode(&s, &mk(&[("y", 2), ("x", 1)]), &mut b2);
		assert_eq!(b1, b2, "dict encoding is insertion-order-independent");
	}

	#[test]
	fn record_round_trips() {
		let s = Schema::Record(vec![
			("age".to_string(), Schema::Int),
			("name".to_string(), Schema::String),
		]);
		let mut map = HashMap::new();
		map.insert("name".to_string(), string("ada"));
		map.insert("age".to_string(), int(36));
		round_trip(&s, &Value::Record(Rc::new(map)));
	}

	#[test]
	fn enum_round_trips() {
		// option int: some=tag 0, none=tag 1.
		let s = Schema::Enum {
			qualified: "__prelude__.option".to_string(),
			variants: vec![
				("some".to_string(), vec![Schema::Int]),
				("none".to_string(), vec![]),
			],
		};
		round_trip(&s, &variant("__prelude__.option", "some", vec![int(7)]));
		round_trip(&s, &variant("__prelude__.option", "none", vec![]));

		// Tag is positional: `none` (tag 1) is a single byte `1`.
		let mut out = Vec::new();
		encode(&s, &variant("__prelude__.option", "none", vec![]), &mut out);
		assert_eq!(out, vec![1]);
	}

	#[test]
	fn nested_round_trips() {
		// list (option (tuple int string))
		let s = Schema::List(Box::new(Schema::Enum {
			qualified: "__prelude__.option".to_string(),
			variants: vec![
				(
					"some".to_string(),
					vec![Schema::Tuple(vec![Schema::Int, Schema::String])],
				),
				("none".to_string(), vec![]),
			],
		}));
		let some = |n, txt| {
			variant(
				"__prelude__.option",
				"some",
				vec![Value::Tuple(Rc::new(vec![int(n), string(txt)]))],
			)
		};
		let none = variant("__prelude__.option", "none", vec![]);
		round_trip(&s, &Value::list(vec![some(1, "a"), none, some(2, "b")]));
	}

	/// `decode_all` must fail with exactly `expected`. (`Value` has no
	/// `PartialEq`, so we can't `assert_eq!` the whole `Result`.)
	fn assert_decode_err(schema: &Schema, bytes: &[u8], expected: WireError) {
		match decode_all(schema, bytes) {
			Err(e) => assert_eq!(e, expected),
			Ok(_) => panic!("expected decode error {:?}, got Ok", expected),
		}
	}

	#[test]
	fn recursive_enum_round_trips() {
		// enum tree { leaf int  node tree tree } — cycle cut with EnumRef.
		let tree = Schema::Enum {
			qualified: "m.tree".to_string(),
			variants: vec![
				("leaf".to_string(), vec![Schema::Int]),
				(
					"node".to_string(),
					vec![
						Schema::EnumRef("m.tree".to_string()),
						Schema::EnumRef("m.tree".to_string()),
					],
				),
			],
		};
		let leaf = |n| variant("m.tree", "leaf", vec![int(n)]);
		let node = |l, r| variant("m.tree", "node", vec![l, r]);
		// node(leaf 1, node(leaf 2, leaf 3))
		round_trip(&tree, &node(leaf(1), node(leaf(2), leaf(3))));
		round_trip(&tree, &leaf(42));
	}

	#[test]
	fn decode_rejects_trailing_bytes() {
		let mut bytes = Vec::new();
		encode(&Schema::Int, &int(5), &mut bytes);
		bytes.push(0xff);
		assert_decode_err(&Schema::Int, &bytes, WireError::TrailingBytes(1));
	}

	#[test]
	fn decode_rejects_truncation() {
		assert_decode_err(&Schema::Int, &[], WireError::UnexpectedEnd);
		// length prefix says 4 bytes, none follow
		assert_decode_err(&Schema::String, &[4], WireError::UnexpectedEnd);
	}

	#[test]
	fn decode_rejects_bad_enum_tag() {
		let s = Schema::Enum {
			qualified: "__prelude__.option".to_string(),
			variants: vec![
				("some".to_string(), vec![Schema::Int]),
				("none".to_string(), vec![]),
			],
		};
		// tag 5 has no variant
		assert_decode_err(&s, &[5], WireError::InvalidTag(5));
	}

	#[test]
	fn decode_rejects_invalid_utf8() {
		// length 2, then an invalid UTF-8 sequence
		assert_decode_err(&Schema::String, &[2, 0xff, 0xfe], WireError::InvalidUtf8);
	}

	#[test]
	fn fingerprint_is_stable_and_structural() {
		let rec = |fields: &[(&str, Schema)]| {
			Schema::Record(
				fields
					.iter()
					.map(|(n, s)| (n.to_string(), s.clone()))
					.collect(),
			)
		};

		// Same shape → same fingerprint (recomputed independently).
		let a = rec(&[("age", Schema::Int), ("name", Schema::String)]);
		let b = rec(&[("age", Schema::Int), ("name", Schema::String)]);
		assert_eq!(fingerprint(&a), fingerprint(&b));

		// A retyped field drifts.
		let retyped = rec(&[("age", Schema::Float), ("name", Schema::String)]);
		assert_ne!(fingerprint(&a), fingerprint(&retyped));

		// A renamed field drifts (names ride the schema).
		let renamed = rec(&[("age", Schema::Int), ("nick", Schema::String)]);
		assert_ne!(fingerprint(&a), fingerprint(&renamed));

		// An added field drifts.
		let extra = rec(&[
			("age", Schema::Int),
			("name", Schema::String),
			("admin", Schema::Bool),
		]);
		assert_ne!(fingerprint(&a), fingerprint(&extra));

		// Distinct primitives don't collide.
		assert_ne!(fingerprint(&Schema::Int), fingerprint(&Schema::Float));
		// A field-set can't run together: {ab,c} vs {a,bc} (length-prefixing).
		let abc = rec(&[("ab", Schema::Bool), ("c", Schema::Bool)]);
		let a_bc = rec(&[("a", Schema::Bool), ("bc", Schema::Bool)]);
		assert_ne!(fingerprint(&abc), fingerprint(&a_bc));

		// Enum drift: an added variant changes the fingerprint.
		let e1 = Schema::Enum {
			qualified: "m.status".to_string(),
			variants: vec![("active".to_string(), vec![])],
		};
		let e2 = Schema::Enum {
			qualified: "m.status".to_string(),
			variants: vec![
				("active".to_string(), vec![]),
				("banned".to_string(), vec![Schema::Int]),
			],
		};
		assert_ne!(fingerprint(&e1), fingerprint(&e2));
	}

	#[test]
	fn schema_from_value_parses_tree() {
		// s-list (s-int)
		let v = variant(
			"core.wire.wire-schema",
			"s-list",
			vec![variant("core.wire.wire-schema", "s-int", vec![])],
		);
		assert_eq!(
			schema_from_value(&v),
			Some(Schema::List(Box::new(Schema::Int)))
		);
	}
}
