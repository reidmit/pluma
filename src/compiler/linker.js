import tokenize from './tokenizer';
import parse from './parser';
import LinkerError from './linker-error';
import fs from 'fs';
import path from 'path';

function fail(message, fileName) {
  throw new LinkerError(message, fileName);
}

function fileToAst(filePath) {
  let source;

  try {
    source = fs.readFileSync(filePath, 'UTF-8');
  } catch (err) {
    fail(err.message, filePath);
  }

  const tokens = tokenize({ source });
  return parse({ tokens, source });
}

function importedModuleToFilePath(sourceDirectory, node) {
  if (node.kind === 'Identifier') {
    return path.resolve(sourceDirectory, node.value + '.plum');
  }

  return path.resolve(
    sourceDirectory,
    path.join(
      ...node.parts.map(
        (part, i) => part.value + (i === node.parts.length - 1 ? '.plum' : '')
      )
    )
  );
}

function resolveName(sourceDirectory, filePath, nameNode) {
  if (!filePath && nameNode) return nameNode.value;

  filePath = filePath.replace(sourceDirectory, '');
  const parsedPath = path.parse(filePath);

  const pathParts = parsedPath.dir.split(path.sep).filter(Boolean);

  pathParts.push(parsedPath.name);
  return pathParts.join('.');
}

function resolveImports(sourceDirectory, ast, currentFile, filesSeen) {
  return ast.imports.map(importNode => {
    const importedFilePath = importedModuleToFilePath(
      sourceDirectory,
      importNode.module
    );

    const resolvedName = resolveName(sourceDirectory, importedFilePath);

    if (filesSeen[importedFilePath]) {
      const previousImportingFile =
        filesSeen[importedFilePath] === true
          ? 'entry point'
          : `'${filesSeen[importedFilePath]}'`;

      fail(
        `Circular dependency detected. Module '${resolvedName}' was already imported by ${previousImportingFile}.`,
        currentFile
      );
    }

    filesSeen[importedFilePath] = resolvedName;

    const importedAst = fileToAst(importedFilePath);
    importedAst.resolvedName = resolvedName;
    importedAst.resolvedImports = resolveImports(
      sourceDirectory,
      importedAst,
      importedFilePath,
      filesSeen
    );

    return importedAst;
  });
}

function link(options = {}) {
  let { entry, sourceDirectory } = options;

  if (!entry) {
    fail('No entry file given.');
  }

  if (!sourceDirectory) {
    sourceDirectory = path.dirname(entry);
  }

  const filesSeen = { [entry]: true };
  const ast = fileToAst(entry);
  ast.resolvedName = resolveName(sourceDirectory, null, ast.name);
  ast.resolvedImports = resolveImports(sourceDirectory, ast, entry, filesSeen);

  return ast;
}

export default link;
