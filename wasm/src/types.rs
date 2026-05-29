// The WasmGC type section: the `$value` rec group plus its scalar/heap subtypes,
// and arity-keyed function types.
//
// The bytecode VM is uniformly boxed; the WASM backend instead gives `int`→i64,
// `float`→f64, `bool`→i32 *locals* and represents every *boxed* (`Repr::Boxed`)
// value as a GC reference to a `$value` subtype. The `$value` supertype carries
// an `i32` discriminant tag (mirroring `vm::Value`'s variants) so polymorphic
// code and tag-inspecting ops (`Match`, structural `Eq`, host-side `print`
// formatting) can read the runtime kind off any reference.
//
// Tag values are a cross-cutting contract: the emitter writes them, the host
// `print`/`debug` glue reads them to format a value. Keep `tag` in sync with the
// host formatter (see the wasm differential harness).

use wasm_encoder::{
	AbstractHeapType, CompositeInnerType, CompositeType, FieldType, HeapType, RefType, StorageType,
	StructType, SubType, TypeSection, ValType,
};

// --------------------------------------------------------------------------
// Concrete type indices. The scalar/heap subtypes occupy a fixed prefix; the
// arity-keyed function types follow, assigned by `FuncTypes`.
// --------------------------------------------------------------------------

pub const T_VALUE: u32 = 0; // struct { i32 tag }                 — the boxed supertype
pub const T_BYTES: u32 = 1; // array (mut i8)                     — UTF-8 / raw bytes backing
pub const T_INT: u32 = 2; // struct { i32 tag, i64 }
pub const T_FLOAT: u32 = 3; // struct { i32 tag, f64 }
pub const T_BOOL: u32 = 4; // struct { i32 tag, i32 }
pub const T_STR: u32 = 5; // struct { i32 tag, (ref $bytes) }
pub const T_VALARRAY: u32 = 6; // array (mut (ref null $value))   — captures / payload backing
pub const T_CLOSURE: u32 = 7; // struct { i32 tag, i32 fn_index, (ref $valarray) captures }
pub const T_VARIANT: u32 = 8; // struct { i32 tag, i32 vtag, (ref $str) name, (ref $valarray) payload }
pub const T_CTOR: u32 = 9; // struct { i32 tag, i32 vtag, i32 arity }  — a partial variant ctor
pub const T_METHODDICT: u32 = 10; // struct { i32 tag, (ref $valarray) methods }
pub const T_TUPLE: u32 = 11; // struct { i32 tag, (ref $valarray) elems }
pub const T_LIST: u32 = 12; // struct { i32 tag, (ref $valarray) elems }
pub const T_RECORD: u32 = 13; // struct { i32 tag, (ref $valarray) names, (ref $valarray) values }
const T_FIRST_FUNC: u32 = 14;

// --------------------------------------------------------------------------
// Runtime tags carried in the `$value` discriminant field. Mirror `vm::Value`'s
// variants; the host formatter switches on these.
// --------------------------------------------------------------------------

pub const TAG_NOTHING: i32 = 0;
pub const TAG_BOOL: i32 = 1;
pub const TAG_INT: i32 = 2;
pub const TAG_FLOAT: i32 = 3;
pub const TAG_STR: i32 = 4;
pub const TAG_DURATION: i32 = 5;
#[allow(dead_code)] // part of the tag contract; emitted once instants are boxed
pub const TAG_INSTANT: i32 = 6;
pub const TAG_CLOSURE: i32 = 7;
pub const TAG_VARIANT: i32 = 8;
pub const TAG_CTOR: i32 = 9;
pub const TAG_METHODDICT: i32 = 10;
pub const TAG_TUPLE: i32 = 11;
pub const TAG_LIST: i32 = 12;
pub const TAG_RECORD: i32 = 13;

/// `(ref null $valarray)` — a reference to a value array (closure captures or
/// variant payload).
pub fn valarray_ref() -> ValType {
	ValType::Ref(RefType {
		nullable: false,
		heap_type: HeapType::Concrete(T_VALARRAY),
	})
}

/// `(ref null $value)` — the uniform boxed-value type used for params, results,
/// captures, and every `Boxed` local.
pub fn value_ref() -> ValType {
	ValType::Ref(RefType {
		nullable: true,
		heap_type: HeapType::Concrete(T_VALUE),
	})
}

/// `(ref $bytes)` — a non-null reference to the byte-array backing of a string.
pub fn bytes_ref() -> ValType {
	ValType::Ref(RefType {
		nullable: false,
		heap_type: HeapType::Concrete(T_BYTES),
	})
}

