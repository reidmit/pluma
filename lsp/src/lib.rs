use compiler::{Diagnostic as PlumaDiagnostic, DiagnosticKind};
use dashmap::DashMap;
use hover::HoverHit;
use semantic_tokens::collect_semantic_tokens;
use std::sync::Arc;
use std::time::Duration;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

mod analysis;
mod completion;
mod goto;
mod hover;
mod inlay_hints;
mod semantic_tokens;
mod signature_help;
mod symbols;

// How long to wait for typing to pause before analyzing. A burst of
// keystrokes only triggers one analysis — the last one — instead of one per
// character. Short enough to feel instant, long enough to coalesce a fast
// typist's run of edits.
const ANALYSIS_DEBOUNCE: Duration = Duration::from_millis(150);

// Per-URI monotonic edit counter, shared across `Backend` clones. Each edit
// bumps a document's counter; a debounced analysis captures the value at
// schedule time and proceeds only while it's still the latest. That single
// predicate powers both debounce (a superseded edit's task bails before
// analyzing) and cancellation (an analysis whose result went stale mid-run
// bails before publishing).
#[derive(Clone, Default)]
struct Revisions(Arc<DashMap<String, u64>>);

impl Revisions {
	// Record a new edit and return the document's current revision.
	fn bump(&self, uri: &str) -> u64 {
		let mut entry = self.0.entry(uri.to_string()).or_insert(0);
		*entry += 1;
		*entry
	}

	// Whether `rev` is still the latest edit for `uri`.
	fn is_current(&self, uri: &str, rev: u64) -> bool {
		self.0.get(uri).map(|r| *r) == Some(rev)
	}

	// Forget a document (on close), so any pending analysis for it bails.
	fn forget(&self, uri: &str) {
		self.0.remove(uri);
	}
}

