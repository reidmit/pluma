import tokenize from './tokenizer';
import parse from './parser';
import LinkerError from './linker-error';
import topologicalSort from './topological-sort';
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

function resolveName(sourceDirectory, filePath) {
  filePath = filePath.replace(sourceDirectory, '');
  const parsedPath = path.parse(filePath);

  const pathParts = parsedPath.dir.split(path.sep).filter(Boolean);

  pathParts.push(parsedPath.name);
  return pathParts.join('.');
}

function collectAsts(
  sourceDirectory,
  filePath,
  filePathsToAsts,
  dependencyGraph,
  moduleNamesToFilePaths
) {
  if (filePathsToAsts[filePath]) return;

  const ast = fileToAst(filePath);
  const resolvedName = resolveName(sourceDirectory, filePath);
  const entryExports =
    ast.exports && ast.exports.length
      ? `module$${resolvedName.replace('.', '$')}`
      : null;
  ast.moduleName = resolvedName;

  filePathsToAsts[filePath] = ast;
  dependencyGraph[resolvedName] = dependencyGraph[resolvedName] || new Set();
  moduleNamesToFilePaths[resolvedName] = filePath;

  ast.imports.forEach(importNode => {
    const importedFilePath = importedModuleToFilePath(
      sourceDirectory,
      importNode.module
    );

    const resolvedImportName = resolveName(sourceDirectory, importedFilePath);
    dependencyGraph[resolvedName].add(resolvedImportName);

    collectAsts(
      sourceDirectory,
      importedFilePath,
      filePathsToAsts,
      dependencyGraph,
      moduleNamesToFilePaths
    );
  });

  return { filePathsToAsts, entryExports };
}

function link(options = {}) {
  if (!options.entry) {
    fail('No entry file given.');
  }

  if (!options.sourceDirectory) {
    options.sourceDirectory = path.dirname(options.entry);
  }

  const dependencyGraph = {};
  const moduleNamesToFilePaths = {};
  const { filePathsToAsts, entryExports } = collectAsts(
    options.sourceDirectory,
    options.entry,
    {},
    dependencyGraph,
    moduleNamesToFilePaths
  );

  const sortedModules = topologicalSort(dependencyGraph, cycles => {
    fail(
      `Cyclical imports detected. The following cycles were found in the module dependency graph:\n\n${cycles
        .map(line => '    ' + line)
        .join('\n')}`
    );
  });

  return {
    asts: sortedModules
      .map(moduleName => moduleNamesToFilePaths[moduleName])
      .map(filePath => filePathsToAsts[filePath]),
    entryExports
  };
}

export default link;