/// `anyref` — the abstract top of the GC reference hierarchy. Host imports take
/// their boxed args as `anyref` (the wasm caller passes a `(ref null $value)`,
/// a valid subtype) so the host glue need not name the module's concrete types.
pub fn any_ref() -> ValType {
	ValType::Ref(RefType {
		nullable: true,
		heap_type: HeapType::Abstract {
			shared: false,
			ty: AbstractHeapType::Any,
		},
	})
}

fn val_field(t: ValType, mutable: bool) -> FieldType {
	FieldType {
		element_type: StorageType::Val(t),
		mutable,
	}
}

fn struct_subtype(super_idx: Option<u32>, fields: Vec<FieldType>, is_final: bool) -> SubType {
	SubType {
		is_final,
		supertype_idx: super_idx,
		composite_type: CompositeType {
			inner: CompositeInnerType::Struct(StructType {
				fields: fields.into_boxed_slice(),
			}),
			shared: false,
			descriptor: None,
			describes: None,
		},
	}
}

/// Arity-keyed function-type interner. In the uniform-boxed contract every
/// function takes `n` boxed params and returns one boxed value, so a function's
/// wasm type is fully determined by its arity. (Monomorphization will later vary
/// this; that's a follow-on.)
/// One interned function type: a Pluma function (boxed params + boxed result) or
/// a host import (`anyref` params, optional boxed result).
#[derive(PartialEq, Eq, Hash, Clone, Copy)]
enum FuncKind {
	Pluma(usize),
	Host(usize, bool),
	/// The structural-equality runtime helper: `(value, value) -> i32`.
	Eq,
	/// A runtime helper taking `n` boxed args and returning a boxed value.
	Helper(usize),
	/// The array-concat helper: `(valarray, valarray) -> valarray`.
	ArrConcat,
	/// The bytes-concat helper: `(bytes, bytes) -> bytes`.
	BytesConcat,
	/// The float-format host import: `(f64, anyref /*$bytes buf*/) -> i32 len`.
	FloatToStr,
}

pub struct FuncTypes {
	keys: std::collections::HashMap<FuncKind, u32>,
	pending: Vec<FuncKind>,
}

impl FuncTypes {
	pub fn new() -> Self {
		Self {
			keys: std::collections::HashMap::new(),
			pending: Vec::new(),
		}
	}

	fn intern(&mut self, k: FuncKind) -> u32 {
		if let Some(&i) = self.keys.get(&k) {
			return i;
		}
		let idx = T_FIRST_FUNC + self.pending.len() as u32;
		self.keys.insert(k, idx);
		self.pending.push(k);
		idx
	}

	/// The type index for a Pluma function of the given arity (boxed in/out).
	pub fn for_arity(&mut self, arity: usize) -> u32 {
		self.intern(FuncKind::Pluma(arity))
	}

	/// The type index for a host import taking `arity` `anyref` args and either
	/// returning a boxed value (`returns_value`) or nothing.
	pub fn for_host(&mut self, arity: usize, returns_value: bool) -> u32 {
		self.intern(FuncKind::Host(arity, returns_value))
	}

	/// The type index for the structural-equality helper `(value, value) -> i32`.
	pub fn for_eq(&mut self) -> u32 {
		self.intern(FuncKind::Eq)
	}

	/// The type index for a runtime helper: `n` boxed args -> boxed value.
	pub fn for_helper(&mut self, n: usize) -> u32 {
		self.intern(FuncKind::Helper(n))
	}

	/// The type index for the array-concat helper: `(valarray, valarray) -> valarray`.
	pub fn for_arrconcat(&mut self) -> u32 {
		self.intern(FuncKind::ArrConcat)
	}

	/// The type index for the bytes-concat helper: `(bytes, bytes) -> bytes`.
	pub fn for_bytesconcat(&mut self) -> u32 {
		self.intern(FuncKind::BytesConcat)
	}

	/// The type index for the float-format host import: `(f64, anyref) -> i32`.
	pub fn for_float_to_str(&mut self) -> u32 {
		self.intern(FuncKind::FloatToStr)
	}