// Shared state is `Arc`-wrapped so the whole `Backend` clones cheaply into a
// spawned debounce task (which needs `'static`), while every clone still sees
// the same maps. The DashMaps themselves are concurrent, so reads/writes from
// request handlers and the analysis task don't need extra locking.
#[derive(Clone)]
struct Backend {
	client: Client,
	document_map: Arc<DashMap<String, String>>,
	// Per-URI hover index, rebuilt on each successful analysis. We can't
	// store the analyzed `Module` directly: it carries `Rc<RefCell<_>>`
	// dispatch cells, which aren't `Send` — DashMap values need to be.
	// The hover index is a precomputed `Vec<HoverHit>` of Send-only data
	// (`Range` + `Type`), which is what hover actually needs anyway.
	hover_map: Arc<DashMap<String, Arc<Vec<HoverHit>>>>,
	// Inferred-type inlay hints, rebuilt alongside the hover index from the
	// same analysis pass. Send-only (`position` + `String`) for the same
	// reason the hover index is.
	inlay_map: Arc<DashMap<String, Arc<Vec<inlay_hints::InlayHint>>>>,
	revisions: Revisions,
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
				// Quick-fixes built from the linter's autofixes: each fixable lint
				// at the cursor becomes a clickable edit.
				code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
				hover_provider: Some(HoverProviderCapability::Simple(true)),
				definition_provider: Some(OneOf::Left(true)),
				document_symbol_provider: Some(OneOf::Left(true)),
				inlay_hint_provider: Some(OneOf::Left(true)),
				completion_provider: Some(CompletionOptions {
					// `.` re-triggers for member access (`list.`); `/` drills into
					// the next segment of a `use` module path (`std/` → its
					// contents). The client also triggers on identifier characters.
					trigger_characters: Some(vec![".".to_string(), "/".to_string()]),
					..CompletionOptions::default()
				}),
				signature_help_provider: Some(SignatureHelpOptions {
					// `(` opens a parenthesized/interpolated call; ` ` advances to the
					// next argument in Pluma's paren-free application syntax. Both
					// also re-trigger while the popup is open so the active-parameter
					// highlight tracks each argument as it's typed.
					trigger_characters: Some(vec!["(".to_string(), " ".to_string()]),
					retrigger_characters: Some(vec![" ".to_string()]),
					work_done_progress_options: WorkDoneProgressOptions::default(),
				}),
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
		self.schedule_analysis(uri);
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
		self.schedule_analysis(uri);
	}

	async fn did_save(&self, _: DidSaveTextDocumentParams) {}

	async fn did_close(&self, params: DidCloseTextDocumentParams) {
		let uri_str = params.text_document.uri.to_string();
		self.document_map.remove(&uri_str);
		self.hover_map.remove(&uri_str);
		self.inlay_map.remove(&uri_str);
		// Drop the revision too: any debounced analysis still pending for this
		// file finds no matching revision and bails instead of re-publishing
		// diagnostics for a closed document.
		self.revisions.forget(&uri_str);
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

	async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
		// Only quick-fixes are offered here; if the client asked exclusively for
		// other kinds (e.g. a refactor or organize-imports request), there's
		// nothing to return.
		if let Some(only) = &params.context.only {
			if !only
				.iter()
				.any(|k| CodeActionKind::QUICKFIX.as_str().starts_with(k.as_str()))
			{
				return Ok(None);
			}
		}

		let uri = params.text_document.uri;
		let text = match self.document_map.get(&uri.to_string()) {
			Some(text) => text.clone(),
			None => return Ok(None),
		};

		// Re-run the linter against the current text to recover the structured
		// fix edits (the published diagnostics keep only the message). On a parse
		// error the linter returns `Err` and there are no fixes to offer.
		let Ok(findings) = linter::lint_findings(text.as_bytes()) else {
			return Ok(None);
		};

		let actions: Vec<CodeActionOrCommand> = findings
			.into_iter()
			.filter(|f| !f.fixes.is_empty())
			.filter_map(|f| {
				// Offer a finding's fix only when its diagnostic overlaps the range
				// the client asked about (the cursor or selection).
				let diag_range = pluma_range_to_lsp(&f.diagnostic.range?);
				if !ranges_overlap(&diag_range, &params.range) {
					return None;
				}

				let edits: Vec<TextEdit> = f
					.fixes
					.iter()
					.map(|fix| TextEdit {
						range: pluma_range_to_lsp(&fix.range),
						new_text: fix.replacement.clone(),
					})
					.collect();

				let mut changes = std::collections::HashMap::new();
				changes.insert(uri.clone(), edits);

				// The help line ("replace the wrapper with `f` directly") reads as
				// the action; fall back to the diagnostic message if a rule has none.
				let title = f
					.diagnostic
					.help
					.clone()
					.unwrap_or_else(|| f.diagnostic.message.clone());

				Some(CodeActionOrCommand::CodeAction(CodeAction {
					title,
					kind: Some(CodeActionKind::QUICKFIX),
					diagnostics: Some(vec![pluma_diagnostic_to_lsp(&f.diagnostic, &uri)]),
					edit: Some(WorkspaceEdit {
						changes: Some(changes),
						..WorkspaceEdit::default()
					}),
					..CodeAction::default()
				}))
			})
			.collect();

		Ok(Some(actions))
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

	async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
		let pos = params.text_document_position;
		let uri = pos.text_document.uri;

		let text = match self.document_map.get(&uri.to_string()) {
			Some(text) => text.clone(),
			None => return Ok(None),
		};
		// A non-file URI can't anchor cross-module lookups; in-file completion
		// (scope names, local enums) still works with an empty path.
		let path = uri.to_file_path().unwrap_or_default();

		let items: Vec<CompletionItem> = completion::complete(
			text.as_bytes(),
			&path,
			pos.position.line,
			pos.position.character,
		)
		.into_iter()
		.map(completion_to_lsp)
		.collect();

		Ok(Some(CompletionResponse::Array(items)))
	}

	async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
		let uri = params.text_document.uri.to_string();
		let hints = match self.inlay_map.get(&uri) {
			Some(h) => h.clone(),
			None => return Ok(None),
		};

		// The client asks for hints over the visible range; only return the
		// ones that fall inside it so off-screen hints aren't sent every scroll.
		let range = params.range;
		let result = hints
			.iter()
			.filter(|h| position_in_range(h.line, h.col, &range))
			.map(|h| InlayHint {
				position: Position {
					line: h.line,
					character: h.col,
				},
				label: InlayHintLabel::String(h.label.clone()),
				kind: Some(InlayHintKind::TYPE),
				text_edits: None,
				tooltip: None,
				// No left pad: the hint reads as `name: T`, sitting flush
				// against the binder like a written annotation.
				padding_left: Some(false),
				padding_right: Some(false),
				data: None,
			})
			.collect();

		Ok(Some(result))
	}

	async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
		let tdp = params.text_document_position_params;
		let uri = tdp.text_document.uri;
		let pos = tdp.position;

		let text = match self.document_map.get(&uri.to_string()) {
			Some(text) => text.clone(),
			None => return Ok(None),
		};
		// A non-file URI can't anchor cross-module resolution; bare local calls
		// still resolve with an empty path.
		let path = uri.to_file_path().unwrap_or_default();

		let Some(help) =
			signature_help::signature_help(text.as_bytes(), &path, pos.line, pos.character)
		else {
			return Ok(None);
		};
		Ok(Some(sig_help_to_lsp(help)))
	}
}

