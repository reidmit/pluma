// The `wire` binary codec (FULLSTACK.md, Layer 1).
//
// A `wire a` dictionary is reified at runtime as a *schema descriptor* — a
// compact tree mirroring `a`'s static structure. The `wire-encode` /
// `wire-decode` builtins interpret that schema to turn values into a tight
// positional binary encoding and back. The schema is what makes decode
// possible: a byte buffer doesn't know whether it's an `option int` or a
// record, so the target type has to drive the parse.
//
// Format (see FULLSTACK.md "Format"):
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

use crate::value::{Value, VariantData};
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

/// Encode `value` per `schema`, appending to `out`. The value is assumed to
/// match the schema (the type checker guarantees it); a mismatch is a compiler
/// bug, caught by `debug_assert`/`unreachable` in debug builds.
pub fn encode(schema: &Schema, value: &Value, out: &mut Vec<u8>) {
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
			write_uvarint(out, xs.len() as u64);
			for x in xs.iter() {
				encode(inner, x, out);
			}
		}
		(Schema::Tuple(schemas), Value::Tuple(xs)) => {
			debug_assert_eq!(schemas.len(), xs.len(), "tuple arity vs schema");
			for (s, x) in schemas.iter().zip(xs.iter()) {
				encode(s, x, out);
			}
		}
		(Schema::Record(fields), Value::Record(map)) => {
			for (name, s) in fields {
				let v = map
					.get(name)
					.unwrap_or_else(|| unreachable!("record missing field `{}`", name));
				encode(s, v, out);
			}
		}
		(Schema::Enum { variants, .. }, Value::Variant(v)) => {
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
				encode(s, x, out);
			}
		}
		(s, v) => unreachable!("wire encode: schema {:?} vs value {}", s, v),
	}
}

// ---- decode ---------------------------------------------------------------

/// Decode one `schema`-shaped value from the cursor, advancing it.
pub fn decode(schema: &Schema, cur: &mut &[u8]) -> Result<Value, WireError> {
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
				xs.push(decode(inner, cur)?);
			}
			Ok(Value::List(Rc::new(xs)))
		}
		Schema::Tuple(schemas) => {
			let mut xs = Vec::with_capacity(schemas.len());
			for s in schemas {
				xs.push(decode(s, cur)?);
			}
			Ok(Value::Tuple(Rc::new(xs)))
		}
		Schema::Record(fields) => {
			let mut map = HashMap::with_capacity(fields.len());
			for (name, s) in fields {
				map.insert(name.clone(), decode(s, cur)?);
			}
			Ok(Value::Record(Rc::new(map)))
		}
		Schema::Enum {
			qualified,
			variants,
		} => {
			let tag = read_uvarint(cur)?;
			let idx = usize::try_from(tag).ok().filter(|i| *i < variants.len());
			let idx = idx.ok_or(WireError::InvalidTag(tag as i64))?;
			let (name, field_schemas) = &variants[idx];
			let mut payload = Vec::with_capacity(field_schemas.len());
			for s in field_schemas {
				payload.push(decode(s, cur)?);
			}
			Ok(Value::Variant(Rc::new(VariantData {
				qualified_enum: Rc::new(qualified.clone()),
				variant: Rc::new(name.clone()),
				payload,
			})))
		}
	}
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
	let value = decode(schema, &mut cur)?;
	if !cur.is_empty() {
		return Err(WireError::TrailingBytes(cur.len() as i64));
	}
	Ok(value)
}

// ---- schema fingerprint ---------------------------------------------------
//
// A stable structural hash of a schema (FULLSTACK.md "Version skew"). Two
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

fn as_list(v: &Value) -> Option<&[Value]> {
	match v {
		Value::List(xs) => Some(xs),
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
		round_trip(&s, &Value::List(Rc::new(vec![int(1), int(2), int(-3)])));
		round_trip(&s, &Value::List(Rc::new(vec![])));
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
		round_trip(
			&s,
			&Value::List(Rc::new(vec![some(1, "a"), none, some(2, "b")])),
		);
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
