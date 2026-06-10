use compiler::{Diagnostic as PlumaDiagnostic, DiagnosticKind};
use dashmap::DashMap;
use hover::HoverHit;
use semantic_tokens::collect_semantic_tokens;
use std::sync::Arc;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

mod analysis;
mod goto;
mod hover;
mod semantic_tokens;
mod symbols;

struct Backend {
	client: Client,
	document_map: DashMap<String, String>,
	// Per-URI hover index, rebuilt on each successful analysis. We can't
	// store the analyzed `Module` directly: it carries `Rc<RefCell<_>>`
	// dispatch cells, which aren't `Send` — DashMap values need to be.
	// The hover index is a precomputed `Vec<HoverHit>` of Send-only data
	// (`Range` + `Type`), which is what hover actually needs anyway.
	hover_map: DashMap<String, Arc<Vec<HoverHit>>>,
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
	async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
		Ok(InitializeResult {
			server_info: None,
			offset_encoding: None,
			capabilities: ServerCapabilities {
				text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
				workspace: None,
				// Plain options, not RegistrationOptions: the latter carries a
				// `document_selector` keyed on the LSP languageId, which differs
				// per editor (VS Code sends "pluma", Zed sends "Pluma"). A
				// mismatched selector silently suppresses all token requests —
				// which is why highlighting worked in VS Code but not Zed. The
				// plain options have no selector, so the client requests tokens
				// for whatever documents it routes to this server.
				semantic_tokens_provider: Some(SemanticTokensServerCapabilities::SemanticTokensOptions(
					SemanticTokensOptions {
						work_done_progress_options: WorkDoneProgressOptions::default(),
						legend: SemanticTokensLegend {
							token_types: semantic_tokens::TOKEN_TYPES.into(),
							token_modifiers: vec![],
						},
						range: Some(false),
						full: Some(SemanticTokensFullOptions::Bool(true)),
					},
				)),
				document_formatting_provider: Some(OneOf::Left(true)),
				hover_provider: Some(HoverProviderCapability::Simple(true)),
				definition_provider: Some(OneOf::Left(true)),
				document_symbol_provider: Some(OneOf::Left(true)),
				..ServerCapabilities::default()
			},
		})
	}

	async fn shutdown(&self) -> Result<()> {
		Ok(())
	}

	async fn did_open(&self, params: DidOpenTextDocumentParams) {
		let uri = params.text_document.uri.clone();
		self
			.on_document_change(TextDocumentItem {
				language_id: "pluma".into(),
				uri: params.text_document.uri,
				text: params.text_document.text,
				version: params.text_document.version,
			})
			.await;
		self.refresh_analysis(uri).await;
	}

	async fn did_change(&self, mut params: DidChangeTextDocumentParams) {
		let uri = params.text_document.uri.clone();
		self
			.on_document_change(TextDocumentItem {
				language_id: "pluma".into(),
				uri: params.text_document.uri,
				text: std::mem::take(&mut params.content_changes[0].text),
				version: params.text_document.version,
			})
			.await;
		self.refresh_analysis(uri).await;
	}

	async fn did_save(&self, _: DidSaveTextDocumentParams) {}

	async fn did_close(&self, params: DidCloseTextDocumentParams) {
		let uri_str = params.text_document.uri.to_string();
		self.document_map.remove(&uri_str);
		self.hover_map.remove(&uri_str);
		// Clear diagnostics for the closed file so VS Code doesn't leave
		// stale squiggles in the problems panel.
		self
			.client
			.publish_diagnostics(params.text_document.uri, vec![], None)
			.await;
	}

	async fn semantic_tokens_full(
		&self,
		params: SemanticTokensParams,
	) -> Result<Option<SemanticTokensResult>> {
		let uri = params.text_document.uri.to_string();
		let text = match self.document_map.get(&uri) {
			Some(text) => text.clone(),
			None => return Ok(None),
		};

		let semantic_tokens = collect_semantic_tokens(&text.into_bytes());

		Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
			result_id: None,
			data: semantic_tokens,
		})))
	}

	async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
		let uri = params.text_document.uri.to_string();

		let text = match self.document_map.get(&uri) {
			Some(text) => text.clone(),
			None => return Ok(None),
		};

		let formatted = match formatter::format_source(text.as_bytes()) {
			Ok(s) => s,
			Err(_) => {
				// Parse errors block formatting — surface them via diagnostics
				// later; for now, signal "no edits" so VS Code doesn't replace
				// the user's text with a partial format.
				return Ok(None);
			}
		};

		if formatted == text {
			return Ok(Some(vec![]));
		}

		Ok(Some(vec![TextEdit {
			range: full_document_range(&text),
			new_text: formatted,
		}]))
	}

	async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
		let uri = params
			.text_document_position_params
			.text_document
			.uri
			.to_string();
		let pos = params.text_document_position_params.position;

		let hits = match self.hover_map.get(&uri) {
			Some(h) => h.clone(),
			None => return Ok(None),
		};

		let Some(hit) = hover::lookup(&hits, pos.line, pos.character) else {
			return Ok(None);
		};

		// The doc resolves through the usage to its definition, so it needs the
		// source text and path; the direct hit's own doc (def name) is the
		// fallback.
		let doc = match (
			self.document_map.get(&uri).map(|t| t.clone()),
			params
				.text_document_position_params
				.text_document
				.uri
				.to_file_path()
				.ok(),
		) {
			(Some(text), Some(path)) => {
				hover::doc_for_hover(&hits, text.as_bytes(), &path, pos.line, pos.character)
			}
			_ => hit.doc.clone(),
		};

		// Type in a code fence; the doc comment (if any) as prose below a rule.
		let mut value = String::new();
		if !matches!(hit.ty, compiler::types::Type::Unknown) {
			value.push_str(&format!("```pluma\n{}\n```", hit.ty));
		}
		if let Some(doc) = &doc {
			if !value.is_empty() {
				value.push_str("\n\n---\n\n");
			}
			value.push_str(doc);
		}
		if value.is_empty() {
			return Ok(None);
		}

		Ok(Some(Hover {
			contents: HoverContents::Markup(MarkupContent {
				kind: MarkupKind::Markdown,
				value,
			}),
			range: Some(pluma_range_to_lsp(&hit.range)),
		}))
	}

	async fn goto_definition(
		&self,
		params: GotoDefinitionParams,
	) -> Result<Option<GotoDefinitionResponse>> {
		let uri = params.text_document_position_params.text_document.uri;
		let pos = params.text_document_position_params.position;

		let text = match self.document_map.get(&uri.to_string()) {
			Some(text) => text.clone(),
			None => return Ok(None),
		};
		// A non-file URI can't anchor cross-module lookups; same-file
		// resolution still works with an empty path.
		let path = uri.to_file_path().unwrap_or_default();

		let location = match goto::goto_definition(text.as_bytes(), &path, pos.line, pos.character) {
			Some(goto::Target::Here(range)) => Location {
				uri,
				range: pluma_range_to_lsp(&range),
			},
			Some(goto::Target::OtherFile { path, range }) => {
				let Ok(target_uri) = Url::from_file_path(&path) else {
					return Ok(None);
				};
				Location {
					uri: target_uri,
					range: pluma_range_to_lsp(&range),
				}
			}
			None => return Ok(None),
		};

		Ok(Some(GotoDefinitionResponse::Scalar(location)))
	}

	async fn document_symbol(
		&self,
		params: DocumentSymbolParams,
	) -> Result<Option<DocumentSymbolResponse>> {
		let text = match self.document_map.get(&params.text_document.uri.to_string()) {
			Some(text) => text.clone(),
			None => return Ok(None),
		};

		let symbols = symbols::document_symbols(text.as_bytes());
		Ok(Some(DocumentSymbolResponse::Nested(symbols)))
	}
}

