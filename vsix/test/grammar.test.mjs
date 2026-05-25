// Regression tests for the Pluma TextMate grammar (syntaxes/pluma.tmLanguage.json).
//
// These run the grammar through vscode-textmate + vscode-oniguruma — the exact
// engine VS Code uses — so what we assert here is what VS Code will render.
//
// The grammar is a *second* definition of Pluma's syntax (the real one lives in
// compiler/src/tokenizer.rs). It has no compiler to keep it honest, so it drifts:
// this file exists to catch that. Each case below pins a construct that has
// broken before or is easy to break. The broad smoke test at the end tokenizes
// every tests/run fixture so a catastrophic regression can't slip through.
//
// Run with `npm test` (from vsix/) or `just test-grammar` (from the repo root).

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";
import vsctm from "vscode-textmate";
import oniguruma from "vscode-oniguruma";

const { Registry, parseRawGrammar, INITIAL } = vsctm;
const { loadWASM, OnigScanner, OnigString } = oniguruma;

const require = createRequire(import.meta.url);
const grammarPath = fileURLToPath(
	new URL("../syntaxes/pluma.tmLanguage.json", import.meta.url),
);
const fixturesDir = fileURLToPath(new URL("../../tests/run", import.meta.url));

// vscode-oniguruma ships a wasm blob; load it once before building the registry.
await loadWASM(readFileSync(require.resolve("vscode-oniguruma/release/onig.wasm")).buffer);

const registry = new Registry({
	onigLib: Promise.resolve({
		createOnigScanner: (patterns) => new OnigScanner(patterns),
		createOnigString: (s) => new OnigString(s),
	}),
	loadGrammar: async (scopeName) =>
		scopeName === "source.pluma"
			? parseRawGrammar(readFileSync(grammarPath, "utf8"), grammarPath)
			: null,
});

const grammar = await registry.loadGrammar("source.pluma");

// Tokenize source into a flat list of { text, scopes } across all lines.
function tokenize(source) {
	let ruleStack = INITIAL;
	const out = [];
	for (const line of source.split("\n")) {
		const { tokens, ruleStack: next } = grammar.tokenizeLine(line, ruleStack);
		for (const t of tokens) {
			out.push({ text: line.slice(t.startIndex, t.endIndex), scopes: t.scopes });
		}
		ruleStack = next;
	}
	return out;
}

// The scope stack of the first token whose text is exactly `text`.
function scopesOf(source, text) {
	const tok = tokenize(source).find((t) => t.text === text);
	assert.ok(tok, `no token exactly equal to ${JSON.stringify(text)} in ${JSON.stringify(source)}`);
	return tok.scopes;
}

const hasScope = (scopes, name) => scopes.includes(name);

test("regex literals are backtick-delimited (not /.../)", () => {
	const src = "def r = `^ \"hi\" $`";
	const toks = tokenize(src);
	const ticks = toks.filter((t) => t.text === "`");
	assert.equal(ticks.length, 2, "expected an opening and closing backtick");
	assert.ok(hasScope(ticks[0].scopes, "punctuation.definition.regex.begin.pluma"));
	// The string atom inside the regex sits under the regexp scope.
	const body = toks.find((t) => t.text === "hi");
	assert.ok(body && hasScope(body.scopes, "string.regexp.pluma"), "regex body should sit under string.regexp");
});

test("`/` outside a regex is division, not a regex start", () => {
	// The old grammar started a regex on `/`; make sure a/b and a / b are arithmetic.
	assert.ok(hasScope(scopesOf("def q = a / b", "/"), "keyword.operator.arithmetic.pluma"));
	assert.ok(!scopesOf("def q = a / b", "/").some((s) => s.startsWith("string.regexp")));
});

test("dict is a builtin type in type position", () => {
	assert.ok(hasScope(scopesOf("let m :: dict int = e", "dict"), "support.type.builtin.pluma"));
	// ...and `map` is no longer a recognized builtin (it was renamed to dict).
	assert.ok(!scopesOf("let m :: map int = e", "map").some((s) => s.startsWith("support.type")));
});

