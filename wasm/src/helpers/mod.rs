// Synthetic runtime-helper builders. Each `build_*_fn` emits one self-contained
// `wasm_encoder::Function` over the GC `$value` layout — the inline-WASM routines
// the assembler appends after the IR functions (only those a reachable program
// needs). They take each other's already-resolved wasm indices, never call each
// other at the Rust level, so they live one-domain-per-file.

mod bytes;
mod dict;
mod eq;
mod list;
mod record;
mod tostring;
mod wire;
mod wrapper;

pub(crate) use bytes::*;
pub(crate) use dict::*;
pub(crate) use eq::*;
pub(crate) use list::*;
pub(crate) use record::*;
pub(crate) use tostring::*;
pub(crate) use wire::*;
pub(crate) use wrapper::*;
