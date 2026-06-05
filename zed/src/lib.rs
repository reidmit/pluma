use zed_extension_api::settings::LspSettings;
use zed_extension_api::{self as zed, LanguageServerId, Result};

// The language server is a subcommand of the `pluma` CLI (`pluma language-server`),
// built by `cargo build` / installed by `cargo install --path cli` from the repo.
const SERVER_BINARY: &str = "pluma";
const SERVER_SUBCOMMAND: &str = "language-server";

struct PlumaExtension;

impl zed::Extension for PlumaExtension {
	fn new() -> Self {
		PlumaExtension
	}

	fn language_server_command(
		&mut self,
		language_server_id: &LanguageServerId,
		worktree: &zed::Worktree,
	) -> Result<zed::Command> {
		// Resolve the binary in priority order:
		//   1. An explicit path from the user's settings.json, under
		//      `"lsp": { "pluma": { "binary": { "path", "arguments" } } }`.
		//      This is the recommended dev loop — point it at
		//      `<repo>/target/debug/pluma` and just rebuild.
		//   2. `pluma` on PATH (e.g. after `cargo install --path cli`).
		let binary = LspSettings::for_worktree(language_server_id.as_ref(), worktree)
			.ok()
			.and_then(|settings| settings.binary);

		let configured_args = binary.as_ref().and_then(|b| b.arguments.clone());

		let command = match binary.and_then(|b| b.path) {
			Some(path) => path,
			None => worktree.which(SERVER_BINARY).ok_or_else(|| {
				format!(
					"`{SERVER_BINARY}` not found on PATH. Install it with \
					 `cargo install --path cli` from the Pluma repo, or set \
					 `\"lsp\": {{ \"pluma\": {{ \"binary\": {{ \"path\": \"…\" }} }} }}` \
					 in your Zed settings to point at a built binary, e.g. \
					 `<repo>/target/debug/{SERVER_BINARY}`."
				)
			})?,
		};

		// `pluma language-server` — the subcommand is the first argument, with any
		// user-configured arguments appended.
		let mut args = vec![SERVER_SUBCOMMAND.to_string()];
		args.extend(configured_args.unwrap_or_default());

		Ok(zed::Command {
			command,
			args,
			env: worktree.shell_env(),
		})
	}
}

zed::register_extension!(PlumaExtension);
