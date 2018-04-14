import link from '../../src/compiler/linker';
import tokenize from '../../src/compiler/tokenizer';
import parse from '../../src/compiler/parser';
import generate from '../../src/compiler/generator';
import path from 'path';

const sourceDirectory = path.resolve(__dirname, './fixtures');

function expectJsFromSource(source) {
  const tokens = tokenize({ source });
  const ast = parse({ source, tokens });

  return expectJs([ast]);
}

function expectJsFromFile(entryFile) {
  const { asts, entryExports } = link({
    entry: path.resolve(sourceDirectory, entryFile)
  });

  return expectJs(asts, entryExports);
}

function expectJs(asts, entryExports) {
  const js = generate({ asts, entryExports });
  expect(js).toBeDefined();

  let evalError;
  try {
    eval(js);
  } catch (err) {
    evalError = err;
    console.log(js); //eslint-disable-line
    throw err;
  }
  expect(evalError).not.toBeDefined();

  return js;
}

describe('generate', () => {
  test('empty module', () => {
    const js = expectJsFromSource('module TestModule');
    expect(js).toBe(`(function(global, factory) {
  typeof exports === 'object' && typeof module !== 'undefined' ? module.exports = factory() :
  typeof define === 'function' && define.amd ? define(factory) :
  (global.Pluma = factory());
})(this, function() { 'use strict';

var module$TestModule = (function() {
    return {};
})();
});`);
  });

  test('module with exports from entry point', () => {
    const js = expectJsFromFile('Exporting.plum');
    expect(js).toContain('return module$Exporting;');
  });

  test('lots of simple examples', () => {
    expectJsFromFile('HelloWorld.plum');
  });
});