impl Backend {
	async fn on_document_change(&self, params: TextDocumentItem) {
		self
			.document_map
			.insert(params.uri.to_string(), params.text.clone());
	}

	// Run the analyzer against the current in-memory text, cache the
	// resulting hover index, and publish any diagnostics back to the
	// client.
	async fn refresh_analysis(&self, uri: Url) {
		let uri_str = uri.to_string();
		let Some(text) = self.document_map.get(&uri_str).map(|s| s.clone()) else {
			return;
		};
		let Ok(path) = uri.to_file_path() else {
			return;
		};

		// Run analysis in a sync block and consume `AnalysisResult` fully
		// before any await — `Module` carries `Rc<RefCell<_>>` dispatch
		// cells, so it isn't `Send` and can't be held across the publish
		// await. We extract everything we need (the hover index + a
		// pre-converted LSP diagnostic list) into Send-only values.
		let source = text.into_bytes();
		let lsp_diags: Vec<Diagnostic> = {
			let result = analysis::analyze_document(&path, source.clone());

			let module_name = result.module.as_ref().map(|m| m.module_name.clone());
			if let Some(module) = result.module.as_ref() {
				self
					.hover_map
					.insert(uri_str.clone(), Arc::new(hover::build_index(module)));
			}

			let mut diags: Vec<Diagnostic> = result
				.diagnostics
				.iter()
				.filter(|d| diagnostic_belongs_to(d, module_name.as_deref()))
				.map(|d| pluma_diagnostic_to_lsp(d, &uri))
				.collect();

			// Lint warnings are computed from the same in-memory text. They only
			// exist when the module parses; on a parse error the linter returns
			// `Err`, and the analysis diagnostics above already carry the parse
			// errors, so we just skip linting.
			if let Ok(warnings) = linter::lint_source(&source) {
				diags.extend(warnings.iter().map(|d| pluma_diagnostic_to_lsp(d, &uri)));
			}

			diags
		};

		self.client.publish_diagnostics(uri, lsp_diags, None).await;
	}
}