test("dict in call position is a namespace, not a type", () => {
	assert.ok(hasScope(scopesOf("def e = dict.empty ()", "dict"), "entity.name.namespace.pluma"));
});

test("kebab-case identifiers are one token, not keyword/operator soup", () => {
	// `is` must not be picked out of `this-is-it`.
	const scopes = scopesOf("def x = this-is-it", "this-is-it");
	assert.ok(hasScope(scopes, "variable.other.pluma"));
	assert.ok(!scopes.some((s) => s.startsWith("keyword")));

	// `to-string` is a single identifier (the `-` is not subtraction).
	const toks = tokenize("def y = to-string n");
	assert.ok(toks.some((t) => t.text === "to-string"));
	assert.ok(!toks.some((t) => t.text === "-"));
});

test("spaced minus is subtraction; unspaced hyphen is part of an identifier", () => {
	assert.ok(hasScope(scopesOf("def d = a - b", "-"), "keyword.operator.arithmetic.pluma"));
	assert.ok(!tokenize("def d = a-b").some((t) => t.text === "-"));
});

test("negative numeric literals are still numbers", () => {
	// The unary minus must not swallow the digit's number scope (regression).
	assert.ok(hasScope(scopesOf("def n = print -1", "1"), "constant.numeric.decimal.pluma"));
});

test("bytes literals and their escapes", () => {
	const src = "def b = '\\x89PNG'";
	const toks = tokenize(src);
	assert.ok(toks.some((t) => t.scopes.includes("string.quoted.single.pluma")));
	assert.ok(toks.some((t) => t.text === "\\x89" && t.scopes.includes("constant.character.escape.pluma")));
});

test("++ is the concat operator, distinct from +", () => {
	assert.ok(hasScope(scopesOf('def c = a ++ b', "++"), "keyword.operator.concat.pluma"));
	assert.ok(hasScope(scopesOf("def c = a + b", "+"), "keyword.operator.arithmetic.pluma"));
});

test("keywords the old grammar was missing", () => {
	assert.ok(hasScope(scopesOf("def f = try g", "try"), "keyword.control.pluma"));
	assert.ok(hasScope(scopesOf('test "name" { e }', "test"), "keyword.declaration.pluma"));
	assert.ok(hasScope(scopesOf('def f = built-in "x"', "built-in"), "keyword.other.builtin.pluma"));
});

test("string interpolation exposes the inner expression", () => {
	const src = 'def s = "hi $(name)"';
	const toks = tokenize(src);
	assert.ok(toks.some((t) => t.text === "$(" && t.scopes.includes("punctuation.section.interpolation.begin.pluma")));
	const name = toks.find((t) => t.text === "name");
	assert.ok(name && name.scopes.includes("variable.other.pluma"), "interpolated name should highlight as an identifier");
});

// Broad smoke test: every real fixture must tokenize without throwing, and no
// *word-bearing* token (identifier, keyword, number, literal) may fall through
// to the bare root scope. Structural punctuation ({}, (), commas, …) is left
// unscoped on purpose, so we ignore tokens with no alphanumeric content. Even
// the catch-all identifier rule should scope anything word-like, so a hit here
// means the grammar structurally broke on some construct — the net that catches
// drift no targeted case above happens to cover.
test("every tests/run fixture tokenizes cleanly", () => {
	const dirs = readdirSync(fixturesDir, { withFileTypes: true }).filter((d) => d.isDirectory());
	assert.ok(dirs.length > 0, "expected to find run fixtures");
	for (const d of dirs) {
		let source;
		try {
			source = readFileSync(`${fixturesDir}/${d.name}/main.pa`, "utf8");
		} catch {
			continue; // some fixtures may not have a main.pa
		}
		const toks = tokenize(source);
		const unclassified = toks.filter(
			(t) => t.scopes.length === 1 && /[A-Za-z0-9]/.test(t.text),
		);
		assert.equal(
			unclassified.length,
			0,
			`${d.name}: ${unclassified.length} unclassified token(s), e.g. ${JSON.stringify(unclassified[0]?.text)}`,
		);
	}
});
