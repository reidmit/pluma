import { ParseError, CompileError } from './errors';
import { Token, TokenKind } from './tokens';
import * as fs from 'fs';
import * as path from 'path';
import * as util from 'util';
import { FILE_EXTENSION } from './constants';
import { IModuleNode } from './nodes/ModuleNode';
import { parse } from './parse';

const stat = util.promisify(fs.stat);
const readDir = util.promisify(fs.readdir);
const readFile = util.promisify(fs.readFile);

function filePathToModuleName(rootDir: string, absoluteFilePath: string) {
  return path
    .relative(rootDir, absoluteFilePath)
    .replace(/\//g, '.')
    .replace(new RegExp(FILE_EXTENSION + '$'), '');
}

class SourceModule {
  absoluteFilePath: string;
  moduleName?: string;
  source?: string;
  ast?: IModuleNode;

  static fromFilePath(rootDir: string, filePath: string) {
    const absoluteFilePath = path.resolve(rootDir, filePath);
    const moduleName = filePathToModuleName(rootDir, absoluteFilePath);
    return new SourceModule(absoluteFilePath, moduleName);
  }

  // static fromModuleName(rootDir: string, moduleName: string) {}

  constructor(absoluteFilePath: string, moduleName: string) {
    this.absoluteFilePath = absoluteFilePath;
  }

  async readAndParse() {
    try {
      this.source = await readFile(this.absoluteFilePath, 'utf8');
    } catch (err) {
      throw new CompileError(err.message, this.absoluteFilePath, this.moduleName || '');
    }
  }
}

export class Compiler {
  rootDir: string;
  modules: Map<string, SourceModule>;

  constructor(rootDir: string) {
    if (!fs.statSync(rootDir).isDirectory()) {
      throw new Error(`${rootDir} is not a directory`);
    }

    this.rootDir = rootDir;
    this.modules = new Map();
  }

  async addDefaultEntryModule() {
    const entryFilePath = path.resolve(this.rootDir, `Main${FILE_EXTENSION}`);
    this.addSourceFile(entryFilePath);
  }

  async addSourceFile(relativeFilePath: string) {
    const mod = SourceModule.fromFilePath(this.rootDir, relativeFilePath);
    await mod.readAndParse();
    this.modules.set(mod.absoluteFilePath, mod);
    console.log(this.modules);
  }
}