// Whether two LSP ranges share any position. Used to offer a lint's quick-fix
// only when its diagnostic touches the range the client asked about. Touching at
// a single boundary point counts (a zero-width cursor sitting at a span's edge).
fn ranges_overlap(a: &Range, b: &Range) -> bool {
	!(position_lt(&a.end, &b.start) || position_lt(&b.end, &a.start))
}

// Strict less-than on LSP positions: earlier line, or same line and earlier column.
fn position_lt(a: &Position, b: &Position) -> bool {
	a.line < b.line || (a.line == b.line && a.character < b.character)
}

fn position_in_range(line: u32, col: u32, range: &Range) -> bool {
	let after_start =
		line > range.start.line || (line == range.start.line && col >= range.start.character);
	let before_end = line < range.end.line || (line == range.end.line && col <= range.end.character);
	after_start && before_end
}

impl Backend {
	async fn on_document_change(&self, params: TextDocumentItem) {
		self
			.document_map
			.insert(params.uri.to_string(), params.text.clone());
	}

	// Record a new edit for `uri` and schedule a debounced analysis. Returns
	// immediately so a burst of keystrokes doesn't each block on a full
	// analysis; only the last edit in the burst actually runs (see
	// `is_current`). Spawns a detached task — hence the cheap `self.clone()`.
	fn schedule_analysis(&self, uri: Url) {
		let uri_str = uri.to_string();
		let rev = self.revisions.bump(&uri_str);
		let this = self.clone();
		tokio::spawn(async move {
			tokio::time::sleep(ANALYSIS_DEBOUNCE).await;
			// A newer edit landed during the debounce window — let its own
			// task do the analysis and skip this stale one.
			if !this.revisions.is_current(&uri_str, rev) {
				return;
			}
			this.refresh_analysis(uri, rev).await;
		});
	}

