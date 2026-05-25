import * as vscode from "vscode";
import {
	type Executable,
	LanguageClient,
	type LanguageClientOptions,
	type LogMessageParams,
	MessageType,
	type ServerOptions,
} from "vscode-languageclient/node";

let client: LanguageClient;

export async function activate(context: vscode.ExtensionContext) {
	const log = vscode.window.createOutputChannel("Pluma language server");

	log.appendLine("Activated Pluma extension!");

	const command = process.env.SERVER_PATH || "pluma-language-server";

	log.appendLine(`Using language server at ${command}`);

	const runOptions: Executable = {
		command,
		options: {
			env: {
				...process.env,
				RUST_LOG: "debug",
			},
		},
	};

	const serverOptions: ServerOptions = {
		run: runOptions,
		debug: runOptions,
	};

	const clientOptions: LanguageClientOptions = {
		documentSelector: [{ scheme: "file", language: "pluma" }],
		traceOutputChannel: log,
	};

	client = new LanguageClient("pluma", "Pluma", serverOptions, clientOptions);

	await client.start();

	log.appendLine("Started language client");

	context.subscriptions.push(
		client.onNotification("window/logMessage", (data: LogMessageParams) => {
			const prefix = {
				[MessageType.Error]: "[server:error] ",
				[MessageType.Warning]: "[server:warning] ",
				[MessageType.Info]: "[server:info] ",
				[MessageType.Log]: "[server:log] ",
				[MessageType.Debug]: "[server:debug] ",
			};

			log.appendLine(prefix[data.type] + data.message);
		}),
	);

	vscode.workspace.onDidChangeTextDocument(
		(evt) => {
			if (evt.document.languageId !== "pluma") {
				return;
			}

			log.appendLine(evt.document.fileName);
		},
		null,
		context.subscriptions,
	);
}

export function deactivate(): Thenable<void> | undefined {
	if (!client) {
		return;
	}

	return client.stop();
}
