use dashmap::DashMap;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

mod semantic_tokens;

#[derive(Debug)]
struct Backend {
	client: Client,
	document_map: DashMap<String, String>,
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
	async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
		Ok(InitializeResult {
			server_info: None,
			offset_encoding: None,
			capabilities: ServerCapabilities {
				inlay_hint_provider: None,
				text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
				completion_provider: None,
				execute_command_provider: None,
				workspace: None,
				semantic_tokens_provider: Some(
					SemanticTokensServerCapabilities::SemanticTokensRegistrationOptions(
						SemanticTokensRegistrationOptions {
							text_document_registration_options: {
								TextDocumentRegistrationOptions {
									document_selector: Some(vec![DocumentFilter {
										language: Some("pluma".to_string()),
										scheme: Some("file".to_string()),
										pattern: None,
									}]),
								}
							},
							semantic_tokens_options: SemanticTokensOptions {
								work_done_progress_options: WorkDoneProgressOptions::default(),
								legend: SemanticTokensLegend {
									token_types: semantic_tokens::TOKEN_TYPES.into(),
									token_modifiers: vec![],
								},
								range: Some(false),
								full: Some(SemanticTokensFullOptions::Bool(true)),
							},
							static_registration_options: StaticRegistrationOptions::default(),
						},
					),
				),
				// definition_provider: Some(OneOf::Left(true)),
				// references_provider: Some(OneOf::Left(true)),
				// rename_provider: Some(OneOf::Left(true)),
				..ServerCapabilities::default()
			},
		})
	}

	async fn shutdown(&self) -> Result<()> {
		Ok(())
	}

	async fn did_open(&self, params: DidOpenTextDocumentParams) {
		self
			.client
			.log_message(MessageType::INFO, "file opened!")
			.await;

		self
			.on_document_change(TextDocumentItem {
				language_id: "pluma".into(),
				uri: params.text_document.uri,
				text: params.text_document.text,
				version: params.text_document.version,
			})
			.await
	}

	async fn did_change(&self, mut params: DidChangeTextDocumentParams) {
		self
			.on_document_change(TextDocumentItem {
				language_id: "pluma".into(),
				uri: params.text_document.uri,
				text: std::mem::take(&mut params.content_changes[0].text),
				version: params.text_document.version,
			})
			.await
	}

	async fn did_save(&self, _: DidSaveTextDocumentParams) {
		self
			.client
			.log_message(MessageType::INFO, "file saved!")
			.await;
	}

	async fn did_close(&self, _: DidCloseTextDocumentParams) {
		self
			.client
			.log_message(MessageType::INFO, "file closed!")
			.await;
	}

	async fn semantic_tokens_full(
		&self,
		params: SemanticTokensParams,
	) -> Result<Option<SemanticTokensResult>> {
		let uri = params.text_document.uri.to_string();

		self
			.client
			.log_message(MessageType::LOG, "semantic_token_full")
			.await;

		// let semantic_tokens = || -> Option<Vec<SemanticToken>> {
		// 	let mut im_complete_tokens = self.semantic_token_map.get_mut(&uri)?;
		// 	let rope = self.document_map.get(&uri)?;
		// 	let ast = self.ast_map.get(&uri)?;
		// 	let extends_tokens = semantic_token_from_ast(&ast);
		// 	im_complete_tokens.extend(extends_tokens);
		// 	im_complete_tokens.sort_by(|a, b| a.start.cmp(&b.start));
		// 	let mut pre_line = 0;
		// 	let mut pre_start = 0;
		// 	let semantic_tokens = im_complete_tokens
		// 		.iter()
		// 		.filter_map(|token| {
		// 			let line = rope.try_byte_to_line(token.start).ok()? as u32;
		// 			let first = rope.try_line_to_char(line as usize).ok()? as u32;
		// 			let start = rope.try_byte_to_char(token.start).ok()? as u32 - first;
		// 			let delta_line = line - pre_line;
		// 			let delta_start = if delta_line == 0 {
		// 				start - pre_start
		// 			} else {
		// 				start
		// 			};
		// 			let ret = Some(SemanticToken {
		// 				delta_line,
		// 				delta_start,
		// 				length: token.length as u32,
		// 				token_type: token.token_type as u32,
		// 				token_modifiers_bitset: 0,
		// 			});
		// 			pre_line = line;
		// 			pre_start = start;
		// 			ret
		// 		})
		// 		.collect::<Vec<_>>();
		// 	Some(semantic_tokens)
		// }();

		// if let Some(semantic_token) = semantic_tokens {
		// 	return Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
		// 		result_id: None,
		// 		data: semantic_token,
		// 	})));
		// }

		Ok(None)
	}

	// async fn semantic_tokens_range(
	// 	&self,
	// 	params: SemanticTokensRangeParams,
	// ) -> Result<Option<SemanticTokensRangeResult>> {
	// 	let uri = params.text_document.uri.to_string();
	// 	let semantic_tokens = || -> Option<Vec<SemanticToken>> {
	// 		let im_complete_tokens = self.semantic_token_map.get(&uri)?;
	// 		let rope = self.document_map.get(&uri)?;
	// 		let mut pre_line = 0;
	// 		let mut pre_start = 0;
	// 		let semantic_tokens = im_complete_tokens
	// 			.iter()
	// 			.filter_map(|token| {
	// 				let line = rope.try_byte_to_line(token.start).ok()? as u32;
	// 				let first = rope.try_line_to_char(line as usize).ok()? as u32;
	// 				let start = rope.try_byte_to_char(token.start).ok()? as u32 - first;
	// 				let ret = Some(SemanticToken {
	// 					delta_line: line - pre_line,
	// 					delta_start: if start >= pre_start {
	// 						start - pre_start
	// 					} else {
	// 						start
	// 					},
	// 					length: token.length as u32,
	// 					token_type: token.token_type as u32,
	// 					token_modifiers_bitset: 0,
	// 				});
	// 				pre_line = line;
	// 				pre_start = start;
	// 				ret
	// 			})
	// 			.collect::<Vec<_>>();
	// 		Some(semantic_tokens)
	// 	}();

	// 	if let Some(semantic_token) = semantic_tokens {
	// 		return Ok(Some(SemanticTokensRangeResult::Tokens(SemanticTokens {
	// 			result_id: None,
	// 			data: semantic_token,
	// 		})));
	// 	}

	// 	Ok(None)
	// }
}

impl Backend {
	async fn on_document_change(&self, params: TextDocumentItem) {
		self
			.document_map
			.insert(params.uri.to_string(), params.text.clone());

		// let c = compiler::
	}
}

#[tokio::main]
async fn main() {
	let stdin = tokio::io::stdin();
	let stdout = tokio::io::stdout();

	let (service, socket) = LspService::build(|client| Backend {
		client,
		document_map: DashMap::new(),
	})
	.finish();

	Server::new(stdin, stdout, socket).serve(service).await;
}