	/// Encode the full type section: the fixed `$value` prefix, then every
	/// interned function type in index order.
	pub fn encode(&self) -> TypeSection {
		let mut types = TypeSection::new();
		// 0: $value — the open, subtypeable boxed supertype.
		types.ty().subtype(&struct_subtype(
			None,
			vec![val_field(ValType::I32, false)],
			false,
		));
		// 1: $bytes — array (mut i8).
		types.ty().subtype(&SubType {
			is_final: true,
			supertype_idx: None,
			composite_type: CompositeType {
				inner: CompositeInnerType::Array(wasm_encoder::ArrayType(FieldType {
					element_type: StorageType::I8,
					mutable: true,
				})),
				shared: false,
				descriptor: None,
				describes: None,
			},
		});
		// 2..6: scalar/heap subtypes of $value.
		let scalar = |payload: ValType| {
			struct_subtype(
				Some(T_VALUE),
				vec![val_field(ValType::I32, false), val_field(payload, false)],
				true,
			)
		};
		types.ty().subtype(&scalar(ValType::I64)); // 2 $int
		types.ty().subtype(&scalar(ValType::F64)); // 3 $float
		types.ty().subtype(&scalar(ValType::I32)); // 4 $bool
		types.ty().subtype(&scalar(bytes_ref())); // 5 $str
																						// 6 $valarray — array (mut (ref null $value)).
		types.ty().subtype(&SubType {
			is_final: true,
			supertype_idx: None,
			composite_type: CompositeType {
				inner: CompositeInnerType::Array(wasm_encoder::ArrayType(val_field(value_ref(), true))),
				shared: false,
				descriptor: None,
				describes: None,
			},
		});
		// 7 $closure — { tag, i32 fn_index, (ref $valarray) captures }.
		types.ty().subtype(&struct_subtype(
			Some(T_VALUE),
			vec![
				val_field(ValType::I32, false),
				val_field(ValType::I32, false),
				val_field(valarray_ref(), false),
			],
			true,
		));
		// 8 $variant — { tag, i32 variant_tag, (ref $str) display-name, (ref $valarray) payload }.
		types.ty().subtype(&struct_subtype(
			Some(T_VALUE),
			vec![
				val_field(ValType::I32, false),
				val_field(ValType::I32, false),
				val_field(value_ref(), false),
				val_field(valarray_ref(), false),
			],
			true,
		));
		// 9 $ctor — a partial variant constructor: { tag, i32 variant_tag, i32 arity }.
		types.ty().subtype(&struct_subtype(
			Some(T_VALUE),
			vec![
				val_field(ValType::I32, false),
				val_field(ValType::I32, false),
				val_field(ValType::I32, false),
			],
			true,
		));
		// 10 $methoddict — { tag, (ref $valarray) methods } (positional method values).
		types.ty().subtype(&struct_subtype(
			Some(T_VALUE),
			vec![
				val_field(ValType::I32, false),
				val_field(valarray_ref(), false),
			],
			true,
		));
		// 11 $tuple / 12 $list — { tag, (ref $valarray) elems }.
		let elems_struct = || {
			struct_subtype(
				Some(T_VALUE),
				vec![
					val_field(ValType::I32, false),
					val_field(valarray_ref(), false),
				],
				true,
			)
		};
		types.ty().subtype(&elems_struct());
		types.ty().subtype(&elems_struct());
		// 13 $record — { tag, (ref $valarray) names, (ref $valarray) values } (name-sorted).
		types.ty().subtype(&struct_subtype(
			Some(T_VALUE),
			vec![
				val_field(ValType::I32, false),
				val_field(valarray_ref(), false),
				val_field(valarray_ref(), false),
			],
			true,
		));
		// Interned function types, in index order. A Pluma function takes an
		// implicit closure-environment param first (`env`, the `$closure` ref or
		// null for a capture-free direct call), then its `arity` boxed params.
		for k in &self.pending {
			let (param_ty, count, results): (ValType, usize, Vec<ValType>) = match *k {
				FuncKind::Pluma(arity) => (value_ref(), arity + 1, vec![value_ref()]),
				FuncKind::Host(arity, returns_value) => (
					any_ref(),
					arity,
					if returns_value {
						vec![value_ref()]
					} else {
						vec![]
					},
				),
				FuncKind::Eq => (value_ref(), 2, vec![ValType::I32]),
				FuncKind::Helper(n) => (value_ref(), n, vec![value_ref()]),
				FuncKind::ArrConcat => (valarray_ref(), 2, vec![valarray_ref()]),
				FuncKind::BytesConcat => (bytes_ref(), 2, vec![bytes_ref()]),
				// Heterogeneous params — built directly below rather than via `param_ty`.
				FuncKind::FloatToStr => {
					types
						.ty()
						.function([ValType::F64, any_ref()], [ValType::I32]);
					continue;
				}
			};
			let params = std::iter::repeat(param_ty).take(count);
			types.ty().function(params, results);
		}
		types
	}
}
