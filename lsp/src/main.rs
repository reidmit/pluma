use dashmap::DashMap;
use semantic_tokens::collect_semantic_tokens;
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
				text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
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

		let text = match self.document_map.get(&uri) {
			Some(text) => text.clone(),
			None => return Ok(None),
		};

		let tokens = collect_semantic_tokens(&text.into_bytes());

		if let Some(semantic_tokens) = tokens {
			for t in &semantic_tokens {
				self
					.client
					.log_message(MessageType::LOG, format!("{:#?}", t))
					.await
			}

			return Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
				result_id: None,
				data: semantic_tokens,
			})));
		}

		Ok(None)
	}
}

impl Backend {
	async fn on_document_change(&self, params: TextDocumentItem) {
		self
			.document_map
			.insert(params.uri.to_string(), params.text.clone());
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
