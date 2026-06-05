# Pluma for Zed

A [Zed](https://zed.dev) extension for the Pluma language (`.pa` files).

It is deliberately thin: it does nothing but tell Zed how to launch
`pluma language-server`. Every feature you see in the editor — diagnostics,
hover types, formatting, and syntax highlighting — is served by that language
server over LSP, so the same work benefits any editor with an LSP client (Zed,
VS Code, Neovim, …) instead of being reimplemented per editor.

Highlighting in particular comes from the server's **semantic tokens** rather
than a Tree-sitter grammar, which is why this extension ships no grammar.

## Setup

### 1. Build the language server

The language server is the `pluma language-server` subcommand of the CLI. From
the repo root:

```sh
cargo build --bin pluma                          # -> target/debug/pluma
# or, to put it on your PATH:
cargo install --path cli
```

### 2. Install the extension into Zed

In Zed: open the command palette → **zed: install dev extension** → select this
`zed/` directory. Zed compiles the extension to wasm itself (you need the
`wasm32-wasip1` Rust target: `rustup target add wasm32-wasip1`).

### 3. Point Zed at the binary and enable LSP highlighting

Add to your Zed `settings.json` (command palette → **zed: open settings**):

```json
{
  "languages": {
    "Pluma": {
      // Drive highlighting from the language server's semantic tokens.
      // Without this, .pa files open unhighlighted (there is no grammar).
      "semantic_tokens": "full"
    }
  },
  "lsp": {
    "pluma": {
      "binary": {
        // Absolute path to the binary built in step 1. Skip this block
        // entirely if you ran `cargo install --path cli` (it's on PATH).
        // The extension appends the `language-server` subcommand for you.
        "path": "/absolute/path/to/pluma/target/debug/pluma"
      }
    }
  }
}
```

Pointing `binary.path` at `target/debug/pluma` is the smoothest dev loop:
rebuild the CLI, then reload the Zed window to pick it up.

## How the binary is located

`language_server_command` (in `src/lib.rs`) resolves the server in this order:

1. `lsp.pluma.binary.path` from your settings (with optional `binary.arguments`).
2. `pluma` on `PATH`.

Either way the extension invokes it as `pluma language-server`.

If neither resolves, Zed surfaces an error explaining both options.

## What you get

| Feature        | Source                                         |
| -------------- | ---------------------------------------------- |
| Diagnostics    | `textDocument/publishDiagnostics`              |
| Hover types    | `textDocument/hover`                           |
| Formatting     | `textDocument/formatting` (the `pluma` formatter) |
| Highlighting   | `textDocument/semanticTokens/full`             |

## Tuning highlight colors (optional)

Semantic token types map to your theme. To override specific ones, use
`global_lsp_settings.semantic_token_rules` in `settings.json` — e.g. the server
emits `namespace`, `enumMember`, `parameter`, `property`, and `type` tokens.
