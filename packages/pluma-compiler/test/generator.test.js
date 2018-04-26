import link from '../src/linker';
import tokenize from '../src/tokenizer';
import parse from '../src/parser';
import generate from '../src/generator';
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
    throw err;
  }
  expect(evalError).not.toBeDefined();

  return js;
}

describe('generate', () => {
  test('empty module', () => {
    const js = expectJsFromSource('module TestModule');
    expect(js).toMatchSnapshot();
  });

  test('module with exports from entry point', () => {
    const js = expectJsFromFile('Exporting.plum');
    expect(js).toMatchSnapshot();
  });

  test('lots of simple examples', () => {
    const js = expectJsFromFile('HelloWorld.plum');
    expect(js).toMatchSnapshot();
  });

  test('type declarations', () => {
    const js = expectJsFromFile('Types.plum');
    expect(js).toMatchSnapshot();
  });
});