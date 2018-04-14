import link from '../../src/compiler/linker';
import generate from '../../src/compiler/generator';
import path from 'path';

const sourceDirectory = path.resolve(__dirname, './fixtures');

describe('generate', () => {
  test('basic examples', () => {
    const linkedAsts = link({
      entry: path.resolve(sourceDirectory, 'HelloWorld.plum')
    });

    const generatedJs = generate({ asts: linkedAsts });
    expect(generatedJs).toBeDefined();
    console.log(generatedJs);

    let evalError;
    try {
      eval(generatedJs);
    } catch (err) {
      evalError = err;
      throw err;
    }
    expect(evalError).not.toBeDefined();
  });
});