	// Run the analyzer against the current in-memory text, cache the resulting
	// hover/inlay indices, and publish diagnostics — unless a newer edit
	// superseded `rev` while the analysis ran, in which case the result is
	// dropped so the client never sees stale diagnostics.
	async fn refresh_analysis(&self, uri: Url, rev: u64) {
		let uri_str = uri.to_string();
		let Some(text) = self.document_map.get(&uri_str).map(|s| s.clone()) else {
			return;
		};
		let Ok(path) = uri.to_file_path() else {
			return;
		};

		// Run analysis in a sync block and consume `AnalysisResult` fully
		// before any await — `Module` carries `Rc<RefCell<_>>` dispatch cells,
		// so it isn't `Send` and can't be held across an await. We extract
		// everything we need (the hover/inlay indices + a pre-converted LSP
		// diagnostic list) into Send-only values.
		let source = text.into_bytes();
		let (hover_index, inlay_hints, lsp_diags): (
			Option<Arc<Vec<HoverHit>>>,
			Option<Arc<Vec<inlay_hints::InlayHint>>>,
			Vec<Diagnostic>,
		) = {
			let result = analysis::analyze_document(&path, source.clone());

			let module_name = result.module.as_ref().map(|m| m.module_name.clone());
			let hover_index = result
				.module
				.as_ref()
				.map(|m| Arc::new(hover::build_index(m)));
			let inlay_hints = result
				.module
				.as_ref()
				.map(|m| Arc::new(inlay_hints::build_hints(m)));

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

			(hover_index, inlay_hints, diags)
		};

		// Superseded by a newer edit while we were analyzing: drop the result
		// rather than overwrite fresh indices / publish stale diagnostics.
		if !self.revisions.is_current(&uri_str, rev) {
			return;
		}

		if let Some(index) = hover_index {
			self.hover_map.insert(uri_str.clone(), index);
		}
		if let Some(hints) = inlay_hints {
			self.inlay_map.insert(uri_str.clone(), hints);
		}

		self.client.publish_diagnostics(uri, lsp_diags, None).await;

		// The hint set was just rebuilt against the new text; ask the client to
		// re-pull so inferred types track edits without waiting for the next
		// incidental refresh. Ignored by clients that don't support it.
		self.client.inlay_hint_refresh().await.ok();
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

fn completion_to_lsp(c: completion::Completion) -> CompletionItem {
	use completion::CompletionKind;
	let kind = match c.kind {
		CompletionKind::Function => CompletionItemKind::FUNCTION,
		CompletionKind::Value => CompletionItemKind::CONSTANT,
		CompletionKind::EnumType => CompletionItemKind::ENUM,
		CompletionKind::Trait => CompletionItemKind::INTERFACE,
		CompletionKind::Alias => CompletionItemKind::CLASS,
		CompletionKind::Variant => CompletionItemKind::ENUM_MEMBER,
		CompletionKind::Field => CompletionItemKind::FIELD,
		CompletionKind::Module => CompletionItemKind::MODULE,
		CompletionKind::Folder => CompletionItemKind::FOLDER,
		CompletionKind::Keyword => CompletionItemKind::KEYWORD,
	};
	let text_edit = c.edit.map(|(range, new_text)| {
		CompletionTextEdit::Edit(TextEdit {
			range: pluma_range_to_lsp(&range),
			new_text,
		})
	});
	// A directory item re-opens completion on accept, so drilling through a
	// module path (`std/` → `sys/` → `process`) stays a single flow.
	let command = c.retrigger.then(|| Command {
		title: "Suggest".to_string(),
		command: "editor.action.triggerSuggest".to_string(),
		arguments: None,
	});
	CompletionItem {
		label: c.label,
		kind: Some(kind),
		detail: c.detail,
		documentation: c.doc.map(|d| {
			Documentation::MarkupContent(MarkupContent {
				kind: MarkupKind::Markdown,
				value: d,
			})
		}),
		filter_text: c.filter_text,
		text_edit,
		command,
		..CompletionItem::default()
	}
}

fn sig_help_to_lsp(help: signature_help::SigHelp) -> SignatureHelp {
	let parameters: Vec<ParameterInformation> = help
		.params
		.iter()
		.map(|(start, end)| ParameterInformation {
			// Offsets into the signature label, so the client highlights the
			// active parameter in place — the label is ASCII, so char offsets
			// equal the UTF-16 units the protocol wants.
			label: ParameterLabel::LabelOffsets([*start, *end]),
			documentation: None,
		})
		.collect();

	let documentation = help.doc.map(|d| {
		Documentation::MarkupContent(MarkupContent {
			kind: MarkupKind::Markdown,
			value: d,
		})
	});

	SignatureHelp {
		signatures: vec![SignatureInformation {
			label: help.label,
			documentation,
			parameters: Some(parameters),
			active_parameter: Some(help.active_param),
		}],
		active_signature: Some(0),
		active_parameter: Some(help.active_param),
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
///
/// The runtime is deliberately single-threaded. Analysis reuses per-thread
/// caches (the baked-in stdlib's exports and an incremental user-module
/// cache); those carry `Rc`-based, non-`Send` data, so they can't be shared
/// across threads. Pinning every task to one thread keeps the caches warm
/// across edits — a multi-threaded runtime would scatter analyses over workers
/// and rebuild the caches cold each time. An interactive editor is not a
/// throughput workload, so one thread is ample, and debouncing keeps analysis
/// off the critical path of fast requests like completion and hover.
pub fn run() {
	let runtime = tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()
		.expect("failed to start the async runtime");
	runtime.block_on(serve());
}

async fn serve() {
	let stdin = tokio::io::stdin();
	let stdout = tokio::io::stdout();

	let (service, socket) = LspService::build(|client| Backend {
		client,
		document_map: Arc::new(DashMap::new()),
		hover_map: Arc::new(DashMap::new()),
		inlay_map: Arc::new(DashMap::new()),
		revisions: Revisions::default(),
	})
	.finish();

	Server::new(stdin, stdout, socket).serve(service).await;
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::sync::atomic::{AtomicU32, Ordering};

	fn range(sl: u32, sc: u32, el: u32, ec: u32) -> Range {
		Range {
			start: Position {
				line: sl,
				character: sc,
			},
			end: Position {
				line: el,
				character: ec,
			},
		}
	}

	#[test]
	fn ranges_overlap_detects_touching_and_disjoint() {
		// A cursor (zero-width) sitting inside the span overlaps.
		assert!(ranges_overlap(&range(2, 0, 2, 10), &range(2, 4, 2, 4)));
		// Touching exactly at a boundary point counts.
		assert!(ranges_overlap(&range(2, 0, 2, 5), &range(2, 5, 2, 8)));
		// Fully disjoint on the same line does not.
		assert!(!ranges_overlap(&range(2, 0, 2, 4), &range(2, 6, 2, 9)));
		// Disjoint across lines does not.
		assert!(!ranges_overlap(&range(1, 0, 1, 9), &range(3, 0, 3, 9)));
		// Order-independent.
		assert!(ranges_overlap(&range(2, 5, 2, 8), &range(2, 0, 2, 6)));
	}

	#[test]
	fn revision_supersedes_and_forgets() {
		let revs = Revisions::default();
		let a = revs.bump("a");
		assert_eq!(a, 1);
		assert!(revs.is_current("a", 1));

		// A second edit supersedes the first: only the latest is current.
		let a2 = revs.bump("a");
		assert_eq!(a2, 2);
		assert!(!revs.is_current("a", 1), "the earlier revision is stale");
		assert!(revs.is_current("a", 2));

		// Documents are tracked independently.
		assert_eq!(revs.bump("b"), 1);
		assert!(revs.is_current("a", 2) && revs.is_current("b", 1));

		// Forgetting (on close) makes every pending revision stale.
		revs.forget("a");
		assert!(!revs.is_current("a", 2));
		assert!(
			revs.is_current("b", 1),
			"forgetting one document leaves others"
		);
	}

	// The debounce contract, exercised with the real `Revisions` + the same
	// sleep-then-recheck flow `schedule_analysis` uses: in a burst of edits,
	// every task but the last finds its revision superseded and skips the
	// (expensive) analysis. Drives wall-clock time via tokio's test clock so
	// it's deterministic and instant.
	#[tokio::test(start_paused = true)]
	async fn debounce_runs_only_the_last_edit_in_a_burst() {
		let revs = Revisions::default();
		let analyses = Arc::new(AtomicU32::new(0));

		// Five rapid edits, each 10ms apart — well inside the 150ms window.
		let mut tasks = Vec::new();
		for _ in 0..5 {
			let rev = revs.bump("doc");
			let revs = revs.clone();
			let analyses = analyses.clone();
			tasks.push(tokio::spawn(async move {
				tokio::time::sleep(ANALYSIS_DEBOUNCE).await;
				if revs.is_current("doc", rev) {
					analyses.fetch_add(1, Ordering::SeqCst);
				}
			}));
			tokio::time::sleep(Duration::from_millis(10)).await;
		}
		for t in tasks {
			t.await.unwrap();
		}

		assert_eq!(
			analyses.load(Ordering::SeqCst),
			1,
			"a burst of 5 edits should collapse to a single analysis"
		);
	}
}
