// SHA-1, for the WebSocket handshake (RFC 6455 `Sec-WebSocket-Accept`). A thin wrapper over
// the `sha1` crate, vendored rather than reimplemented in boxed-value wasm. It exists here
// only for the handshake's accept-key digest — not as a general-purpose hash (SHA-1 is not
// collision-resistant), which is why no `std` module exposes it.
//
// Engine-independent (no V8); the marshalling glue is `v8host/crypto.rs`. It rides the same
// read-marshalling ABI as the `std/compress` codecs.

use sha1::{Digest, Sha1};

/// SHA-1 digest of `src` (20 bytes). Infallible, but returns `Result` to share the codec
/// callback shape with the compress builtins; the `Err` arm is unreachable.
pub fn sha1(src: &[u8]) -> Result<Vec<u8>, String> {
	let mut hasher = Sha1::new();
	hasher.update(src);
	Ok(hasher.finalize().to_vec())
}
