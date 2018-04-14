import link from '../../src/compiler/linker';
import generate from '../../src/compiler/generator';
import path from 'path';

const sourceDirectory = path.resolve(__dirname, './fixtures');

describe('generate', () => {
  describe('basic examples', () => {
    const linkedAst = link({
      entry: path.resolve(sourceDirectory, 'HelloWorld.plum')
    });

    const js = generate({ ast: linkedAst });
    console.log(js);

    try {
      eval(js);
    } catch (err) {
      console.error(err);
    }
  });
});