fn diagnostic_belongs_to(d: &PlumaDiagnostic, module_name: Option<&str>) -> bool {
	match (&d.module_name, module_name) {
		(Some(diag_mod), Some(active_mod)) => diag_mod == active_mod,
		// Diagnostics without a module (e.g. top-level "no such file")
		// flow back to whatever module the user is currently editing.
		(None, _) => true,
		_ => false,
	}
}

fn pluma_diagnostic_to_lsp(d: &PlumaDiagnostic, uri: &Url) -> Diagnostic {
	let severity = match d.kind {
		DiagnosticKind::Error => DiagnosticSeverity::ERROR,
		DiagnosticKind::Warning => DiagnosticSeverity::WARNING,
	};
	let range = d
		.range
		.map(|r| pluma_range_to_lsp(&r))
		.unwrap_or_else(|| Range {
			start: Position {
				line: 0,
				character: 0,
			},
			end: Position {
				line: 0,
				character: 0,
			},
		});

	// Fold the structured help/notes into the message so they show in clients
	// that only render the message text.
	let mut message = d.message.clone();
	if let Some(help) = &d.help {
		message.push_str(&format!("\nhelp: {}", help));
	}
	for note in &d.notes {
		message.push_str(&format!("\nnote: {}", note));
	}

	// Secondary spans (e.g. "previous definition here") become related
	// information, which clients surface as clickable cross-references.
	let related_information = if d.labels.is_empty() {
		None
	} else {
		Some(
			d.labels
				.iter()
				.map(|label| DiagnosticRelatedInformation {
					location: Location {
						uri: uri.clone(),
						range: pluma_range_to_lsp(&label.range),
					},
					message: label.message.clone(),
				})
				.collect(),
		)
	};

	Diagnostic {
		range,
		severity: Some(severity),
		code: d.code.map(|c| NumberOrString::String(c.to_string())),
		code_description: None,
		source: Some("pluma".to_string()),
		message,
		related_information,
		tags: None,
		data: None,
	}
}

fn pluma_range_to_lsp(r: &compiler::Range) -> Range {
	Range {
		start: Position {
			line: r.start.line as u32,
			character: r.start.col as u32,
		},
		end: Position {
			line: r.end.line as u32,
			character: r.end.col as u32,
		},
	}
}

// LSP positions are in UTF-16 code units. For Pluma source — overwhelmingly
// ASCII — chars and code units match for everything outside string/comment
// content; over-estimating the end column is fine since LSP clients clamp it.
fn full_document_range(text: &str) -> Range {
	let mut last_line = 0u32;
	let mut last_col = 0u32;
	for ch in text.chars() {
		if ch == '\n' {
			last_line += 1;
			last_col = 0;
		} else {
			last_col += ch.len_utf16() as u32;
		}
	}
	Range {
		start: Position {
			line: 0,
			character: 0,
		},
		end: Position {
			line: last_line,
			character: last_col,
		},
	}
}

/// Run the Pluma language server over stdio until the client disconnects. This
/// is the entry point for `pluma language-server`; it owns its own Tokio runtime
/// so the (synchronous) CLI can call it directly.
pub fn run() {
	let runtime = tokio::runtime::Runtime::new().expect("failed to start the async runtime");
	runtime.block_on(serve());
}

async fn serve() {
	let stdin = tokio::io::stdin();
	let stdout = tokio::io::stdout();

	let (service, socket) = LspService::build(|client| Backend {
		client,
		document_map: DashMap::new(),
		hover_map: DashMap::new(),
	})
	.finish();

	Server::new(stdin, stdout, socket).serve(service).await;
}
