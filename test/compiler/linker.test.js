import link from '../../src/compiler/linker';
import path from 'path';

const sourceDirectory = path.resolve(__dirname, './fixtures');

describe('linker', () => {
  test('a single file with no imports', () => {
    const linkedAst = link({
      entry: path.resolve(sourceDirectory, 'Main.plum')
    });
  });

  test('multiple files with imports', () => {
    const linkedAst = link({
      entry: path.resolve(sourceDirectory, 'Importing.plum')
    });
  });
});
