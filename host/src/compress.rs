// `std/compress` — gzip and brotli, the two codecs behind HTTP `Content-Encoding`.
// Thin wrappers over `flate2` (pure-Rust DEFLATE) and `brotli` (pure-Rust), vendored per
// the stdlib heavy-lifting rule rather than reimplemented in boxed-value wasm. Encoding
// can't fail (the sink is an in-memory `Vec`); decoding can, on malformed input, so its
// `Err` carries the message the Pluma side surfaces as `result.err`.
//
// Engine-independent (no V8); the marshalling glue is `v8host/compress.rs`. v1 runs
// synchronously on the scheduler thread — fine for typical HTTP bodies; offloading large
// ones to the blocking pool is a later refinement.

use std::io::{Read, Write};

pub fn gzip_encode(src: &[u8]) -> Result<Vec<u8>, String> {
	let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
	enc.write_all(src).map_err(|e| e.to_string())?;
	enc.finish().map_err(|e| e.to_string())
}

pub fn gzip_decode(src: &[u8]) -> Result<Vec<u8>, String> {
	let mut out = Vec::new();
	flate2::read::GzDecoder::new(src)
		.read_to_end(&mut out)
		.map_err(|e| format!("gzip decode: {e}"))?;
	Ok(out)
}

pub fn brotli_encode(src: &[u8]) -> Result<Vec<u8>, String> {
	// quality 5 / lgwin 22: a balanced default (brotli's own CLI default is 11, but that's
	// slow; 4–6 is the usual on-the-fly web-server range). `into_inner` finalises the stream.
	let mut enc = brotli::CompressorWriter::new(Vec::new(), 4096, 5, 22);
	enc.write_all(src).map_err(|e| e.to_string())?;
	Ok(enc.into_inner())
}

pub fn brotli_decode(src: &[u8]) -> Result<Vec<u8>, String> {
	let mut out = Vec::new();
	brotli::Decompressor::new(src, 4096)
		.read_to_end(&mut out)
		.map_err(|e| format!("brotli decode: {e}"))?;
	Ok(out)
}
